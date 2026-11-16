//! 真实 PostgreSQL 的数据生成端到端验证。
//!
//! 仅在显式提供 `FLUXDB_PG_GENERATION=host:port:user:password:database` 时运行。
//! 测试自建并清理 `gen_e2e` schema；走生产路径：
//! `AppController::load_data_generation_source` → `generate_table_data` → 生成器 →
//! 临时 JSON → 导入管线 → PostgreSQL 参数化 INSERT，最后回读真实表数据断言。
//!
//! 覆盖：
//! - 物理外键引用父表已有值；
//! - 逻辑引用（无物理约束，等价于用户自定义 ER 外键）同样取父表已有值；
//! - 唯一列在真实约束下不重复；
//! - 追加 / 清空两种写入模式与取消路径。

use fluxdb_app::{AppCommand, AppController};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellValue, ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ErrorKind,
    GenerationColumnSpec, GenerationReference, GenerationSpec, ObjectKind, ObjectPath,
    PostgresBasicOptions, PostgresConnectionProfile, PostgresScopeOptions, QueryExecutionOptions,
    QueryMode, QueryRequest, SecretRef, TableImportMode,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;

const SCHEMA: &str = "gen_e2e";
const CHILD: &str = "child";
const ROWS: u64 = 60;

fn live_config() -> Option<(ConnectionConfig, String)> {
    let raw = std::env::var("FLUXDB_PG_GENERATION").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_PG_GENERATION=host:port:user:password:database"
    );
    let host = parts[0].to_string();
    let port: u16 = parts[1].parse().expect("PostgreSQL port");
    let database = parts[4].to_string();
    let mut options = BTreeMap::new();
    options.insert("host".into(), host.clone());
    options.insert("port".into(), port.to_string());
    options.insert("maintenance_database".into(), database.clone());
    options.insert("username".into(), parts[2].into());
    options.insert("password".into(), parts[3].into());
    options.insert("default_schema".into(), SCHEMA.into());
    let config = ConnectionConfig {
        id: ConnectionId(9_915),
        name: "PostgreSQL generation".into(),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host,
            port,
            database: Some(database.clone()),
        },
        options,
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: parts[0].into(),
                port,
                maintenance_database: database.clone(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            scope: PostgresScopeOptions {
                default_schema: SCHEMA.into(),
                ..PostgresScopeOptions::default()
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
    .postgres_resolved();
    Some((config, database))
}

fn execute(config: &ConnectionConfig, database: &str, text: &str) {
    let connector = connector_for(config).expect("PostgreSQL connector");
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some(database.into()),
            schema: None,
            session_id: None,
            text: text.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        })
        .expect("PostgreSQL execute");
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        panic!("SQL failed: {}\n{}", summary.message, text);
    }
}

fn child_object(config: &ConnectionConfig, database: &str) -> ObjectPath {
    ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: Some(SCHEMA.into()),
        name: CHILD.into(),
        kind: ObjectKind::Table,
    }
}

/// 与桌面端 `generation_column_form` 同形状：外键列用 foreign_ref，其余按名/类型推断。
fn spec_for(
    source: &fluxdb_app::DataGenerationSource,
    database: &str,
    rows: u64,
    seed: &str,
) -> GenerationSpec {
    let columns = source
        .columns
        .iter()
        .map(|column| {
            let reference = source
                .foreign_keys
                .iter()
                .find(|foreign_key| {
                    foreign_key.column == column.name && foreign_key.columns.len() <= 1
                })
                .map(|foreign_key| GenerationReference {
                    database: Some(database.into()),
                    schema: foreign_key.ref_schema.clone(),
                    table: foreign_key.ref_table.clone(),
                    column: foreign_key.ref_column.clone(),
                    label: format!("{}.{}", foreign_key.ref_table, foreign_key.ref_column),
                })
                .or_else(|| {
                    // 逻辑引用：没有物理外键，等价于用户自定义 ER 外键的取值方式。
                    (column.name == "mentor_id").then(|| GenerationReference {
                        database: Some(database.into()),
                        schema: Some(SCHEMA.into()),
                        table: "parent".into(),
                        column: "id".into(),
                        label: "parent.id".into(),
                    })
                });
            let mut generator = if reference.is_some() {
                "foreign_ref".to_string()
            } else {
                fluxdb_core::infer_generator(&column.name, &column.type_name).to_string()
            };
            let mut params = BTreeMap::new();
            // `nick` 是唯一索引；按名称推断会落到词表很小的 lorem.word，
            // 唯一性在 60 行时本就无解。改用 2 位数字：只有 100 种取值，
            // 60 行必然碰撞，能真正验证「重试会换到新值」。
            if column.name == "nick" {
                generator = "random.digit".into();
                params.insert("count".into(), "2".into());
            }
            GenerationColumnSpec {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
                nullable: column.nullable,
                unique: column.unique,
                generator,
                params,
                null_percent: 0,
                reference,
                reference_pool: Vec::new(),
            }
        })
        .collect();
    GenerationSpec {
        rows,
        seed: seed.into(),
        locale: "zh_cn".into(),
        unique_conflict: fluxdb_core::UniqueConflict::Abort,
        columns,
    }
}

fn read_child(config: &ConnectionConfig, database: &str) -> Vec<BTreeMap<String, CellValue>> {
    let connector = connector_for(config).expect("PostgreSQL connector");
    let page = connector
        .load_data(&child_object(config, database), 0, 5_000, &[], &[])
        .expect("read child rows");
    page.rows
        .iter()
        .map(|row| {
            page.columns
                .iter()
                .zip(row.values.iter())
                .map(|(column, value)| (column.name.clone(), value.clone()))
                .collect()
        })
        .collect()
}

