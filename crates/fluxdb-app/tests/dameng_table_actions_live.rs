//! 达梦表右键操作的显式真实库冒烟测试。
//!
//! 仅在提供 `FLUXDB_DM_SMOKE=host:port:user:password` 时运行；测试只操作本次创建的
//! `FLUXDB_TEST_ACTION_*` 表，并在结束时清理。

use fluxdb_app::{
    AppCommand, AppController, AppEvent, CreateTableColumnField, CreateTableState,
    ForeignKeyCheckMode, TabKind, TableImportFormat, TableImportRequest,
    copy_table_sql_preview_with_source_ddl, create_table_provider, rename_table_sql_preview,
    truncate_table_sql_preview,
};
use fluxdb_connectors::{connector_for, database_backup, table_data_import};
use fluxdb_core::{
    BackupObjectKind, BackupObjectRef, BackupRequest, BackupScope, ConnectionConfig,
    ConnectionDraft, ConnectionId, DamengAdvancedOptions, DamengBasicOptions,
    DamengConnectionProfile, DatabaseKind, Endpoint, ObjectKind, ObjectPath, QueryExecutionOptions,
    QueryMode, QueryRequest, RestoreOptions, RestoreRequest, SecretRef,
};
use std::collections::BTreeMap;
use std::io::Write;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_DM_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    Some(ConnectionConfig {
        id: ConnectionId(9_901),
        name: "DM8 table actions".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port: parts[1].parse().ok()?,
            database: Some("DAMENG".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: parts[0].into(),
                port: parts[1].parse().ok()?,
                database: "DAMENG".into(),
                schema: parts[2].to_ascii_uppercase(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            advanced: DamengAdvancedOptions::default(),
            ssh: Default::default(),
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    })
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE 指向可达的 DM8"]
fn dameng_controller_removes_default_and_preserves_existing_definitions() {
    let config = smoke_config().expect("DM8 test connection");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM8 profile")
        .basic
        .schema
        .clone();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    // DM8 禁止分区表含自增列，分别验证两类表，不用重建表规避这一限制。
    for partitioned in [false, true] {
        let table = format!("FLUXDB_TEST_ATTR_{}_{partitioned}", &suffix[..8]);
        let qualified = format!("\"{schema}\".\"{table}\"");
        let identity = if partitioned { "" } else { "IDENTITY(10,5)" };
        let partition = if partitioned {
            "PARTITION BY RANGE(\"ID\") (PARTITION P_LOW VALUES LESS THAN(1000), PARTITION P_HIGH VALUES LESS THAN(MAXVALUE))"
        } else {
            ""
        };
        execute(
            &config,
            &schema,
            &format!(
                "CREATE TABLE {qualified} (\"ID\" INT {identity} PRIMARY KEY, \
             \"TITLE\" VARCHAR(30) DEFAULT 'seed' NOT NULL) {partition}"
            ),
        );
        let result = std::panic::catch_unwind(|| {
            let connector = connector_for(&config).expect("DM8 connector");
            let request = |text: String| QueryRequest {
                connection_id: config.id,
                database: Some("DAMENG".into()),
                schema: Some(schema.clone()),
                session_id: None,
                text,
                mode: QueryMode::All,
                options: QueryExecutionOptions {
                    split_statements: false,
                    ..Default::default()
                },
            };
            let trigger_sql = format!(
                "CREATE TRIGGER \"{schema}\".\"TRG_{table}\" BEFORE INSERT ON {qualified} FOR EACH ROW BEGIN NULL; END;"
            );
            let trigger_result = connector
                .execute(&request(trigger_sql))
                .expect("create trigger");
            assert!(
                trigger_result
                    .summaries
                    .iter()
                    .all(|summary| summary.success)
            );
            execute(
                &config,
                &schema,
                &format!(
                    "INSERT INTO {qualified} ({}) VALUES ({})",
                    if partitioned { "\"ID\"" } else { "\"TITLE\"" },
                    if partitioned { "10" } else { "'seed'" },
                ),
            );
            let path = ObjectPath {
                connection_id: config.id,
                database: Some("DAMENG".into()),
                schema: Some(schema.clone()),
                name: table.clone(),
                kind: ObjectKind::Table,
            };
            let mut controller = AppController::new();
            controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
            let tab_id = match controller.dispatch(AppCommand::OpenDesignTable(path.clone())) {
                AppEvent::TabOpened(tab_id) => tab_id,
                AppEvent::Failed(error) => panic!("open DM8 designer: {}", error.message),
                _ => panic!("expected table design tab"),
            };
            let TabKind::CreateTable(create) = &controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("design tab")
                .kind
            else {
                panic!("table design state");
            };
            let column = create
                .columns
                .iter()
                .find(|column| column.name == "TITLE")
                .expect("TITLE column");
            assert_eq!(column.default_value, "'seed'");
            let column_id = column.id;
            controller.dispatch(AppCommand::SetCreateTableColumnField {
                tab_id,
                column_id,
                field: CreateTableColumnField::DefaultValue,
                value: String::new(),
            });
            let TabKind::CreateTable(create) = &controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("design tab")
                .kind
            else {
                panic!("table design state");
            };
            assert_eq!(
                create.sql_preview().expect("default removal preview"),
                format!("ALTER TABLE {qualified} ALTER COLUMN \"TITLE\" DROP DEFAULT;")
            );
            let event = controller.dispatch(AppCommand::ApplyCreateTable(tab_id));
            assert!(
                matches!(event, AppEvent::CreateTableApplied(_)),
                "{event:?}"
            );
            controller.dispatch(AppCommand::FinishCreateTableApply {
                tab_id,
                result: Ok(()),
            });
            let TabKind::CreateTable(refreshed) = &controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("refreshed design tab")
                .kind
            else {
                panic!("table design state");
            };
            assert!(
                refreshed
                    .columns
                    .iter()
                    .find(|column| column.name == "TITLE")
                    .expect("refreshed TITLE")
                    .default_value
                    .is_empty()
            );
            assert_eq!(refreshed.validation_error(), Some("没有需要保存的变更"));
            let page = connector
                .load_data(&path, 0, 10, &[], &[])
                .expect("existing data");
            assert_eq!(page.rows.len(), 1);
            assert_eq!(page.rows[0].values[0].display_label(), "10");
            assert_eq!(page.rows[0].values[1].display_label(), "seed");
            let ddl = connector.table_ddl(&path).expect("updated DDL");
            assert!(!ddl.contains("DEFAULT 'seed'"), "{ddl}");
            assert!(ddl.contains("PRIMARY KEY"));
            assert!(!page.columns[1].nullable);
            let triggers = connector.execute(&request(format!(
                "SELECT TRIGGER_NAME, STATUS FROM ALL_TRIGGERS WHERE OWNER='{schema}' AND TABLE_NAME='{table}'"
            ))).expect("trigger metadata");
            assert_eq!(triggers.results[0].rows.len(), 1);
            assert_eq!(
                triggers.results[0].rows[0].values[0].display_label(),
                format!("TRG_{table}")
            );
            let partitions = connector.execute(&request(format!(
                "SELECT PARTITION_NAME FROM ALL_TAB_PARTITIONS WHERE TABLE_OWNER='{schema}' AND TABLE_NAME='{table}' ORDER BY PARTITION_NAME"
            ))).expect("partition metadata");
            if partitioned {
                let names = partitions.results[0]
                    .rows
                    .iter()
                    .map(|row| row.values[0].display_label())
                    .collect::<Vec<_>>();
                assert_eq!(names, vec!["P_HIGH", "P_LOW"]);
            }
            // 再次保存还必须使用已刷新的结构基线，不能误判自己的 DROP DEFAULT 为外部变更。
            controller.dispatch(AppCommand::SetCreateTableColumnField {
                tab_id,
                column_id,
                field: CreateTableColumnField::DefaultValue,
                value: "'renewed'".into(),
            });
            let event = controller.dispatch(AppCommand::ApplyCreateTable(tab_id));
            assert!(
                matches!(event, AppEvent::CreateTableApplied(_)),
                "{event:?}"
            );
            execute(
                &config,
                &schema,
                &if partitioned {
                    format!("INSERT INTO {qualified} (\"ID\") VALUES (20)")
                } else {
                    format!("INSERT INTO {qualified} (\"TITLE\") VALUES (DEFAULT)")
                },
            );
            let page = connector
                .load_data(&path, 0, 10, &[], &[])
                .expect("new default data");
            assert_eq!(page.rows.len(), 2);
            assert_eq!(
                page.rows[1].values[0].display_label(),
                if partitioned { "20" } else { "15" }
            );
            assert_eq!(page.rows[1].values[1].display_label(), "renewed");
        });
        execute(&config, &schema, &format!("DROP TABLE {qualified}"));
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

fn execute(config: &ConnectionConfig, schema: &str, text: &str) {
    let connector = connector_for(config).expect("DM connector");
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.into()),
            session_id: None,
            text: text.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        })
        .expect("execute DM SQL");
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        panic!("DM SQL failed: {}\n{}", summary.message, text);
    }
}

