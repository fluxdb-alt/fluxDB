//! 跨类型结构同步真实库验证（MySQL ↔ PostgreSQL）。
//!
//! 仅在显式提供两端 smoke 环境变量时运行，默认忽略：
//! - `FLUXDB_MYSQL_SMOKE=host:port:user:password`
//! - `FLUXDB_PG_SMOKE=host:port:user:password:database`
//!
//! 覆盖 MySQL → PostgreSQL 与 PostgreSQL → MySQL 两个方向：
//! 预览生成的目标端 DDL、执行后回读的列类型/自增/索引/外键/注释/默认值，
//! 以及目标库上真实写数据（自增生效、外键真实约束）。
//! 另含显式表配对（`table_map`）的改名落库验证：结构同步建表与数据同步写入
//! 都必须落在用户指定的目标表，而不是同名的另一张表。

use fluxdb_app::{
    AppCommand, AppController, DataSyncOptions, DataSyncRequest, StructureSyncOptions,
    StructureSyncRequest, SyncEndpoint, TableStructureState,
};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlBasicOptions,
    MysqlConnectionProfile, ObjectKind, ObjectPath, PostgresBasicOptions,
    PostgresConnectionProfile, QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

const MYSQL_SOURCE_ID: u64 = 9_981;
const MYSQL_TARGET_ID: u64 = 9_982;
const PG_TARGET_ID: u64 = 9_983;
const PG_SOURCE_ID: u64 = 9_984;

const MYSQL_SOURCE_DB: &str = "fluxdb_cross_src";
const MYSQL_TARGET_DB: &str = "fluxdb_cross_dst_mysql";

fn smoke_parts(key: &str, expected: usize) -> Option<Vec<String>> {
    let raw = std::env::var(key).ok()?;
    let parts = raw
        .splitn(expected, ':')
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(parts.len(), expected, "{key} 参数个数不对: {raw}");
    Some(parts)
}

fn mysql_config(id: u64, database: &str) -> ConnectionConfig {
    let parts = smoke_parts("FLUXDB_MYSQL_SMOKE", 4).expect("FLUXDB_MYSQL_SMOKE");
    let port = parts[1].parse().expect("MySQL 端口");
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("MySQL {database}"),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: parts[0].clone(),
                port,
                database: database.into(),
                username: parts[2].clone(),
                password: SecretRef::inline(parts[3].clone()),
                ..MysqlBasicOptions::default()
            },
            ..MysqlConnectionProfile::default()
        }),
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
    .mysql_resolved()
}

fn pg_config(id: u64, database: &str) -> ConnectionConfig {
    let parts = smoke_parts("FLUXDB_PG_SMOKE", 5).expect("FLUXDB_PG_SMOKE");
    let port = parts[1].parse().expect("PostgreSQL 端口");
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("PostgreSQL {database}"),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: parts[0].clone(),
                port,
                maintenance_database: parts[4].clone(),
                username: parts[2].clone(),
                password: SecretRef::inline(parts[3].clone()),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn execute(config: &ConnectionConfig, sql: &str) {
    let result = connector_for(config)
        .expect("connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::CurrentStatement,
            options: QueryExecutionOptions {
                page_size: 100,
                ..QueryExecutionOptions::default()
            },
            session_id: None,
        })
        .unwrap_or_else(|error| panic!("执行失败: {sql}: {error}"));
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "语句失败: {sql}: {result:?}"
    );
}

/// 期望语句失败；用于证明目标端外键真的在约束数据。
fn execute_expect_error(config: &ConnectionConfig, sql: &str) -> String {
    match connector_for(config)
        .expect("connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::CurrentStatement,
            options: QueryExecutionOptions {
                page_size: 100,
                ..QueryExecutionOptions::default()
            },
            session_id: None,
        }) {
        Err(error) => error.message,
        // 部分方言把失败写进逐条 summary 而不是返回 Err。
        Ok(result) => result
            .summaries
            .iter()
            .find(|summary| !summary.success)
            .map(|summary| summary.message.clone())
            .unwrap_or_else(|| panic!("语句本应失败: {sql}: {result:?}")),
    }
}

fn table_path(
    config: &ConnectionConfig,
    database: &str,
    schema: Option<&str>,
    name: &str,
) -> ObjectPath {
    ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: schema.map(str::to_string),
        name: name.into(),
        kind: ObjectKind::Table,
    }
}

