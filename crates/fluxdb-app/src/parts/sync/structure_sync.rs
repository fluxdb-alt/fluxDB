//! 结构同步：预览比对与执行编排。
//!
//! 与数据同步一致：预览与执行各自重新读取两端元数据，不复用旧快照。执行前按
//! [`super::structure_sql::change_order`] 重排变更，整表方言能力不足时直接放弃该表，
//! 避免「改了一半才报不支持」。

use std::sync::atomic::{AtomicBool, Ordering};

use fluxdb_connectors::connector_for;
use fluxdb_core::{ConnectionConfig, DatabaseKind, Error, ErrorKind, ObjectPath, WriteIntent};

use crate::AppController;

use super::cross_type_ddl::{
    create_table_statements, foreign_key_statements, map_table_def, map_table_def_with,
};
use super::data_sync::{ensure_data_sync_comparable, structure_kinds_compatible};
use super::inventory;
use super::model::{
    StructureChange, StructureChangeAction, StructureChangeDetail, StructureObject,
    StructureSkipReason, StructureSyncOptions, StructureSyncPreview, StructureSyncReport,
    StructureSyncRequest, TableStructureDiff, TableStructureReport, TableStructureState,
};
use super::structure_diff::{
    TableDef, diff_table, rebase_foreign_key_schema, table_def_from_metadata,
};
use super::structure_sql::{
    change_order, change_sql, drop_table_sql, ensure_changes_supported, execute_change_sql,
    rewrite_table_ddl,
};

