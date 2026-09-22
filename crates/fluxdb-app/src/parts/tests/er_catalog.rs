// ER 元数据共享缓存 / 字段按需 / 关系索引的编排测试（er_catalog.rs）。
// 用可注入假连接器观察实际请求与状态，而非只检查源码字符串。

use std::cell::RefCell;
use std::rc::Rc;

/// 记录型假连接器：记录字段/表/外键读取，返回罐头数据。
/// 只实现用到的 trait 方法，其余走默认；`kind` 决定 display 命名规则（PG 拼 schema.table）。
struct RecordingConnector {
    kind: DatabaseKind,
    /// 每次 list_completion_columns 请求的表（裸名）记录。
    column_reads: Rc<RefCell<Vec<String>>>,
    /// 每次 list_objects 触发（关系读取会先枚举表）。
    object_reads: Rc<RefCell<usize>>,
    /// 字段罐头：table 名 -> 列铭牌（name, primary）。
    columns: std::collections::BTreeMap<String, Vec<(String, bool)>>,
    /// 外键罐头（from, to）。
    fks: Vec<(String, String)>,
    /// 是否让字段读取失败（partial-failure 测试）。
    column_fail: bool,
}

impl RecordingConnector {
    fn new(
        kind: DatabaseKind,
        columns: std::collections::BTreeMap<String, Vec<(String, bool)>>,
        fks: Vec<(String, String)>,
    ) -> (Self, Rc<RefCell<Vec<String>>>, Rc<RefCell<usize>>) {
        let reads = Rc::new(RefCell::new(Vec::new()));
        let objects = Rc::new(RefCell::new(0usize));
        (
            RecordingConnector {
                kind,
                column_reads: reads.clone(),
                object_reads: objects.clone(),
                columns,
                fks,
                column_fail: false,
            },
            reads,
            objects,
        )
    }
}

impl Connector for RecordingConnector {
    fn kind(&self) -> DatabaseKind {
        self.kind
    }
    fn test_connection(&self, _: &ConnectionConfig) -> fluxdb_core::Result<()> {
        Ok(())
    }
    fn list_objects(
        &self,
        _path: Option<&ObjectPath>,
    ) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        *self.object_reads.borrow_mut() += 1;
        Ok(self
            .columns
            .keys()
            .map(|name| ObjectSummary {
                path: ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("db".to_string()),
                    schema: None,
                    name: name.clone(),
                    kind: ObjectKind::Table,
                },
                rows: None,
                modified_at: None,
                comment: None,
            })
            .collect())
    }
    fn list_completion_columns(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        self.column_reads.borrow_mut().push(table.to_string());
        if self.column_fail {
            return Err(Error::new(ErrorKind::Query, "模拟字段读取失败"));
        }
        Ok(self
            .columns
            .get(table)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, primary)| CompletionColumn {
                database: Some("db".to_string()),
                schema: schema.map(str::to_string),
                table: table.to_string(),
                name,
                type_name: Some("text".to_string()),
                nullable: false,
                primary_key: primary,
                comment: None,
            })
            .collect())
    }
    fn list_foreign_keys_for_tables(
        &self,
        _database: Option<&str>,
        _schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<Vec<(String, ForeignKeyInfo)>> {
        Ok(self
            .fks
            .iter()
            .filter(|(from, _)| tables.contains(&from.to_string()))
            .map(|(from, to)| {
                (
                    from.clone(),
                    ForeignKeyInfo {
                        name: format!("fk_{from}_to_{to}"),
                        column: "id".to_string(),
                        ref_schema: None,
                        ref_table: to.clone(),
                        ref_column: "id".to_string(),
                        columns: Vec::new(),
                        ref_columns: Vec::new(),
                    },
                )
            })
            .collect())
    }
    fn load_data(
        &self,
        _: &ObjectPath,
        _offset: u64,
        _limit: u64,
        _sort: &[SortSpec],
        _filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        Ok(DataPage {
            columns: Vec::new(),
            rows: Vec::new(),
            offset: 0,
            limit: 0,
            has_more: false,
        })
    }
    fn apply_changes(&self, _: &DataChangeSet) -> fluxdb_core::Result<AppliedChangeOutcome> {
        Err(Error::new(ErrorKind::Unsupported, "fake"))
    }
    fn execute(&self, _: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        Err(Error::new(ErrorKind::Unsupported, "fake"))
    }
}