fn int_of(value: Option<&CellValue>) -> i64 {
    match value {
        Some(CellValue::I64(value)) => *value,
        other => panic!("期望整数单元格，实际 {other:?}"),
    }
}

fn text_of(value: Option<&CellValue>) -> String {
    match value {
        Some(CellValue::Null) | None => panic!("期望非空文本单元格"),
        Some(value) => value.display_label(),
    }
}

#[test]
fn generates_rows_into_real_postgres_with_fk_and_unique_constraints() {
    let Some((config, database)) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_PG_GENERATION");
        return;
    };
    execute(
        &config,
        &database,
        &format!(
            "DROP SCHEMA IF EXISTS {SCHEMA} CASCADE;
             CREATE SCHEMA {SCHEMA};
             CREATE TABLE {SCHEMA}.parent (id integer PRIMARY KEY, name text NOT NULL);
             CREATE TABLE {SCHEMA}.child (
                 id integer PRIMARY KEY,
                 parent_id integer REFERENCES {SCHEMA}.parent(id),
                 mentor_id integer,
                 email varchar(80) NOT NULL,
                 nick varchar(20) UNIQUE,
                 note text,
                 created_at timestamp
             );
             INSERT INTO {SCHEMA}.parent
                 SELECT g, 'parent-' || g FROM generate_series(1, 7) AS g;"
        ),
    );

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
    let source = controller
        .load_data_generation_source(&child_object(&config, &database))
        .expect("load generation source");

    // 表结构：主键/单列唯一索引识别为 unique；物理外键被读取；可空列标记正确。
    let by_name = |name: &str| {
        source
            .columns
            .iter()
            .find(|column| column.name == name)
            .unwrap_or_else(|| panic!("缺少列 {name}"))
    };
    assert!(by_name("id").primary_key && by_name("id").unique);
    assert!(by_name("nick").unique, "单列唯一索引应标记 unique");
    assert!(!by_name("email").nullable);
    assert!(by_name("note").nullable);
    assert!(
        source
            .foreign_keys
            .iter()
            .any(|foreign_key| foreign_key.column == "parent_id"
                && foreign_key.ref_table == "parent"
                && foreign_key.ref_column == "id"),
        "应读取到物理外键 parent_id → parent.id：{:?}",
        source.foreign_keys
    );

    let object = child_object(&config, &database);
    let cancel = AtomicBool::new(false);
    let mut stages = Vec::new();
    let report = controller
        .generate_table_data(
            &object,
            spec_for(&source, &database, ROWS, "e2e-pg"),
            TableImportMode::Append,
            0,
            &cancel,
            &mut |progress| stages.push(format!("{}:{}", progress.stage, progress.message)),
        )
        .expect("generate and import");
    assert_eq!(report.rows, ROWS);
    assert!(
        report.warnings.is_empty(),
        "父表可读时不应有回退告警：{:?}",
        report.warnings
    );
    assert!(
        stages.iter().any(|stage| stage.starts_with("完成")),
        "应有完成进度：{stages:?}"
    );

    let rows = read_child(&config, &database);
    assert_eq!(rows.len() as u64, ROWS, "真实表行数应与生成行数一致");

    let parent_ids = (1..=7).collect::<BTreeSet<_>>();
    let mut nicks = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for row in &rows {
        let id = int_of(row.get("id"));
        assert!(ids.insert(id), "主键重复：{id}");

        // 物理外键：取值必须全部来自父表现有 id。
        let parent_id = int_of(row.get("parent_id"));
        assert!(
            parent_ids.contains(&parent_id),
            "parent_id={parent_id} 不在父表 id 集合内"
        );

        // 逻辑引用（ER 外键等价物）：同样必须来自父表。
        let mentor_id = int_of(row.get("mentor_id"));
        assert!(
            parent_ids.contains(&mentor_id),
            "mentor_id={mentor_id} 不在父表 id 集合内"
        );

        let email = text_of(row.get("email"));
        assert!(email.contains('@'), "email 格式异常：{email}");

        let nick = text_of(row.get("nick"));
        assert!(nicks.insert(nick.clone()), "唯一列出现重复：{nick}");

        let created_at = text_of(row.get("created_at"));
        assert!(
            created_at.len() >= 10 && created_at[..4].chars().all(|c| c.is_ascii_digit()),
            "created_at 解析异常：{created_at}"
        );
    }
    assert!(stages.iter().any(|stage| stage.contains("已生成")));

    // 取消：不写入任何行。
    let cancel = AtomicBool::new(true);
    let error = controller
        .generate_table_data(
            &object,
            spec_for(&source, &database, 10, "e2e-cancel"),
            TableImportMode::Append,
            0,
            &cancel,
            &mut |_| {},
        )
        .expect_err("取消应返回错误");
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(
        read_child(&config, &database).len() as u64,
        ROWS,
        "取消后行数不变"
    );

    // 清空后写入：真实表只剩本次生成的行。
    let cancel = AtomicBool::new(false);
    let report = controller
        .generate_table_data(
            &object,
            spec_for(&source, &database, 5, "e2e-truncate"),
            TableImportMode::Truncate,
            0,
            &cancel,
            &mut |_| {},
        )
        .expect("truncate and import");
    assert_eq!(report.rows, 5);
    let rows = read_child(&config, &database);
    assert_eq!(rows.len(), 5, "清空模式应只保留新写入行");
    let ids = rows
        .iter()
        .map(|row| int_of(row.get("id")))
        .collect::<BTreeSet<_>>();
    assert_eq!(ids, (1..=5).collect::<BTreeSet<_>>());

    execute(
        &config,
        &database,
        &format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"),
    );
}
