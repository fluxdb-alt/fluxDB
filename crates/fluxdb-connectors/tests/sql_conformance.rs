//! 跨 SQL 引擎一致性场景。
//!
//! SQLite 每次使用临时文件运行；其它引擎只在显式提供连接环境变量时运行。
//! 场景共享同一份 fixture 与断言，不能在各引擎入口复制一套更宽松的检查。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellUpdate, CellValue, ConnectionConfig, Connector, DataChangeSet, FilterOp, FilterSpec,
    ObjectKind, ObjectPath, QueryExecutionOptions, QueryExecutionResult, QueryMode, QueryRequest,
    Row, RowIdentity, RowUpdate, SortDirection, SortSpec,
};
use std::collections::BTreeMap;

#[path = "sql_conformance/adapters.rs"]
mod adapters;

const COLUMNS: [&str; 4] = ["id", "label", "amount", "note"];
const UNICODE_LABEL: &str = "Zoë 李 🐉 'quoted'";

struct ConformanceTarget {
    label: &'static str,
    config: ConnectionConfig,
    table: ObjectPath,
    catalog_parent: ObjectPath,
    catalog_database: Option<ObjectPath>,
    bootstrap_sql: Vec<String>,
    bootstrap_teardown_sql: Vec<String>,
    setup_sql: Vec<String>,
    teardown_sql: Vec<String>,
    select_sql: String,
    read_only_write_sql: String,
}

fn run_suite(target: &ConformanceTarget) {
    let connector = connector_for(&target.config)
        .unwrap_or_else(|error| panic!("{}: 创建连接器失败: {error}", target.label));
    let connector = connector.as_ref();
    let _cleanup = FixtureCleanup { connector, target };

    connector
        .test_connection(&target.config)
        .unwrap_or_else(|error| panic!("{}: 连接失败: {error}", target.label));
    for sql in &target.bootstrap_sql {
        execute_bootstrap_required(connector, target, sql);
    }
    for sql in &target.setup_sql {
        execute_required(connector, target, sql);
    }

    connection_and_query_round_trip(connector, target);
    paging_sort_and_filter(connector, target);
    catalog_round_trip(connector, target);
    table_metadata_round_trip(connector, target);
    apply_changes_round_trip(connector, target);
    failed_change_set_rolls_back(connector, target);
    read_only_rejects_writes(connector, target);
    cancelled_before_start_does_not_write(connector, target);
}

fn connection_and_query_round_trip(connector: &dyn Connector, target: &ConformanceTarget) {
    let execution = execute_required(connector, target, &target.select_sql);
    let page = execution
        .results
        .first()
        .unwrap_or_else(|| panic!("{}: 查询没有返回结果页", target.label));
    assert_eq!(
        page.columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        COLUMNS,
        "{}: 列名与顺序必须稳定",
        target.label
    );
    assert_eq!(page.rows.len(), 3, "{}: fixture 行数", target.label);

    assert_eq!(
        integer_value(&page.rows[0].values[0]),
        Some(1),
        "{}",
        target.label
    );
    assert_eq!(
        page.rows[0].values[1],
        CellValue::Text("alpha".into()),
        "{}",
        target.label
    );
    assert_eq!(
        decimal_number(&page.rows[0].values[2]),
        Some(12.5),
        "{}: DECIMAL 必须保留数值",
        target.label
    );
    assert_eq!(
        page.rows[0].values[3],
        CellValue::Null,
        "{}: NULL 不能被渲染成空字符串",
        target.label
    );

    assert_eq!(
        page.rows[1].values[1],
        CellValue::Text(UNICODE_LABEL.into()),
        "{}: Unicode 与引号必须原样往返",
        target.label
    );
    assert_eq!(
        decimal_number(&page.rows[1].values[2]),
        Some(-1.25),
        "{}: actual={:?}",
        target.label,
        page.rows[1].values[2]
    );
    assert_eq!(
        page.rows[1].values[3],
        CellValue::Text("kept".into()),
        "{}",
        target.label
    );

    assert_eq!(
        integer_value(&page.rows[2].values[0]),
        Some(3),
        "{}: id",
        target.label
    );
    assert_eq!(
        page.rows[2].values[1],
        CellValue::Text("nulls".into()),
        "{}",
        target.label
    );
    assert_eq!(
        page.rows[2].values[2],
        CellValue::Null,
        "{}: DECIMAL NULL",
        target.label
    );
}

