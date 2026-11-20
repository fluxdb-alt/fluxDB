//! 结构差异算法（纯逻辑，不连库）。
//!
//! 输入是两端已经读好的结构快照，输出对象级变更列表。约定：
//!
//! - 列/索引/外键按**名称**配对，同名不同定义记为「修改」，改名表现为删除 + 新增。
//! - 主键单独成项：PG/MySQL 的主键索引名由数据库自动生成，不能按名称比对。
//! - 类型与默认值先规范化再比较，避免仅大小写或空白差异产生噪声变更。
//! - 列顺序差异不处理：重排列会触发表重建，代价远大于收益。
//!
//! 这里不生成 SQL，保证算法可以脱离方言单测；语句生成在 [`super::structure_sql`]。

use std::collections::BTreeMap;

use fluxdb_core::{ObjectPath, TableDesignMetadata};

use super::model::{
    CheckDef, ColumnDef, ForeignKeyDef, IndexDef, StructureChange, StructureChangeAction,
    StructureChangeDetail, StructureObject, StructureSkipReason,
};

/// 单表结构快照。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableDef {
    pub columns: Vec<ColumnDef>,
    pub primary_key: Option<IndexDef>,
    /// 不含主键索引。
    pub indexes: Vec<IndexDef>,
    pub foreign_keys: Vec<ForeignKeyDef>,
    pub checks: Vec<CheckDef>,
    /// 两端是否都读取了 CHECK；任一端缺失时跳过 CHECK 比对，避免误删。
    pub checks_loaded: bool,
    /// 表注释。跨方言建表时尽量保留；同方言同步沿用源端 DDL。
    pub comment: Option<String>,
}

/// 从设计表元数据投影结构快照。
///
/// 缺类型文本时无法生成安全 ALTER，整表标记「元数据不完整」，不猜测类型。
pub(crate) fn table_def_from_metadata(
    metadata: &TableDesignMetadata,
) -> Result<TableDef, StructureSkipReason> {
    let mut columns = Vec::with_capacity(metadata.columns.len());
    for column in &metadata.columns {
        let Some(type_text) = column
            .type_name
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return Err(StructureSkipReason::IncompleteMetadata);
        };
        columns.push(ColumnDef {
            name: column.name.clone(),
            type_text: type_text.to_string(),
            nullable: column.nullable,
            primary_key: column.primary_key,
            auto_increment: metadata
                .auto_increment_columns
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&column.name)),
            default_expr: metadata.column_defaults.get(&column.name).cloned(),
            generated: metadata.generated_columns.get(&column.name).cloned(),
            comment: column.comment.clone(),
        });
    }

    let primary_index = metadata.indexes.iter().find(|index| index.is_primary);
    let primary_key = match primary_index {
        Some(index) => Some(IndexDef {
            name: index.name.clone(),
            columns: index.columns.clone(),
            unique: true,
            primary: true,
            index_type: None,
            predicate: None,
            has_expression: false,
        }),
        None => {
            // 部分方言不返回主键索引，只给列上的 primary_key 标记。
            let key_columns = columns
                .iter()
                .filter(|column| column.primary_key)
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            (!key_columns.is_empty()).then(|| IndexDef {
                name: String::new(),
                columns: key_columns,
                unique: true,
                primary: true,
                index_type: None,
                predicate: None,
                has_expression: false,
            })
        }
    };

    let indexes = metadata
        .indexes
        .iter()
        .filter(|index| !index.is_primary)
        .map(|index| {
            // 附加属性是旁路读取的：缺失即「未读取」，不能当作「没有方法/谓词」。
            let attrs = metadata.index_attributes.get(&index.name);
            IndexDef {
                name: index.name.clone(),
                columns: index.columns.clone(),
                unique: index.is_unique,
                primary: false,
                index_type: attrs.and_then(|attrs| attrs.index_type.clone()),
                predicate: attrs.and_then(|attrs| attrs.predicate.clone()),
                has_expression: attrs.is_some_and(|attrs| attrs.has_expression),
            }
        })
        .collect();

    let foreign_keys = metadata
        .foreign_keys
        .iter()
        .map(|foreign_key| {
            let columns = if foreign_key.columns.is_empty() {
                vec![foreign_key.column.clone()]
            } else {
                foreign_key.columns.clone()
            };
            let ref_columns = if foreign_key.ref_columns.is_empty() {
                vec![foreign_key.ref_column.clone()]
            } else {
                foreign_key.ref_columns.clone()
            };
            let actions = metadata.foreign_key_actions.get(&foreign_key.name);
            ForeignKeyDef {
                name: foreign_key.name.clone(),
                columns,
                ref_schema: foreign_key.ref_schema.clone(),
                ref_table: foreign_key.ref_table.clone(),
                ref_columns,
                on_delete: actions.and_then(|actions| actions.on_delete.clone()),
                on_update: actions.and_then(|actions| actions.on_update.clone()),
            }
        })
        .collect();

    let checks = metadata
        .checks
        .iter()
        .map(|check| CheckDef {
            name: check.name.clone(),
            expression: check.expression.clone(),
        })
        .collect();

    Ok(TableDef {
        columns,
        primary_key,
        indexes,
        foreign_keys,
        checks,
        checks_loaded: metadata.checks_loaded,
        comment: metadata.table_comment.clone(),
    })
}

