//! MariaDB 真实连接集成测试。
//!
//! 仅在显式提供 `FLUXDB_MARIADB_SMOKE=host:port:user:password` 时运行；默认忽略，
//! 避免常规测试依赖 Podman 或外部数据库。测试会创建并删除
//! `fluxdb_mariadb_live_<pid>` 库，只清理本次创建的对象。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellUpdate, CellValue, ConnectionConfig, ConnectionId, CreateDatabaseRequest,
    CreatePrincipalInput, DatabaseKind, DatabaseUserIdentity, Endpoint, FilterOp, FilterSpec,
    MysqlBasicOptions, MysqlConnectionProfile, ObjectKind, ObjectPath, QueryExecutionOptions,
    QueryMode, QueryRequest, RowIdentity, RowUpdate, SecretRef, SortDirection, SortSpec,
    UserResourceLimits,
};
use std::collections::BTreeMap;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_MARIADB_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let port = parts[1].parse().ok()?;
    Some(
        ConnectionConfig {
            id: ConnectionId(9_911),
            name: "MariaDB live".into(),
            kind: DatabaseKind::MariaDb,
            endpoint: Endpoint::Tcp {
                host: parts[0].into(),
                port,
                database: None,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: Some(MysqlConnectionProfile {
                basic: MysqlBasicOptions {
                    host: parts[0].into(),
                    port,
                    database: String::new(),
                    username: parts[2].into(),
                    password: SecretRef::inline(parts[3]),
                },
                ..MysqlConnectionProfile::default()
            }),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
        .mysql_resolved(),
    )
}

fn execute(connector: &dyn fluxdb_core::Connector, config: &ConnectionConfig, sql: &str) {
    connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .unwrap_or_else(|error| panic!("execute failed: {sql}: {error}"));
}

fn query(
    connector: &dyn fluxdb_core::Connector,
    config: &ConnectionConfig,
    sql: &str,
) -> fluxdb_core::QueryExecutionResult {
    connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .expect("query")
}

fn query_text(
    connector: &dyn fluxdb_core::Connector,
    config: &ConnectionConfig,
    sql: &str,
) -> String {
    let result = query(connector, config, sql);
    result
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first())
        .map(CellValue::display_label)
        .unwrap_or_default()
}

