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

/// 字段缓存键：带足连接、连接修订、database、schema、table 身份；
/// PG 跨 schema 同名表不串数据，连接变更不误用旧连接结果。
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ErColumnKey {
    connection_id: ConnectionId,
    conn_rev: u64,
    database: String,
    schema: Option<String>,
    table: String,
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
    }

    fn relation_key(&self, database: &str, schema: Option<&str>, connection_id: ConnectionId) -> ErRelationKey {
        ErRelationKey {
            connection_id,
            conn_rev: self.revision(connection_id),
            database: database.to_string(),
            schema: schema.map(str::to_string),
        }
    }

    fn column_key(
        &self,
        database: &str,
        schema: Option<&str>,
        table: &str,
        connection_id: ConnectionId,
    ) -> ErColumnKey {
        ErColumnKey {
            connection_id,
            conn_rev: self.revision(connection_id),
            database: database.to_string(),
            schema: schema.map(str::to_string),
            table: table.to_string(),
        }
    }
}

/// 关系索引读取结果：已缓存边 + 状态。
/// edges 用拥有型 Vec（非 Rc），便于跨后台线程（background_spawn）传递（Send）。
pub struct ErRelationSnapshot {
    pub edges: Vec<ErForeignKeyEdge>,
    pub status: ErLoadStatus,
}

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
    pub fn er_columns_for_tables(
        &self,
        config: &ConnectionConfig,
        database: &str,
        schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<ErColumnBatch> {
        let connector = connector_for(config)?;
        er_columns_core(&self.er_catalog, config, database, schema, tables, connector.as_ref())
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
    let key = {
        let guard = cache.lock().unwrap();
        guard.relation_key(database, schema, config.id)
    };
    // 已缓存且 Loaded / Failed：直接复用（Failed 保留，避免每帧重试震铃；由用户刷新决定）。
    {
        let guard = cache.lock().unwrap();
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
    }
    // 本调用成为加载者：标记 inflight，锁外执行 DB 读取。
    {
        let mut guard = cache.lock().unwrap();
        guard.relations_inflight.insert(key.clone());
        guard.relation_status.insert(key.clone(), ErLoadStatus::Loading);
    }
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
pub fn er_columns_core(
    cache: &Mutex<ErCatalogCache>,
    config: &ConnectionConfig,
    database: &str,
    schema: Option<&str>,
    tables: &[String],
    connector: &dyn Connector,
) -> fluxdb_core::Result<ErColumnBatch> {
    if tables.is_empty() {
        return Ok(ErColumnBatch { tables: Vec::new() });
    }
    // 锁内过滤：只留「未加载且未在飞」的表；已缓存命中直接取回。
    // `tables` 使用展示名（与 ErTableNode.name 对齐：PG 为 `schema.table`，其余裸名）。
    // 缓存键用每表推导的 schema（PG 从展示名取；其余用作用域 schema）保证跨 schema 不串。
    let keys: Vec<ErColumnKey> = {
        let guard = cache.lock().unwrap();
        tables
            .iter()
            .map(|t| {
                let (per_table_schema, _) = er_split_display(config.kind, t, schema);
                guard.column_key(database, per_table_schema.as_deref(), t, config.id)
            })
            .collect()
    };
    let mut missing: Vec<ErColumnKey> = Vec::new();
    let mut cached: Vec<(String, Vec<ErColumn>, ErLoadStatus)> = Vec::new();
    {
        let guard = cache.lock().unwrap();
        for (t, key) in tables.iter().zip(keys.iter()) {
            match guard.column_status.get(key) {
                Some(ErLoadStatus::Loaded) => {
                    cached.push((
                        t.clone(),
                        guard.columns.get(key).cloned().unwrap_or_default(),
                        ErLoadStatus::Loaded,
                    ));
                }
                Some(ErLoadStatus::Failed) => {
                    cached.push((t.clone(), Vec::new(), ErLoadStatus::Failed));
                }
                Some(ErLoadStatus::NotLoaded) | Some(ErLoadStatus::Loading) | None
                    if guard.columns_inflight.contains(key) =>
                {
                    // 已在飞（其它视图/前一轮）：标记 Loading，本轮不重复发起。
                    cached.push((t.clone(), Vec::new(), ErLoadStatus::Loading));
                }
                Some(ErLoadStatus::NotLoaded) | None => {
                    missing.push(key.clone());
                }
                Some(ErLoadStatus::Loading) => {
                    // 单表仅缺字段且未在飞：也应发起读取（Loading 但不在飞表示可重试）。
                    missing.push(key.clone());
                }
            }
        }
    }
    // 标记待读表为 inflight，锁外批量读取。
    if !missing.is_empty() {
        {
            let mut guard = cache.lock().unwrap();
            for k in &missing {
                guard.columns_inflight.insert(k.clone());
                guard.column_status.insert(k.clone(), ErLoadStatus::Loading);
            }
        }
        let rev_at_start = {
            let guard = cache.lock().unwrap();
            guard.revision(config.id)
        };
        // 收集待读表的裸表名发给连接器（PG 展示名 `schema.table` 需拆出裸名）；
        // 结果按 display 名归并（见读回映射）。
        let raw: Vec<String> = missing
            .iter()
            .map(|k| er_split_display(config.kind, &k.table, schema).1)
            .collect();
        // 结束态记录：每表 Loaded / Failed；已加载缓存表也并入返回，便于调用方一次合并。
        let mut result: Vec<(String, Vec<ErColumn>, ErLoadStatus)> = cached;
        match connector.list_completion_columns_for_tables_with_cancel(
            Some(database),
            schema,
            &raw,
            &|| false,
        ) {
            Ok(columns) => {
                // 按 display 名分组列（与 ErTableNode.name 对齐：PG schema.table，其余裸名）。
                let mut by_display: BTreeMap<String, Vec<ErColumn>> = BTreeMap::new();
                for col in columns {
                    let display = if config.kind == DatabaseKind::Postgres {
                        match (&col.schema, col.table.as_str()) {
                            (Some(s), name) => format!("{s}.{name}"),
                            (None, name) => name.to_string(),
                        }
                    } else {
                        col.table.clone()
                    };
                    by_display.entry(display).or_default().push(ErColumn {
                        name: col.name,
                        type_name: col.type_name,
                        primary_key: col.primary_key,
                        nullable: col.nullable,
                    });
                }
                let mut guard = cache.lock().unwrap();
                let rev_now = guard.revision(config.id);
                for k in &missing {
                    guard.columns_inflight.remove(k);
                    // 修订变化 → 丢弃该批结果（旧连接不可复用）。
                    if rev_now != rev_at_start || rev_now != k.conn_rev {
                        guard.column_status.remove(k);
                        continue;
                    }
                    if let Some(cols) = by_display.remove(&k.table) {
                        guard.columns.insert(k.clone(), cols.clone());
                        guard.column_status.insert(k.clone(), ErLoadStatus::Loaded);
                        result.push((k.table.clone(), cols, ErLoadStatus::Loaded));
                    } else {
                        // 没有该表列：视为该表无字段（空 ≠ 未读），标记 Loaded。
                        guard.columns.insert(k.clone(), Vec::new());
                        guard.column_status.insert(k.clone(), ErLoadStatus::Loaded);
                        result.push((k.table.clone(), Vec::new(), ErLoadStatus::Loaded));
                    }
                }
            }
            Err(_) => {
                let mut guard = cache.lock().unwrap();
                let rev_now = guard.revision(config.id);
                for k in &missing {
                    guard.columns_inflight.remove(k);
                    if rev_now != rev_at_start || rev_now != k.conn_rev {
                        guard.column_status.remove(k);
                        continue;
                    }
                    guard.column_status.insert(k.clone(), ErLoadStatus::Failed);
                    result.push((k.table.clone(), Vec::new(), ErLoadStatus::Failed));
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

/// 由展示表名拆出「(每表 schema, 裸表名)」。
/// PG 展示名为 `schema.table`（er_service 拼接），拆最后一个 '.'；无 '.' 视为无 schema。
/// 其余方言展示名即裸名，schema 沿用作用域参数。
fn er_split_display(
    kind: DatabaseKind,
    display: &str,
    scope_schema: Option<&str>,
) -> (Option<String>, String) {
    if kind == DatabaseKind::Postgres {
        match display.rsplit_once('.') {
            Some((s, bare)) if !s.is_empty() && !bare.is_empty() => {
                (Some(s.to_string()), bare.to_string())
            }
            _ => (None, display.to_string()),
        }
    } else {
        (scope_schema.map(str::to_string), display.to_string())
    }
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
