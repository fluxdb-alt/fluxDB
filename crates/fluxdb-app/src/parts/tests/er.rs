    // ER 关系图「画布原型」加载编排（er_service::load_er_graph_in_background）的集成测试。
    // 用临时 SQLite 建 3 张表 + 1 条外键，验证表节点与连线都正确生成。

    #[test]
    fn load_er_graph_reads_tables_and_foreign_keys() {
        let path = temp_sqlite_path("er-graph");
        // 清理可能残留的临时文件（上一次失败中断时）。
        let _ = std::fs::remove_file(&path);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE customers (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL
                )",
            )
            .execute(&mut connection)
            .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE orders (
                    id INTEGER PRIMARY KEY,
                    product_id INTEGER REFERENCES products(id),
                    customer_id INTEGER REFERENCES customers(id)
                )",
            )
            .execute(&mut connection)
            .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let config = ConnectionConfig {
            id: ConnectionId(999),
            name: "er-test".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        };
        // load_er_graph 内部自起连接，测试无需额外 runtime。
        // 使用同步连接器读取（其内部自建 tokio runtime，非阻塞当前线程）。
        let graph = std::thread::scope(|scope| {
            scope.spawn(|| load_er_graph_in_background(&config, Some("main"), None))
                .join()
                .expect("er load thread panicked")
                .expect("load_er_graph_in_background should succeed")
        });

        // 三张表都成为节点，且按名称稳定排序。
        let table_names: Vec<&str> = graph.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            table_names,
            vec!["customers", "orders", "products"],
            "应读到三张表且按名排序"
        );

        // 外键连线：orders.product_id -> products.id、orders.customer_id -> customers.id。
        let mut edges: Vec<(&str, &str, &str, &str)> = graph
            .edges
            .iter()
            .map(|e| {
                (
                    e.from_table.as_str(),
                    e.from_column.as_str(),
                    e.to_table.as_str(),
                    e.to_column.as_str(),
                )
            })
            .collect();
        edges.sort();
        assert!(
            edges.contains(&("customers", "id", "orders", "customer_id"))
                || edges.contains(&("orders", "customer_id", "customers", "id")),
            "orders 应连到 customers，实际边：{edges:?}"
        );
        assert!(
            edges.contains(&("orders", "product_id", "products", "id"))
                || edges.contains(&("products", "id", "orders", "product_id")),
            "orders 应连到 products，实际边：{edges:?}"
        );

        // 主键列被正确标记，便于节点内高亮。
        let orders = graph.tables.iter().find(|t| t.name == "orders").unwrap();
        assert!(orders.columns.iter().any(|c| c.name == "id" && c.primary_key));

        let _ = std::fs::remove_file(&path);
    }

    // 桌面端 SQLite Demo 连接实际打开 ER 标签时会读这个 demo 库；
    // 用它做端到端数据验证，确保真实示例库能出 3 表 2 连线。
    #[test]
    fn load_er_graph_reads_demo_database() {
        let demo_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("demo.db");
        if !demo_path.exists() {
            eprintln!("跳过：demo.db 不存在");
            return;
        }
        let config = ConnectionConfig {
            id: ConnectionId(1001),
            name: "er-demo".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: demo_path,
                read_only: true,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        };
        let graph = std::thread::scope(|scope| {
            scope
                .spawn(|| load_er_graph_in_background(&config, Some("main"), None))
                .join()
                .expect("er load thread panicked")
                .expect("load_er_graph_in_background(demo) should succeed")
        });

        assert_eq!(graph.tables.len(), 3, "demo 库应有 3 张表：{graph:?}");
        assert!(
            graph.tables.iter().all(|t| !t.columns.is_empty()),
            "demo 表都应带列"
        );
        // demo.db 的 orders 经 PRAGMA foreign_key 声明了两条外键。
        assert_eq!(graph.edges.len(), 2, "demo 库应有 2 条外键连线：{graph:?}");
        assert!(
            graph.edges.iter().all(|e| e.from_table != e.to_table),
            "demo 无自关联边"
        );
    }
