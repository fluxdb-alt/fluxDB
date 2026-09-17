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
            tables: vec![],
            include_views: true,
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
        let (tables, stmts) = partition_statements(&p, DatabaseKind::Postgres, &cancel).unwrap();
        let kids_and_parents: Vec<&str> = tables.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(kids_and_parents, vec!["parents", "kids"]);
        let kids = tables.iter().find(|t| t.key == "kids").unwrap();
        assert!(kids.has_ddl_create && kids.has_data && kids.has_constraints);
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn reconstruct_respects_overwrite_append_skip() {
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
        // a 覆盖，b 跳过
        let (sql, _w) = reconstruct_sql(
            &stmts,
            &[
                PerTableDecision { table: "a".into(), action: RestoreTableAction::Overwrite },
                PerTableDecision { table: "b".into(), action: RestoreTableAction::Skip },
            ],
        );
        assert!(sql.contains("DROP TABLE IF EXISTS `a`"));
        assert!(sql.contains("INSERT INTO `a`"));
        assert!(!sql.contains("`b`"), "b 被跳过不应出现，got: {sql}");
        assert!(sql.contains("SET @x = 1"));
        // a 追加：只保留 INSERT，无 DDL
        let (sql, _w) = reconstruct_sql(
            &stmts,
            &[PerTableDecision { table: "a".into(), action: RestoreTableAction::Append }],
        );
        assert!(sql.contains("INSERT INTO `a`"));
        assert!(!sql.contains("CREATE TABLE `a`"), "追加不应有 CREATE，got {sql}");
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
        // kids 覆盖但 parents 跳过 → kids 的 FK 约束应被剔除并告警
        let (sql, warnings) = reconstruct_sql(
            &stmts,
            &[
                PerTableDecision { table: "kids".into(), action: RestoreTableAction::Overwrite },
                PerTableDecision { table: "parents".into(), action: RestoreTableAction::Skip },
            ],
        );
        assert!(sql.contains("CREATE TABLE public.kids"));
        assert!(!sql.contains("ADD CONSTRAINT"), "引用表被跳过应剔除 FK，got {sql}");
        assert!(!warnings.is_empty());
        fs::remove_file(&p).unwrap();
    }
}