#[test]
#[ignore = "需要 FLUXDB_MARIADB_SMOKE=host:port:user:password 指向可达的 MariaDB"]
fn mariadb_live_connection_objects_query_and_design() {
    let config = smoke_config().expect("FLUXDB_MARIADB_SMOKE must be host:port:user:password");
    let database = format!("fluxdb_mariadb_live_{}", std::process::id());
    let connector = connector_for(&config).expect("create MariaDB connector");

    // 类型必须保留 MariaDB，不能被 MySQL 连接器实现吞掉。
    assert_eq!(connector.kind(), DatabaseKind::MariaDb);
    connector
        .test_connection(&config)
        .expect("test MariaDB connection");

    let version = query_text(connector.as_ref(), &config, "SELECT VERSION()");
    assert!(
        version.to_ascii_lowercase().contains("mariadb"),
        "unexpected server version: {version}"
    );

    execute(
        connector.as_ref(),
        &config,
        &format!("DROP DATABASE IF EXISTS `{database}`"),
    );
    connector
        .create_database(&CreateDatabaseRequest {
            connection_id: config.id,
            name: database.clone(),
            charset: "utf8mb4".into(),
            collation: "utf8mb4_general_ci".into(),
            owner: String::new(),
            template: String::new(),
            path: None,
        })
        .expect("create MariaDB database");

    let databases = connector.list_objects(None).expect("list databases");
    assert!(databases.iter().any(|object| {
        object.path.kind == ObjectKind::Database && object.path.name == database
    }));

    execute(
        connector.as_ref(),
        &config,
        &format!(
            "CREATE TABLE `{database}`.`parent` (\
             `id` INT NOT NULL AUTO_INCREMENT, \
             `name` VARCHAR(80) NOT NULL, \
             `amount` DECIMAL(18,4) NULL, \
             PRIMARY KEY (`id`), \
             UNIQUE KEY `uq_parent_name` (`name`), \
             KEY `idx_parent_amount` (`amount`)\
             ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
        ),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!(
            "CREATE TABLE `{database}`.`child` (\
             `id` INT NOT NULL, \
             `parent_id` INT NOT NULL, \
             PRIMARY KEY (`id`), \
             CONSTRAINT `fk_child_parent` FOREIGN KEY (`parent_id`) \
             REFERENCES `{database}`.`parent` (`id`)\
             ) ENGINE=InnoDB"
        ),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!(
            "CREATE TRIGGER `{database}`.`trg_child_bi` BEFORE INSERT ON `{database}`.`child` \
             FOR EACH ROW SET NEW.parent_id = IFNULL(NEW.parent_id, 1)"
        ),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!(
            "INSERT INTO `{database}`.`parent` (`name`, `amount`) VALUES \
             ('alpha', 12.3400), ('中文', 56.7800), ('gamma', 99.0000)"
        ),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!("INSERT INTO `{database}`.`child` (`id`, `parent_id`) VALUES (10, 1)"),
    );

    let parent = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: "parent".into(),
        kind: ObjectKind::Table,
    };
    let child = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: "child".into(),
        kind: ObjectKind::Table,
    };

    let tables = connector
        .list_objects(Some(&ObjectPath {
            connection_id: config.id,
            database: Some(database.clone()),
            schema: None,
            name: database.clone(),
            kind: ObjectKind::Database,
        }))
        .expect("list tables");
    assert!(tables.iter().any(|object| object.path.name == "parent"));
    assert!(tables.iter().any(|object| object.path.name == "child"));

    // 分页与排序：按 id 升序，第二页只剩一行。
    let first = connector
        .load_data(
            &parent,
            0,
            2,
            &[SortSpec {
                field: "id".into(),
                direction: SortDirection::Asc,
            }],
            &[],
        )
        .expect("first page");
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    let second = connector
        .load_data(
            &parent,
            2,
            2,
            &[SortSpec {
                field: "id".into(),
                direction: SortDirection::Asc,
            }],
            &[],
        )
        .expect("second page");
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);

    // 筛选：中文值按等值命中，验证连接字符集与参数绑定。
    let filtered = connector
        .load_data(
            &parent,
            0,
            10,
            &[],
            &[FilterSpec {
                field: "name".into(),
                op: FilterOp::Eq,
                values: vec![CellValue::Text("中文".into())],
                enabled: true,
            }],
        )
        .expect("filtered page");
    assert_eq!(filtered.rows.len(), 1);

    let indexes = connector.list_indexes(&parent).expect("indexes");
    assert!(indexes.iter().any(|index| index.is_primary));
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "uq_parent_name" && index.is_unique)
    );
    let foreign_keys = connector.list_foreign_keys(&child).expect("foreign keys");
    assert!(foreign_keys.iter().any(|fk| fk.name == "fk_child_parent"));
    let triggers = connector.list_triggers(&child).expect("triggers");
    assert!(
        triggers
            .iter()
            .any(|trigger| trigger.name == "trg_child_bi")
    );
    let ddl = connector.table_ddl(&parent).expect("table ddl");
    assert!(ddl.contains("CREATE TABLE"));
    let design = connector
        .load_table_design_metadata(&parent)
        .expect("design metadata");
    assert!(design.details_loaded);
    assert!(design.columns.iter().any(|column| column.name == "name"));

    // 数据编辑链路：更新一行，再按主键删除。
    let mut identity = BTreeMap::new();
    identity.insert("id".to_string(), CellValue::I64(3));
    connector
        .apply_changes(&fluxdb_core::DataChangeSet {
            object: parent.clone(),
            inserts: Vec::new(),
            updates: vec![RowUpdate {
                identity: RowIdentity { values: identity },
                cells: vec![CellUpdate::new("name", CellValue::Text("gamma-2".into()))],
            }],
            deletes: Vec::new(),
            insert_intents: None,
        })
        .expect("update row");
    assert_eq!(
        query_text(
            connector.as_ref(),
            &config,
            &format!("SELECT name FROM `{database}`.`parent` WHERE id = 3")
        ),
        "gamma-2"
    );

    // 表数据导入走 MariaDB 分支，确认复用 MySQL 导入链路。
    execute(
        connector.as_ref(),
        &config,
        &format!(
            "CREATE TABLE `{database}`.`import_items` (`id` INT PRIMARY KEY, `name` VARCHAR(40))"
        ),
    );
    let dir = std::env::temp_dir().join(format!("fluxdb-mariadb-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let sample = dir.join("items.csv");
    std::fs::write(&sample, "id,name\n1,alpha\n2,中文\n").expect("sample csv");
    let import_object = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: "import_items".into(),
        kind: ObjectKind::Table,
    };
    let report = fluxdb_connectors::table_data_import(DatabaseKind::MariaDb)
        .expect("MariaDB import provider")
        .import(
            &fluxdb_core::TableImportRequest {
                config: config.clone(),
                object: import_object.clone(),
                source: sample,
                format: fluxdb_core::TableImportFormat::Csv,
                options: fluxdb_core::TableImportOptions::new(),
                column_mapping: Vec::new(),
            },
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("MariaDB import");
    assert_eq!(report.imported_rows, 2);
    let imported = connector
        .load_data(&import_object, 0, 10, &[], &[])
        .expect("imported rows");
    assert_eq!(imported.rows.len(), 2);
    let _ = std::fs::remove_dir_all(dir);

    // 备份 provider 必须独立识别 MariaDB，而不是回落到 MySQL 类型。
    let backup =
        fluxdb_connectors::database_backup(DatabaseKind::MariaDb).expect("backup provider");
    assert_eq!(backup.kind(), DatabaseKind::MariaDb);

    // 用户与权限：MariaDB 的账号 DDL 与 MySQL 8 有语法差异，必须真实执行验证。
    let provider =
        fluxdb_core::database_user_admin_provider(DatabaseKind::MariaDb).expect("provider");
    let pid = std::process::id();
    let user_name = format!("fluxdb_live_u_{pid}");
    let role_name = format!("fluxdb_live_r_{pid}");
    let user = DatabaseUserIdentity {
        user: user_name.clone(),
        host: "%".into(),
        plugin: None,
    };
    // MariaDB 的角色是无 host 的账号，列表与 GRANT 都只认角色名。
    let role = DatabaseUserIdentity {
        user: role_name.clone(),
        host: String::new(),
        plugin: None,
    };
    execute(
        connector.as_ref(),
        &config,
        &format!("DROP USER IF EXISTS '{}'@'%'", user_name),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!("DROP ROLE IF EXISTS '{role_name}'"),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider.create_user_sql(&CreatePrincipalInput {
            user: user_name.clone(),
            host: "%".into(),
            password: "live-pass-1".into(),
            auth_plugin: Some(provider.default_auth_plugin().into()),
        }),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider.alter_auth_plugin_sql(&user, "mysql_native_password", Some("live-pass-2")),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider
            .alter_password_expiry_sql(&user, "NEVER")
            .expect("expiry sql"),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider
            .alter_resource_limits_sql(
                &user,
                &UserResourceLimits {
                    max_queries_per_hour: Some(50),
                    max_user_connections: Some(5),
                    ..UserResourceLimits::default()
                },
            )
            .expect("limits sql"),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider
            .alter_ssl_requirement_sql(&user, "ANY", "", "", "")
            .expect("ssl sql"),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider.alter_login_sql(&user, false),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider.alter_login_sql(&user, true),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!("CREATE ROLE '{role_name}'"),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider.grant_role_sql(&role, &user),
    );
    execute(
        connector.as_ref(),
        &config,
        &provider.set_default_roles_sql(&user, std::slice::from_ref(&role)),
    );
    let grants = fluxdb_core::grants_from_query_result(&query(
        connector.as_ref(),
        &config,
        &provider.show_grants_sql(&user),
    ));
    let memberships =
        fluxdb_core::role_memberships_from_grants(std::slice::from_ref(&role), &grants, &user);
    assert!(
        memberships
            .iter()
            .any(|membership| membership.granted && membership.default_role),
        "MariaDB 默认角色未从 SHOW GRANTS 解析出来: {grants:?}"
    );
    execute(
        connector.as_ref(),
        &config,
        &format!("DROP USER '{}'@'%'", user_name),
    );
    execute(
        connector.as_ref(),
        &config,
        &format!("DROP ROLE '{role_name}'"),
    );

    execute(
        connector.as_ref(),
        &config,
        &format!("DROP DATABASE `{database}`"),
    );
    let remaining = connector
        .list_objects(None)
        .expect("list databases after drop");
    assert!(!remaining.iter().any(|object| object.path.name == database));
}