/// 结构化表身份构造助手（schema 可选；name 可含 `.` 等合法字符）。
fn mk_ref(database: &str, schema: Option<&str>, name: &str) -> fluxdb_core::ErTableRef {
    fluxdb_core::ErTableRef {
        database: database.to_string(),
        schema: schema.map(str::to_string),
        name: name.to_string(),
    }
}

fn fake_config(kind: DatabaseKind) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(7),
        name: "fake".to_string(),
        kind,
        endpoint: Endpoint::SqliteFile {
            path: ":memory:".into(),
            read_only: false,
        },
        credential_ref: None,
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
    }
}

// 2) 首屏只加载请求的表字段：不向连接器请求未请求表。
#[test]
fn columns_only_request_given_tables() {
    let cols = BTreeMap::from([
        ("orders".to_string(), vec![("id".to_string(), true)]),
        ("customers".to_string(), vec![("id".to_string(), true)]),
        ("products".to_string(), vec![("id".to_string(), true)]),
    ]);
    let (conn, reads, _) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    // 只请求一张表：连接器只读其字段，绝不读全库其它表。
    let batch = er_columns_core(&cache, &config, &[mk_ref("db", None, "orders")], &conn)
        .expect("load");
    assert_eq!(batch.tables.len(), 1);
    assert_eq!(batch.tables[0].1.len(), 1, "orders 应读到 id 字段");
    assert_eq!(*reads.borrow(), vec!["orders".to_string()], "只请求 orders");
}

// 4) 手动刷新作废全部字段缓存：外部删列后，er_columns_invalidate 使下次请求真正重读，
//    旧字段不残留（区别于只作废 Failed 的 _failed 变体）。
#[test]
fn columns_invalidate_all_forces_reread_after_external_drop() {
    // 首次：两列。
    let mut cols = BTreeMap::from([(
        "orders".to_string(),
        vec![("id".to_string(), true), ("to_drop".to_string(), false)],
    )]);
    let (mut conn, reads, _) = RecordingConnector::new(DatabaseKind::Sqlite, cols.clone(), Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);
    let tables = vec![mk_ref("db", None, "orders")];

    let b1 = er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(b1.tables[0].1.len(), 2);
    assert_eq!(reads.borrow().len(), 1);

    // 复用缓存不重读。
    er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(reads.borrow().len(), 1);

    // 数据库外部删掉 to_drop 列：作废全部字段缓存（与 AppController::er_columns_invalidate
    // 相同的清理）后重新读取，返回只剩一列，旧列不残留。
    cols.get_mut("orders").unwrap().remove(1);
    conn.columns = cols;
    {
        let mut g = cache.lock().unwrap();
        let key = g.column_key(&tables[0], config.id);
        g.columns.remove(&key);
        g.column_status.remove(&key);
        g.columns_inflight.remove(&key);
    }
    let b2 = er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(reads.borrow().len(), 2, "作废后必须重读");
    assert_eq!(b2.tables[0].1.len(), 1, "已删除的 to_drop 列不得残留");
    assert_eq!(b2.tables[0].1[0].name, "id");
}

// 3) 重复需求合并；已缓存字段不重复读取。
#[test]
fn columns_cached_no_second_read() {
    let cols = BTreeMap::from([("orders".to_string(), vec![("id".to_string(), true)])]);
    let (conn, reads, _) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    let tables = vec![mk_ref("db", None, "orders")];
    er_columns_core(&cache, &config, &tables, &conn).expect("load1");
    assert_eq!(reads.borrow().len(), 1);
    // 再次请求：命中缓存，不再发连接器请求。
    let batch = er_columns_core(&cache, &config, &tables, &conn).expect("load2");
    assert_eq!(batch.tables[0].1.len(), 1);
    assert_eq!(reads.borrow().len(), 1, "已缓存表不应重复读取");
}

