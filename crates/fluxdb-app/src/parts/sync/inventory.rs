//! 端点的表清单与配对键。
//!
//! 配对规则与结构同步一致：`schema.name`（无 schema 时用 `name`）相同即视为同一张表。
//! 只枚举基础表，不把视图/例程卷进数据同步。

use std::collections::BTreeMap;

use fluxdb_connectors::connector_for;
use fluxdb_core::{ObjectKind, ObjectPath};

use crate::AppController;

use super::model::SyncEndpoint;

/// 跨连接配对键。
pub(crate) fn table_key(path: &ObjectPath) -> String {
    match path.schema.as_deref().filter(|schema| !schema.is_empty()) {
        Some(schema) => format!("{schema}.{}", path.name),
        None => path.name.clone(),
    }
}

/// 枚举端点内的全部基础表，按配对键索引。
pub(crate) fn endpoint_tables(
    controller: &AppController,
    endpoint: &SyncEndpoint,
) -> fluxdb_core::Result<BTreeMap<String, ObjectPath>> {
    let config = controller
        .connection_config(endpoint.connection_id)
        .ok_or_else(|| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, "连接不存在"))?;
    let connector = connector_for(config)?;
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
    let mut tables = Vec::new();
    collect_tables(connector.as_ref(), &parent, 0, &mut tables)?;
    Ok(tables
        .into_iter()
        .map(|path| (table_key(&path), path))
        .collect())
}

/// 在另一端清单里找配对表。
///
/// 先按完整配对键（`schema.name`）匹配；跨类型时两端 schema 维度可能不一致
/// （MySQL 无 schema，PG 有），再按唯一表名回退。同名多张表时不猜，返回 `None`。
pub(crate) fn paired<'a>(
    tables: &'a BTreeMap<String, ObjectPath>,
    source: &ObjectPath,
) -> Option<&'a ObjectPath> {
    if let Some(path) = tables.get(&table_key(source)) {
        return Some(path);
    }
    let mut candidates = tables.values().filter(|path| path.name == source.name);
    let first = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(first)
}

/// 用户显式指定的目标表；目标端不存在该键时返回 `None`，由调用方按缺表处理。
pub(crate) fn explicit<'a>(
    tables: &'a BTreeMap<String, ObjectPath>,
    table_map: &BTreeMap<String, String>,
    source_key: &str,
) -> Option<&'a ObjectPath> {
    tables.get(table_map.get(source_key)?)
}

/// 该目标表是否被某个源表显式配对；删除多余表时不能把它算成多余。
pub(crate) fn is_mapped_target(table_map: &BTreeMap<String, String>, target_key: &str) -> bool {
    table_map.values().any(|mapped| mapped == target_key)
}

/// 解析目标表：显式配对优先；显式指定但目标端不存在时返回 `None`，
/// 不再回退按名称自动配对（用户意图优先）。
pub(crate) fn resolve<'a>(
    tables: &'a BTreeMap<String, ObjectPath>,
    source: &ObjectPath,
    source_key: &str,
    table_map: &BTreeMap<String, String>,
) -> Option<&'a ObjectPath> {
    if table_map.contains_key(source_key) {
        return explicit(tables, table_map, source_key);
    }
    paired(tables, source)
}

/// 目标端路径：显式配对优先，否则按源端名称推导。
pub(crate) fn target_path(
    endpoint: &SyncEndpoint,
    kind: fluxdb_core::DatabaseKind,
    table_map: &BTreeMap<String, String>,
    source_key: &str,
    source: &ObjectPath,
) -> ObjectPath {
    match table_map.get(source_key) {
        Some(mapped) => endpoint.object_for_key(kind, mapped),
        None => endpoint.object(kind, source),
    }
}

/// 数据库下一层可能是表（MySQL/SQLite），也可能是 schema（PG / SQL Server）。
fn collect_tables(
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
            ObjectKind::Schema => collect_tables(connector, &child.path, depth + 1, tables)?,
            _ => {}
        }
    }
    Ok(())
}

impl AppController {
    /// 枚举端点内可选的基础表。界面第一步用它渲染表清单；只读，失败直接返回错误。
    pub fn list_sync_tables(
        &self,
        endpoint: &SyncEndpoint,
    ) -> fluxdb_core::Result<Vec<super::model::SyncTableInfo>> {
        let tables = endpoint_tables(self, endpoint)?;
        Ok(tables
            .into_iter()
            .map(|(key, path)| super::model::SyncTableInfo {
                key,
                name: path.name,
                schema: path.schema,
            })
            .collect())
    }

    /// 读取单表字段清单，供「字段设置」弹框按需加载；只读。
    ///
    /// 返回完整列信息（含 `primary_key`），弹框据此预选比对键。
    /// `schema` 为空时回退端点选择的 schema（MySQL 家族没有 schema 维度）。
    pub fn list_sync_columns(
        &self,
        endpoint: &SyncEndpoint,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<fluxdb_core::CompletionColumn>> {
        let config = self
            .connection_config(endpoint.connection_id)
            .ok_or_else(|| {
                fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, "连接不存在")
            })?;
        let connector = connector_for(config)?;
        let database = endpoint.database()?;
        let schema = schema
            .filter(|schema| !schema.is_empty())
            .or(endpoint.schema());
        connector.list_completion_columns(Some(database), schema, table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::ConnectionId;

    fn path(schema: Option<&str>, name: &str) -> ObjectPath {
        ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("app".into()),
            schema: schema.map(str::to_string),
            name: name.into(),
            kind: ObjectKind::Table,
        }
    }

    #[test]
    fn pairs_across_schema_dimension_only_when_name_is_unique() {
        let target = BTreeMap::from([
            ("public.orders".to_string(), path(Some("public"), "orders")),
            ("sales.orders".to_string(), path(Some("sales"), "orders")),
        ]);
        // 同名多 schema 时不猜。
        assert!(paired(&target, &path(None, "orders")).is_none());

        let target =
            BTreeMap::from([("public.orders".to_string(), path(Some("public"), "orders"))]);
        assert!(paired(&target, &path(None, "orders")).is_some());
        // 完整键优先。
        let source = path(Some("public"), "orders");
        assert_eq!(
            paired(&target, &source).map(|path| path.schema.clone()),
            Some(Some("public".into()))
        );
    }

    #[test]
    fn explicit_map_wins_over_name_pairing() {
        let target = BTreeMap::from([
            ("public.orders".to_string(), path(Some("public"), "orders")),
            (
                "public.orders_old".to_string(),
                path(Some("public"), "orders_old"),
            ),
        ]);
        let table_map = BTreeMap::from([("orders".to_string(), "public.orders_old".to_string())]);

        let mapped = explicit(&target, &table_map, "orders").expect("显式配对");
        assert_eq!(mapped.name, "orders_old");
        assert!(is_mapped_target(&table_map, "public.orders_old"));
        assert!(!is_mapped_target(&table_map, "public.orders"));

        // 目标端不存在指定表：视为缺表，而不是退回同名自动配对。
        let missing = BTreeMap::from([("orders".to_string(), "public.absent".to_string())]);
        assert!(explicit(&target, &missing, "orders").is_none());
        assert!(resolve(&target, &path(None, "orders"), "orders", &missing).is_none());

        // 未显式指定时仍按名称自动配对。
        assert_eq!(
            resolve(&target, &path(None, "orders"), "orders", &BTreeMap::new())
                .map(|path| path.name.clone()),
            Some("orders".to_string())
        );
    }
}