fn paging_sort_and_filter(connector: &dyn Connector, target: &ConformanceTarget) {
    let first = connector
        .load_data(
            &target.table,
            0,
            1,
            &[SortSpec {
                field: "id".into(),
                direction: SortDirection::Asc,
            }],
            &[],
        )
        .unwrap_or_else(|error| panic!("{}: 第一页读取失败: {error}", target.label));
    assert_eq!(first.rows.len(), 1, "{}", target.label);
    assert_eq!(
        integer_value(&first.rows[0].values[0]),
        Some(1),
        "{}",
        target.label
    );
    assert!(first.has_more, "{}: 第一页之后还有数据", target.label);

    let rest = connector
        .load_data(
            &target.table,
            1,
            10,
            &[SortSpec {
                field: "id".into(),
                direction: SortDirection::Asc,
            }],
            &[],
        )
        .unwrap_or_else(|error| panic!("{}: 第二页读取失败: {error}", target.label));
    assert_eq!(rest.rows.len(), 2, "{}", target.label);
    assert_eq!(
        integer_value(&rest.rows[0].values[0]),
        Some(2),
        "{}",
        target.label
    );
    assert_eq!(
        integer_value(&rest.rows[1].values[0]),
        Some(3),
        "{}",
        target.label
    );
    assert!(!rest.has_more, "{}: fixture 已读完", target.label);

    let descending = connector
        .load_data(
            &target.table,
            0,
            10,
            &[SortSpec {
                field: "id".into(),
                direction: SortDirection::Desc,
            }],
            &[],
        )
        .unwrap_or_else(|error| panic!("{}: 降序读取失败: {error}", target.label));
    assert_eq!(
        descending
            .rows
            .iter()
            .map(|row| integer_value(&row.values[0]))
            .collect::<Vec<_>>(),
        vec![Some(3), Some(2), Some(1)],
        "{}: 排序方向",
        target.label
    );

    let filtered = connector
        .load_data(
            &target.table,
            0,
            10,
            &[],
            &[FilterSpec {
                field: "label".into(),
                op: FilterOp::Eq,
                values: vec![CellValue::Text("alpha".into())],
                enabled: true,
            }],
        )
        .unwrap_or_else(|error| panic!("{}: 等值筛选失败: {error}", target.label));
    assert_eq!(filtered.rows.len(), 1, "{}: label = alpha", target.label);
    assert_eq!(
        integer_value(&filtered.rows[0].values[0]),
        Some(1),
        "{}",
        target.label
    );

    let nulls = connector
        .load_data(
            &target.table,
            0,
            10,
            &[],
            &[FilterSpec {
                field: "note".into(),
                op: FilterOp::IsNull,
                values: Vec::new(),
                enabled: true,
            }],
        )
        .unwrap_or_else(|error| panic!("{}: NULL 筛选失败: {error}", target.label));
    assert_eq!(nulls.rows.len(), 2, "{}: note IS NULL", target.label);
    assert_eq!(
        nulls
            .rows
            .iter()
            .map(|row| integer_value(&row.values[0]))
            .collect::<Vec<_>>(),
        vec![Some(1), Some(3)],
        "{}: note IS NULL 行集合",
        target.label
    );
}