fn run(
    request: &StructureSyncRequest,
) -> (
    AppController,
    fluxdb_app::StructureSyncPreview,
    fluxdb_app::StructureSyncReport,
) {
    let mut controller = AppController::new();
    let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![
        mysql_config(MYSQL_SOURCE_ID, MYSQL_SOURCE_DB),
        mysql_config(MYSQL_TARGET_ID, MYSQL_TARGET_DB),
        pg_config(PG_TARGET_ID, "fluxdb_cross_dst"),
        pg_config(PG_SOURCE_ID, "fluxdb_cross_dst"),
    ]));
    let cancel = AtomicBool::new(false);
    let preview = controller
        .preview_structure_sync(request, &cancel, &mut |_| {})
        .expect("预览失败");
    assert!(!preview.tables.is_empty(), "{preview:?}");
    for table in &preview.tables {
        assert!(table.error.is_none(), "{table:?}");
        assert!(
            table.ddl.iter().any(|sql| sql.contains("CREATE TABLE")),
            "预览应给出建表语句: {table:?}"
        );
    }
    let report = controller
        .execute_structure_sync(request, &cancel, &mut |_| {})
        .expect("执行失败");
    (controller, preview, report)
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE 与 FLUXDB_PG_SMOKE 指向可达的 MySQL 8 / PostgreSQL"]
fn mysql_types_and_constraints_land_in_postgres() {
    if smoke_parts("FLUXDB_MYSQL_SMOKE", 4).is_none() || smoke_parts("FLUXDB_PG_SMOKE", 5).is_none()
    {
        return;
    }
    let mysql = mysql_config(MYSQL_SOURCE_ID, MYSQL_SOURCE_DB);
    let pg = pg_config(PG_TARGET_ID, "fluxdb_cross_dst");

    execute(&mysql, "DROP TABLE IF EXISTS cross_child");
    execute(&mysql, "DROP TABLE IF EXISTS cross_parent");
    execute(
        &mysql,
        "CREATE TABLE cross_parent (
            id bigint unsigned NOT NULL AUTO_INCREMENT COMMENT '主键',
            name varchar(64) NOT NULL COMMENT '名称',
            status enum('active','archived') NOT NULL DEFAULT 'active',
            is_active tinyint(1) NOT NULL DEFAULT 1,
            price decimal(10,2) NOT NULL DEFAULT 0.00,
            ratio decimal(10,2) NULL,
            created_at datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
            doc text NULL,
            payload varbinary(16) NULL,
            PRIMARY KEY (id),
            UNIQUE KEY uk_name (name),
            KEY idx_status (status)
        ) COMMENT='父表'",
    );
    execute(
        &mysql,
        "CREATE TABLE cross_child (
            id int NOT NULL AUTO_INCREMENT,
            parent_id bigint unsigned NOT NULL,
            note varchar(32) NULL,
            PRIMARY KEY (id),
            KEY idx_parent (parent_id),
            CONSTRAINT fk_child_parent FOREIGN KEY (parent_id) REFERENCES cross_parent (id)
        ) COMMENT='子表'",
    );

    execute(&pg, "DROP TABLE IF EXISTS public.cross_child");
    execute(&pg, "DROP TABLE IF EXISTS public.cross_parent");

    let request = StructureSyncRequest {
        source: SyncEndpoint::new(ConnectionId(MYSQL_SOURCE_ID), MYSQL_SOURCE_DB, None),
        target: SyncEndpoint::new(
            ConnectionId(PG_TARGET_ID),
            "fluxdb_cross_dst",
            Some("public".into()),
        ),
        tables: vec!["cross_parent".into(), "cross_child".into()],
        options: StructureSyncOptions {
            // 用户显式覆盖：源端 decimal(10,2) 在目标端用 numeric(12,4)。
            type_overrides: BTreeMap::from([(
                "cross_parent".to_string(),
                BTreeMap::from([("ratio".to_string(), "numeric(12,4)".to_string())]),
            )]),
            ..StructureSyncOptions::default()
        },
    };
    let (controller, preview, report) = run(&request);
    for table in &report.tables {
        assert!(table.error.is_none(), "{report:?}");
    }
    let parent_preview = preview
        .tables
        .iter()
        .find(|table| table.name == "cross_parent")
        .expect("父表预览");
    assert!(
        parent_preview
            .warnings
            .iter()
            .any(|warning| warning.contains("无符号")),
        "{parent_preview:?}"
    );

    // 再比一次：目标端结构按自身方言归一后必须判定无差异，避免跨类型误报。
    let cancel = AtomicBool::new(false);
    let again = controller
        .preview_structure_sync(&request, &cancel, &mut |_| {})
        .expect("二次预览失败");
    assert!(
        again
            .tables
            .iter()
            .all(|table| table.state == TableStructureState::Equal),
        "{again:?}"
    );

    // 回读目标端元数据：类型、identity、注释、索引、外键都必须真实落库。
    let parent = connector_for(&pg)
        .expect("PG connector")
        .load_table_design_metadata(&table_path(
            &pg,
            "fluxdb_cross_dst",
            Some("public"),
            "cross_parent",
        ))
        .expect("读取 PG 父表元数据");
    let column = |name: &str| {
        parent
            .columns
            .iter()
            .find(|column| column.name == name)
            .unwrap_or_else(|| panic!("缺少列 {name}: {:?}", parent.columns))
    };
    assert!(
        column("id")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("bigint")
    );
    assert!(
        column("name")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("varchar(64)")
    );
    assert!(
        column("is_active")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("bool")
    );
    assert!(
        column("price")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("numeric(10,2)")
    );
    assert!(
        column("payload")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("bytea")
    );
    assert!(
        column("ratio")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("numeric(12,4)"),
        "{:?}",
        column("ratio")
    );
    assert!(
        parent_preview
            .warnings
            .iter()
            .any(|warning| warning.contains("自定义映射")),
        "{parent_preview:?}"
    );
    assert!(
        parent.auto_increment_columns.contains("id"),
        "{:?}",
        parent.auto_increment_columns
    );
    assert_eq!(parent.table_comment.as_deref(), Some("父表"));
    assert_eq!(column("name").comment.as_deref(), Some("名称"));
    assert!(
        parent
            .indexes
            .iter()
            .any(|index| index.name == "uk_name" && index.is_unique),
        "{:?}",
        parent.indexes
    );
    assert!(
        parent
            .column_defaults
            .get("created_at")
            .is_some_and(|value| value.to_lowercase().contains("current_timestamp")),
        "{:?}",
        parent.column_defaults
    );

    let child = connector_for(&pg)
        .expect("PG connector")
        .load_table_design_metadata(&table_path(
            &pg,
            "fluxdb_cross_dst",
            Some("public"),
            "cross_child",
        ))
        .expect("读取 PG 子表元数据");
    assert!(
        child
            .foreign_keys
            .iter()
            .any(|foreign_key| foreign_key.ref_table == "cross_parent"),
        "{:?}",
        child.foreign_keys
    );

    // 自增真实可用：不给 id 也能插入。
    execute(
        &pg,
        "INSERT INTO public.cross_parent (name, status) VALUES ('demo', 'active')",
    );
    // 外键真实生效：引用不存在的父行必须失败。
    let error = execute_expect_error(
        &pg,
        "INSERT INTO public.cross_child (parent_id, note) VALUES (999, 'orphan')",
    );
    assert!(!error.is_empty(), "外键未生效");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE 与 FLUXDB_PG_SMOKE 指向可达的 MySQL 8 / PostgreSQL"]