/// 同类型跨库同步：外键里引用源库 / schema 的部分改成目标端。
///
/// MySQL 的 `ref_schema` 是库名、PG 是 schema 名。源表引用自身所在的库时，
/// 目标端应落在目标库；不重写会一直判为差异，生成的 DDL 还会指向源库。
/// 只重写「等于源端自身库 / schema」的引用，显式引用其它库的保持原样。
pub(crate) fn rebase_foreign_key_schema(
    def: &mut TableDef,
    source_path: &ObjectPath,
    target_path: &ObjectPath,
) {
    let source_scope = source_path
        .schema
        .as_deref()
        .or(source_path.database.as_deref());
    let target_scope = target_path
        .schema
        .as_deref()
        .or(target_path.database.as_deref())
        .map(str::to_string);
    for foreign_key in &mut def.foreign_keys {
        if foreign_key.ref_schema.as_deref() == source_scope {
            foreign_key.ref_schema.clone_from(&target_scope);
        }
    }
}

/// 比对两端结构，返回源端相对目标端的变更。
///
/// 输出顺序与执行顺序无关；执行前由 [`super::structure_sql`] 按依赖重排。
pub(crate) fn diff_table(source: &TableDef, target: &TableDef) -> Vec<StructureChange> {
    let mut changes = Vec::new();
    diff_columns(source, target, &mut changes);
    diff_primary_key(source, target, &mut changes);
    diff_indexes(source, target, &mut changes);
    diff_foreign_keys(source, target, &mut changes);
    diff_checks(source, target, &mut changes);
    changes
}

/// 列按名称配对；`BTreeMap` 让输出顺序稳定，便于快照测试。
fn diff_columns(source: &TableDef, target: &TableDef, changes: &mut Vec<StructureChange>) {
    let source_columns = source
        .columns
        .iter()
        .map(|column| (column.name.clone(), column))
        .collect::<BTreeMap<_, _>>();
    let target_columns = target
        .columns
        .iter()
        .map(|column| (column.name.clone(), column))
        .collect::<BTreeMap<_, _>>();

    for (name, after) in &source_columns {
        match target_columns.get(name) {
            None => changes.push(StructureChange {
                object: StructureObject::Column,
                action: StructureChangeAction::Create,
                name: name.clone(),
                detail: StructureChangeDetail::Column {
                    before: None,
                    after: Some((*after).clone()),
                },
            }),
            Some(before) => {
                if !columns_equal(before, after) {
                    changes.push(StructureChange {
                        object: StructureObject::Column,
                        action: StructureChangeAction::Alter,
                        name: name.clone(),
                        detail: StructureChangeDetail::Column {
                            before: Some((*before).clone()),
                            after: Some((*after).clone()),
                        },
                    });
                }
            }
        }
    }
    for (name, before) in &target_columns {
        if !source_columns.contains_key(name) {
            changes.push(StructureChange {
                object: StructureObject::Column,
                action: StructureChangeAction::Drop,
                name: name.clone(),
                detail: StructureChangeDetail::Column {
                    before: Some((*before).clone()),
                    after: None,
                },
            });
        }
    }
}