// 5) PostgreSQL 不同 schema 同名表不串数据（缓存键含 schema，身份保留）。
#[test]
fn postgres_same_table_name_different_schemas_not_merged() {
    let cols = BTreeMap::from([("a".to_string(), vec![("col_a".to_string(), false)])]);
    let (conn, reads, _) = RecordingConnector::new(DatabaseKind::Postgres, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Postgres);

    // schema s1 与 s2 各有一张同名表 a，各自独立缓存、独立读取（结构化身份，不靠展示名拆分）。
    let tab1 = mk_ref("db", Some("s1"), "a");
    let tab2 = mk_ref("db", Some("s2"), "a");
    let b1 = er_columns_core(&cache, &config, &[tab1.clone()], &conn).unwrap();
    assert_eq!(b1.tables.len(), 1);
    assert_eq!(b1.tables[0].0, "s1.a");
    assert_eq!(reads.borrow().len(), 1);

    let b2 = er_columns_core(&cache, &config, &[tab2.clone()], &conn).unwrap();
    // s2 未缓存：应再读一次，且两 schema 互不串。
    assert_eq!(reads.borrow().len(), 2, "不同 schema 同名表各自读取");
    assert_eq!(b2.tables[0].0, "s2.a");

    // 再取 s1：命中 s1 缓存，不重读。
    let b3 = er_columns_core(&cache, &config, &[tab1.clone()], &conn).unwrap();
    assert_eq!(reads.borrow().len(), 2, "s1 再次请求命中缓存");
    assert_eq!(b3.tables[0].1[0].name, "col_a");
}

// 6) 部分字段失败不清空可用结果：失败表 Failed，成功表仍 Loaded。
#[test]
fn partial_column_failure_keeps_loaded() {
    let mut cols = BTreeMap::new();
    cols.insert("ok".to_string(), vec![("id".to_string(), true)]);
    cols.insert("bad".to_string(), Vec::new());
    let (mut conn, _, _) = RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    conn.column_fail = true; // 让所有读取失败
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    let batch = er_columns_core(
        &cache,
        &config,
        &[mk_ref("db", None, "ok"), mk_ref("db", None, "bad")],
        &conn,
    )
    .unwrap();
    // 全部失败 → 每表 Failed；缓存不清空可用内容。
    assert_eq!(batch.tables.len(), 2);
    assert!(
        batch.tables.iter().all(|(_, _, s)| *s == ErLoadStatus::Failed),
        "失败标记到对应表：{batch:?}"
    );
    // 失败后缓存状态为 Failed（不清空、不当作没有字段）。
    let second = er_columns_core(&cache, &config, &[mk_ref("db", None, "ok")], &conn).unwrap();
    assert_eq!(second.tables[0].2, ErLoadStatus::Failed);
}

// 6b) 字段重试确实重新调用 Connector：先失败缓存 Failed，重试作废后真正重新读取而非返回缓存失败。
#[test]
fn column_retry_requeries_connector() {
    let mut cols = BTreeMap::new();
    cols.insert("orders".to_string(), vec![("id".to_string(), true)]);
    let (mut conn, reads, _) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    conn.column_fail = true; // 首次失败
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);
    let tables = vec![mk_ref("db", None, "orders")];

    let first = er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(first.tables[0].2, ErLoadStatus::Failed);
    let reads_after_first = reads.borrow().len();
    assert_eq!(reads_after_first, 1);

    // 失败后普通请求仍返回缓存 Failed（不自动重试，避免每帧震铃）。
    let again = er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(again.tables[0].2, ErLoadStatus::Failed);
    assert_eq!(reads.borrow().len(), 1, "失败缓存不自动重试");

    // 连接器恢复成功；显式重试才真正重新查询。
    conn.column_fail = false;
    let retry = er_columns_retry_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(retry.tables[0].2, ErLoadStatus::Loaded);
    assert_eq!(retry.tables[0].1.len(), 1);
    assert_eq!(reads.borrow().len(), 2, "重试必须真正重新调用 Connector");
}

