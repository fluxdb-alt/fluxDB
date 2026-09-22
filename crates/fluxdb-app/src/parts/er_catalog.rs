// ER 元数据共享缓存与并发编排（er-design.md §3.4/§4.2）。
//
// 分层：本文件在 fluxdb-app（应用层）。AppController 持有 `Arc<Mutex<ErCatalogCache>>`，
// 桌面端通过 controller clone 调用这里的 `impl AppController` 方法完成字段按需加载、
// 关系索引复用与连接修订失效。desktop 不直接拼 SQL、不亲自维护这些缓存。
//
// 并发模型与 CompletionIndex 一致：网络查询在锁外，只在锁内做缓存读写；
// 同一 (连接修订, database, schema[, table]) 的进行中请求用 inflight 集合去重，
// 避免重复 DB 往返。

/// 连接修订：每处理一次连接变更（connect / UpdateConnection / 删除）对该 ConnectionId
/// 自增。缓存键带修订，旧连接结果天然失效，不误用同 ConnectionId 的旧图。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ErConnRev(pub u64);

/// 关系索引键：同一 (连接, 连接修订, database, schema) 的关系边由各 ER 视图共享复用；
/// 含连接与修订，连接变更后旧索引按修订不可再命中。
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ErRelationKey {
    connection_id: ConnectionId,
    conn_rev: u64,
    database: String,
    schema: Option<String>,
}

/// 字段缓存键：带足连接、连接修订与结构化表身份（database + schema + 裸名）。
/// 用 `ErTableRef` 而非展示名拆分，跨 schema 同名表、以及含 `.` 等合法字符的标识符
/// 都能被唯一区分，不从展示名反推 schema/表名（er-design §5.2 身份与名称分离）。
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ErColumnKey {
    connection_id: ConnectionId,
    conn_rev: u64,
    reference: ErTableRef,
}

/// ER 元数据共享缓存（controller 持有，跨标签复用）。
#[derive(Debug)]
pub struct ErCatalogCache {
    /// 连接修订映射：ConnectionId -> 当前修订。
    revisions: BTreeMap<ConnectionId, u64>,
    /// 关系索引：范围键 -> 全量外键边（节点名对齐）。
    relations: BTreeMap<ErRelationKey, Vec<ErForeignKeyEdge>>,
    relation_status: BTreeMap<ErRelationKey, ErLoadStatus>,
    relations_inflight: BTreeSet<ErRelationKey>,
    /// 字段缓存：表键 -> 已读取列（身份按原始规则保存，不统一转小写）。
    columns: BTreeMap<ErColumnKey, Vec<ErColumn>>,
    column_status: BTreeMap<ErColumnKey, ErLoadStatus>,
    columns_inflight: BTreeSet<ErColumnKey>,
    /// 进行中字段加载的取消旗标：key -> 该次在飞读取的取消信号（关闭/停止时置位）。
    columns_cancel: BTreeMap<ErColumnKey, ErCancelFlag>,
}

impl Default for ErCatalogCache {
    fn default() -> Self {
        Self {
            revisions: BTreeMap::new(),
            relations: BTreeMap::new(),
            relation_status: BTreeMap::new(),
            relations_inflight: BTreeSet::new(),
            columns: BTreeMap::new(),
            column_status: BTreeMap::new(),
            columns_inflight: BTreeSet::new(),
            columns_cancel: BTreeMap::new(),
        }
    }
}

impl ErCatalogCache {
    /// 当前连接修订（未登记则为 0）。
    pub fn revision(&self, connection_id: ConnectionId) -> u64 {
        self.revisions.get(&connection_id).copied().unwrap_or(0)
    }

    /// 连接变更：自增修订。旧修订键不再被接受（写回前校验修订），
    /// 同时清理该连接旧修订下的缓存，避免过期键无限增长。
    pub fn bump_revision(&mut self, connection_id: ConnectionId) -> u64 {
        let next = self.revision(connection_id) + 1;
        let new_rev = ErConnRev(next);
        self.revisions.insert(connection_id, next);
        self.retain_connection(connection_id, new_rev);
        next
    }

    /// 删除连接：移除其全部缓存、修订与在飞标记。
    pub fn invalidate_connection(&mut self, connection_id: ConnectionId) {
        self.revisions.remove(&connection_id);
        self.relations.retain(|k, _| k.connection_id != connection_id);
        self.relation_status.retain(|k, _| k.connection_id != connection_id);
        self.relations_inflight.retain(|k| k.connection_id != connection_id);
        self.columns.retain(|k, _| k.connection_id != connection_id);
        self.column_status.retain(|k, _| k.connection_id != connection_id);
        self.columns_inflight.retain(|k| k.connection_id != connection_id);
        self.columns_cancel.retain(|k, _| k.connection_id != connection_id);
    }