/// 列定义比较：类型、可空、默认值、生成表达式、注释。主键标记由主键项单独处理。
fn columns_equal(before: &ColumnDef, after: &ColumnDef) -> bool {
    normalize_type(&before.type_text) == normalize_type(&after.type_text)
        && before.nullable == after.nullable
        && normalize_default(before.default_expr.as_deref())
            == normalize_default(after.default_expr.as_deref())
        && before
            .generated
            .as_ref()
            .map(|generated| normalize_expr(Some(&generated.expression)))
            == after
                .generated
                .as_ref()
                .map(|generated| normalize_expr(Some(&generated.expression)))
        && before.generated.as_ref().map(|generated| generated.stored)
            == after.generated.as_ref().map(|generated| generated.stored)
        && normalize_comment(before.comment.as_deref())
            == normalize_comment(after.comment.as_deref())
}

fn diff_primary_key(source: &TableDef, target: &TableDef, changes: &mut Vec<StructureChange>) {
    let before = target.primary_key.as_ref();
    let after = source.primary_key.as_ref();
    if before.map(|key| &key.columns) == after.map(|key| &key.columns) {
        return;
    }
    let action = match (before, after) {
        (None, Some(_)) => StructureChangeAction::Create,
        (Some(_), None) => StructureChangeAction::Drop,
        _ => StructureChangeAction::Alter,
    };
    changes.push(StructureChange {
        object: StructureObject::PrimaryKey,
        action,
        name: "PRIMARY".to_string(),
        detail: StructureChangeDetail::PrimaryKey {
            before: before.cloned(),
            after: after.cloned(),
        },
    });
}

