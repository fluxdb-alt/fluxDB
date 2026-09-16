// 数据表 SQL 的方言差异抽象（本文件经 `include!` 汇入 fluxdb-core crate 根作用域，故用 // 而非 //! 头注释）。
//
// 数据展示页「筛选 / 排序 / 分页」的预览 SQL（`SELECT * FROM … WHERE … ORDER BY … LIMIT …`）
// 曾散落在 desktop UI 层用 `DatabaseKind` 分支 + 普通函数拼接。本模块把「标识符引用、
// 对象名限定、分页子句」这三类方言差异收敛为统一 `SqlDialect` 接口：
// - 标识符引用：PostgreSQL/SQLite 双引号（内部 `"` → `""`）、MySQL/TiDB 反引号（`` ` `` → ``` `` ```）。
// - 对象名限定：PostgreSQL 只生成 `"schema"."table"`（不跨库三段名）；MySQL/TiDB 生成
//   `` `db`.`schema`.`table` ``，SQLite 生成 `` `db`.`table` ``（逐段仅在存在时拼接）。
// - 分页子句：三者 `LIMIT … OFFSET …` 语法一致，统一渲染。
//
// UI 只持有 `DatabaseKind` 与 `ObjectPath`，通过 [`sql_dialect`] 取方言对象，不直接判断引号规则。
// connector 层既有 `pg_quote_identifier`/`mysql_quote_identifier` 的转义规则与本模块一致；
// 未来如让 connector 复用可在此收敛（本模块目前只服务「预览生成」，未改 connector 执行路径）。

/// SQL 方言接口：集中预览 SQL 所需的标识符引用 / 对象名限定 / 分页渲染。
pub trait SqlDialect {
    /// 按方言引用单段标识符（表名/字段名/schema 等）。
    fn quote_identifier(&self, value: &str) -> String;

    /// 按方言生成限定对象名（表/视图）。
    fn qualified_object_name(&self, path: &ObjectPath) -> String;

    /// 生成分页子句（`LIMIT n` / `OFFSET o`）。
    fn render_limit_offset(&self, limit: u64, offset: u64) -> String;
}

/// PostgreSQL 方言：标识符双引号（内部 `"` → `""`）；对象名 `"schema"."table"`，不生成跨库三段名。
#[derive(Clone, Copy, Debug)]
pub struct PostgresDialect;

/// MySQL/TiDB 方言：标识符反引号（内部 `` ` `` → ``` `` ```）；对象名 `` `db`.`schema`.`table` ``。
#[derive(Clone, Copy, Debug)]
pub struct MySqlDialect;

/// SQLite 方言：标识符双引号；对象名 `` `db`.`table` ``（SQLite 无 schema 概念）。
#[derive(Clone, Copy, Debug)]
pub struct SqliteDialect;

impl SqlDialect for PostgresDialect {
    fn quote_identifier(&self, value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\"\""))
    }

    fn qualified_object_name(&self, path: &ObjectPath) -> String {
        let mut parts = Vec::with_capacity(2);
        if let Some(schema) = path.schema.as_deref().filter(|s| !s.is_empty()) {
            parts.push(self.quote_identifier(schema));
        }
        parts.push(self.quote_identifier(&path.name));
        parts.join(".")
    }

    fn render_limit_offset(&self, limit: u64, offset: u64) -> String {
        render_limit_offset(limit, offset)
    }
}

impl SqlDialect for MySqlDialect {
    fn quote_identifier(&self, value: &str) -> String {
        format!("`{}`", value.replace('`', "``"))
    }

    fn qualified_object_name(&self, path: &ObjectPath) -> String {
        let mut parts = Vec::with_capacity(3);
        if let Some(database) = path.database.as_deref() {
            parts.push(self.quote_identifier(database));
        }
        if let Some(schema) = path.schema.as_deref().filter(|s| !s.is_empty()) {
            parts.push(self.quote_identifier(schema));
        }
        parts.push(self.quote_identifier(&path.name));
        parts.join(".")
    }

    fn render_limit_offset(&self, limit: u64, offset: u64) -> String {
        render_limit_offset(limit, offset)
    }
}