    /// 仅保留该连接的“当前修订”缓存（修订自增即旧修订键作废并清理）。
    fn retain_connection(&mut self, connection_id: ConnectionId, keep: ErConnRev) {
        self.relations
            .retain(|k, _| k.connection_id != connection_id || k.conn_rev == keep.0);
        self.relation_status
            .retain(|k, _| k.connection_id != connection_id || k.conn_rev == keep.0);
        self.relations_inflight
            .retain(|k| k.connection_id != connection_id || k.conn_rev == keep.0);
        self.columns
            .retain(|k, _| k.connection_id != connection_id || k.conn_rev == keep.0);
        self.column_status
            .retain(|k, _| k.connection_id != connection_id || k.conn_rev == keep.0);
        self.columns_inflight
            .retain(|k| k.connection_id != connection_id || k.conn_rev == keep.0);
        self.columns_cancel
            .retain(|k, _| k.connection_id != connection_id || k.conn_rev == keep.0);
    }

    fn relation_key(&self, database: &str, schema: Option<&str>, connection_id: ConnectionId) -> ErRelationKey {
        ErRelationKey {
            connection_id,
            conn_rev: self.revision(connection_id),
            database: database.to_string(),
            schema: schema.map(str::to_string),
        }
    }

    fn column_key(&self, reference: &ErTableRef, connection_id: ConnectionId) -> ErColumnKey {
        ErColumnKey {
            connection_id,
            conn_rev: self.revision(connection_id),
            reference: reference.clone(),
        }
    }
}

/// 关系索引读取结果：已缓存边 + 状态。
/// edges 用拥有型 Vec（非 Rc），便于跨后台线程（background_spawn）传递（Send）。
pub struct ErRelationSnapshot {
    pub edges: Vec<ErForeignKeyEdge>,
    pub status: ErLoadStatus,
}

/// 一次进行中加载的取消信号：桌面端「关闭标签 / 停止加载」时置位，加载线程据此
/// 在逐表之间停止后续工作，并在完成时丢弃该次结果（不写入缓存）。驱动是阻塞式
/// 单次调用时无法即时中断，但能停止默认逐表循环的后续工作并丢弃过期结果。
/// 同一 key 的多个并发加载方共享同一旗标，任一置位即整体取消。
type ErCancelFlag = Arc<AtomicBool>;

/// 字段按需加载的一次结果：每表列 + 状态。
#[derive(Debug)]
pub struct ErColumnBatch {
    /// (display_table_name, columns, status)；display 名与 ErTableNode.name 对齐。
    pub tables: Vec<(String, Vec<ErColumn>, ErLoadStatus)>,
}

impl AppController {
    /// 集中路由到的连接修订。connection 变更（connect/UpdateConnection/删除）时调用，
    /// 使旧连接缓存失效。返回新修订。
    pub fn er_bump_connection_revision(&self, connection_id: ConnectionId) -> u64 {
        self.er_catalog
            .lock()
            .map(|mut c| c.bump_revision(connection_id))
            .unwrap_or(0)
    }

    /// 连接删除：移除该连接全部 ER 缓存与修订。
    pub fn er_invalidate_connection(&self, connection_id: ConnectionId) {
        if let Ok(mut c) = self.er_catalog.lock() {
            c.invalidate_connection(connection_id);
        }
    }