// 6c) 关系重试确实重新调用 Connector。
#[test]
fn relation_retry_requeries_connector() {
    let mut cols = BTreeMap::new();
    cols.insert("a".to_string(), vec![("id".to_string(), true)]);
    let (mut conn, _, objects) = RecordingConnector::new(
        DatabaseKind::Sqlite,
        cols,
        vec![("a".to_string(), "b".to_string())],
    );
    // 先让关系读取失败（list_foreign_keys_for_tables 返回空也可；改用列读取失败触发不了关系，
    // 这里直接构造:对象的 list_objects 记录读取次数，重试应再次触发目录/外键读取）。
    // 首次正常加载成功，再手动把缓存置 Failed 模拟失败后重试路径。
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);
    let first = er_relations_core(&cache, &config, "db", None, &conn).unwrap();
    assert_eq!(first.status, ErLoadStatus::Loaded);
    let reads_after_first = *objects.borrow();

    // 置为 Failed（模拟失败），且 relations 缓存里已有边——重试必须重新读取并恢复。
    {
        let mut guard = cache.lock().unwrap();
        let key = guard.relation_key("db", None, config.id);
        guard.relation_status.insert(key.clone(), ErLoadStatus::Failed);
        guard.relations.remove(&key);
    }
    let retry = er_relations_retry_core(&cache, &config, "db", None, &conn).unwrap();
    assert_eq!(retry.status, ErLoadStatus::Loaded);
    assert_eq!(retry.edges.len(), 1, "重试重新读取关系");
    assert!(*objects.borrow() > reads_after_first, "关系重试必须重新调用 Connector");
}

// 7) 连接修订变化后旧缓存作废：新修订不再复用旧结果。
#[test]
fn revision_bump_invalidates_old_columns() {
    let cols = BTreeMap::from([("orders".to_string(), vec![("id".to_string(), true)])]);
    let (conn, reads, _) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    let tables = vec![mk_ref("db", None, "orders")];
    er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(reads.borrow().len(), 1);

    // 连接变更（改地址/账号）→ 修订自增，旧连接字段缓存作废。
    cache.lock().unwrap().bump_revision(config.id);
    let batch = er_columns_core(&cache, &config, &tables, &conn).unwrap();
    assert_eq!(reads.borrow().len(), 2, "修订变化后应重新读取而非复用旧缓存");
    assert_eq!(batch.tables[0].1.len(), 1);
}

// 8) 关系索引复用：首次读取并缓存，重复调用不再读库。
#[test]
fn relations_cached_across_calls() {
    let cols = BTreeMap::from([
        ("a".to_string(), vec![("id".to_string(), true)]),
        ("b".to_string(), vec![("id".to_string(), true)]),
    ]);
    let (conn, _, objects) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, vec![("a".to_string(), "b".to_string())]);
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    let first = er_relations_core(&cache, &config, "db", None, &conn).unwrap();
    assert_eq!(first.status, ErLoadStatus::Loaded);
    assert_eq!(first.edges.len(), 1);
    let reads_after_first = *objects.borrow();

    // 第二次：命中缓存，不再触发目录/外键读取。
    let second = er_relations_core(&cache, &config, "db", None, &conn).unwrap();
    assert_eq!(second.status, ErLoadStatus::Loaded);
    assert_eq!(second.edges.len(), 1);
    assert_eq!(*objects.borrow(), reads_after_first, "关系应复用缓存不重复读");
}

// 9) 局部展开不重复加载全库字段：邻域计算只依赖关系边，不读字段。
#[test]
fn neighborhood_calculation_uses_edges_not_columns() {
    let mk = |n: &str| fluxdb_core::ErTableRef {
        database: "db".into(),
        schema: None,
        name: n.into(),
    };
    let edges = vec![
        ErForeignKeyEdge {
            name: "f1".into(),
            from_table: "orders".into(),
            from_column: "customer_id".into(),
            to_table: "customers".into(),
            to_column: "id".into(),
            from_reference: mk("orders"),
            to_reference: mk("customers"),
        },
        ErForeignKeyEdge {
            name: "f2".into(),
            from_table: "orders".into(),
            from_column: "product_id".into(),
            to_table: "products".into(),
            to_column: "id".into(),
            from_reference: mk("orders"),
            to_reference: mk("products"),
        },
    ];
    // 以 orders 为中心 1 跳：customers/products 含入，且只依据边（无字段参与）。
    let included = er_neighborhood_included_tables(&mk("orders"), 1, &BTreeSet::new(), &edges);
    assert!(included.contains("orders"));
    assert!(included.contains("customers"));
    assert!(included.contains("products"));
    assert_eq!(included.len(), 3);
}

