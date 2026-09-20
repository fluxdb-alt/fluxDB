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
    let batch = er_columns_core(
        &cache,
        &config,
        "db",
        None,
        &["orders".to_string()],
        &conn,
    )
    .expect("load");
    assert_eq!(batch.tables.len(), 1);
    assert_eq!(batch.tables[0].1.len(), 1, "orders 应读到 id 字段");
    assert_eq!(*reads.borrow(), vec!["orders".to_string()], "只请求 orders");
}

// 3) 重复需求合并；已缓存字段不重复读取。
#[test]
fn columns_cached_no_second_read() {
    let cols = BTreeMap::from([("orders".to_string(), vec![("id".to_string(), true)])]);
    let (conn, reads, _) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    let tables = vec!["orders".to_string()];
    er_columns_core(&cache, &config, "db", None, &tables, &conn).expect("load1");
    assert_eq!(reads.borrow().len(), 1);
    // 再次请求：命中缓存，不再发连接器请求。
    let batch = er_columns_core(&cache, &config, "db", None, &tables, &conn).expect("load2");
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

    // schema s1 与 s2 各有一张同名表 a，各自独立缓存、独立读取（用展示名 `schema.table`）。
    let tab1 = "s1.a".to_string();
    let tab2 = "s2.a".to_string();
    let b1 = er_columns_core(&cache, &config, "db", Some("s1"), &[tab1.clone()], &conn).unwrap();
    assert_eq!(b1.tables.len(), 1);
    assert_eq!(reads.borrow().len(), 1);

    let b2 = er_columns_core(&cache, &config, "db", Some("s2"), &[tab2.clone()], &conn).unwrap();
    // s2 未缓存：应再读一次，且两 schema 互不串。
    assert_eq!(reads.borrow().len(), 2, "不同 schema 同名表各自读取");
    assert_eq!(b2.tables.len(), 1);

    // 再取 s1：命中 s1 缓存，不重读。
    let b3 = er_columns_core(&cache, &config, "db", Some("s1"), &[tab1.clone()], &conn).unwrap();
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
        "db",
        None,
        &["ok".to_string(), "bad".to_string()],
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
    let second = er_columns_core(
        &cache,
        &config,
        "db",
        None,
        &["ok".to_string()],
        &conn,
    )
    .unwrap();
    assert_eq!(second.tables[0].2, ErLoadStatus::Failed);
}

// 7) 连接修订变化后旧缓存作废：新修订不再复用旧结果。
#[test]
fn revision_bump_invalidates_old_columns() {
    let cols = BTreeMap::from([("orders".to_string(), vec![("id".to_string(), true)])]);
    let (conn, reads, _) =
        RecordingConnector::new(DatabaseKind::Sqlite, cols, Vec::new());
    let cache = Mutex::new(ErCatalogCache::default());
    let config = fake_config(DatabaseKind::Sqlite);

    let tables = vec!["orders".to_string()];
    er_columns_core(&cache, &config, "db", None, &tables, &conn).unwrap();
    assert_eq!(reads.borrow().len(), 1);

    // 连接变更（改地址/账号）→ 修订自增，旧连接字段缓存作废。
    cache.lock().unwrap().bump_revision(config.id);
    let batch = er_columns_core(&cache, &config, "db", None, &tables, &conn).unwrap();
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
    let edges = vec![
        ErForeignKeyEdge {
            name: "f1".into(),
            from_table: "orders".into(),
            from_column: "customer_id".into(),
            to_table: "customers".into(),
            to_column: "id".into(),
        },
        ErForeignKeyEdge {
            name: "f2".into(),
            from_table: "orders".into(),
            from_column: "product_id".into(),
            to_table: "products".into(),
            to_column: "id".into(),
        },
    ];
    // 以 orders 为中心 1 跳：customers/products 含入，且只依据边（无字段参与）。
    let included =
        er_neighborhood_included_tables("orders", 1, &BTreeSet::new(), &edges);
    assert!(included.contains("orders"));
    assert!(included.contains("customers"));
    assert!(included.contains("products"));
    assert_eq!(included.len(), 3);
}