fn postgres_types_land_in_mysql() {
    if smoke_parts("FLUXDB_MYSQL_SMOKE", 4).is_none() || smoke_parts("FLUXDB_PG_SMOKE", 5).is_none()
    {
        return;
    }
    let pg = pg_config(PG_SOURCE_ID, "fluxdb_cross_dst");
    let mysql = mysql_config(MYSQL_TARGET_ID, MYSQL_TARGET_DB);

    execute(&pg, "DROP TABLE IF EXISTS public.pg_origin");
    execute(
        &pg,
        "CREATE TABLE public.pg_origin (
            id bigserial PRIMARY KEY,
            title text NOT NULL,
            active boolean NOT NULL DEFAULT true,
            amount numeric(12,2) NULL,
            uid uuid NULL,
            payload jsonb NULL,
            created_at timestamptz NOT NULL DEFAULT now(),
            label varchar(20) UNIQUE
        )",
    );
    // TEXT 列默认值 + TEXT 列索引：MySQL 目标端都必须改写/降级。
    execute(&pg, "DROP TABLE IF EXISTS public.pg_text_default");
    execute(
        &pg,
        "CREATE TABLE public.pg_text_default (
            id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
            status text NOT NULL DEFAULT 'active',
            label varchar(20) NULL
        )",
    );
    execute(
        &pg,
        "CREATE INDEX idx_text_status ON public.pg_text_default (status)",
    );
    execute(&mysql, "DROP TABLE IF EXISTS pg_origin");
    execute(&mysql, "DROP TABLE IF EXISTS pg_text_default");

    let request = StructureSyncRequest {
        source: SyncEndpoint::new(
            ConnectionId(PG_SOURCE_ID),
            "fluxdb_cross_dst",
            Some("public".into()),
        ),
        target: SyncEndpoint::new(ConnectionId(MYSQL_TARGET_ID), MYSQL_TARGET_DB, None),
        tables: vec!["public.pg_origin".into(), "public.pg_text_default".into()],
        options: StructureSyncOptions::default(),
    };
    let (_controller, preview, report) = run(&request);
    for table in &report.tables {
        assert!(table.error.is_none(), "{report:?}");
    }

    let metadata = connector_for(&mysql)
        .expect("MySQL connector")
        .load_table_design_metadata(&table_path(&mysql, MYSQL_TARGET_DB, None, "pg_origin"))
        .expect("读取 MySQL 目标元数据");
    let column = |name: &str| {
        metadata
            .columns
            .iter()
            .find(|column| column.name == name)
            .unwrap_or_else(|| panic!("缺少列 {name}: {:?}", metadata.columns))
    };
    assert!(
        column("active")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("tinyint"),
        "{:?}",
        column("active")
    );
    assert!(
        column("uid")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("char(36)"),
        "{:?}",
        column("uid")
    );
    assert!(
        column("payload")
            .type_name
            .as_deref()
            .unwrap_or("")
            .contains("json"),
        "{:?}",
        column("payload")
    );
    assert!(
        metadata.auto_increment_columns.contains("id"),
        "{:?}",
        metadata.auto_increment_columns
    );

    // 自增列在 MySQL 上也必须真实可用。
    execute(
        &mysql,
        "INSERT INTO pg_origin (title, label) VALUES ('demo', 'label-1')",
    );

    // PG 的 enum → text，MySQL 的 TEXT 列不允许字面量默认值，必须移除并提示。
    let text_table = connector_for(&mysql)
        .expect("MySQL connector")
        .load_table_design_metadata(&table_path(
            &mysql,
            MYSQL_TARGET_DB,
            None,
            "pg_text_default",
        ))
        .expect("读取 MySQL pg_text_default 元数据");
    let status = text_table
        .columns
        .iter()
        .find(|column| column.name == "status")
        .expect("status 列");
    assert!(
        status.type_name.as_deref().unwrap_or("").contains("text"),
        "{status:?}"
    );
    assert!(
        !text_table.column_defaults.contains_key("status"),
        "{:?}",
        text_table.column_defaults
    );
    assert!(
        text_table
            .indexes
            .iter()
            .any(|index| index.name == "idx_text_status"),
        "{:?}",
        text_table.indexes
    );
    let text_warnings = preview
        .tables
        .iter()
        .find(|table| table.name == "pg_text_default")
        .map(|table| table.warnings.join("\n"))
        .expect("pg_text_default 预览");
    assert!(text_warnings.contains("不支持默认值"), "{text_warnings}");
    assert!(text_warnings.contains("前缀索引"), "{text_warnings}");
}