fn catalog_round_trip(connector: &dyn Connector, target: &ConformanceTarget) {
    let roots = connector
        .list_objects(None)
        .unwrap_or_else(|error| panic!("{}: 根对象读取失败: {error}", target.label));
    if let Some(database) = &target.catalog_database {
        assert!(
            roots.iter().any(|object| {
                object.path.name == database.name && object.path.kind == ObjectKind::Database
            }),
            "{}: 根对象缺少数据库 {}",
            target.label,
            database.name
        );
        let schemas = connector
            .list_objects(Some(database))
            .unwrap_or_else(|error| panic!("{}: schema 读取失败: {error}", target.label));
        assert!(
            schemas.iter().any(|object| {
                object.path.name == target.catalog_parent.name
                    && object.path.kind == ObjectKind::Schema
            }),
            "{}: 数据库 {} 缺少 schema {}",
            target.label,
            database.name,
            target.catalog_parent.name
        );
    } else {
        assert!(
            roots.iter().any(|object| {
                object.path.name == target.catalog_parent.name
                    && object.path.kind == target.catalog_parent.kind
            }),
            "{}: 根对象缺少 {}",
            target.label,
            target.catalog_parent.name
        );
    }

    let objects = connector
        .list_objects(Some(&target.catalog_parent))
        .unwrap_or_else(|error| panic!("{}: 表对象读取失败: {error}", target.label));
    assert!(
        objects.iter().any(|object| {
            object.path.name == target.table.name && object.path.kind == ObjectKind::Table
        }),
        "{}: {} 未出现在对象树中",
        target.label,
        target.table.name
    );
}

fn table_metadata_round_trip(connector: &dyn Connector, target: &ConformanceTarget) {
    let columns = connector
        .list_completion_columns(
            target.table.database.as_deref(),
            target.table.schema.as_deref(),
            &target.table.name,
        )
        .unwrap_or_else(|error| panic!("{}: 列元数据读取失败: {error}", target.label));
    assert_eq!(
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        COLUMNS,
        "{}: 补全列名与顺序",
        target.label
    );
    assert!(
        columns
            .iter()
            .find(|column| column.name == "id")
            .is_some_and(|column| column.primary_key),
        "{}: id 必须标记为主键",
        target.label
    );

    let indexes = connector
        .list_indexes(&target.table)
        .unwrap_or_else(|error| panic!("{}: 索引读取失败: {error}", target.label));
    assert!(
        indexes
            .iter()
            .any(|index| { index.is_unique && index.columns == vec!["label".to_string()] }),
        "{}: 缺少 label 唯一索引",
        target.label
    );

    let ddl = connector
        .table_ddl(&target.table)
        .unwrap_or_else(|error| panic!("{}: DDL 读取失败: {error}", target.label));
    assert!(
        ddl.contains(&target.table.name),
        "{}: DDL 表名",
        target.label
    );
    assert!(
        ddl.to_ascii_uppercase().contains("PRIMARY KEY"),
        "{}: DDL 主键",
        target.label
    );

    let metadata = connector
        .load_table_design_metadata(&target.table)
        .unwrap_or_else(|error| panic!("{}: 表设计元数据读取失败: {error}", target.label));
    assert!(metadata.details_loaded, "{}: 表设计详情", target.label);
    assert_eq!(
        metadata
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        COLUMNS,
        "{}: 表设计列名",
        target.label
    );
}

