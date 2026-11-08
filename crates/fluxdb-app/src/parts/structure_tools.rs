/// 结构对比的一端：连接 + 库 + 可选 schema。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureEndpoint {
    pub connection_id: fluxdb_core::ConnectionId,
    pub database: String,
    pub schema: Option<String>,
}

impl StructureEndpoint {
    fn database(&self) -> fluxdb_core::Result<&str> {
        let database = self.database.trim();
        if database.is_empty() {
            return Err(Error::new(ErrorKind::Query, "请选择数据库"));
        }
        Ok(database)
    }

    fn schema(&self) -> Option<&str> {
        self.schema
            .as_deref()
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructureDiffStatus {
    /// 目标缺失；同步时可按源端 DDL 新建。
    Added,
    /// 目标多出；按需删除。
    Removed,
    /// 两端都存在但结构不同。
    Modified,
    /// 结构一致。
    Same,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructureChangeKind {
    MissingInTarget,
    ExtraInTarget,
    /// 字段/索引集合一致，但 DDL（默认值、约束等）不同。
    DefinitionChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureChange {
    pub kind: StructureChangeKind,
    /// 技术标识，如 `column|id|bigint|false|true`。
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureCompareTable {
    /// `schema.name` 或 `name`，用于跨连接配对。
    pub key: String,
    pub name: String,
    pub schema: Option<String>,
    pub status: StructureDiffStatus,
    pub source_ddl: Option<String>,
    pub target_ddl: Option<String>,
    pub changes: Vec<StructureChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureCompareResult {
    pub source_kind: DatabaseKind,
    pub target_kind: DatabaseKind,
    pub tables: Vec<StructureCompareTable>,
}

impl StructureCompareResult {
    pub fn count(&self, status: StructureDiffStatus) -> usize {
        self.tables
            .iter()
            .filter(|table| table.status == status)
            .count()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureCompareRequest {
    pub source: StructureEndpoint,
    pub target: StructureEndpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncOptions {
    /// 目标缺失时按源端 DDL 新建。
    pub sync_added: bool,
    /// 目标多出时删除（必须先勾选，避免误删）。
    pub drop_removed: bool,
    /// 结构不同时先删后建。
    pub recreate_modified: bool,
}

impl Default for StructureSyncOptions {
    fn default() -> Self {
        Self {
            sync_added: true,
            drop_removed: false,
            recreate_modified: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncRequest {
    pub compare: StructureCompareRequest,
    /// 空表示全部差异对象。
    pub tables: Vec<String>,
    pub options: StructureSyncOptions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncReport {
    pub created: usize,
    pub dropped: usize,
    pub recreated: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataTransferMode {
    /// 追加写入，目标已有数据保留。
    Append,
    /// 写入前清空目标表。注意：清空与后续批次不在同一事务内。
    Overwrite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTransferRequest {
    pub source: ObjectPath,
    pub target: ObjectPath,
    pub mode: DataTransferMode,
    /// 目标表不存在时按源端 DDL 建表。
    pub create_target: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTransferReport {
    pub rows: u64,
    pub batches: u64,
}

impl AppController {
    /// 对比两个库的表结构。按 `schema.name` 配对；只比较基础表，不比较视图/例程。
    pub fn compare_structures(
        &self,
        request: &StructureCompareRequest,
    ) -> fluxdb_core::Result<StructureCompareResult> {
        let source_config = self.structure_endpoint_config(&request.source)?;
        let target_config = self.structure_endpoint_config(&request.target)?;
        let source_tables = self.structure_endpoint_tables(&request.source)?;
        let target_tables = self.structure_endpoint_tables(&request.target)?;
        let source_map = source_tables
            .into_iter()
            .map(|path| (structure_key(&path), path))
            .collect::<BTreeMap<_, _>>();
        let target_map = target_tables
            .into_iter()
            .map(|path| (structure_key(&path), path))
            .collect::<BTreeMap<_, _>>();
        let keys = source_map
            .keys()
            .chain(target_map.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut tables = Vec::with_capacity(keys.len());
        for key in keys {
            let source_path = source_map.get(&key);
            let target_path = target_map.get(&key);
            tables.push(compare_structure_table(
                source_config,
                target_config,
                source_path,
                target_path,
                key,
            )?);
        }
        Ok(StructureCompareResult {
            source_kind: source_config.kind,
            target_kind: target_config.kind,
            tables,
        })
    }

    /// 按结构对比结果执行同步。每次同步前重新对比，避免使用界面上的过期快照。
    ///
    /// 不同数据库类型之间不自动翻译 DDL：返回明确错误，而不是生成可能破坏数据的语句。
    pub fn sync_structures(
        &self,
        request: &StructureSyncRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<StructureSyncReport> {
        let result = self.compare_structures(&request.compare)?;
        if result.source_kind != result.target_kind {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "结构同步要求源端与目标端数据库类型一致；跨类型请手工处理 DDL",
            ));
        }
        let target_config = self.structure_endpoint_config(&request.compare.target)?;
        let selected = request
            .tables
            .iter()
            .map(|table| table.as_str())
            .collect::<BTreeSet<_>>();
        let mut report = StructureSyncReport {
            created: 0,
            dropped: 0,
            recreated: 0,
            skipped: 0,
            errors: Vec::new(),
        };
        for table in result.tables {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::new(ErrorKind::Cancelled, "结构同步已取消"));
            }
            if !selected.is_empty() && !selected.contains(table.key.as_str()) {
                continue;
            }
            let should_create = matches!(
                table.status,
                StructureDiffStatus::Added | StructureDiffStatus::Modified
            ) && match table.status {
                StructureDiffStatus::Added => request.options.sync_added,
                StructureDiffStatus::Modified => request.options.recreate_modified,
                _ => false,
            };
            let should_drop = match table.status {
                StructureDiffStatus::Removed => request.options.drop_removed,
                StructureDiffStatus::Modified => request.options.recreate_modified,
                _ => false,
            };
            if !should_create && !should_drop {
                report.skipped += 1;
                continue;
            }
            let source_path = ObjectPath {
                connection_id: request.compare.source.connection_id,
                database: Some(request.compare.source.database()?.to_string()),
                schema: table.schema.clone(),
                name: table.name.clone(),
                kind: ObjectKind::Table,
            };
            let target_path = target_table_path(&request.compare.target, &source_path);
            let source_config = self.structure_endpoint_config(&request.compare.source)?;
            let action = match (should_drop, should_create) {
                (true, true) => "重建",
                (true, false) => "删除",
                (false, true) => "新建",
                (false, false) => continue,
            };
            progress(&format!("{action} {}", table.key));
            if should_drop {
                let drop_sql = drop_table_sql(target_config.kind, &target_path);
                if let Err(error) = execute_change_sql(target_config, &target_path, &drop_sql) {
                    report
                        .errors
                        .push(format!("{}: {}", table.key, error.message));
                    continue;
                }
                report.dropped += 1;
            }
            if should_create {
                let ddl = match table.source_ddl {
                    Some(ddl) => ddl,
                    None => {
                        report
                            .errors
                            .push(format!("{}: 源端没有可用的 DDL", table.key));
                        continue;
                    }
                };
                let ddl = rewrite_table_ddl(
                    source_config.kind,
                    &source_path,
                    &target_path,
                    &ddl,
                );
                if let Err(error) = execute_change_sql(target_config, &target_path, &ddl) {
                    report
                        .errors
                        .push(format!("{}: {}", table.key, error.message));
                    continue;
                }
                if should_drop {
                    report.recreated += 1;
                } else {
                    report.created += 1;
                }
            }
        }
        Ok(report)
    }

    /// 把源表数据写入另一连接的目标表。
    ///
    /// 源端使用连接器的一致快照分页（PG 为单事务快照），按页交给目标库分批 INSERT，
    /// 因此内存占用只与单页行数相关，不随表大小增长。
    pub fn transfer_table_data(
        &self,
        request: &DataTransferRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<DataTransferReport> {
        let source_config = self
            .connection_config(request.source.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "源连接不存在"))?
            .clone();
        let target_config = self
            .connection_config(request.target.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "目标连接不存在"))?
            .clone();
        if request.source.connection_id == request.target.connection_id
            && request.source.database == request.target.database
            && request.source.schema == request.target.schema
            && request.source.name == request.target.name
        {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "源表与目标表相同，无法传输",
            ));
        }
        ensure_read_only_allows(&target_config, WriteIntent::Mutation)?;
        let source_columns = connector_for(&source_config)?
            .list_completion_columns(
                request.source.database.as_deref(),
                request.source.schema.as_deref(),
                &request.source.name,
            )?
            .into_iter()
            .map(|column| column.name)
            .collect::<Vec<_>>();
        if source_columns.is_empty() {
            return Err(Error::new(ErrorKind::Query, "源表没有可读取字段"));
        }
        if request.create_target {
            create_target_table_if_missing(&source_config, &target_config, request)?;
        }
        let writer =
            fluxdb_connectors::TableRowWriter::new(&target_config, &request.target, &source_columns)?;
        if request.mode == DataTransferMode::Overwrite {
            progress("清空目标表");
            writer.truncate(cancel)?;
        }
        let mut rows = 0u64;
        let mut batches = 0u64;
        let mut write_error: Option<String> = None;
        self.export_pages_for_connection(
            &request.source,
            &[],
            &[],
            &|| cancel.load(Ordering::Relaxed),
            &mut |page| {
                if page.rows.is_empty() {
                    return true;
                }
                let values = page
                    .rows
                    .into_iter()
                    .map(|row| row.values)
                    .collect::<Vec<_>>();
                match writer.write_rows(&values, cancel, &mut |written, _| {
                    progress(&format!("已写入 {} 行", rows + written));
                }) {
                    Ok(written) => {
                        rows += written;
                        batches += 1;
                        true
                    }
                    Err(error) => {
                        progress(&format!("写入失败：{}", error.message));
                        write_error = Some(error.message);
                        false
                    }
                }
            },
        )?;
        if let Some(message) = write_error {
            return Err(Error::new(ErrorKind::Query, message));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::new(ErrorKind::Cancelled, "数据传输已取消"));
        }
        Ok(DataTransferReport { rows, batches })
    }

    fn structure_endpoint_config(
        &self,
        endpoint: &StructureEndpoint,
    ) -> fluxdb_core::Result<&ConnectionConfig> {
        self.connection_config(endpoint.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))
    }

    /// 枚举库内全部基础表。数据库下一层可能是表（MySQL/SQLite），也可能是 schema（PG）。
    fn structure_endpoint_tables(
        &self,
        endpoint: &StructureEndpoint,
    ) -> fluxdb_core::Result<Vec<ObjectPath>> {
        let config = self.structure_endpoint_config(endpoint)?;
        let database = endpoint.database()?;
        let parent = ObjectPath {
            connection_id: endpoint.connection_id,
            database: Some(database.to_string()),
            schema: endpoint.schema().map(str::to_string),
            name: endpoint.schema().unwrap_or(database).to_string(),
            kind: if endpoint.schema().is_some() {
                ObjectKind::Schema
            } else {
                ObjectKind::Database
            },
        };
        let connector = connector_for(config)?;
        let mut tables = Vec::new();
        collect_endpoint_tables(connector.as_ref(), &parent, 0, &mut tables)?;
        Ok(tables)
    }
}

fn collect_endpoint_tables(
    connector: &dyn fluxdb_core::Connector,
    parent: &ObjectPath,
    depth: usize,
    tables: &mut Vec<ObjectPath>,
) -> fluxdb_core::Result<()> {
    if depth > 2 {
        return Ok(());
    }
    for child in connector.list_objects(Some(parent))? {
        match child.path.kind {
            ObjectKind::Table => tables.push(child.path),
            ObjectKind::Schema => {
                collect_endpoint_tables(connector, &child.path, depth + 1, tables)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn structure_key(path: &ObjectPath) -> String {
    match path.schema.as_deref().filter(|schema| !schema.is_empty()) {
        Some(schema) => format!("{schema}.{}", path.name),
        None => path.name.clone(),
    }
}

fn compare_structure_table(
    source_config: &ConnectionConfig,
    target_config: &ConnectionConfig,
    source_path: Option<&ObjectPath>,
    target_path: Option<&ObjectPath>,
    key: String,
) -> fluxdb_core::Result<StructureCompareTable> {
    let name = source_path
        .or(target_path)
        .map(|path| path.name.clone())
        .unwrap_or_default();
    let schema = source_path
        .or(target_path)
        .and_then(|path| path.schema.clone());
    let source = match source_path {
        Some(path) => Some(load_table_signature(source_config, path)?),
        None => None,
    };
    let target = match target_path {
        Some(path) => Some(load_table_signature(target_config, path)?),
        None => None,
    };
    let (status, changes) = match (&source, &target) {
        (Some(_), None) => (StructureDiffStatus::Added, Vec::new()),
        (None, Some(_)) => (StructureDiffStatus::Removed, Vec::new()),
        (None, None) => (StructureDiffStatus::Same, Vec::new()),
        (Some(source), Some(target)) => {
            let mut changes = source
                .items
                .difference(&target.items)
                .map(|detail| StructureChange {
                    kind: StructureChangeKind::MissingInTarget,
                    detail: detail.clone(),
                })
                .chain(target.items.difference(&source.items).map(|detail| {
                    StructureChange {
                        kind: StructureChangeKind::ExtraInTarget,
                        detail: detail.clone(),
                    }
                }))
                .collect::<Vec<_>>();
            if changes.is_empty() && source.ddl != target.ddl {
                changes.push(StructureChange {
                    kind: StructureChangeKind::DefinitionChanged,
                    detail: "DDL".to_string(),
                });
            }
            let status = if changes.is_empty() {
                StructureDiffStatus::Same
            } else {
                StructureDiffStatus::Modified
            };
            (status, changes)
        }
    };
    Ok(StructureCompareTable {
        key,
        name,
        schema,
        status,
        source_ddl: source.map(|signature| signature.ddl),
        target_ddl: target.map(|signature| signature.ddl),
        changes,
    })
}

struct TableSignature {
    items: BTreeSet<String>,
    ddl: String,
}

fn load_table_signature(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<TableSignature> {
    let metadata = connector_for(config)?.load_table_design_metadata(path)?;
    Ok(TableSignature {
        items: structure_items(&metadata),
        ddl: normalize_ddl(&metadata.ddl),
    })
}

/// 把列/索引/外键压成稳定文本，避免依赖方言 DDL 的排版差异。
fn structure_items(metadata: &fluxdb_core::TableDesignMetadata) -> BTreeSet<String> {
    let mut items = BTreeSet::new();
    for column in &metadata.columns {
        items.insert(format!(
            "column|{}|{}|{}|{}",
            column.name.to_ascii_lowercase(),
            column.type_name.as_deref().unwrap_or("").to_ascii_lowercase(),
            column.nullable,
            column.primary_key
        ));
    }
    for index in &metadata.indexes {
        if index.is_primary {
            continue;
        }
        items.insert(format!(
            "index|{}|{}|{}",
            index.name.to_ascii_lowercase(),
            index.columns.join(",").to_ascii_lowercase(),
            index.is_unique
        ));
    }
    for foreign_key in &metadata.foreign_keys {
        let columns = if foreign_key.columns.is_empty() {
            foreign_key.column.clone()
        } else {
            foreign_key.columns.join(",")
        };
        let ref_columns = if foreign_key.ref_columns.is_empty() {
            foreign_key.ref_column.clone()
        } else {
            foreign_key.ref_columns.join(",")
        };
        items.insert(format!(
            "fk|{}|{}|{}|{}",
            foreign_key.name.to_ascii_lowercase(),
            columns.to_ascii_lowercase(),
            foreign_key.ref_table.to_ascii_lowercase(),
            ref_columns.to_ascii_lowercase()
        ));
    }
    items
}

/// DDL 仅用于兜底判断默认值/约束变化；去掉会随数据变化的表选项，防止误报。
fn normalize_ddl(ddl: &str) -> String {
    ddl.split_whitespace()
        .filter(|token| !token.to_ascii_uppercase().starts_with("AUTO_INCREMENT="))
        .collect::<Vec<_>>()
        .join(" ")
}

fn target_table_path(endpoint: &StructureEndpoint, source: &ObjectPath) -> ObjectPath {
    let database = endpoint.database.trim();
    ObjectPath {
        connection_id: endpoint.connection_id,
        // 目标端显式库优先；未填写时沿用源端库名（SQLite 等单库场景）。
        database: (!database.is_empty())
            .then(|| database.to_string())
            .or_else(|| source.database.clone()),
        schema: endpoint.schema().map(str::to_string).or_else(|| source.schema.clone()),
        name: source.name.clone(),
        kind: ObjectKind::Table,
    }
}

fn rewrite_table_ddl(
    kind: DatabaseKind,
    source: &ObjectPath,
    target: &ObjectPath,
    ddl: &str,
) -> String {
    let dialect = fluxdb_core::sql_dialect(kind);
    let source_name = dialect.qualified_object_name(source);
    let target_name = dialect.qualified_object_name(target);
    if source_name == target_name {
        ddl.to_string()
    } else {
        ddl.replace(&source_name, &target_name)
    }
}

fn drop_table_sql(kind: DatabaseKind, path: &ObjectPath) -> String {
    let name = fluxdb_core::sql_dialect(kind).qualified_object_name(path);
    if kind == DatabaseKind::Oracle {
        format!("DROP TABLE {name}")
    } else {
        format!("DROP TABLE IF EXISTS {name}")
    }
}

fn execute_change_sql(
    config: &ConnectionConfig,
    path: &ObjectPath,
    text: &str,
) -> fluxdb_core::Result<()> {
    ensure_read_only_allows(config, WriteIntent::Mutation)?;
    let result = connector_for(config)?.execute(&QueryRequest {
        connection_id: config.id,
        database: path.database.clone(),
        schema: path.schema.clone(),
        session_id: None,
        text: text.to_string(),
        mode: fluxdb_core::QueryMode::All,
        options: QueryExecutionOptions::default(),
    })?;
    match result.summaries.iter().find(|summary| !summary.success) {
        Some(summary) => Err(Error::new(ErrorKind::Query, summary.message.clone())),
        None => Ok(()),
    }
}

fn create_target_table_if_missing(
    source_config: &ConnectionConfig,
    target_config: &ConnectionConfig,
    request: &DataTransferRequest,
) -> fluxdb_core::Result<()> {
    let target_connector = connector_for(target_config)?;
    let existing = target_connector.list_completion_columns(
        request.target.database.as_deref(),
        request.target.schema.as_deref(),
        &request.target.name,
    )?;
    if !existing.is_empty() {
        return Ok(());
    }
    if source_config.kind != target_config.kind {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "目标表不存在，且源端与目标端数据库类型不同，无法自动建表",
        ));
    }
    let ddl = connector_for(source_config)?.table_ddl(&request.source)?;
    let ddl = rewrite_table_ddl(source_config.kind, &request.source, &request.target, &ddl);
    execute_change_sql(target_config, &request.target, &ddl)
}

#[cfg(test)]
mod structure_tools_tests {
    use super::*;

    #[test]
    fn structure_items_ignore_primary_index_and_normalize_case() {
        let metadata = fluxdb_core::TableDesignMetadata {
            columns: vec![fluxdb_core::CompletionColumn {
                database: None,
                schema: None,
                table: "orders".into(),
                name: "ID".into(),
                type_name: Some("BIGINT".into()),
                nullable: false,
                primary_key: true,
                comment: None,
                stable: None,
            }],
            indexes: vec![
                fluxdb_core::IndexInfo {
                    name: "PRIMARY".into(),
                    columns: vec!["ID".into()],
                    is_unique: true,
                    is_primary: true,
                    index_type: None,
                    comment: None,
                },
                fluxdb_core::IndexInfo {
                    name: "IDX_ID".into(),
                    columns: vec!["ID".into()],
                    is_unique: false,
                    is_primary: false,
                    index_type: None,
                    comment: None,
                },
            ],
            foreign_keys: Vec::new(),
            triggers: Vec::new(),
            ddl: String::new(),
            details_loaded: true,
        };
        let items = structure_items(&metadata);
        assert!(items.contains("column|id|bigint|false|true"));
        assert!(items.contains("index|idx_id|id|false"));
        assert!(!items.iter().any(|item| item.starts_with("index|primary")));
    }

    #[test]
    fn sqlite_compare_sync_and_transfer_round_trip() {
        use fluxdb_core::{CellValue, ConnectionDraft, Endpoint};

        let mut controller = AppController::new();
        let stamp = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let source_path = std::env::temp_dir().join(format!("fluxdb-ct-source-{stamp}.db"));
        let target_path = std::env::temp_dir().join(format!("fluxdb-ct-target-{stamp}.db"));
        let create = |controller: &mut AppController, name: &str, path: &std::path::Path| {
            controller.dispatch(AppCommand::CreateConnection(ConnectionDraft {
                name: name.into(),
                kind: DatabaseKind::Sqlite,
                endpoint: Endpoint::SqliteFile {
                    path: path.to_path_buf(),
                    read_only: false,
                },
                options: std::collections::BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                sqlserver_profile: None,
                mongo_profile: None,
                oracle_profile: None,
            }));
            controller
                .connection_configs()
                .into_iter()
                .find(|config| config.name == name)
                .expect("created connection")
        };
        let source = create(&mut controller, "ct-source", &source_path);
        let target = create(&mut controller, "ct-target", &target_path);

        for (config, sql) in [
            (
                &source,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            ),
            (&source, "INSERT INTO items (id, name) VALUES (1, 'a'), (2, 'b')"),
        ] {
            let result = connector_for(config)
                .expect("connector")
                .execute(&QueryRequest {
                    connection_id: config.id,
                    database: Some("main".into()),
                    schema: None,
                    session_id: None,
                    text: sql.into(),
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions::default(),
                })
                .expect("sqlite execute");
            assert!(result.summaries.iter().all(|summary| summary.success));
        }

        let compare = StructureCompareRequest {
            source: StructureEndpoint {
                connection_id: source.id,
                database: "main".into(),
                schema: None,
            },
            target: StructureEndpoint {
                connection_id: target.id,
                database: "main".into(),
                schema: None,
            },
        };
        let before = controller.compare_structures(&compare).expect("compare");
        assert_eq!(before.count(StructureDiffStatus::Added), 1);

        let cancel = AtomicBool::new(false);
        let sync = controller
            .sync_structures(
                &StructureSyncRequest {
                    compare: compare.clone(),
                    tables: vec!["items".into()],
                    options: StructureSyncOptions {
                        sync_added: true,
                        drop_removed: false,
                        recreate_modified: false,
                    },
                },
                &cancel,
                &mut |_| {},
            )
            .expect("sync");
        assert_eq!(sync.created, 1);
        assert!(sync.errors.is_empty());
        let after = controller.compare_structures(&compare).expect("compare");
        assert_eq!(after.count(StructureDiffStatus::Same), 1);

        let object = |config: &fluxdb_core::ConnectionConfig| ObjectPath {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            name: "items".into(),
            kind: ObjectKind::Table,
        };
        let report = controller
            .transfer_table_data(
                &DataTransferRequest {
                    source: object(&source),
                    target: object(&target),
                    mode: DataTransferMode::Append,
                    create_target: false,
                },
                &cancel,
                &mut |_| {},
            )
            .expect("transfer");
        assert_eq!(report.rows, 2);
        let page = connector_for(&target)
            .expect("connector")
            .load_data(&object(&target), 0, 10, &[], &[])
            .expect("load target");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[1], CellValue::Text("a".into()));

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    #[test]
    fn normalize_ddl_drops_data_dependent_mysql_auto_increment() {
        assert_eq!(
            normalize_ddl("CREATE TABLE `t` (`id` int) ENGINE=InnoDB AUTO_INCREMENT=42"),
            "CREATE TABLE `t` (`id` int) ENGINE=InnoDB"
        );
    }
}