fn execute_best_effort(config: &ConnectionConfig, schema: &str, text: &str) {
    let connector = connector_for(config).expect("DM connector");
    let _ = connector.execute(&QueryRequest {
        connection_id: config.id,
        database: Some("DAMENG".into()),
        schema: Some(schema.into()),
        session_id: None,
        text: text.into(),
        mode: QueryMode::All,
        options: QueryExecutionOptions {
            continue_on_error: false,
            split_statements: true,
            ..QueryExecutionOptions::default()
        },
    });
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_table_actions_create_rename_copy_truncate_backup_and_drop() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM profile")
        .basic
        .schema
        .clone();
    let source = "FLUXDB_TEST_ACTION_SOURCE";
    let renamed = "FLUXDB_TEST_ACTION_RENAMED";
    let copied = "FLUXDB_TEST_ACTION_COPY";
    let qualified = |name: &str| format!("\"{schema}\".\"{name}\"");

    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {}", qualified(copied)),
    );
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {}", qualified(renamed)),
    );
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {}", qualified(source)),
    );

    let result = std::panic::catch_unwind(|| {
        let mut create =
            CreateTableState::new(config.id, Some("DAMENG".into()), DatabaseKind::Dameng);
        create.schema = schema.clone();
        create.table_name = source.into();
        create.columns[0].name = "ID".into();
        create.columns[0].data_type = "int".into();
        create.columns[0].auto_increment = true;
        create.columns[0].nullable = false;
        create.add_column();
        let name = create.columns.last_mut().expect("name column");
        name.name = "NAME".into();
        name.data_type = "varchar".into();
        name.length = "80".into();
        name.nullable = false;
        let create_sql = create_table_provider(DatabaseKind::Dameng)
            .sql_preview(&create)
            .expect("DM create table SQL");
        execute(&config, &schema, &create_sql);
        execute(
            &config,
            &schema,
            &format!(
                "CREATE INDEX \"IDX_{}_NAME\" ON {} (\"NAME\")",
                source,
                qualified(source)
            ),
        );
        execute(
            &config,
            &schema,
            &format!(
                "INSERT INTO {} (\"NAME\") VALUES ('alpha')",
                qualified(source)
            ),
        );

        execute(
            &config,
            &schema,
            &rename_table_sql_preview(DatabaseKind::Dameng, Some(&schema), source, renamed)
                .expect("DM rename SQL"),
        );
        let connector = connector_for(&config).expect("DM connector");
        let source_path = ObjectPath {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.clone()),
            name: renamed.into(),
            kind: ObjectKind::Table,
        };
        let ddl = connector.table_ddl(&source_path).expect("DM source DDL");
        assert!(ddl.contains("IDENTITY(1,1)"));
        assert!(ddl.contains("CREATE INDEX"));
        let copy_sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::Dameng,
            Some(&schema),
            renamed,
            copied,
            true,
            Some(&ddl),
        )
        .expect("DM copy table SQL");
        execute(&config, &schema, &copy_sql);

        let copied_path = ObjectPath {
            name: copied.into(),
            ..source_path.clone()
        };
        let page = connector
            .load_data(&copied_path, 0, 10, &[], &[])
            .expect("DM copied data");
        assert_eq!(page.rows.len(), 1);
        assert!(
            page.rows[0]
                .values
                .iter()
                .any(|value| value.display_label() == "alpha")
        );

        // 备份在清空之前做：备份对象要确实带数据。
        let output = std::env::temp_dir().join(format!(
            "fluxdb-dm-table-actions-{}.sql",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let request = BackupRequest {
            config: config.clone(),
            database: "DAMENG".into(),
            output: output.clone(),
            execution: fluxdb_core::BackupExecution::SqlDump,
            tool: Default::default(),
            tool_version: None,
            scope: BackupScope::Objects(vec![BackupObjectRef {
                schema: Some(schema.clone()),
                kind: BackupObjectKind::Table,
                name: copied.into(),
            }]),
            include_schema: true,
            include_data: true,
            include_routines: false,
            single_transaction: true,
            lock_tables: false,
            include_owner: false,
            include_acl: false,
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        database_backup(DatabaseKind::Dameng)
            .expect("DM backup provider")
            .backup(&request, &cancel, &mut |_| {})
            .expect("DM logical backup");
        let backup = std::fs::read_to_string(&output).expect("read DM backup");
        assert!(backup.contains("CREATE TABLE"));
        assert!(backup.contains("INSERT INTO"));
        let _ = std::fs::remove_file(output);

        execute(
            &config,
            &schema,
            &truncate_table_sql_preview(
                DatabaseKind::Dameng,
                Some(&schema),
                copied,
                false,
                ForeignKeyCheckMode::Default,
            )
            .expect("DM truncate SQL"),
        );
        let empty = connector
            .load_data(&copied_path, 0, 10, &[], &[])
            .expect("DM truncated data");
        assert!(empty.rows.is_empty());
    });

    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {}", qualified(copied)),
    );
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {}", qualified(renamed)),
    );
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {}", qualified(source)),
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_imports_txt_csv_json_and_xml_export_formats() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM profile")
        .basic
        .schema
        .clone();
    let table = "FLUXDB_TEST_IMPORT_FORMATS";
    let qualified = format!("\"{schema}\".\"{table}\"");
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {qualified}"),
    );
    execute(
        &config,
        &schema,
        &format!(
            "CREATE TABLE {qualified} (\"ID\" INT IDENTITY(1,1) PRIMARY KEY, \
             \"NAME\" VARCHAR(40), \"AMOUNT\" DECIMAL(10,2), \"CREATED_AT\" TIMESTAMP)"
        ),
    );
    let result = std::panic::catch_unwind(|| {
        let dir = std::env::temp_dir().join(format!("fluxdb-import-live-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for (format, extension, body) in [
            (
                TableImportFormat::Txt,
                "txt",
                "ID\tNAME\tAMOUNT\tCREATED_AT\n9999\talpha\t12.50\t2026-01-02 03:04:05\nNULL\tbeta\tNULL\tNULL\n",
            ),
            (
                TableImportFormat::Csv,
                "csv",
                "ID,NAME,AMOUNT,CREATED_AT\n9999,gamma,13.25,2026-02-03 04:05:06\nNULL,delta,NULL,NULL\n",
            ),
            (
                TableImportFormat::Json,
                "json",
                r#"[{"ID":9999,"NAME":"epsilon","AMOUNT":14.5,"CREATED_AT":"2026-03-04 05:06:07"},{"ID":null,"NAME":"zeta","AMOUNT":null,"CREATED_AT":null}]"#,
            ),
            (
                TableImportFormat::Xml,
                "xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><rows><row><field name="ID">9999</field><field name="NAME">eta</field><field name="AMOUNT">15.75</field><field name="CREATED_AT">2026-04-05 06:07:08</field></row><row><field name="ID">NULL</field><field name="NAME">theta</field><field name="AMOUNT">NULL</field><field name="CREATED_AT">NULL</field></row></rows>"#,
            ),
        ] {
            let path = dir.join(format!("sample.{extension}"));
            let mut file = std::fs::File::create(&path).expect("sample file");
            file.write_all(body.as_bytes()).expect("write sample");
            let object = ObjectPath {
                connection_id: config.id,
                database: Some("DAMENG".into()),
                schema: Some(schema.clone()),
                name: table.into(),
                kind: ObjectKind::Table,
            };
            let report = table_data_import(DatabaseKind::Dameng)
                .expect("DM import provider")
                .import(
                    &TableImportRequest {
                        config: config.clone(),
                        object,
                        source: path,
                        format,
                        options: fluxdb_core::TableImportOptions::new(),
                        column_mapping: Vec::new(),
                    },
                    &std::sync::atomic::AtomicBool::new(false),
                    &mut |_| {},
                )
                .expect("DM import");
            assert_eq!(report.imported_rows, 2, "{format:?}");
            assert!(report.warnings.iter().any(|warning| warning.contains("ID")));
        }
        let connector = connector_for(&config).expect("DM connector");
        let path = ObjectPath {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.clone()),
            name: table.into(),
            kind: ObjectKind::Table,
        };
        let page = connector
            .load_data(&path, 0, 20, &[], &[])
            .expect("imported rows");
        assert_eq!(page.rows.len(), 8);
        assert!(
            page.rows
                .iter()
                .all(|row| row.values[0].display_label() != "9999")
        );
        assert!(
            page.rows
                .iter()
                .any(|row| row.values[1].display_label() == "alpha")
        );
        assert!(
            page.rows
                .iter()
                .any(|row| row.values[2].display_label() == "12.50")
        );
        assert!(
            page.rows
                .iter()
                .any(|row| row.values[3].display_label().starts_with("2026-01-02"))
        );
    });
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {qualified}"),
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_backup_restores_into_new_schema() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM profile")
        .basic
        .schema
        .clone();
    let source_table = "FLUXDB_TEST_RESTORE_SOURCE";
    let source_qualified = format!("\"{schema}\".\"{source_table}\"");
    let target_schema = format!("FLUXDB_RESTORE_{}", std::process::id());
    execute_best_effort(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {source_qualified}"),
    );
    execute_best_effort(
        &config,
        &schema,
        &format!("DROP SCHEMA IF EXISTS \"{target_schema}\" RESTRICT"),
    );
    execute(
        &config,
        &schema,
        &format!("CREATE TABLE {source_qualified} (\"ID\" INT PRIMARY KEY, \"NAME\" VARCHAR(30))"),
    );
    execute(
        &config,
        &schema,
        &format!("INSERT INTO {source_qualified} VALUES (1, 'alpha')"),
    );
    let backup_path =
        std::env::temp_dir().join(format!("fluxdb-dm-restore-{}.sql", std::process::id()));
    let _ = std::fs::remove_file(&backup_path);
    let result = std::panic::catch_unwind(|| {
        let provider = database_backup(DatabaseKind::Dameng).expect("DM backup provider");
        let backup_request = BackupRequest {
            config: config.clone(),
            database: "DAMENG".into(),
            output: backup_path.clone(),
            execution: fluxdb_core::BackupExecution::SqlDump,
            tool: Default::default(),
            tool_version: None,
            scope: BackupScope::Objects(vec![BackupObjectRef {
                schema: Some(schema.clone()),
                kind: BackupObjectKind::Table,
                name: source_table.into(),
            }]),
            include_schema: true,
            include_data: true,
            include_routines: false,
            single_transaction: true,
            lock_tables: false,
            include_owner: false,
            include_acl: false,
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let manifest = provider
            .backup(&backup_request, &cancel, &mut |_| {})
            .expect("DM backup");
        let restore_request = RestoreRequest {
            config: config.clone(),
            source: backup_path.clone(),
            target: target_schema.clone(),
            create_target: true,
            tool: Default::default(),
            manifest: Some(manifest),
            table_decisions: Vec::new(),
            options: RestoreOptions::default(),
        };
        let plan = provider
            .inspect_restore(&restore_request, &cancel)
            .expect("DM restore plan");
        let outcome = provider
            .restore(&restore_request, &plan, &cancel, &mut |_| {})
            .expect("DM restore");
        assert_eq!(outcome.total_objects, 1);
        let connector = connector_for(&config).expect("DM connector");
        let path = ObjectPath {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(target_schema.clone()),
            name: source_table.into(),
            kind: ObjectKind::Table,
        };
        let page = connector
            .load_data(&path, 0, 10, &[], &[])
            .expect("restored data");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].values[1].display_label(), "alpha");
    });
    let _ = std::fs::remove_file(backup_path);
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {source_qualified}"),
    );
    execute_best_effort(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS \"{target_schema}\".\"{source_table}\""),
    );
    execute_best_effort(
        &config,
        &schema,
        &format!("DROP SCHEMA IF EXISTS \"{target_schema}\" RESTRICT"),
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_design_table_applies_alter_statements() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM profile")
        .basic
        .schema
        .clone();
    let table = "FLUXDB_TEST_ACTION_DESIGN";
    let qualified = format!("\"{schema}\".\"{table}\"");

    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {qualified}"),
    );

    let result = std::panic::catch_unwind(|| {
        execute(
            &config,
            &schema,
            &format!(
                "CREATE TABLE {qualified} (\"ID\" INT IDENTITY(1,1) NOT NULL, \
                 \"NAME\" VARCHAR(50), PRIMARY KEY (\"ID\"))"
            ),
        );

        let connector = connector_for(&config).expect("DM connector");
        let path = ObjectPath {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.clone()),
            name: table.into(),
            kind: ObjectKind::Table,
        };
        let columns = connector
            .list_completion_columns(None, Some(&schema), table)
            .expect("DM columns");
        let indexes = connector.list_indexes(&path).expect("DM indexes");
        let foreign_keys = connector.list_foreign_keys(&path).expect("DM foreign keys");
        let ddl = connector.table_ddl(&path).expect("DM DDL");

        let mut create = CreateTableState::design(
            path.clone(),
            DatabaseKind::Dameng,
            columns,
            indexes,
            foreign_keys,
            Vec::new(),
            Some(ddl),
        );
        // 改字段名、加字段、改注释：这些都要落到真实 ALTER 语句上。
        create.columns[1].name = "TITLE".into();
        create.columns[1].comment = "标题".into();
        create.add_column();
        let note = create.columns.last_mut().expect("note column");
        note.name = "NOTE".into();
        note.data_type = "varchar".into();
        note.length = "200".into();

        let statements = fluxdb_app::create_table_provider(DatabaseKind::Dameng)
            .design_statements(&create)
            .expect("DM design statements");
        assert!(
            statements
                .iter()
                .any(|sql| sql.contains("RENAME COLUMN") && sql.contains("TITLE"))
        );
        assert!(
            statements
                .iter()
                .any(|sql| sql.contains("ADD COLUMN \"NOTE\" varchar(200)"))
        );
        for sql in &statements {
            execute(&config, &schema, sql);
        }

        let after = connector
            .list_completion_columns(None, Some(&schema), table)
            .expect("DM columns after design");
        assert!(after.iter().any(|column| column.name == "TITLE"));
        assert!(after.iter().any(|column| column.name == "NOTE"));
        assert!(
            after
                .iter()
                .find(|column| column.name == "TITLE")
                .and_then(|column| column.comment.as_deref())
                .is_some_and(|comment| comment == "标题")
        );
    });

    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {qualified}"),
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// 走生产命令派发路径（AppCommand → AppController）验证达梦表操作。
#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_controller_dispatch_runs_table_actions() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM profile")
        .basic
        .schema
        .clone();
    let table = "FLUXDB_TEST_ACTION_DISPATCH";
    let qualified = format!("\"{schema}\".\"{table}\"");

    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {qualified}"),
    );

    let result = std::panic::catch_unwind(|| {
        let mut controller = AppController::new();
        let AppEvent::ConnectionCreated(created) =
            controller.dispatch(AppCommand::CreateConnection(ConnectionDraft {
                name: "DM8 dispatch".into(),
                kind: DatabaseKind::Dameng,
                endpoint: config.endpoint.clone(),
                options: BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: config.dameng_profile.clone(),
                sqlserver_profile: None,
                mongo_profile: None,
                oracle_profile: None,
            }))
        else {
            panic!("CreateConnection must return ConnectionCreated");
        };

        let path = ObjectPath {
            connection_id: created.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.clone()),
            name: table.into(),
            kind: ObjectKind::Table,
        };

        // 先建源表供复制表用（复制需要真实 DDL）。
        execute(
            &config,
            &schema,
            &format!(
                "CREATE TABLE {qualified} (\"ID\" INT IDENTITY(1,1) NOT NULL, \
                 \"NAME\" VARCHAR(50), PRIMARY KEY (\"ID\"))"
            ),
        );
        let ddl = controller
            .load_table_ddl(&path)
            .expect("controller loads DM DDL");
        let copy_sql = fluxdb_app::copy_table_sql_preview_with_source_ddl(
            DatabaseKind::Dameng,
            Some(&schema),
            table,
            &format!("{table}_COPY"),
            true,
            Some(&ddl),
        )
        .expect("DM copy SQL");
        execute(&config, &schema, &copy_sql);
        let copied = ObjectPath {
            name: format!("{table}_COPY"),
            ..path.clone()
        };

        // 重命名：走生产派发。
        let event = controller.dispatch(AppCommand::RenameTable {
            object: path.clone(),
            new_name: format!("{table}_RENAMED"),
        });
        assert!(
            matches!(event, AppEvent::TableRenamed { .. }),
            "rename event"
        );

        // 清空复制表：走生产派发。
        let event = controller.dispatch(AppCommand::TruncateTable {
            object: copied,
            foreign_key_check: ForeignKeyCheckMode::Default,
            restart_identity: false,
        });
        assert!(
            matches!(event, AppEvent::TableTruncated(_)),
            "truncate event"
        );

        // 删除：走生产派发。
        let event = controller.dispatch(AppCommand::DropTable {
            object: ObjectPath {
                name: format!("{table}_RENAMED"),
                ..path.clone()
            },
            foreign_key_check: ForeignKeyCheckMode::Default,
        });
        assert!(matches!(event, AppEvent::TableDropped(_)), "drop event");
    });

    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS \"{schema}\".\"{table}_COPY\""),
    );
    execute(
        &config,
        &schema,
        &format!("DROP TABLE IF EXISTS {qualified}"),
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