fn apply_changes_round_trip(connector: &dyn Connector, target: &ConformanceTarget) {
    connector
        .apply_changes(&DataChangeSet {
            object: target.table.clone(),
            inserts: vec![Row {
                values: vec![
                    CellValue::I64(7),
                    CellValue::Text("inserted".into()),
                    CellValue::Text("9.5".into()),
                    CellValue::Text("row7".into()),
                ],
            }],
            updates: vec![RowUpdate {
                identity: identity(2),
                cells: vec![CellUpdate::new("label", CellValue::Text("updated".into()))],
            }],
            deletes: vec![identity(3)],
            insert_intents: None,
        })
        .unwrap_or_else(|error| panic!("{}: 提交增删改失败: {error}", target.label));

    let page = connector
        .load_data(
            &target.table,
            0,
            10,
            &[SortSpec {
                field: "id".into(),
                direction: SortDirection::Asc,
            }],
            &[],
        )
        .unwrap_or_else(|error| panic!("{}: 写入后读取失败: {error}", target.label));
    assert_eq!(
        page.rows
            .iter()
            .map(|row| integer_value(&row.values[0]))
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(7)],
        "{}: 增删改后的行集合",
        target.label
    );
    assert_eq!(
        page.rows[1].values[1],
        CellValue::Text("updated".into()),
        "{}: 更新值",
        target.label
    );
    assert_eq!(
        page.rows[2].values[1],
        CellValue::Text("inserted".into()),
        "{}: 插入值",
        target.label
    );
}

fn failed_change_set_rolls_back(connector: &dyn Connector, target: &ConformanceTarget) {
    let error = connector
        .apply_changes(&DataChangeSet {
            object: target.table.clone(),
            inserts: vec![Row {
                values: vec![
                    CellValue::I64(1),
                    CellValue::Text("duplicate".into()),
                    CellValue::Text("0".into()),
                    CellValue::Null,
                ],
            }],
            updates: vec![RowUpdate {
                identity: identity(2),
                cells: vec![CellUpdate::new(
                    "label",
                    CellValue::Text("must-rollback".into()),
                )],
            }],
            deletes: Vec::new(),
            insert_intents: None,
        })
        .expect_err("主键冲突必须让整批变更失败");
    assert!(!error.message.is_empty(), "{}: 错误必须可读", target.label);

    let page = connector
        .load_data(&target.table, 0, 10, &[], &[])
        .unwrap_or_else(|error| panic!("{}: 回滚后读取失败: {error}", target.label));
    let row = page
        .rows
        .iter()
        .find(|row| integer_value(&row.values[0]) == Some(2))
        .unwrap_or_else(|| panic!("{}: 回滚后 id=2 应仍存在", target.label));
    assert_eq!(
        row.values[1],
        CellValue::Text("updated".into()),
        "{}: 同一批次里的 UPDATE 必须随失败 INSERT 一起回滚",
        target.label
    );
}