    /// 阶段 1：读取表目录（不含字段/关系），先展示节点用。
    pub fn er_catalog_tables(
        &self,
        config: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<Vec<ErTableNode>> {
        load_er_tables_in_background(config, database, schema)
    }

    /// 阶段 3：读取/复用关系索引。首次调用执行一次 DB 读取并缓存；后续同一
    /// (连接修订, database, schema) 直接复用缓存，供整库与局部 ER 视图共享。
    /// 进行中由 inflight 去重；等待方返回 Loading，由调用方稍后复查。
    pub fn er_relations(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<ErRelationSnapshot> {
        let connector = connector_for(config)?;
        er_relations_core(&self.er_catalog, config, database, schema, connector.as_ref())
    }

    /// 阶段 2：按需读取指定表字段。只请求缺失（未缓存/未在飞）的表；
    /// 已缓存表直接返回不重复查询。按方言批量读取（MySQL/PG 单查询，SQLite 逐表）。
    /// `tables` 为结构化身份（ErTableRef），缓存键与结果归并都据此进行，不从展示名反推
    /// schema/表名（PG 跨 schema 同名表与含点标识符不串表、不漏字段）。
    pub fn er_columns_for_tables(
        &self,
        config: &ConnectionConfig,
        tables: &[ErTableRef],
    ) -> fluxdb_core::Result<ErColumnBatch> {
        let connector = connector_for(config)?;
        er_columns_core(&self.er_catalog, config, tables, connector.as_ref())
    }

    /// 仅作废指定表（结构化身份）的 Failed 字段缓存（无数据库读取，可在 UI 线程调用）。
    /// 保留其它表的成功/在飞结果；在飞表不处理（避免并发竞争）。
    pub fn er_columns_invalidate_failed(
        &self,
        config: &ConnectionConfig,
        tables: &[ErTableRef],
    ) {
        let mut guard = self.er_catalog.lock().unwrap();
        for t in tables {
            let key = guard.column_key(t, config.id);
            if guard.column_status.get(&key) == Some(&ErLoadStatus::Failed)
                && !guard.columns_inflight.contains(&key)
            {
                guard.column_status.remove(&key);
            }
        }
    }

    /// 手动刷新：作废指定表（结构化身份）的**全部**字段缓存（含 Loaded 与在飞），使下次
    /// 按需加载真正重读数据库（外部删列/改列后旧字段不残留）。连接修订不变（不动其它 tab）。
    pub fn er_columns_invalidate(
        &self,
        config: &ConnectionConfig,
        tables: &[ErTableRef],
    ) {
        let mut guard = self.er_catalog.lock().unwrap();
        for t in tables {
            let key = guard.column_key(t, config.id);
            guard.columns.remove(&key);
            guard.column_status.remove(&key);
            guard.columns_inflight.remove(&key);
            guard.columns_cancel.remove(&key);
        }
    }

    /// 取消指定范围的进行中字段加载：置位在飞读取的取消旗标，默认逐表读取会在两表之间
    /// 停止后续工作，完成时丢弃该次结果并不写入缓存。只影响 in-flight 请求，已缓存结果不动。
    /// 阻塞式驱动单次调用无法即时中断（如实保留），但能停止多表循环后续工作并丢弃过期结果。
    /// `tables` 为 None 时取消该连接/数据库范围内全部在飞读取。
    pub fn er_columns_cancel(
        &self,
        config: &ConnectionConfig,
        database: &str,
        tables: Option<&[ErTableRef]>,
    ) {
        let guard = self.er_catalog.lock().unwrap();
        for (key, flag) in guard.columns_cancel.iter() {
            if key.connection_id != config.id || key.reference.database != database {
                continue;
            }
            if let Some(tables) = tables
                && !tables.contains(&key.reference)
            {
                continue;
            }
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// 字段重试：作废指定表的 Failed 缓存后再真正重新读取。既不每帧自动重试，
    /// 也不清空成功缓存；仅在用户显式触发重试时调用。对“加载中”或成功表无影响。
    pub fn er_columns_retry(
        &self,
        config: &ConnectionConfig,
        tables: &[ErTableRef],
    ) -> fluxdb_core::Result<ErColumnBatch> {
        let connector = connector_for(config)?;
        er_columns_retry_core(&self.er_catalog, config, tables, connector.as_ref())
    }

    /// 仅作废某范围 Failed 的关系缓存（无数据库读取，可在 UI 线程调用）。
    pub fn er_relations_invalidate_failed(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema: Option<&str>,
    ) {
        let mut guard = self.er_catalog.lock().unwrap();
        let key = guard.relation_key(database, schema, config.id);
        if guard.relation_status.get(&key) == Some(&ErLoadStatus::Failed)
            && !guard.relations_inflight.contains(&key)
        {
            guard.relation_status.remove(&key);
            guard.relations.remove(&key);
        }
    }

    /// 手动刷新：作废某作用域的**全部**关系索引缓存（含 Loaded 与在飞），使下次读取真正
    /// 重读数据库（外部加/删外键后旧连线不残留）。连接修订不变（不动其它范围/tab）。
    pub fn er_relations_invalidate(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema: Option<&str>,
    ) {
        let mut guard = self.er_catalog.lock().unwrap();
        let key = guard.relation_key(database, schema, config.id);
        guard.relations.remove(&key);
        guard.relation_status.remove(&key);
        guard.relations_inflight.remove(&key);
    }

    /// 关系索引重试：作废 Failed 的关系缓存后再真正重新读取整范围边。
    /// 仅用户显式重试时调用，不每帧自动重试；成功/在飞状态不动。
    pub fn er_relations_retry(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<ErRelationSnapshot> {
        let connector = connector_for(config)?;
        er_relations_retry_core(&self.er_catalog, config, database, schema, connector.as_ref())
    }
}

/// 关系索引核心（可注入 connector 便于测试）。逻辑与 `AppController::er_relations` 相同，
/// 但其内存共享缓存经调用方传入，不依 `self` —— 便于用假连接器观察实际请求。
pub fn er_relations_core(
    cache: &Mutex<ErCatalogCache>,
    config: &ConnectionConfig,
    database: &str,
    schema: Option<&str>,
    connector: &dyn Connector,
) -> fluxdb_core::Result<ErRelationSnapshot> {
    // 单次加锁内完成「命中缓存判定 + 登记 inflight」，杜绝检查与登记分开加锁时
    // 两个并发调用都判定为可加载、各自读取数据库的竞争窗口。
    let key = {
        let mut guard = cache.lock().unwrap();
        let key = guard.relation_key(database, schema, config.id);
        // 已缓存且 Loaded / Failed：直接复用（Failed 保留，避免每帧重试震铃；由用户刷新决定）。
        if let Some(status) = guard.relation_status.get(&key) {
            match status {
                ErLoadStatus::Loaded | ErLoadStatus::Failed => {
                    return Ok(ErRelationSnapshot {
                        edges: guard.relations.get(&key).cloned().unwrap_or_default(),
                        status: *status,
                    });
                }
                _ => {}
            }
        }
        if guard.relations_inflight.contains(&key) {
            // 已有加载在进行：不重复 DB 读，返回 Loading 供调用方稍后复查。
            return Ok(ErRelationSnapshot {
                edges: Vec::new(),
                status: ErLoadStatus::Loading,
            });
        }
        // 本调用成为加载者：标记 inflight（与检查同锁，原子判定），锁外执行 DB 读取。
        guard.relations_inflight.insert(key.clone());
        guard.relation_status.insert(key.clone(), ErLoadStatus::Loading);
        key
    };
    // 锁外做网络/DB 读取。
    let result = load_er_relations_from_db_with_connector(connector, config, Some(database), schema);
    let mut guard = cache.lock().unwrap();
    guard.relations_inflight.remove(&key);
    match result {
        Ok(edges) if guard.revision(config.id) == key.conn_rev => {
            guard.relations.insert(key.clone(), edges.clone());
            guard.relation_status.insert(key, ErLoadStatus::Loaded);
            Ok(ErRelationSnapshot {
                edges,
                status: ErLoadStatus::Loaded,
            })
        }
        Ok(_) => Ok(ErRelationSnapshot {
            edges: Vec::new(),
            status: ErLoadStatus::NotLoaded,
        }), // 修订已变：结果过期，丢弃。
        Err(_) => {
            guard.relation_status.insert(key, ErLoadStatus::Failed);
            Ok(ErRelationSnapshot {
                edges: Vec::new(),
                status: ErLoadStatus::Failed,
            })
        }
    }
}

/// 字段按需读取核心（可注入 connector 便于测试）。逻辑与
/// `AppController::er_columns_for_tables` 相同，缓存经 `cache` 传入。
/// 把缺失表按 schema 分组，调用连接器的逐 schema 批量读取，返回 (reference, columns)。
/// 一个 schema 一次调用；PG 多 schema / 含点表名据此按真实 schema 查，避免裸名跨 schema 歧义。
fn read_columns_by_schema(
    database: &str,
    refs: &[ErTableRef],
    should_cancel: &dyn Fn() -> bool,
    connector: &dyn Connector,
) -> fluxdb_core::Result<Vec<(ErTableRef, Vec<ErColumn>)>> {
    let mut by_schema: BTreeMap<Option<String>, Vec<&ErTableRef>> = BTreeMap::new();
    for r in refs {
        by_schema.entry(r.schema.clone()).or_default().push(r);
    }
    let mut out = Vec::new();
    for (schema, group) in by_schema {
        let bare: Vec<String> = group.iter().map(|r| r.name.clone()).collect();
        let columns = connector.list_completion_columns_for_tables_with_cancel(
            Some(database),
            schema.as_deref(),
            &bare,
            should_cancel,
        )?;
        // 按「裸表名 + 声明 schema」归并到对应结构化身份（结果自带真实 schema，据此对齐）。
        let mut by_ref: BTreeMap<ErTableRef, Vec<ErColumn>> = BTreeMap::new();
        for col in columns {
            // 用调用方本批 ref 的 (schema, name) 精确对齐；连接器返回列自带 schema 用于校验。
            let want = group.iter().find(|r| &r.name == &col.table);
            let Some(r) = want else {
                continue; // 不属于本批请求的表列，忽略（不串表）。
            };
            by_ref.entry((*r).clone()).or_default().push(ErColumn {
                name: col.name,
                type_name: col.type_name,
                primary_key: col.primary_key,
                nullable: col.nullable,
            });
        }
        out.extend(by_ref);
    }
    Ok(out)
}

pub fn er_columns_core(
    cache: &Mutex<ErCatalogCache>,
    config: &ConnectionConfig,
    tables: &[ErTableRef],
    connector: &dyn Connector,
) -> fluxdb_core::Result<ErColumnBatch> {
    if tables.is_empty() {
        return Ok(ErColumnBatch { tables: Vec::new() });
    }
    // `tables` 为结构化身份（ErTableRef）；缓存键、结果归并都据此进行，不从展示名反推
    // schema/表名（跨 schema 同名、含点标识符不串表、不漏字段）。
    //
    // 单次加锁内完成「命中缓存判定 + 登记 inflight + 取/建取消旗标」，杜绝检查与登记
    // 分开加锁时两个并发调用都判定为缺失、各自发起重复读取的竞争窗口。
    let mut missing: Vec<ErColumnKey> = Vec::new();
    let mut missing_cancel: Vec<ErCancelFlag> = Vec::new();
    let mut cached: Vec<(String, Vec<ErColumn>, ErLoadStatus)> = Vec::new();
    {
        let mut guard = cache.lock().unwrap();
        for t in tables {
            let key = guard.column_key(t, config.id);
            match guard.column_status.get(&key) {
                Some(ErLoadStatus::Loaded) => {
                    cached.push((
                        t.display(),
                        guard.columns.get(&key).cloned().unwrap_or_default(),
                        ErLoadStatus::Loaded,
                    ));
                }
                Some(ErLoadStatus::Failed) => {
                    cached.push((t.display(), Vec::new(), ErLoadStatus::Failed));
                }
                Some(ErLoadStatus::NotLoaded) | Some(ErLoadStatus::Loading) | None
                    if guard.columns_inflight.contains(&key) =>
                {
                    // 已在飞（其它视图/前一轮）：标记 Loading，本轮不重复发起。
                    cached.push((t.display(), Vec::new(), ErLoadStatus::Loading));
                }
                Some(ErLoadStatus::NotLoaded) | None | Some(ErLoadStatus::Loading) => {
                    // 缺失（含 Loading 但未在飞=可重试）：记账并登记 inflight，取该 key 现有
                    // 取消旗标（若已有在飞方共享同一旗标），否则新建。
                    let cancel = guard
                        .columns_cancel
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
                    guard.columns_inflight.insert(key.clone());
                    guard.column_status.insert(key.clone(), ErLoadStatus::Loading);
                    guard.columns_cancel.insert(key.clone(), cancel.clone());
                    missing.push(key.clone());
                    missing_cancel.push(cancel);
                }
            }
        }
    }
    // 锁外批量读取（去抖合并由调用方做）。
    if !missing.is_empty() {
        let rev_at_start = {
            let guard = cache.lock().unwrap();
            guard.revision(config.id)
        };
        let missing_refs: Vec<ErTableRef> = missing.iter().map(|k| k.reference.clone()).collect();
        // 取消旗标：连接器默认逐表循环在两张表之间读取它；任一表被取消即停止后续工作。
        let flags_for_cancel = missing_cancel.clone();
        let should_cancel = move || flags_for_cancel.iter().any(|c| c.load(Ordering::Relaxed));
        // 结束态记录：每表 Loaded / Failed；已加载缓存表也并入返回，便于调用方一次合并。
        let mut result: Vec<(String, Vec<ErColumn>, ErLoadStatus)> = cached;
        let database = missing_refs[0].database.clone();
        let load =
            read_columns_by_schema(&database, &missing_refs, &should_cancel, connector);
        match load {
            Ok(by_ref) => {
                let mut by_display: BTreeMap<String, Vec<ErColumn>> = by_ref
                    .into_iter()
                    .map(|(r, cols)| (r.display(), cols))
                    .collect();
                let mut guard = cache.lock().unwrap();
                let rev_now = guard.revision(config.id);
                for k in &missing {
                    guard.columns_inflight.remove(k);
                    let cancelled = guard
                        .columns_cancel
                        .get(&k)
                        .map(|c| c.load(Ordering::Relaxed))
                        .unwrap_or(false);
                    guard.columns_cancel.remove(&k);
                    // 被取消：丢弃该表结果并清状态（调用方将保持未读，其后重新请求）。
                    // 修订变化：丢弃该批结果（旧连接不可复用）。
                    if cancelled || rev_now != rev_at_start || rev_now != k.conn_rev {
                        guard.column_status.remove(&k);
                        continue;
                    }
                    let display = k.reference.display();
                    if let Some(cols) = by_display.remove(&display) {
                        guard.columns.insert(k.clone(), cols.clone());
                        guard.column_status.insert(k.clone(), ErLoadStatus::Loaded);
                        result.push((display, cols, ErLoadStatus::Loaded));
                    } else {
                        // 没有该表列：视为该表无字段（空 ≠ 未读），标记 Loaded。
                        guard.columns.insert(k.clone(), Vec::new());
                        guard.column_status.insert(k.clone(), ErLoadStatus::Loaded);
                        result.push((display, Vec::new(), ErLoadStatus::Loaded));
                    }
                }
            }
            Err(_) => {
                let mut guard = cache.lock().unwrap();
                let rev_now = guard.revision(config.id);
                for k in &missing {
                    guard.columns_inflight.remove(k);
                    let cancelled = guard
                        .columns_cancel
                        .get(&k)
                        .map(|c| c.load(Ordering::Relaxed))
                        .unwrap_or(false);
                    guard.columns_cancel.remove(&k);
                    if cancelled {
                        // 被取消：不标记 Failed，保留未读供后续重试。
                        guard.column_status.remove(&k);
                        continue;
                    }
                    if rev_now != rev_at_start || rev_now != k.conn_rev {
                        guard.column_status.remove(&k);
                        continue;
                    }
                    guard.column_status.insert(k.clone(), ErLoadStatus::Failed);
                    result.push((k.reference.display(), Vec::new(), ErLoadStatus::Failed));
                }
            }
        }
        Ok(ErColumnBatch { tables: result })
    } else {
        // 全部命中缓存或已在飞：返回已缓存 + 在飞标记。
        Ok(ErColumnBatch {
            tables: cached,
        })
    }
}

/// 字段重试核心（可注入 cache+connector 便于测试）：作废 Failed 后重新读取。
pub fn er_columns_retry_core(
    cache: &Mutex<ErCatalogCache>,
    config: &ConnectionConfig,
    tables: &[ErTableRef],
    connector: &dyn Connector,
) -> fluxdb_core::Result<ErColumnBatch> {
    if tables.is_empty() {
        return Ok(ErColumnBatch { tables: Vec::new() });
    }
    {
        let mut guard = cache.lock().unwrap();
        for t in tables {
            let key = guard.column_key(t, config.id);
            if guard.column_status.get(&key) == Some(&ErLoadStatus::Failed)
                && !guard.columns_inflight.contains(&key)
            {
                guard.column_status.remove(&key);
            }
        }
    }
    er_columns_core(cache, config, tables, connector)
}

/// 关系重试核心（可注入 cache+connector 便于测试）：作废 Failed 后重新读取。
pub fn er_relations_retry_core(
    cache: &Mutex<ErCatalogCache>,
    config: &ConnectionConfig,
    database: &str,
    schema: Option<&str>,
    connector: &dyn Connector,
) -> fluxdb_core::Result<ErRelationSnapshot> {
    {
        let mut guard = cache.lock().unwrap();
        let key = guard.relation_key(database, schema, config.id);
        if guard.relation_status.get(&key) == Some(&ErLoadStatus::Failed)
            && !guard.relations_inflight.contains(&key)
        {
            guard.relation_status.remove(&key);
            guard.relations.remove(&key);
        }
    }
    er_relations_core(cache, config, database, schema, connector)
}

/// 用注入 connector 读取整库关系边（供 er_relations_core 测试注入）。
fn load_er_relations_from_db_with_connector(
    connector: &dyn Connector,
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<Vec<ErForeignKeyEdge>> {
    // 复用 er_service 里与 connector 无关的纯关系组装：临时构造等价于
    // load_er_relations_from_db 但用注入 connector 的读取。
    load_er_relations_from_db_with(connector, config, database, schema)
}