fn diff_indexes(source: &TableDef, target: &TableDef, changes: &mut Vec<StructureChange>) {
    // 函数/表达式索引无法按列名重建，跳过比对（预览层会给出提示）。
    let source_indexes = source
        .indexes
        .iter()
        .filter(|index| !index.has_expression)
        .map(|index| (index.name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let target_indexes = target
        .indexes
        .iter()
        .filter(|index| !index.has_expression)
        .map(|index| (index.name.clone(), index))
        .collect::<BTreeMap<_, _>>();

    for (name, after) in &source_indexes {
        match target_indexes.get(name) {
            None => changes.push(StructureChange {
                object: StructureObject::Index,
                action: StructureChangeAction::Create,
                name: name.clone(),
                detail: StructureChangeDetail::Index {
                    before: None,
                    after: Some((*after).clone()),
                },
            }),
            Some(before) => {
                if !indexes_equal(before, after) {
                    changes.push(StructureChange {
                        object: StructureObject::Index,
                        action: StructureChangeAction::Alter,
                        name: name.clone(),
                        detail: StructureChangeDetail::Index {
                            before: Some((*before).clone()),
                            after: Some((*after).clone()),
                        },
                    });
                }
            }
        }
    }
    for (name, before) in &target_indexes {
        if !source_indexes.contains_key(name) {
            changes.push(StructureChange {
                object: StructureObject::Index,
                action: StructureChangeAction::Drop,
                name: name.clone(),
                detail: StructureChangeDetail::Index {
                    before: Some((*before).clone()),
                    after: None,
                },
            });
        }
    }
}

fn indexes_equal(before: &IndexDef, after: &IndexDef) -> bool {
    before.unique == after.unique
        && before.columns == after.columns
        // 方法/谓词只在两端都读到（或都有值）时比较：缺失表示未读取。
        && option_text_equal(before.index_type.as_deref(), after.index_type.as_deref())
        && option_text_equal(before.predicate.as_deref(), after.predicate.as_deref())
}

fn diff_foreign_keys(source: &TableDef, target: &TableDef, changes: &mut Vec<StructureChange>) {
    let source_keys = source
        .foreign_keys
        .iter()
        .map(|foreign_key| (foreign_key.name.clone(), foreign_key))
        .collect::<BTreeMap<_, _>>();
    let target_keys = target
        .foreign_keys
        .iter()
        .map(|foreign_key| (foreign_key.name.clone(), foreign_key))
        .collect::<BTreeMap<_, _>>();

    for (name, after) in &source_keys {
        match target_keys.get(name) {
            None => changes.push(StructureChange {
                object: StructureObject::ForeignKey,
                action: StructureChangeAction::Create,
                name: name.clone(),
                detail: StructureChangeDetail::ForeignKey {
                    before: None,
                    after: Some((*after).clone()),
                },
            }),
            Some(before) => {
                if !foreign_keys_equal(before, after) {
                    changes.push(StructureChange {
                        object: StructureObject::ForeignKey,
                        action: StructureChangeAction::Alter,
                        name: name.clone(),
                        detail: StructureChangeDetail::ForeignKey {
                            before: Some((*before).clone()),
                            after: Some((*after).clone()),
                        },
                    });
                }
            }
        }
    }
    for (name, before) in &target_keys {
        if !source_keys.contains_key(name) {
            changes.push(StructureChange {
                object: StructureObject::ForeignKey,
                action: StructureChangeAction::Drop,
                name: name.clone(),
                detail: StructureChangeDetail::ForeignKey {
                    before: Some((*before).clone()),
                    after: None,
                },
            });
        }
    }
}

fn foreign_keys_equal(before: &ForeignKeyDef, after: &ForeignKeyDef) -> bool {
    before.columns == after.columns
        && before.ref_schema == after.ref_schema
        && before.ref_table.eq_ignore_ascii_case(&after.ref_table)
        && before.ref_columns == after.ref_columns
        && option_text_equal(before.on_delete.as_deref(), after.on_delete.as_deref())
        && option_text_equal(before.on_update.as_deref(), after.on_update.as_deref())
}

/// CHECK 约束按名称配对；名称不同但表达式相同视为同一约束（自动命名跨库不一致）。
/// 任一端未读取 CHECK 时整体跳过，不能把「未读取」当成「没有约束」。
fn diff_checks(source: &TableDef, target: &TableDef, changes: &mut Vec<StructureChange>) {
    if !source.checks_loaded || !target.checks_loaded {
        return;
    }
    let source_checks = source
        .checks
        .iter()
        .map(|check| (check.name.clone(), check))
        .collect::<BTreeMap<_, _>>();
    let mut target_checks = target
        .checks
        .iter()
        .map(|check| (check.name.clone(), check))
        .collect::<BTreeMap<_, _>>();
    let mut pending = Vec::new();

    for (name, after) in &source_checks {
        match target_checks.remove(name) {
            Some(before) if checks_equal(before, after) => {}
            Some(before) => changes.push(StructureChange {
                object: StructureObject::Check,
                action: StructureChangeAction::Alter,
                name: name.clone(),
                detail: StructureChangeDetail::Check {
                    before: Some(before.clone()),
                    after: Some((*after).clone()),
                },
            }),
            None => pending.push(*after),
        }
    }
    // 名称命中失败后再按表达式配对：自动生成名（如 `tbl_chk_1` / `tbl_col_check`）
    // 在不同库里通常不同，按名比对会制造无意义的删建。
    for after in pending {
        let matched = target_checks.iter().find_map(|(name, before)| {
            (normalize_expr(Some(&before.expression)) == normalize_expr(Some(&after.expression)))
                .then(|| name.clone())
        });
        match matched {
            Some(name) => {
                target_checks.remove(&name);
            }
            None => changes.push(StructureChange {
                object: StructureObject::Check,
                action: StructureChangeAction::Create,
                name: after.name.clone(),
                detail: StructureChangeDetail::Check {
                    before: None,
                    after: Some(after.clone()),
                },
            }),
        }
    }
    for (name, before) in target_checks {
        changes.push(StructureChange {
            object: StructureObject::Check,
            action: StructureChangeAction::Drop,
            name,
            detail: StructureChangeDetail::Check {
                before: Some(before.clone()),
                after: None,
            },
        });
    }
}

fn checks_equal(before: &CheckDef, after: &CheckDef) -> bool {
    normalize_expr(Some(&before.expression)) == normalize_expr(Some(&after.expression))
}

/// 文本项比较：任一侧缺失（方言未读取）时视为相同，避免把「未知」当成差异。
fn option_text_equal(before: Option<&str>, after: Option<&str>) -> bool {
    match (normalize_expr(before), normalize_expr(after)) {
        (Some(before), Some(after)) => before.eq_ignore_ascii_case(&after),
        _ => true,
    }
}

/// 类型比较前统一大小写并去掉空白：`VARCHAR(100)` 与 `varchar(100)` 等价。
fn normalize_type(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 默认值只做首尾空白归一；字符串字面量大小写敏感，不能整体小写。
fn normalize_default(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalize_comment(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// 表达式/方法文本归一：去首尾空白并压缩内部连续空白。
fn normalize_expr(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::{
        CheckMeta, CompletionColumn, ForeignKeyActions, ForeignKeyInfo, GeneratedColumn,
        IndexAttributes, IndexInfo,
    };

    fn column(name: &str, type_text: &str, nullable: bool) -> CompletionColumn {
        CompletionColumn {
            database: None,
            schema: None,
            table: "orders".into(),
            name: name.into(),
            type_name: Some(type_text.into()),
            nullable,
            primary_key: false,
            comment: None,
            stable: None,
        }
    }

    fn metadata(
        columns: Vec<CompletionColumn>,
        indexes: Vec<IndexInfo>,
        foreign_keys: Vec<ForeignKeyInfo>,
    ) -> TableDesignMetadata {
        TableDesignMetadata {
            columns,
            indexes,
            foreign_keys,
            triggers: Vec::new(),
            column_defaults: BTreeMap::new(),
            auto_increment_columns: Default::default(),
            generated_columns: Default::default(),
            index_attributes: Default::default(),
            foreign_key_actions: Default::default(),
            checks: Vec::new(),
            checks_loaded: false,
            table_comment: None,
            ddl: String::new(),
            details_loaded: true,
        }
    }

    fn plain_columns(names: &[&str]) -> Vec<CompletionColumn> {
        names
            .iter()
            .map(|name| column(name, "integer", false))
            .collect()
    }

    #[test]
    fn identical_structures_produce_no_changes() {
        let source =
            table_def_from_metadata(&metadata(plain_columns(&["id", "name"]), vec![], vec![]))
                .unwrap();
        let target =
            table_def_from_metadata(&metadata(plain_columns(&["id", "name"]), vec![], vec![]))
                .unwrap();
        assert!(diff_table(&source, &target).is_empty());
    }

    #[test]
    fn column_create_alter_drop_are_reported() {
        let source = table_def_from_metadata(&metadata(
            vec![
                column("id", "integer", false),
                column("added", "text", true),
            ],
            vec![],
            vec![],
        ))
        .unwrap();
        let target = table_def_from_metadata(&metadata(
            vec![
                column("ID", "integer", false),
                column("removed", "text", true),
            ],
            vec![],
            vec![],
        ))
        .unwrap();
        let changes = diff_table(&source, &target);
        // 名称区分大小写：`id` 与 `ID` 视为不同列（Create id + Drop ID）。
        assert_eq!(changes.len(), 4);
        assert!(
            changes
                .iter()
                .any(|change| change.action == StructureChangeAction::Create
                    && change.name == "added")
        );
        assert!(
            changes
                .iter()
                .any(|change| change.action == StructureChangeAction::Drop
                    && change.name == "removed")
        );
        assert!(
            changes
                .iter()
                .any(|change| change.action == StructureChangeAction::Drop && change.name == "ID")
        );
    }

    #[test]
    fn type_case_and_whitespace_are_equivalent() {
        let source = table_def_from_metadata(&metadata(
            vec![column("id", "VARCHAR( 100 )", true)],
            vec![],
            vec![],
        ))
        .unwrap();
        let target = table_def_from_metadata(&metadata(
            vec![column("id", "varchar(100)", true)],
            vec![],
            vec![],
        ))
        .unwrap();
        assert!(diff_table(&source, &target).is_empty());
    }

    #[test]
    fn column_defaults_are_compared_only_when_present() {
        let mut source_metadata = metadata(vec![column("id", "integer", false)], vec![], vec![]);
        source_metadata
            .column_defaults
            .insert("id".into(), "42".into());
        let source = table_def_from_metadata(&source_metadata).unwrap();
        let target = table_def_from_metadata(&metadata(
            vec![column("id", "integer", false)],
            vec![],
            vec![],
        ))
        .unwrap();
        let changes = diff_table(&source, &target);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn nullable_change_is_alter() {
        let source = table_def_from_metadata(&metadata(
            vec![column("id", "integer", true)],
            vec![],
            vec![],
        ))
        .unwrap();
        let target = table_def_from_metadata(&metadata(
            vec![column("id", "integer", false)],
            vec![],
            vec![],
        ))
        .unwrap();
        let changes = diff_table(&source, &target);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn primary_key_is_compared_by_columns_not_name() {
        let source = table_def_from_metadata(&metadata(
            plain_columns(&["id", "other"]),
            vec![IndexInfo {
                name: "orders_src_pkey".into(),
                columns: vec!["id".into()],
                is_unique: true,
                is_primary: true,
                index_type: None,
                comment: None,
            }],
            vec![],
        ))
        .unwrap();
        let target = table_def_from_metadata(&metadata(
            plain_columns(&["id", "other"]),
            vec![IndexInfo {
                name: "orders_pkey".into(),
                columns: vec!["id".into()],
                is_unique: true,
                is_primary: true,
                index_type: None,
                comment: None,
            }],
            vec![],
        ))
        .unwrap();
        assert!(diff_table(&source, &target).is_empty());

        let changed = table_def_from_metadata(&metadata(
            plain_columns(&["id", "other"]),
            vec![IndexInfo {
                name: "orders_pkey".into(),
                columns: vec!["id".into(), "other".into()],
                is_unique: true,
                is_primary: true,
                index_type: None,
                comment: None,
            }],
            vec![],
        ))
        .unwrap();
        let changes = diff_table(&changed, &target);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object, StructureObject::PrimaryKey);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn index_changes_detect_definition_mismatch() {
        let source = table_def_from_metadata(&metadata(
            plain_columns(&["id"]),
            vec![IndexInfo {
                name: "idx_a".into(),
                columns: vec!["id".into()],
                is_unique: true,
                is_primary: false,
                index_type: None,
                comment: None,
            }],
            vec![],
        ))
        .unwrap();
        let target = table_def_from_metadata(&metadata(
            plain_columns(&["id"]),
            vec![IndexInfo {
                name: "idx_a".into(),
                columns: vec!["id".into()],
                is_unique: false,
                is_primary: false,
                index_type: None,
                comment: None,
            }],
            vec![],
        ))
        .unwrap();
        let changes = diff_table(&source, &target);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object, StructureObject::Index);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn composite_foreign_key_pairing_is_order_sensitive() {
        let foreign_key = |columns: &[&str], ref_columns: &[&str]| ForeignKeyInfo {
            name: "fk_order".into(),
            column: columns[0].into(),
            ref_schema: None,
            ref_table: "customers".into(),
            ref_column: ref_columns[0].into(),
            columns: columns.iter().map(|column| (*column).into()).collect(),
            ref_columns: ref_columns.iter().map(|column| (*column).into()).collect(),
        };
        let source = table_def_from_metadata(&metadata(
            plain_columns(&["a", "b"]),
            vec![],
            vec![foreign_key(&["a", "b"], &["x", "y"])],
        ))
        .unwrap();
        let target = table_def_from_metadata(&metadata(
            plain_columns(&["a", "b"]),
            vec![],
            vec![foreign_key(&["a", "b"], &["y", "x"])],
        ))
        .unwrap();
        let changes = diff_table(&source, &target);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object, StructureObject::ForeignKey);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn checks_are_created_dropped_and_altered_by_expression() {
        let mut source_meta = metadata(plain_columns(&["id"]), vec![], vec![]);
        source_meta.checks = vec![
            CheckMeta {
                name: "chk_a".into(),
                expression: "id > 0".into(),
                definition: String::new(),
            },
            CheckMeta {
                name: "chk_new".into(),
                expression: "id < 100".into(),
                definition: String::new(),
            },
        ];
        source_meta.checks_loaded = true;
        let mut target_meta = metadata(plain_columns(&["id"]), vec![], vec![]);
        target_meta.checks = vec![
            CheckMeta {
                name: "chk_a".into(),
                expression: "id > 1".into(),
                definition: String::new(),
            },
            CheckMeta {
                name: "chk_gone".into(),
                expression: "id <> 0".into(),
                definition: String::new(),
            },
        ];
        target_meta.checks_loaded = true;
        let changes = diff_table(
            &table_def_from_metadata(&source_meta).unwrap(),
            &table_def_from_metadata(&target_meta).unwrap(),
        );
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert!(
            changes
                .iter()
                .any(|change| change.name == "chk_a"
                    && change.action == StructureChangeAction::Alter)
        );
        assert!(changes.iter().any(
            |change| change.name == "chk_new" && change.action == StructureChangeAction::Create
        ));
        assert!(changes.iter().any(
            |change| change.name == "chk_gone" && change.action == StructureChangeAction::Drop
        ));
        assert!(
            changes
                .iter()
                .all(|change| change.object == StructureObject::Check)
        );
    }

    #[test]
    fn unloaded_checks_are_never_dropped() {
        let mut source_meta = metadata(plain_columns(&["id"]), vec![], vec![]);
        source_meta.checks_loaded = true;
        let changes = diff_table(
            &table_def_from_metadata(&source_meta).unwrap(),
            &table_def_from_metadata(&metadata(plain_columns(&["id"]), vec![], vec![])).unwrap(),
        );
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn generated_expression_change_is_alter_but_whitespace_is_ignored() {
        let generated = |expression: &str| GeneratedColumn {
            expression: expression.into(),
            stored: Some(true),
        };
        let with_generated = |expression: &str| {
            let mut meta = metadata(vec![column("id", "integer", false)], vec![], vec![]);
            meta.generated_columns
                .insert("id".into(), generated(expression));
            meta
        };
        let changes = diff_table(
            &table_def_from_metadata(&with_generated("a  +  b")).unwrap(),
            &table_def_from_metadata(&with_generated(" a + b ")).unwrap(),
        );
        assert!(changes.is_empty(), "{changes:?}");
        let changes = diff_table(
            &table_def_from_metadata(&with_generated("a + b")).unwrap(),
            &table_def_from_metadata(&with_generated("a - b")).unwrap(),
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object, StructureObject::Column);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn index_type_change_is_alter() {
        let with_type = |index_type: &str| {
            let mut meta = metadata(
                plain_columns(&["id"]),
                vec![IndexInfo {
                    name: "idx_a".into(),
                    columns: vec!["id".into()],
                    is_unique: false,
                    is_primary: false,
                    index_type: None,
                    comment: None,
                }],
                vec![],
            );
            meta.index_attributes.insert(
                "idx_a".into(),
                IndexAttributes {
                    index_type: Some(index_type.into()),
                    ..Default::default()
                },
            );
            meta
        };
        let changes = diff_table(
            &table_def_from_metadata(&with_type("HASH")).unwrap(),
            &table_def_from_metadata(&with_type("BTREE")).unwrap(),
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object, StructureObject::Index);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn foreign_key_action_change_is_alter() {
        let with_action = |action: &str| {
            let mut meta = metadata(
                plain_columns(&["customer_id"]),
                vec![],
                vec![ForeignKeyInfo {
                    name: "fk_order".into(),
                    column: "customer_id".into(),
                    ref_schema: None,
                    ref_table: "customers".into(),
                    ref_column: "id".into(),
                    columns: vec!["customer_id".into()],
                    ref_columns: vec!["id".into()],
                }],
            );
            meta.foreign_key_actions.insert(
                "fk_order".into(),
                ForeignKeyActions {
                    on_delete: Some(action.into()),
                    on_update: None,
                },
            );
            meta
        };
        let changes = diff_table(
            &table_def_from_metadata(&with_action("CASCADE")).unwrap(),
            &table_def_from_metadata(&with_action("RESTRICT")).unwrap(),
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].object, StructureObject::ForeignKey);
        assert_eq!(changes[0].action, StructureChangeAction::Alter);
    }

    #[test]
    fn foreign_key_schema_is_rebased_to_target_database() {
        let path = |database: &str| ObjectPath {
            connection_id: fluxdb_core::ConnectionId(1),
            database: Some(database.into()),
            schema: None,
            name: "orders".into(),
            kind: fluxdb_core::ObjectKind::Table,
        };
        let mut def = TableDef {
            columns: Vec::new(),
            primary_key: None,
            indexes: Vec::new(),
            foreign_keys: vec![
                ForeignKeyDef {
                    name: "fk_local".into(),
                    columns: vec!["id".into()],
                    ref_schema: Some("src_db".into()),
                    ref_table: "parent".into(),
                    ref_columns: vec!["id".into()],
                    on_delete: None,
                    on_update: None,
                },
                ForeignKeyDef {
                    name: "fk_remote".into(),
                    columns: vec!["id".into()],
                    ref_schema: Some("shared_db".into()),
                    ref_table: "parent".into(),
                    ref_columns: vec!["id".into()],
                    on_delete: None,
                    on_update: None,
                },
            ],
            checks: Vec::new(),
            checks_loaded: false,
            comment: None,
        };
        rebase_foreign_key_schema(&mut def, &path("src_db"), &path("dst_db"));
        assert_eq!(def.foreign_keys[0].ref_schema.as_deref(), Some("dst_db"));
        // 显式引用其它库的外键不动。
        assert_eq!(def.foreign_keys[1].ref_schema.as_deref(), Some("shared_db"));
    }

    #[test]
    fn missing_type_text_skips_table_instead_of_guessing() {
        let mut column = column("id", "integer", false);
        column.type_name = None;
        let result = table_def_from_metadata(&metadata(vec![column], vec![], vec![]));
        assert_eq!(result, Err(StructureSkipReason::IncompleteMetadata));
    }
}