fn live_controller() -> AppController {
    let mut controller = AppController::new();
    let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![
        mysql_config(MYSQL_SOURCE_ID, MYSQL_SOURCE_DB),
        mysql_config(MYSQL_TARGET_ID, MYSQL_TARGET_DB),
        pg_config(PG_TARGET_ID, "fluxdb_cross_dst"),
        pg_config(PG_SOURCE_ID, "fluxdb_cross_dst"),
    ]));
    controller
}

fn live_rows(
    config: &ConnectionConfig,
    database: &str,
    schema: Option<&str>,
    name: &str,
) -> Vec<Vec<fluxdb_core::CellValue>> {
    connector_for(config)
        .expect("connector")
        .load_data(
            &table_path(config, database, schema, name),
            0,
            100,
            &[],
            &[],
        )
        .expect("读取目标行")
        .rows
        .into_iter()
        .map(|row| row.values)
        .collect()
}

/// 显式表配对（结构）：MySQL 源表落成 PG 的另一张表名，自动配对不能参与。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE 与 FLUXDB_PG_SMOKE 指向可达的 MySQL 8 / PostgreSQL"]
fn mysql_structure_pairing_creates_renamed_postgres_table() {
    if smoke_parts("FLUXDB_MYSQL_SMOKE", 4).is_none() || smoke_parts("FLUXDB_PG_SMOKE", 5).is_none()
    {
        return;
    }
    let mysql = mysql_config(MYSQL_SOURCE_ID, MYSQL_SOURCE_DB);
    let pg = pg_config(PG_TARGET_ID, "fluxdb_cross_dst");

    execute(&mysql, "DROP TABLE IF EXISTS pair_orders");
    execute(
        &mysql,
        "CREATE TABLE pair_orders (
            id bigint NOT NULL AUTO_INCREMENT,
            name varchar(64) NOT NULL,
            PRIMARY KEY (id)
        )",
    );
    execute(&pg, "DROP TABLE IF EXISTS public.pg_pair_orders");

    let request = StructureSyncRequest {
        source: SyncEndpoint::new(ConnectionId(MYSQL_SOURCE_ID), MYSQL_SOURCE_DB, None),
        target: SyncEndpoint::new(
            ConnectionId(PG_TARGET_ID),
            "fluxdb_cross_dst",
            Some("public".into()),
        ),
        tables: vec!["pair_orders".into()],
        options: StructureSyncOptions {
            table_map: BTreeMap::from([(
                "pair_orders".to_string(),
                "public.pg_pair_orders".to_string(),
            )]),
            ..StructureSyncOptions::default()
        },
    };
    let (_controller, preview, report) = run(&request);
    assert!(
        report.tables.iter().all(|table| table.error.is_none()),
        "{report:?}"
    );
    assert!(
        preview.tables[0]
            .ddl
            .iter()
            .any(|sql| sql.contains("pg_pair_orders")),
        "DDL 必须使用显式配对的目标表名: {preview:?}"
    );

    // 目标端只有改名表；同名表不能被创建。
    let renamed = connector_for(&pg)
        .expect("PG connector")
        .load_table_design_metadata(&table_path(
            &pg,
            "fluxdb_cross_dst",
            Some("public"),
            "pg_pair_orders",
        ))
        .expect("改名目标表必须存在");
    assert!(
        renamed.columns.iter().any(|column| column.name == "name"),
        "{renamed:?}"
    );
    assert!(
        connector_for(&pg)
            .expect("PG connector")
            .load_table_design_metadata(&table_path(
                &pg,
                "fluxdb_cross_dst",
                Some("public"),
                "pair_orders",
            ))
            .is_err(),
        "不能回退创建同名表"
    );
}

