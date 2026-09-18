#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn sqlite_backup_restore_roundtrip_and_no_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "fluxdb-restore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source.db");
        let backup = dir.join("legacy.sql");
        let target = dir.join("restored.db");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut conn = sqlx::SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&source).create_if_missing(true)).await.unwrap();
            sqlx::raw_sql("CREATE TABLE t(id INTEGER PRIMARY KEY, text TEXT, bytes BLOB); INSERT INTO t VALUES (1,'中文',X'00FF'),(2,NULL,NULL); CREATE VIEW v AS SELECT * FROM t;").execute(&mut conn).await.unwrap();
        });
        let config = ConnectionConfig {
            id: ConnectionId(1),
            name: "temporary".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: source,
                read_only: false,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        };
        let request = BackupRequest {
            config: config.clone(),
            database: "main".into(),
            output: backup.clone(),
            execution: BackupExecution::SqliteBinary,
            tool: PathBuf::new(),
            tool_version: None,
            scope: BackupScope::SqliteSnapshot,
            include_schema: true,
            include_data: true,
            include_routines: false,
            single_transaction: true,
            lock_tables: false,
            include_owner: false,
            include_acl: false,
        };
        let cancel = AtomicBool::new(false);
        let provider = database_backup(DatabaseKind::Sqlite).unwrap();
        let meta = provider.backup(&request, &cancel, &mut |_| {}).unwrap();
        assert!(provider.backup(&request, &cancel, &mut |_| {}).is_err());
        let restore = RestoreRequest {
            config,
            source: backup,
            target: target.to_string_lossy().into_owned(),
            create_target: true,
            tool: PathBuf::new(),
            manifest: Some(meta),
            table_decisions: Vec::new(),
            options: Default::default(),
        };
        let plan = provider.inspect_restore(&restore, &cancel).unwrap();
        assert_eq!(plan.format, BackupFormat::SqliteBinary);
        provider
            .restore(&restore, &plan, &cancel, &mut |_| {})
            .unwrap();
        assert!(
            provider
                .restore(&restore, &plan, &cancel, &mut |_| {})
                .is_err()
        );
        sqlite_runtime(&target, |conn| {
            Box::pin(async move {
                let rows: Vec<(i64, Option<String>, Option<Vec<u8>>)> =
                    sqlx::query_as("SELECT * FROM v ORDER BY id")
                        .fetch_all(conn)
                        .await
                        .map_err(io_error)?;
                assert_eq!(
                    rows,
                    vec![
                        (1, Some("中文".into()), Some(vec![0, 255])),
                        (2, None, None)
                    ]
                );
                Ok(())
            })
        })
        .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod partition_tests {
    use super::*;

    fn tmp_sql(name: &str, sql: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("fluxdb-part-{}-{name}.sql", std::process::id()));
        fs::write(&p, sql).unwrap();
        p
    }

    #[test]
    fn mysql_partitioner_groups_per_table() {
        let p = tmp_sql("mysql", r#"
SET @OLD_FOREIGN_KEY_CHECKS=@@FOREIGN_KEY_CHECKS;
DROP TABLE IF EXISTS `orders`;
CREATE TABLE `orders` (id INT PRIMARY KEY);
INSERT INTO `orders` VALUES (1),(2);
DROP TABLE IF EXISTS `users`;
CREATE TABLE `users` (id INT PRIMARY KEY);
INSERT INTO `users` VALUES (10);
UNLOCK TABLES;
"#);
        let cancel = AtomicBool::new(false);
        let (tables, stmts) = partition_statements(&p, DatabaseKind::MySql, &cancel).unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(names, vec!["orders", "users"]);
        let orders = tables.iter().find(|t| t.key == "orders").unwrap();
        assert!(orders.has_ddl_drop && orders.has_ddl_create && orders.has_data);
        let users = tables.iter().find(|t| t.key == "users").unwrap();
        assert!(!users.has_constraints);
        // 非表语句标记为 NonTable
        let non_table = stmts.iter().filter(|s| s.kind == StmtKind::NonTable).count();
        assert_eq!(non_table, 2); // SET + UNLOCK
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn pg_inserts_partitioner_handles_interleaved_phases() {
        // pg_dump --inserts 输出：先所有 CREATE，再数据，再 ADD CONSTRAINT，再 setval。
        let p = tmp_sql("pg", r#"
SET default_table_access_method = heap;
CREATE TABLE public.parents (id integer PRIMARY KEY);
CREATE TABLE public.kids (id integer PRIMARY KEY, parent_id integer);
INSERT INTO public.parents VALUES (1);
INSERT INTO public.kids VALUES (10, 1);
ALTER TABLE ONLY public.kids ADD CONSTRAINT kids_parent_fk FOREIGN KEY (parent_id) REFERENCES public.parents(id);
SELECT pg_catalog.setval('public.parents_id_seq', 1, true);
"#);
        let cancel = AtomicBool::new(false);
        let (tables, _stmts) = partition_statements(&p, DatabaseKind::Postgres, &cancel).unwrap();
        let kids_and_parents: Vec<&str> = tables.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(kids_and_parents, vec!["parents", "kids"]);
        let kids = tables.iter().find(|t| t.key == "kids").unwrap();
        assert!(kids.has_ddl_create && kids.has_data && kids.has_constraints);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn pg_copy_dump_is_rejected_for_per_table_restore() {
        // COPY 数据行被切分器吞掉，按对象分桶会静默丢数据，必须报错引导整库恢复。
        let p = tmp_sql("pgcopy", r#"
CREATE TABLE public.notes (id integer, body text);
COPY public.notes (id, body) FROM stdin;
1	中文; 'quoted'
2	\N
\.
"#);
        let cancel = AtomicBool::new(false);
        let error = partition_statements(&p, DatabaseKind::Postgres, &cancel).unwrap_err();
        assert!(error.message.contains("COPY"), "报错应点名 COPY，got: {}", error.message);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn reconstruct_respects_recreate_append_skip() {
        let p = tmp_sql("recon", r#"
SET @x = 1;
DROP TABLE IF EXISTS `a`;
CREATE TABLE `a` (id INT);
INSERT INTO `a` VALUES (1);
DROP TABLE IF EXISTS `b`;
CREATE TABLE `b` (id INT);
INSERT INTO `b` VALUES (2);
UNLOCK TABLES;
"#);
        let cancel = AtomicBool::new(false);
        let (_t, stmts) = partition_statements(&p, DatabaseKind::MySql, &cancel).unwrap();
        let sk_a: std::collections::BTreeSet<String> = ["a".to_string()].into();
        // a 重建，b 跳过
        let (sql, _w) = reconstruct_sql(
            DatabaseKind::MySql,
            &stmts,
            &[
                PerTableDecision { table: "a".into(), action: RestoreTableAction::Recreate },
                PerTableDecision { table: "b".into(), action: RestoreTableAction::Skip },
            ],
            &sk_a,
        );
        assert!(sql.contains("DROP TABLE IF EXISTS `a`"));
        assert!(sql.contains("INSERT INTO `a`"));
        assert!(!sql.contains("`b`"), "b 被跳过不应出现，got: {sql}");
        assert!(sql.contains("SET @x = 1"));
        // a 追加：只保留 INSERT，无 DDL
        let (sql, _w) = reconstruct_sql(
            DatabaseKind::MySql,
            &stmts,
            &[PerTableDecision { table: "a".into(), action: RestoreTableAction::Append }],
            &sk_a,
        );
        assert!(sql.contains("INSERT INTO `a`"));
        assert!(!sql.contains("CREATE TABLE `a`"), "追加不应有 CREATE，got {sql}");
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn truncate_and_load_emits_delete_before_data_mysql() {
        let p = tmp_sql("trunc", r#"
CREATE TABLE `a` (id INT);
INSERT INTO `a` VALUES (1);
INSERT INTO `a` VALUES (2);
"#);
        let cancel = AtomicBool::new(false);
        let (_t, stmts) = partition_statements(&p, DatabaseKind::MySql, &cancel).unwrap();
        let sk: std::collections::BTreeSet<String> = ["a".to_string()].into();
        let (sql, _w) = reconstruct_sql(
            DatabaseKind::MySql,
            &stmts,
            &[PerTableDecision { table: "a".into(), action: RestoreTableAction::TruncateAndLoad }],
            &sk,
        );
        // 不重放 DDL；只清空一次并保留全部数据；MySQL 用 FK 开关包裹。
        assert!(!sql.contains("CREATE TABLE"), "清空后导入不应重放 CREATE，got {sql}");
        assert_eq!(sql.matches("DELETE FROM `a`").count(), 1, "只应清空一次，got {sql}");
        assert_eq!(sql.matches("INSERT INTO `a`").count(), 2);
        assert!(sql.starts_with("SET FOREIGN_KEY_CHECKS=0;"));
        assert!(sql.trim_end().ends_with("SET FOREIGN_KEY_CHECKS=1;"));
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn truncate_and_load_uses_schema_qualified_truncate_pg() {
        let p = tmp_sql("truncpg", r#"
CREATE TABLE public.a (id integer);
INSERT INTO public.a VALUES (1);
"#);
        let cancel = AtomicBool::new(false);
        let (_t, stmts) = partition_statements(&p, DatabaseKind::Postgres, &cancel).unwrap();
        let sk: std::collections::BTreeSet<String> = ["a".to_string()].into();
        let (sql, _w) = reconstruct_sql(
            DatabaseKind::Postgres,
            &stmts,
            &[PerTableDecision { table: "a".into(), action: RestoreTableAction::TruncateAndLoad }],
            &sk,
        );
        assert!(
            sql.contains("TRUNCATE TABLE \"public\".\"a\" RESTART IDENTITY;"),
            "PG 应生成 schema 限定的 TRUNCATE，got {sql}"
        );
        assert!(!sql.contains("FOREIGN_KEY_CHECKS"), "PG 不用 MySQL 的 FK 开关，got {sql}");
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn fk_constraint_dropped_when_referenced_table_skipped() {
        let p = tmp_sql("fk", r#"
CREATE TABLE public.parents (id integer PRIMARY KEY);
CREATE TABLE public.kids (id integer PRIMARY KEY, parent_id integer);
ALTER TABLE ONLY public.kids ADD CONSTRAINT k_fk FOREIGN KEY (parent_id) REFERENCES public.parents(id);
"#);
        let cancel = AtomicBool::new(false);
        let (_t, stmts) = partition_statements(&p, DatabaseKind::Postgres, &cancel).unwrap();
        // kids 重建但 parents 跳过 → kids 的 FK 约束应被剔除并告警（parents 不在 structure_keys）
        let sk: std::collections::BTreeSet<String> = ["kids".to_string()].into();
        let (sql, warnings) = reconstruct_sql(
            DatabaseKind::Postgres,
            &stmts,
            &[
                PerTableDecision { table: "kids".into(), action: RestoreTableAction::Recreate },
                PerTableDecision { table: "parents".into(), action: RestoreTableAction::Skip },
            ],
            &sk,
        );
        assert!(sql.contains("CREATE TABLE public.kids"));
        assert!(!sql.contains("ADD CONSTRAINT"), "引用表被跳过应剔除 FK，got {sql}");
        assert!(!warnings.is_empty());
        fs::remove_file(&p).unwrap();
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use std::path::PathBuf;

    /// 构造一个走 MockConnector 的 demo 配置：options["demo"]="true" 时
    /// `connector_for` 返回 mock 实现，其目标对象集为 Product/ProductCategory/Order/Customer。
    fn demo_config(kind: DatabaseKind) -> ConnectionConfig {
        let mut options = BTreeMap::new();
        options.insert("demo".to_string(), "true".to_string());
        ConnectionConfig {
            id: ConnectionId(1),
            name: "demo".into(),
            kind,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 3306,
                database: None,
            },
            credential_ref: None,
            options,
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        }
    }

    #[test]
    fn probe_reports_existence_and_content_without_decisions() {
        let p = std::env::temp_dir().join(format!(
            "fluxdb-probe-{}-mysql.sql",
            std::process::id()
        ));
        fs::write(
            &p,
            r#"
DROP TABLE IF EXISTS `Product`;
CREATE TABLE `Product` (id INT PRIMARY KEY);
INSERT INTO `Product` VALUES (1),(2);
CREATE TABLE `Widget` (id INT PRIMARY KEY);
"#,
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let provider = database_backup(DatabaseKind::MySql).unwrap();
        let request = RestoreRequest {
            config: demo_config(DatabaseKind::MySql),
            source: p.clone(),
            target: "main".into(),
            create_target: false,
            tool: PathBuf::new(),
            manifest: None,
            table_decisions: Vec::new(),
            options: Default::default(),
        };
        let probes = provider.probe_restore(&request, &cancel).unwrap();
        let product = probes.iter().find(|o| o.key == "Product").expect("Product");
        // 与 mock 目标同名 → 已存在；含 DDL 与数据。
        assert!(product.exists_in_target);
        assert!(product.has_ddl && product.has_data);
        let widget = probes.iter().find(|o| o.key == "Widget").expect("Widget");
        // mock 目标无此表 → 不存在；仅结构无数据。
        assert!(!widget.exists_in_target);
        assert!(widget.has_ddl && !widget.has_data);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn sqlite_probe_returns_empty() {
        let p = std::env::temp_dir().join(format!("fluxdb-probe-{}-sqlite.sql", std::process::id()));
        fs::write(&p, "CREATE TABLE t(id INT);\n").unwrap();
        let cancel = AtomicBool::new(false);
        let provider = database_backup(DatabaseKind::Sqlite).unwrap();
        let request = RestoreRequest {
            config: demo_config(DatabaseKind::Sqlite),
            source: p.clone(),
            target: "irrelevant".into(),
            create_target: true,
            tool: PathBuf::new(),
            manifest: None,
            table_decisions: Vec::new(),
            options: Default::default(),
        };
        assert!(provider.probe_restore(&request, &cancel).unwrap().is_empty());
        fs::remove_file(&p).unwrap();
    }
}