fn read_only_rejects_writes(connector: &dyn Connector, target: &ConformanceTarget) {
    let read = execute_with_options(
        connector,
        target,
        &target.select_sql,
        QueryExecutionOptions {
            read_only: true,
            ..QueryExecutionOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{}: 只读 SELECT 应成功: {error}", target.label));
    assert!(
        execution_succeeded(&read),
        "{}: 只读 SELECT 必须有成功摘要",
        target.label
    );

    let write = execute_with_options(
        connector,
        target,
        &target.read_only_write_sql,
        QueryExecutionOptions {
            read_only: true,
            ..QueryExecutionOptions::default()
        },
    );
    assert!(
        !write.as_ref().is_ok_and(execution_succeeded),
        "{}: 只读入口不得成功执行写入",
        target.label
    );

    let page = connector
        .load_data(&target.table, 0, 10, &[], &[])
        .unwrap_or_else(|error| panic!("{}: 只读拒绝后读取失败: {error}", target.label));
    assert!(
        page.rows
            .iter()
            .all(|row| integer_value(&row.values[0]) != Some(99)),
        "{}: 只读拒绝后不能留下写入副作用",
        target.label
    );
}

fn cancelled_before_start_does_not_write(connector: &dyn Connector, target: &ConformanceTarget) {
    let mut summaries = Vec::new();
    let result = connector.execute_with_progress(
        &request(
            target,
            &target.read_only_write_sql,
            QueryExecutionOptions::default(),
        ),
        &mut |summary| summaries.push(summary),
        &|| true,
    );
    let succeeded = result.as_ref().map(execution_succeeded).unwrap_or(false)
        || summaries.iter().any(|summary| summary.success);
    assert!(
        !succeeded,
        "{}: 启动前已取消的语句不得报告成功",
        target.label
    );

    let page = connector
        .load_data(&target.table, 0, 10, &[], &[])
        .unwrap_or_else(|error| panic!("{}: 取消后读取失败: {error}", target.label));
    assert!(
        page.rows
            .iter()
            .all(|row| integer_value(&row.values[0]) != Some(99)),
        "{}: 取消后不能留下写入副作用",
        target.label
    );
}

fn identity(id: i64) -> RowIdentity {
    RowIdentity {
        values: BTreeMap::from([("id".to_string(), CellValue::I64(id))]),
    }
}

fn decimal_number(value: &CellValue) -> Option<f64> {
    match value {
        CellValue::I64(value) => Some(*value as f64),
        CellValue::F64(value) => Some(*value),
        CellValue::Text(value) | CellValue::Json(value) => value.parse().ok(),
        _ => None,
    }
}

fn integer_value(value: &CellValue) -> Option<i64> {
    match value {
        CellValue::I64(value) => Some(*value),
        CellValue::F64(value) if value.fract() == 0.0 => Some(*value as i64),
        CellValue::Text(value) | CellValue::Json(value) => value.parse().ok(),
        _ => None,
    }
}

fn execution_succeeded(result: &QueryExecutionResult) -> bool {
    !result.summaries.is_empty() && result.summaries.iter().all(|summary| summary.success)
}

fn execute_required(
    connector: &dyn Connector,
    target: &ConformanceTarget,
    sql: &str,
) -> QueryExecutionResult {
    let execution = execute_with_options(connector, target, sql, QueryExecutionOptions::default())
        .unwrap_or_else(|error| panic!("{}: 执行失败: {sql}: {error}", target.label));
    assert!(
        execution_succeeded(&execution),
        "{}: 语句没有成功完成: {sql}: {:?}",
        target.label,
        execution.summaries
    );
    execution
}

fn execute_bootstrap_required(
    connector: &dyn Connector,
    target: &ConformanceTarget,
    sql: &str,
) -> QueryExecutionResult {
    let execution = connector
        .execute(&QueryRequest {
            connection_id: target.config.id,
            database: None,
            schema: None,
            text: sql.to_string(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .unwrap_or_else(|error| panic!("{}: bootstrap 失败: {sql}: {error}", target.label));
    assert!(
        execution_succeeded(&execution),
        "{}: bootstrap 没有成功完成: {sql}: {:?}",
        target.label,
        execution.summaries
    );
    execution
}

fn execute_with_options(
    connector: &dyn Connector,
    target: &ConformanceTarget,
    sql: &str,
    options: QueryExecutionOptions,
) -> fluxdb_core::Result<QueryExecutionResult> {
    connector.execute(&request(target, sql, options))
}

fn request(target: &ConformanceTarget, sql: &str, options: QueryExecutionOptions) -> QueryRequest {
    QueryRequest {
        connection_id: target.config.id,
        database: target.table.database.clone(),
        schema: target.table.schema.clone(),
        text: sql.to_string(),
        mode: QueryMode::All,
        options,
        session_id: None,
    }
}

struct FixtureCleanup<'a> {
    connector: &'a dyn Connector,
    target: &'a ConformanceTarget,
}

impl Drop for FixtureCleanup<'_> {
    fn drop(&mut self) {
        for sql in &self.target.teardown_sql {
            let _ = self.connector.execute(&request(
                self.target,
                sql,
                QueryExecutionOptions::default(),
            ));
        }
        for sql in &self.target.bootstrap_teardown_sql {
            let _ = self.connector.execute(&QueryRequest {
                connection_id: self.target.config.id,
                database: None,
                schema: None,
                text: sql.to_string(),
                mode: QueryMode::All,
                options: QueryExecutionOptions::default(),
                session_id: None,
            });
        }
    }
}