/// 显式表配对（数据）：MySQL 源行必须写进 PG 的改名目标表。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE 与 FLUXDB_PG_SMOKE 指向可达的 MySQL 8 / PostgreSQL"]
fn mysql_data_pairing_writes_renamed_postgres_table() {
    if smoke_parts("FLUXDB_MYSQL_SMOKE", 4).is_none() || smoke_parts("FLUXDB_PG_SMOKE", 5).is_none()
    {
        return;
    }
    let mysql = mysql_config(MYSQL_SOURCE_ID, MYSQL_SOURCE_DB);
    let pg = pg_config(PG_TARGET_ID, "fluxdb_cross_dst");

    execute(&mysql, "DROP TABLE IF EXISTS pair_rows");
    execute(
        &mysql,
        "CREATE TABLE pair_rows (id int NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)",
    );
    execute(
        &mysql,
        "INSERT INTO pair_rows (id, name) VALUES (1, 'a'), (2, 'b')",
    );
    execute(&pg, "DROP TABLE IF EXISTS public.pg_pair_rows");
    execute(&pg, "DROP TABLE IF EXISTS public.pair_rows");
    execute(
        &pg,
        "CREATE TABLE public.pg_pair_rows (id integer NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)",
    );

    let request = DataSyncRequest {
        source: SyncEndpoint::new(ConnectionId(MYSQL_SOURCE_ID), MYSQL_SOURCE_DB, None),
        target: SyncEndpoint::new(
            ConnectionId(PG_TARGET_ID),
            "fluxdb_cross_dst",
            Some("public".into()),
        ),
        tables: vec!["pair_rows".into()],
        options: DataSyncOptions {
            insert: true,
            table_map: BTreeMap::from([(
                "pair_rows".to_string(),
                "public.pg_pair_rows".to_string(),
            )]),
            ..DataSyncOptions::default()
        },
    };
    let controller = live_controller();
    let cancel = AtomicBool::new(false);
    let preview = controller
        .preview_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据预览失败");
    assert_eq!(preview.tables.len(), 1, "{preview:?}");
    assert_eq!(preview.tables[0].inserted, 2, "{:?}", preview.tables[0]);

    let report = controller
        .execute_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据执行失败");
    assert!(report.failed_tables().is_empty(), "{report:?}");

    let rows = live_rows(&pg, "fluxdb_cross_dst", Some("public"), "pg_pair_rows");
    assert_eq!(rows.len(), 2, "行必须落到显式配对表: {rows:?}");
    // 目标端另建同名表，证明写入没有回退到同名表。
    execute(
        &pg,
        "CREATE TABLE public.pair_rows (id integer NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)",
    );
    let fallback = live_rows(&pg, "fluxdb_cross_dst", Some("public"), "pair_rows");
    assert!(fallback.is_empty(), "同名表必须保持为空: {fallback:?}");
    execute(&pg, "DROP TABLE IF EXISTS public.pair_rows");
}