impl SqlDialect for SqliteDialect {
    fn quote_identifier(&self, value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\"\""))
    }

    fn qualified_object_name(&self, path: &ObjectPath) -> String {
        let mut parts = Vec::with_capacity(2);
        if let Some(database) = path.database.as_deref() {
            parts.push(self.quote_identifier(database));
        }
        parts.push(self.quote_identifier(&path.name));
        parts.join(".")
    }

    fn render_limit_offset(&self, limit: u64, offset: u64) -> String {
        render_limit_offset(limit, offset)
    }
}

fn render_limit_offset(limit: u64, offset: u64) -> String {
    let mut out = format!(" LIMIT {limit}");
    if offset > 0 {
        out.push_str(&format!(" OFFSET {offset}"));
    }
    out
}

/// 按 `DatabaseKind` 解析出对应方言对象（`&dyn SqlDialect`）。
///
/// MySQL 与 TiDB 共用反引号方言；MongoDB/Redis 无「表数据预览」SQL，回退 MySQL 保持可渲染。
pub fn sql_dialect(kind: DatabaseKind) -> &'static dyn SqlDialect {
    match kind {
        DatabaseKind::Postgres => &PostgresDialect,
        DatabaseKind::Sqlite => &SqliteDialect,
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MongoDb | DatabaseKind::Redis => {
            &MySqlDialect
        }
    }
}

#[cfg(test)]
mod sql_dialect_tests {
    use super::*;

    fn path(database: Option<&str>, schema: Option<&str>, name: &str) -> ObjectPath {
        ObjectPath {
            connection_id: crate::ConnectionId(1),
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            name: name.to_string(),
            kind: crate::ObjectKind::Table,
        }
    }

    #[test]
    fn postgres_quotes_identifiers_and_schema_table() {
        let d = &PostgresDialect;
        assert_eq!(d.quote_identifier("id"), "\"id\"");
        assert_eq!(d.quote_identifier("a\"b"), "\"a\"\"b\"");
        // PG 只生成 schema.table，不跨库三段名。
        assert_eq!(
            d.qualified_object_name(&path(Some("fluxdb_manual"), Some("tenant_a"), "orders")),
            "\"tenant_a\".\"orders\""
        );
        // 无 schema 只用表名。
        assert_eq!(
            d.qualified_object_name(&path(Some("fluxdb_manual"), None, "orders")),
            "\"orders\""
        );
        assert_eq!(d.render_limit_offset(1000, 0), " LIMIT 1000");
        assert_eq!(d.render_limit_offset(50, 100), " LIMIT 50 OFFSET 100");
    }

    #[test]
    fn mysql_quotes_backticks_and_db_schema_table() {
        let d = &MySqlDialect;
        assert_eq!(d.quote_identifier("id"), "`id`");
        assert_eq!(d.quote_identifier("a`b"), "`a``b`");
        assert_eq!(
            d.qualified_object_name(&path(Some("fluxdb_manual"), Some("tenant_a"), "orders")),
            "`fluxdb_manual`.`tenant_a`.`orders`"
        );
        assert_eq!(
            d.qualified_object_name(&path(Some("fluxdb_manual"), None, "orders")),
            "`fluxdb_manual`.`orders`"
        );
        assert_eq!(d.render_limit_offset(1000, 0), " LIMIT 1000");
    }

    #[test]
    fn sqlite_quotes_double_quotes_and_db_table() {
        let d = &SqliteDialect;
        assert_eq!(d.quote_identifier("id"), "\"id\"");
        assert_eq!(
            d.qualified_object_name(&path(Some("main"), None, "users")),
            "\"main\".\"users\""
        );
        assert_eq!(d.render_limit_offset(10, 20), " LIMIT 10 OFFSET 20");
    }

    #[test]
    fn sql_dialect_maps_database_kinds() {
        assert_eq!(
            sql_dialect(DatabaseKind::Postgres).quote_identifier("id"),
            "\"id\""
        );
        assert_eq!(
            sql_dialect(DatabaseKind::MySql).quote_identifier("id"),
            "`id`"
        );
        assert_eq!(
            sql_dialect(DatabaseKind::TiDb).qualified_object_name(&path(
                Some("db"),
                Some("s"),
                "t"
            )),
            "`db`.`s`.`t`"
        );
        assert_eq!(
            sql_dialect(DatabaseKind::Sqlite).quote_identifier("id"),
            "\"id\""
        );
    }
}