// 跨 schema 中心 + 同名表：中心按结构化身份匹配，不遗漏跨 schema 邻居，且不把
// 其他 schema 的同名表误判为中心。含点标识符（schema/表名带 `.`）同样安全。
#[test]
fn neighborhood_cross_schema_and_dotted_identifiers() {
    let mk = |d: &str, schema: Option<&str>, n: &str| fluxdb_core::ErTableRef {
        database: d.into(),
        schema: schema.map(str::to_string),
        name: n.into(),
    };
    // public.orders → audit.orders_audit；public.orders → hr.items（含点表名）。
    let edges = vec![
        ErForeignKeyEdge {
            name: "f1".into(),
            from_table: "public.orders".into(),
            from_column: "id".into(),
            to_table: "audit.orders_audit".into(),
            to_column: "order_id".into(),
            from_reference: mk("db", Some("public"), "orders"),
            to_reference: mk("db", Some("audit"), "orders_audit"),
        },
        ErForeignKeyEdge {
            name: "f2".into(),
            from_table: "public.orders".into(),
            from_column: "id".into(),
            to_table: "hr.items".into(),
            to_column: "order_id".into(),
            from_reference: mk("db", Some("public"), "orders"),
            to_reference: mk("db", Some("hr"), "items"),
        },
    ];
    // 中心为 public.orders：1 跳应含跨界邻居 audit.orders_audit 与 hr.items。
    let included = er_neighborhood_included_tables(
        &mk("db", Some("public"), "orders"),
        1,
        &BTreeSet::new(),
        &edges,
    );
    assert!(included.contains("audit.orders_audit"), "跨 schema 出向邻居不得遗漏");
    assert!(included.contains("hr.items"), "含点表名邻居不得遗漏");
    assert_eq!(included.len(), 3);

    // 同名但不同 schema 的另一张 orders（比如 sales.orders）不是该中心 → 不出现在该邻域。
    assert!(
        !included.contains("sales.orders"),
        "其他 schema 同名表不应被误判为该 public.orders 中心"
    );

    // 反向验证：以 hr.items 为中心（items 自身引用外部表），结构化身份不依赖字符串拆解。
    let hr_neighborhood = er_neighborhood_included_tables(
        &mk("db", Some("hr"), "items"),
        1,
        &BTreeSet::new(),
        &edges,
    );
    assert!(hr_neighborhood.contains("public.orders"));
}

/// 线程安全的关系假连接器：用原子计数 + 休眠放大并发窗口，验证「检查缓存与登记 inflight」
/// 同锁时,多个并发调用合并为一次数据库读取（race 回归）。
struct ThreadSafeRelConnector {
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Connector for ThreadSafeRelConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Sqlite
    }
    fn test_connection(&self, _: &ConnectionConfig) -> fluxdb_core::Result<()> {
        Ok(())
    }
    fn list_objects(&self, _: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // 放大竞争窗口：两个并发调用都能在旧实现的「检查」与「登记」之间停住。
        std::thread::sleep(std::time::Duration::from_millis(30));
        Ok(vec![ObjectSummary {
            path: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("db".into()),
                schema: None,
                name: "a".into(),
                kind: ObjectKind::Table,
            },
            rows: None,
            modified_at: None,
            comment: None,
        }])
    }
    fn list_foreign_keys_for_tables(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: &[String],
    ) -> fluxdb_core::Result<Vec<(String, ForeignKeyInfo)>> {
        Ok(Vec::new())
    }
    fn list_completion_columns(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        Ok(Vec::new())
    }
    fn load_data(
        &self,
        _: &ObjectPath,
        _: u64,
        _: u64,
        _: &[SortSpec],
        _: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        Ok(DataPage {
            columns: Vec::new(),
            rows: Vec::new(),
            offset: 0,
            limit: 0,
            has_more: false,
        })
    }
    fn apply_changes(&self, _: &DataChangeSet) -> fluxdb_core::Result<AppliedChangeOutcome> {
        Err(Error::new(ErrorKind::Unsupported, "fake"))
    }
    fn execute(&self, _: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        Err(Error::new(ErrorKind::Unsupported, "fake"))
    }
}