impl AppController {
    /// 比对两端结构并返回逐表差异。只读，不修改目标库。
    ///
    /// 源端有、目标端缺的表：启用「自动建表」时记为一条建表变更，否则跳过；
    /// 目标端多出的表同理受「删除多余表」控制。
    pub fn preview_structure_sync(
        &self,
        request: &StructureSyncRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<StructureSyncPreview> {
        let (source_config, target_config) =
            self.sync_endpoint_configs(&request.source, &request.target)?;
        ensure_data_sync_comparable(&source_config, &target_config)?;
        let cross_type = !structure_kinds_compatible(source_config.kind, target_config.kind);

        let source_tables = inventory::endpoint_tables(self, &request.source)?;
        let target_tables = inventory::endpoint_tables(self, &request.target)?;
        let selected = selected_tables(&request.tables);

        let mut tables = Vec::new();
        for (key, source_path) in &source_tables {
            if !selected.is_empty() && !selected.contains(key) {
                continue;
            }
            check_cancel(cancel)?;
            let target_path =
                inventory::resolve(&target_tables, source_path, key, &request.options.table_map);
            let Some(target_path) = target_path else {
                tables.push(missing_target_diff(
                    key,
                    source_path,
                    &inventory::target_path(
                        &request.target,
                        target_config.kind,
                        &request.options.table_map,
                        key,
                        source_path,
                    ),
                    &request.options,
                    &load_table_def(&source_config, source_path),
                    cross_type,
                    source_config.kind,
                    target_config.kind,
                ));
                continue;
            };
            progress(&format!("比对 {key}"));
            let result = self.diff_structure_table(
                &source_config,
                &target_config,
                source_path,
                target_path,
                key,
                &request.options,
                cross_type,
            );
            tables.push(match result {
                Ok(table) => table,
                Err(error) if error.kind == ErrorKind::Cancelled => return Err(error),
                Err(error) => TableStructureDiff::failed(
                    key.clone(),
                    source_path.name.clone(),
                    source_path.schema.clone(),
                    error.message,
                ),
            });
        }
        for (key, target_path) in &target_tables {
            if inventory::paired(&source_tables, target_path).is_some()
                || inventory::is_mapped_target(&request.options.table_map, key)
                || (!selected.is_empty() && !selected.contains(key))
            {
                continue;
            }
            tables.push(extra_target_diff(key, target_path, &request.options));
        }

        Ok(StructureSyncPreview {
            source_kind: source_config.kind,
            target_kind: target_config.kind,
            tables,
        })
    }

    /// 执行结构同步。逐表隔离失败；取消与连接错误终止整次任务。
    pub fn execute_structure_sync(
        &self,
        request: &StructureSyncRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(&str),
    ) -> fluxdb_core::Result<StructureSyncReport> {
        let (source_config, target_config) =
            self.sync_endpoint_configs(&request.source, &request.target)?;
        ensure_data_sync_comparable(&source_config, &target_config)?;
        crate::ensure_read_only_allows(&target_config, WriteIntent::Mutation)?;
        let cross_type = !structure_kinds_compatible(source_config.kind, target_config.kind);

        let source_tables = inventory::endpoint_tables(self, &request.source)?;
        let target_tables = inventory::endpoint_tables(self, &request.target)?;
        let selected = selected_tables(&request.tables);

        let mut foreign_key_queue = Vec::new();
        let mut tables = Vec::new();
        for (key, source_path) in &source_tables {
            if !selected.is_empty() && !selected.contains(key) {
                continue;
            }
            check_cancel(cancel)?;
            progress(&format!("同步 {key}"));
            let paired_target =
                inventory::resolve(&target_tables, source_path, key, &request.options.table_map);
            let target_path = inventory::target_path(
                &request.target,
                target_config.kind,
                &request.options.table_map,
                key,
                source_path,
            );
            let report = match paired_target {
                None => create_missing_table(
                    &source_config,
                    &target_config,
                    &target_path,
                    source_path,
                    key,
                    &request.options,
                    cross_type,
                    &mut foreign_key_queue,
                ),
                Some(target_path) => self.apply_structure_table(
                    &source_config,
                    &target_config,
                    source_path,
                    target_path,
                    key,
                    &request.options,
                    cross_type,
                    cancel,
                ),
            };
            if let Err(error) = report {
                if error.kind == ErrorKind::Cancelled {
                    return Err(error);
                }
                tables.push(TableStructureReport {
                    key: key.clone(),
                    name: source_path.name.clone(),
                    applied: 0,
                    skipped: false,
                    error: Some(error.message),
                });
            } else if let Ok(table) = report {
                tables.push(table);
            }
        }
        for (key, target_path) in &target_tables {
            if inventory::paired(&source_tables, target_path).is_some()
                || inventory::is_mapped_target(&request.options.table_map, key)
                || (!selected.is_empty() && !selected.contains(key))
            {
                continue;
            }
            check_cancel(cancel)?;
            tables.push(drop_extra_table(
                &target_config,
                target_path,
                key,
                &request.options,
            ));
        }

        // 外键必须等所有表都建好后再加，否则引用顺序会导致建表失败。
        for (key, target_path, foreign_keys) in foreign_key_queue {
            check_cancel(cancel)?;
            let statements =
                foreign_key_statements(target_config.kind, &target_path, &foreign_keys);
            let mut failed = None;
            let mut added = 0usize;
            for statement in &statements {
                check_cancel(cancel)?;
                if let Err(error) = execute_change_sql(&target_config, &target_path, statement) {
                    failed = Some(error.message);
                    break;
                }
                added += 1;
            }
            if let Some(report) = tables.iter_mut().find(|report| report.key == key) {
                match failed {
                    Some(message) => report.error = Some(message),
                    // 按实际执行成功的语句计数：SQLite 不支持追加外键，语句为空即 0。
                    None => report.applied += added,
                }
            }
        }

        Ok(StructureSyncReport { tables })
    }

    #[allow(clippy::too_many_arguments)]
    fn diff_structure_table(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source_path: &ObjectPath,
        target_path: &ObjectPath,
        key: &str,
        options: &StructureSyncOptions,
        cross_type: bool,
    ) -> fluxdb_core::Result<TableStructureDiff> {
        let mut source = load_table_def(source_config, source_path)?;
        let target = load_table_def(target_config, target_path)?;
        if !cross_type {
            // 同类型跨库同步时，外键里的源库 / schema 名要换成目标端，
            // 否则比对永远有差异，生成的 DDL 还会引用源库。
            rebase_foreign_key_schema(&mut source, source_path, target_path);
        }
        let (source, warnings) = if cross_type {
            map_table_def_with(
                source_config.kind,
                target_config.kind,
                &source,
                options.type_overrides.get(key),
            )
        } else {
            (source, Vec::new())
        };
        // 目标端也过一次映射，避免 `character varying` / `int(11)` 等写法差异
        // 被误判成真实变更。
        let target_cmp = if cross_type {
            map_table_def(target_config.kind, target_config.kind, &target).0
        } else {
            target
        };
        let changes = diff_table(&source, &target_cmp);
        let selected = changes
            .iter()
            .filter(|change| options.allows(change))
            .count();
        let mut diff = TableStructureDiff::ready(
            key.to_string(),
            source_path.name.clone(),
            source_path.schema.clone(),
        );
        diff.state = if changes.is_empty() {
            TableStructureState::Equal
        } else {
            TableStructureState::Different
        };
        diff.selected = selected;
        diff.warnings = warnings;
        // 函数索引不参与比对；显式提示，避免用户误以为已同步。
        let mut warned = std::collections::BTreeSet::new();
        for index in source.indexes.iter().chain(target_cmp.indexes.iter()) {
            if index.has_expression && warned.insert(index.name.clone()) {
                diff.warnings
                    .push(format!("索引 {}：含表达式键项，已跳过", index.name));
            }
        }
        // 差异视图把两端都按目标方言渲染后逐行比对：同类型时源端原文可能带
        // `int(11)` 之类写法差异，直接比原文会产生假变更。
        diff.current_ddl = Some(render_table_ddl(
            target_config.kind,
            target_path,
            &target_cmp,
        ));
        diff.desired_ddl = Some(render_table_ddl(target_config.kind, target_path, &source));
        if cross_type {
            diff.ddl = preview_change_ddl(
                target_config,
                target_path,
                &changes,
                options,
                &mut diff.warnings,
            );
        }
        diff.changes = changes;
        Ok(diff)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_structure_table(
        &self,
        source_config: &ConnectionConfig,
        target_config: &ConnectionConfig,
        source_path: &ObjectPath,
        target_path: &ObjectPath,
        key: &str,
        options: &StructureSyncOptions,
        cross_type: bool,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<TableStructureReport> {
        let mut source = load_table_def(source_config, source_path)?;
        let target = load_table_def(target_config, target_path)?;
        if !cross_type {
            rebase_foreign_key_schema(&mut source, source_path, target_path);
        }
        let (source, target) = if cross_type {
            (
                map_table_def_with(
                    source_config.kind,
                    target_config.kind,
                    &source,
                    options.type_overrides.get(key),
                )
                .0,
                map_table_def(target_config.kind, target_config.kind, &target).0,
            )
        } else {
            (source, target)
        };
        let mut changes = diff_table(&source, &target)
            .into_iter()
            .filter(|change| options.allows(change))
            .collect::<Vec<_>>();
        if changes.is_empty() {
            return Ok(TableStructureReport {
                key: key.to_string(),
                name: source_path.name.clone(),
                applied: 0,
                skipped: true,
                error: None,
            });
        }
        ensure_changes_supported(target_config.kind, &changes)?;
        changes.sort_by_key(change_order);

        let mut applied = 0;
        for change in &changes {
            check_cancel(cancel)?;
            if change.object == StructureObject::Table {
                // 表级变更只在单侧存在时出现，由 create/drop 分支处理。
                continue;
            }
            for statement in change_sql(target_config.kind, target_path, change)? {
                check_cancel(cancel)?;
                execute_change_sql(target_config, target_path, &statement)?;
            }
            applied += 1;
        }
        Ok(TableStructureReport {
            key: key.to_string(),
            name: source_path.name.clone(),
            applied,
            skipped: false,
            error: None,
        })
    }
}

/// 生成预览 SQL；目标方言不支持的对象级变更只提示、不影响其它变更。
fn preview_change_ddl(
    target_config: &ConnectionConfig,
    target_path: &ObjectPath,
    changes: &[StructureChange],
    options: &StructureSyncOptions,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut statements = Vec::new();
    for change in changes
        .iter()
        .filter(|change| options.allows(change))
        .filter(|change| change.object != StructureObject::Table)
    {
        match change_sql(target_config.kind, target_path, change) {
            Ok(generated) => statements.extend(generated),
            Err(error) => warnings.push(format!("{}：{}", change.name, error.message)),
        }
    }
    statements
}

#[allow(clippy::too_many_arguments)]
fn create_missing_table(
    source_config: &ConnectionConfig,
    target_config: &ConnectionConfig,
    target_path: &ObjectPath,
    source_path: &ObjectPath,
    key: &str,
    options: &StructureSyncOptions,
    cross_type: bool,
    foreign_key_queue: &mut Vec<(String, ObjectPath, Vec<super::model::ForeignKeyDef>)>,
) -> fluxdb_core::Result<TableStructureReport> {
    if !options.create_table {
        return Ok(TableStructureReport {
            key: key.to_string(),
            name: source_path.name.clone(),
            applied: 0,
            skipped: true,
            error: Some(StructureSkipReason::TargetMissing.message().to_string()),
        });
    }
    if cross_type {
        let source = load_table_def(source_config, source_path)?;
        let (mapped, _) = map_table_def_with(
            source_config.kind,
            target_config.kind,
            &source,
            options.type_overrides.get(key),
        );
        let plan = create_table_statements(target_config.kind, target_path, &mapped);
        for statement in &plan.statements {
            execute_change_sql(target_config, target_path, statement)?;
        }
        if options.add_foreign_key && !mapped.foreign_keys.is_empty() {
            foreign_key_queue.push((
                key.to_string(),
                target_path.clone(),
                mapped.foreign_keys.clone(),
            ));
        }
        return Ok(TableStructureReport {
            key: key.to_string(),
            name: source_path.name.clone(),
            applied: plan.statements.len().max(1),
            skipped: false,
            error: None,
        });
    }
    // 同类型：沿用源端 DDL，完整保留方言特有属性。
    let ddl = connector_for(source_config)?.table_ddl(source_path)?;
    let ddl = rewrite_table_ddl(source_config.kind, source_path, target_path, &ddl);
    execute_change_sql(target_config, target_path, &ddl)?;
    Ok(TableStructureReport {
        key: key.to_string(),
        name: source_path.name.clone(),
        applied: 1,
        skipped: false,
        error: None,
    })
}

/// 以指定方言渲染整表定义（建表 + 索引/注释 + 外键），供差异视图逐行比对。
///
/// 两端都走同一函数，外键语句也一起渲染，避免单边缺语句被显示成变更。
fn render_table_ddl(kind: DatabaseKind, path: &ObjectPath, table: &TableDef) -> String {
    let mut statements = create_table_statements(kind, path, table).statements;
    statements.extend(foreign_key_statements(kind, path, &table.foreign_keys));
    statements.join("\n")
}

/// 目标端缺表时的预览项。建表与否只影响状态，不读取目标端元数据。
#[allow(clippy::too_many_arguments)]
fn missing_target_diff(
    key: &str,
    source_path: &ObjectPath,
    target_path: &ObjectPath,
    options: &StructureSyncOptions,
    loaded: &fluxdb_core::Result<TableDef>,
    cross_type: bool,
    source_kind: fluxdb_core::DatabaseKind,
    target_kind: fluxdb_core::DatabaseKind,
) -> TableStructureDiff {
    match loaded {
        Err(error) => TableStructureDiff::failed(
            key.to_string(),
            source_path.name.clone(),
            source_path.schema.clone(),
            error.message.clone(),
        ),
        Ok(table) if options.create_table => {
            let mut diff = TableStructureDiff::ready(
                key.to_string(),
                source_path.name.clone(),
                source_path.schema.clone(),
            );
            diff.selected = 1;
            diff.changes.push(StructureChange {
                object: StructureObject::Table,
                action: StructureChangeAction::Create,
                name: source_path.name.clone(),
                detail: StructureChangeDetail::Table,
            });
            let (mapped, warnings) = if cross_type {
                map_table_def_with(
                    source_kind,
                    target_kind,
                    table,
                    options.type_overrides.get(key),
                )
            } else {
                (table.clone(), Vec::new())
            };
            diff.warnings = warnings;
            // 目标端没有该表：差异视图左半部分留空，整表都显示为新增。
            diff.desired_ddl = Some(render_table_ddl(target_kind, target_path, &mapped));
            if cross_type {
                let plan = create_table_statements(target_kind, target_path, &mapped);
                diff.warnings.extend(plan.warnings);
                diff.ddl = plan.statements;
                if options.add_foreign_key {
                    diff.ddl.extend(foreign_key_statements(
                        target_kind,
                        target_path,
                        &mapped.foreign_keys,
                    ));
                }
            }
            diff
        }
        Ok(_) => TableStructureDiff::skipped(
            key.to_string(),
            source_path.name.clone(),
            source_path.schema.clone(),
            StructureSkipReason::TargetMissing,
        ),
    }
}

/// 目标端多出的表的预览项。
fn extra_target_diff(
    key: &str,
    target_path: &ObjectPath,
    options: &StructureSyncOptions,
) -> TableStructureDiff {
    if !options.drop_table {
        return TableStructureDiff::skipped(
            key.to_string(),
            target_path.name.clone(),
            target_path.schema.clone(),
            StructureSkipReason::SourceMissing,
        );
    }
    let mut diff = TableStructureDiff::ready(
        key.to_string(),
        target_path.name.clone(),
        target_path.schema.clone(),
    );
    diff.selected = 1;
    diff.changes.push(StructureChange {
        object: StructureObject::Table,
        action: StructureChangeAction::Drop,
        name: target_path.name.clone(),
        detail: StructureChangeDetail::Table,
    });
    diff
}

fn drop_extra_table(
    target_config: &ConnectionConfig,
    target_path: &ObjectPath,
    key: &str,
    options: &StructureSyncOptions,
) -> TableStructureReport {
    if !options.drop_table {
        return TableStructureReport {
            key: key.to_string(),
            name: target_path.name.clone(),
            applied: 0,
            skipped: true,
            error: Some(StructureSkipReason::SourceMissing.message().to_string()),
        };
    }
    match execute_change_sql(
        target_config,
        target_path,
        &drop_table_sql(target_config.kind, target_path),
    ) {
        Ok(()) => TableStructureReport {
            key: key.to_string(),
            name: target_path.name.clone(),
            applied: 1,
            skipped: false,
            error: None,
        },
        Err(error) => TableStructureReport {
            key: key.to_string(),
            name: target_path.name.clone(),
            applied: 0,
            skipped: false,
            error: Some(error.message),
        },
    }
}

/// 读取单表结构快照；元数据不完整时抛错，由调用方降级为该表失败。
fn load_table_def(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<TableDef> {
    let metadata = connector_for(config)?.load_table_design_metadata(path)?;
    table_def_from_metadata(&metadata).map_err(|reason| {
        Error::new(
            ErrorKind::Unsupported,
            format!("{}：{}", path.name, reason.message()),
        )
    })
}

fn selected_tables(tables: &[String]) -> std::collections::BTreeSet<String> {
    tables
        .iter()
        .map(|table| table.trim().to_string())
        .filter(|table| !table.is_empty())
        .collect()
}

fn check_cancel(cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::new(ErrorKind::Cancelled, "操作已取消"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{create_sqlite, execute, temp_db};
    use super::*;
    use crate::SyncEndpoint;

    #[test]
    fn sqlite_preview_and_execute_add_missing_column_only() {
        let source_path = temp_db("struct-src");
        let target_path = temp_db("struct-tgt");
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "struct-source", &source_path);
        let target = create_sqlite(&mut controller, "struct-target", &target_path);
        execute(
            &source,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, note TEXT DEFAULT 'x')",
        );
        execute(
            &target,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, extra TEXT)",
        );

        let request = StructureSyncRequest {
            source: SyncEndpoint::new(source.id, "main", None),
            target: SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: StructureSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_structure_sync(&request, &cancel, &mut |_| {})
            .expect("preview");
        let table = &preview.tables[0];
        assert_eq!(table.state, TableStructureState::Different, "{table:?}");
        assert_eq!(table.selected, 1, "{table:?}");
        assert!(
            table
                .changes
                .iter()
                .any(|change| change.name == "note"
                    && change.action == StructureChangeAction::Create),
            "{table:?}"
        );
        // 删列默认关闭，但差异仍要展示出来。
        assert!(
            table.changes.iter().any(
                |change| change.name == "extra" && change.action == StructureChangeAction::Drop
            )
        );

        let report = controller
            .execute_structure_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        assert_eq!(report.total_applied(), 1, "{report:?}");
        assert!(report.failed_tables().is_empty(), "{report:?}");

        let columns = target_columns(&target);
        assert!(columns.iter().any(|column| column == "note"), "{columns:?}");
        assert!(
            columns.iter().any(|column| column == "extra"),
            "delete is off by default: {columns:?}"
        );

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    /// 显式表配对：源表与目标目标表不同名时，差异和 DDL 都要落到用户指定的表。
    #[test]
    fn explicit_table_map_targets_renamed_table() {
        let source_path = temp_db("struct-map-src");
        let target_path = temp_db("struct-map-tgt");
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "struct-map-source", &source_path);
        let target = create_sqlite(&mut controller, "struct-map-target", &target_path);
        execute(
            &source,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)",
        );
        execute(&target, "CREATE TABLE items_old (id INTEGER PRIMARY KEY)");

        let request = StructureSyncRequest {
            source: SyncEndpoint::new(source.id, "main", None),
            target: SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: StructureSyncOptions {
                table_map: std::collections::BTreeMap::from([(
                    "items".to_string(),
                    "items_old".to_string(),
                )]),
                ..StructureSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_structure_sync(&request, &cancel, &mut |_| {})
            .expect("preview");
        let table = preview.tables.iter().find(|table| table.key == "items");
        let table = table.expect("显式配对后仍以源表为差异项");
        assert_eq!(table.state, TableStructureState::Different, "{table:?}");
        assert!(
            table
                .changes
                .iter()
                .any(|change| change.name == "name"
                    && change.action == StructureChangeAction::Create),
            "{table:?}"
        );

        let report = controller
            .execute_structure_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        assert_eq!(report.total_applied(), 1, "{report:?}");
        let columns = connector_for(&target)
            .expect("connector")
            .load_data(
                &ObjectPath {
                    connection_id: target.id,
                    database: Some("main".into()),
                    schema: None,
                    name: "items_old".into(),
                    kind: fluxdb_core::ObjectKind::Table,
                },
                0,
                1,
                &[],
                &[],
            )
            .expect("load target")
            .columns
            .into_iter()
            .map(|column| column.name)
            .collect::<Vec<_>>();
        assert!(columns.iter().any(|column| column == "name"), "{columns:?}");

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    #[test]
    fn extra_target_table_is_skipped_without_drop_option() {
        let source_path = temp_db("struct-extra-src");
        let target_path = temp_db("struct-extra-tgt");
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "struct-extra-source", &source_path);
        let target = create_sqlite(&mut controller, "struct-extra-target", &target_path);
        execute(&source, "CREATE TABLE items (id INTEGER PRIMARY KEY)");
        execute(&target, "CREATE TABLE items (id INTEGER PRIMARY KEY)");
        execute(&target, "CREATE TABLE legacy (id INTEGER PRIMARY KEY)");

        let request = StructureSyncRequest {
            source: SyncEndpoint::new(source.id, "main", None),
            target: SyncEndpoint::new(target.id, "main", None),
            tables: Vec::new(),
            options: StructureSyncOptions::default(),
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_structure_sync(&request, &cancel, &mut |_| {})
            .expect("preview");
        let legacy = preview
            .tables
            .iter()
            .find(|table| table.name == "legacy")
            .expect("legacy table");
        assert_eq!(legacy.state, TableStructureState::Skipped);
        assert_eq!(legacy.skip, Some(StructureSkipReason::SourceMissing));

        let report = controller
            .execute_structure_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        assert!(report.tables.iter().all(|table| table.applied == 0));
        let tables = target_table_names(&target);
        assert!(tables.contains(&"legacy".to_string()), "{tables:?}");

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    /// SQLite 独立索引的建删走原生 CREATE/DROP INDEX，不需要整表重建。
    #[test]
    fn sqlite_index_create_and_drop_are_applied() {
        let source_path = temp_db("struct-idx-src");
        let target_path = temp_db("struct-idx-tgt");
        let mut controller = AppController::new();
        let source = create_sqlite(&mut controller, "struct-idx-source", &source_path);
        let target = create_sqlite(&mut controller, "struct-idx-target", &target_path);
        execute(
            &source,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); \
             CREATE INDEX idx_name ON items(name)",
        );
        execute(
            &target,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); \
             CREATE INDEX idx_legacy ON items(name)",
        );

        let request = StructureSyncRequest {
            source: SyncEndpoint::new(source.id, "main", None),
            target: SyncEndpoint::new(target.id, "main", None),
            tables: vec!["items".into()],
            options: StructureSyncOptions {
                drop_index: true,
                ..StructureSyncOptions::default()
            },
        };
        let cancel = AtomicBool::new(false);
        let preview = controller
            .preview_structure_sync(&request, &cancel, &mut |_| {})
            .expect("preview");
        let table = &preview.tables[0];
        assert!(
            table.changes.iter().any(|change| change.name == "idx_name"
                && change.object == StructureObject::Index
                && change.action == StructureChangeAction::Create),
            "{table:?}"
        );
        assert!(
            table
                .changes
                .iter()
                .any(|change| change.name == "idx_legacy"
                    && change.object == StructureObject::Index
                    && change.action == StructureChangeAction::Drop),
            "{table:?}"
        );

        let report = controller
            .execute_structure_sync(&request, &cancel, &mut |_| {})
            .expect("execute");
        // 一条 CREATE INDEX + 一条 DROP INDEX。
        assert_eq!(report.total_applied(), 2, "{report:?}");
        assert!(report.failed_tables().is_empty(), "{report:?}");

        let metadata = connector_for(&target)
            .expect("connector")
            .load_table_design_metadata(&item_path(&target).expect("items path"))
            .expect("target metadata");
        let indexes = metadata
            .indexes
            .iter()
            .map(|index| index.name.as_str())
            .collect::<Vec<_>>();
        assert!(indexes.contains(&"idx_name"), "{indexes:?}");
        assert!(!indexes.contains(&"idx_legacy"), "{indexes:?}");

        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(target_path);
    }

    fn target_columns(config: &ConnectionConfig) -> Vec<String> {
        item_path(config)
            .map(|path| {
                connector_for(config)
                    .expect("connector")
                    .load_data(&path, 0, 1, &[], &[])
                    .expect("load data")
                    .columns
                    .into_iter()
                    .map(|column| column.name)
                    .collect()
            })
            .expect("items table")
    }

    fn target_table_names(config: &ConnectionConfig) -> Vec<String> {
        let path = ObjectPath {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            name: "main".into(),
            kind: fluxdb_core::ObjectKind::Database,
        };
        connector_for(config)
            .expect("connector")
            .list_objects(Some(&path))
            .expect("list objects")
            .into_iter()
            .map(|child| child.path.name)
            .collect()
    }

    fn item_path(config: &ConnectionConfig) -> Option<ObjectPath> {
        let path = ObjectPath {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            name: "main".into(),
            kind: fluxdb_core::ObjectKind::Database,
        };
        connector_for(config)
            .ok()?
            .list_objects(Some(&path))
            .ok()?
            .into_iter()
            .find(|child| child.path.name == "items")
            .map(|child| child.path)
    }
}