/// 显式表配对（数据，反向）：PG 源行写进 MySQL 的改名目标表。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE 与 FLUXDB_PG_SMOKE 指向可达的 MySQL 8 / PostgreSQL"]
fn postgres_data_pairing_writes_renamed_mysql_table() {
    if smoke_parts("FLUXDB_MYSQL_SMOKE", 4).is_none() || smoke_parts("FLUXDB_PG_SMOKE", 5).is_none()
    {
        return;
    }
    let pg = pg_config(PG_SOURCE_ID, "fluxdb_cross_dst");
    let mysql = mysql_config(MYSQL_TARGET_ID, MYSQL_TARGET_DB);

    execute(&pg, "DROP TABLE IF EXISTS public.pg_pair_src");
    execute(
        &pg,
        "CREATE TABLE public.pg_pair_src (id integer NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)",
    );
    execute(
        &pg,
        "INSERT INTO public.pg_pair_src (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c')",
    );
    execute(&mysql, "DROP TABLE IF EXISTS mysql_pair_dst");
    execute(&mysql, "DROP TABLE IF EXISTS pg_pair_src");
    execute(
        &mysql,
        "CREATE TABLE mysql_pair_dst (id int NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)",
    );

    let request = DataSyncRequest {
        source: SyncEndpoint::new(
            ConnectionId(PG_SOURCE_ID),
            "fluxdb_cross_dst",
            Some("public".into()),
        ),
        target: SyncEndpoint::new(ConnectionId(MYSQL_TARGET_ID), MYSQL_TARGET_DB, None),
        tables: vec!["public.pg_pair_src".into()],
        options: DataSyncOptions {
            insert: true,
            table_map: BTreeMap::from([(
                "public.pg_pair_src".to_string(),
                "mysql_pair_dst".to_string(),
            )]),
            ..DataSyncOptions::default()
        },
    };
    let controller = live_controller();
    let cancel = AtomicBool::new(false);
    let report = controller
        .execute_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据执行失败");
    assert!(report.failed_tables().is_empty(), "{report:?}");

    let rows = live_rows(&mysql, MYSQL_TARGET_DB, None, "mysql_pair_dst");
    assert_eq!(rows.len(), 3, "行必须落到显式配对表: {rows:?}");
    execute(
        &mysql,
        "CREATE TABLE pg_pair_src (id int NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)",
    );
    let fallback = live_rows(&mysql, MYSQL_TARGET_DB, None, "pg_pair_src");
    assert!(fallback.is_empty(), "同名表必须保持为空: {fallback:?}");
    execute(&mysql, "DROP TABLE IF EXISTS pg_pair_src");
}