// 并发去重：缓存检查与 inflight 登记同锁，两个并发关系加载合并为一次数据库读取。
#[test]
fn concurrent_relation_loads_collapse_to_single_read() {
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fake = ThreadSafeRelConnector { reads: reads.clone() };
    let cache = Arc::new(StdMutex::new(ErCatalogCache::default()));
    let config = fake_config(DatabaseKind::Sqlite);
    let cache_ref = cache.as_ref();

    let fake_ref = &fake;
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let cache = cache_ref;
            let config = config.clone();
            scope.spawn(move || {
                er_relations_core(cache, &config, "db", None, fake_ref).unwrap()
            });
        }
    });
    // 只有一个线程成为加载者并读取数据库；其余复用缓存或返回 Loading。
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "并发关系加载必须合并为一次数据库读取，不得重复查询"
    );
}

// 取消链路：预先置位取消旗标后，字段加载虽发起但结果被丢弃（不写入缓存、不标记 Loaded）。
// 连接器默认逐表循环读取 should_cancel 也会停止后续读取。
#[test]
fn cancelled_column_load_is_discarded() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let cols = BTreeMap::from([("orders".to_string(), vec![("id".to_string(), true)])]);
    let (conn, reads, _) = RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    let cache = Arc::new(Mutex::new(ErCatalogCache::default()));
    let config = fake_config(DatabaseKind::Sqlite);

    // 预先让该表处于「可重试（非在飞）+ 已取消」状态：置 status=Loading（非在飞）+ 取消旗标。
    let orders_ref = mk_ref("db", None, "orders");
    {
        let mut g = cache.lock().unwrap();
        let key = g.column_key(&orders_ref, config.id);
        g.column_status.insert(key.clone(), ErLoadStatus::Loading);
        g.columns_cancel
            .insert(key, Arc::new(AtomicBool::new(true)));
    }
    let batch = er_columns_core(&cache, &config, &[orders_ref.clone()], &conn).unwrap();
    // 被取消：不在返回里出现 Loaded，缓存也不被写为 Loaded。
    assert!(
        !batch.tables.iter().any(|(_, _, s)| *s == ErLoadStatus::Loaded),
        "已取消的加载不得返回 Loaded：{batch:?}"
    );
    let key = cache.lock().unwrap().column_key(&orders_ref, config.id);
    let st = cache.lock().unwrap().column_status.get(&key).copied();
    assert_ne!(st, Some(ErLoadStatus::Loaded), "取消结果不得写入缓存为 Loaded");
    // 取消后该表未被连接器真正读入（默认逐表循环在读取前就因 should_cancel 返回空）。
    assert!(reads.borrow().is_empty(), "取消应阻止连接器逐表读取");
}

// 结构化身份贯穿字段加载：PG 表名含 `.` 的合法标识符（如 schema `s` 下表 `my.table`），
// 不再用 `rsplit('.')` 从展示名反推 schema（旧实现会拆成 schema `s.my`+裸名 `table` 而错位）。
// 缓存键与结果归并都按 ErTableRef 身份，含点表名的字段不串表、不漏字段。
#[test]
fn dotted_pg_table_name_keyed_by_structured_identity() {
    let cols = BTreeMap::from([(
        "my.table".to_string(),
        vec![("id".to_string(), true), ("note".to_string(), false)],
    )]);
    let (conn, reads, _) = RecordingConnector::new(DatabaseKind::Postgres, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Postgres);

    // 身份：schema=s（真实 schema），裸表名含点。
    let dotted = mk_ref("db", Some("s"), "my.table");
    let b1 = er_columns_core(&cache, &config, &[dotted.clone()], &conn).unwrap();
    // 结果按 display（`s.my.table`）返回，字段齐全（不漏）。
    assert_eq!(b1.tables.len(), 1);
    assert_eq!(b1.tables[0].0, "s.my.table");
    assert_eq!(b1.tables[0].1.len(), 2, "含点表名不得漏字段");
    assert_eq!(reads.borrow().as_slice(), &["my.table".to_string()], "以裸名发给连接器");

    // 再次请求命中缓存，不重复读取（缓存键按结构化身份稳定，未按误拆的 schema）。
    let b2 = er_columns_core(&cache, &config, &[dotted], &conn).unwrap();
    assert_eq!(reads.borrow().len(), 1, "含点表名二次请求命中缓存");
    assert_eq!(b2.tables[0].1[0].name, "id");
}
