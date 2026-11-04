// 数据表 SQL 的方言差异抽象（本文件经 `include!` 汇入 fluxdb-core crate 根作用域，故用 // 而非 //! 头注释）。
//
// 数据展示页「筛选 / 排序 / 分页」的预览 SQL（`SELECT * FROM … WHERE … ORDER BY … LIMIT …`）
// 曾散落在 desktop UI 层用 `DatabaseKind` 分支 + 普通函数拼接。本模块把「标识符引用、
// 对象名限定、分页子句」这三类方言差异收敛为统一 `SqlDialect` 接口：
// - 标识符引用：PostgreSQL/SQLite 双引号（内部 `"` → `""`）、MySQL/TiDB 反引号（`` ` `` → ``` `` ```）。
// - 对象名限定：PostgreSQL 只生成 `"schema"."table"`（不跨库三段名）；MySQL/TiDB 生成
//   `` `db`.`schema`.`table` ``，SQLite 生成 `"db"."table"`（逐段仅在存在时拼接）。
// - 分页子句：PostgreSQL/MySQL/SQLite/达梦 `LIMIT … OFFSET …`；SQL Server 与 Oracle
//   用 `OFFSET … FETCH`（SQL Server 要求 ORDER BY）。
// - 行数上限后缀：通用查询与 AI 取数要给用户写的 SELECT 封顶。默认 ` LIMIT n`；
//   Oracle 用 ` FETCH FIRST n ROWS ONLY`；SQL Server 无法后缀表达（`TOP` 在列位置）。
//   以后新增方言只实现本接口，不再改 app 层的封顶逻辑。
//
// UI 只持有 `DatabaseKind` 与 `ObjectPath`，通过 [`sql_dialect`] 取方言对象，不直接判断引号规则。
// App、Connector 与 UI 共用单段标识符转义；对象限定与分页按各调用方契约选择使用。
// 字符串字面量不是标识符，不通过本接口处理。

/// SQL 方言接口：集中标识符引用、对象名限定与分页渲染。
pub trait SqlDialect {
    /// 按方言引用单段标识符（表名/字段名/schema 等）。
    fn quote_identifier(&self, value: &str) -> String;

    /// 按方言生成限定对象名（表/视图）。
    fn qualified_object_name(&self, path: &ObjectPath) -> String;

    /// 生成分页子句（`LIMIT n` / `OFFSET o`）。
    fn render_limit_offset(&self, limit: u64, offset: u64) -> String;

    /// 给列位置不定、但尾部允许追加子句的 SELECT 加行数上限（通用查询 / AI 取数路径）。
    ///
    /// 返回 `None` 表示该方言不能以后缀形式表达上限：SQL Server 的 `TOP` 必须插在
    /// `SELECT [ALL|DISTINCT]` 之后，由调用方单独插入。
    fn render_row_limit_suffix(&self, limit: u64) -> Option<String> {
        Some(format!(" LIMIT {limit}"))
    }

    /// 行数上限能否与 `FOR UPDATE` 等锁定子句同时出现在一条 SELECT 里。
    ///
    /// 默认可以（上限在前、锁定子句在后）。Oracle 的 row_limiting_clause 与
    /// for_update_clause 互斥，同用会报 ORA-02014，需要跳过追加。
    fn row_limit_allows_locking_clause(&self) -> bool {
        true
    }
}

/// PostgreSQL 方言：标识符双引号（内部 `"` → `""`）；对象名 `"schema"."table"`，不生成跨库三段名。
#[derive(Clone, Copy, Debug)]
pub struct PostgresDialect;

/// MySQL/TiDB 方言：标识符反引号（内部 `` ` `` → ``` `` ```）；对象名 `` `db`.`schema`.`table` ``。
#[derive(Clone, Copy, Debug)]
pub struct MySqlDialect;

/// SQLite 方言：标识符双引号；对象名 `"db"."table"`（SQLite 无 schema 概念）。
#[derive(Clone, Copy, Debug)]
pub struct SqliteDialect;

/// 达梦 DM8 方言：标识符双引号，对象名 `"schema"."table"`，分页 `LIMIT/OFFSET`。
#[derive(Clone, Copy, Debug)]
pub struct DamengDialect;

/// SQL Server 方言：标识符方括号（内部 `]` → `]]`）；对象名 `[schema].[table]`。
///
/// 不生成三段名：Azure SQL Database 与部分 Babelfish 端点不支持跨库引用，
/// 连接器会在目标数据库内执行，database 只用于对象树分组。
#[derive(Clone, Copy, Debug)]
pub struct SqlServerDialect;

/// Oracle 方言：标识符双引号（内部 `"` → `""`，保留大小写）；对象名 `"OWNER"."TABLE"`。
///
/// 只生成两段名：Oracle 的 schema 即对象 Owner，`PDB.SCHEMA.TABLE` 这种三段名不是合法
/// 的普通对象引用。分页用 12c+ 的 `OFFSET … ROWS FETCH NEXT … ROWS ONLY`，
/// **要求调用方已给出 ORDER BY**；无排序时由连接器补主键或 `ORDER BY NULL`。
#[derive(Clone, Copy, Debug)]
pub struct OracleDialect;

impl SqlDialect for PostgresDialect {
    fn quote_identifier(&self, value: &str) -> String {
        quote_identifier_with_delimiter(value, '"')
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
        quote_mysql_identifier(value)
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
        quote_identifier_with_delimiter(value, '"')
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

impl SqlDialect for DamengDialect {
    fn quote_identifier(&self, value: &str) -> String {
        quote_identifier_with_delimiter(value, '"')
    }

    fn qualified_object_name(&self, path: &ObjectPath) -> String {
        let mut parts = Vec::with_capacity(2);
        if let Some(schema) = path.schema.as_deref().filter(|schema| !schema.is_empty()) {
            parts.push(self.quote_identifier(schema));
        }
        parts.push(self.quote_identifier(&path.name));
        parts.join(".")
    }

    fn render_limit_offset(&self, limit: u64, offset: u64) -> String {
        render_limit_offset(limit, offset)
    }
}

impl SqlDialect for SqlServerDialect {
    fn quote_identifier(&self, value: &str) -> String {
        format!("[{}]", value.replace(']', "]]"))
    }

    fn qualified_object_name(&self, path: &ObjectPath) -> String {
        let mut parts = Vec::with_capacity(2);
        if let Some(schema) = path.schema.as_deref().filter(|schema| !schema.is_empty()) {
            parts.push(self.quote_identifier(schema));
        }
        parts.push(self.quote_identifier(&path.name));
        parts.join(".")
    }

    fn render_limit_offset(&self, limit: u64, offset: u64) -> String {
        // OFFSET/FETCH 要求 ORDER BY；调用方在没有排序时需自行补 ORDER BY (SELECT NULL)
        // 或改用 TOP。这里只负责带排序的分页片段。
        format!(" OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY")
    }

    fn render_row_limit_suffix(&self, _limit: u64) -> Option<String> {
        // `TOP` 必须紧跟在 `SELECT [ALL|DISTINCT]` 之后，调用方单独插入。
        None
    }
}

impl SqlDialect for OracleDialect {
    fn quote_identifier(&self, value: &str) -> String {
        quote_identifier_with_delimiter(value, '"')
    }

    fn qualified_object_name(&self, path: &ObjectPath) -> String {
        let mut parts = Vec::with_capacity(2);
        if let Some(schema) = path.schema.as_deref().filter(|schema| !schema.is_empty()) {
            parts.push(self.quote_identifier(schema));
        }
        parts.push(self.quote_identifier(&path.name));
        parts.join(".")
    }

    fn render_limit_offset(&self, limit: u64, offset: u64) -> String {
        format!(" OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY")
    }

    fn render_row_limit_suffix(&self, limit: u64) -> Option<String> {
        // Oracle 没有 `LIMIT`；12c+ 用 row_limiting_clause，单独 `FETCH FIRST` 不要求
        // ORDER BY（只有 OFFSET 才要求）。
        Some(format!(" FETCH FIRST {limit} ROWS ONLY"))
    }

    fn row_limit_allows_locking_clause(&self) -> bool {
        // row_limiting_clause 与 for_update_clause 互斥（ORA-02014）。
        false
    }
}

/// 用 MySQL/TiDB 的反引号引用单段标识符，并转义内部反引号。
pub fn quote_mysql_identifier(value: &str) -> String {
    quote_identifier_with_delimiter(value, '`')
}

fn quote_identifier_with_delimiter(value: &str, delimiter: char) -> String {
    let escaped = value.replace(delimiter, &format!("{delimiter}{delimiter}"));
    format!("{delimiter}{escaped}{delimiter}")
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
/// MySQL / MariaDB / TiDB 共用反引号方言；MongoDB/Redis 无「表数据预览」SQL，回退 MySQL 保持可渲染。
pub fn sql_dialect(kind: DatabaseKind) -> &'static dyn SqlDialect {
    match kind {
        DatabaseKind::Postgres => &PostgresDialect,
        DatabaseKind::Dameng => &DamengDialect,
        DatabaseKind::SqlServer => &SqlServerDialect,
        DatabaseKind::Oracle => &OracleDialect,
        DatabaseKind::Sqlite => &SqliteDialect,
        DatabaseKind::MySql
        | DatabaseKind::TiDb
        | DatabaseKind::MariaDb
        | DatabaseKind::MongoDb
        | DatabaseKind::Redis => &MySqlDialect,
    }
}

#[cfg(test)]
mod sqlserver_dialect_tests {
    use super::{SqlDialect as _, SqlServerDialect};
    use crate::ObjectPath;

    #[test]
    fn quotes_sqlserver_identifiers_with_brackets_and_escapes_closing_brackets() {
        assert_eq!(SqlServerDialect.quote_identifier("order]items"), "[order]]items]");
    }

    #[test]
    fn qualifies_sqlserver_objects_with_schema_only() {
        let path = ObjectPath {
            connection_id: crate::ConnectionId(1),
            database: Some("sales".into()),
            schema: Some("dbo".into()),
            name: "customers".into(),
            kind: crate::ObjectKind::Table,
        };
        assert_eq!(SqlServerDialect.qualified_object_name(&path), "[dbo].[customers]");
    }

    #[test]
    fn renders_sqlserver_offset_fetch_pagination() {
        assert_eq!(
            SqlServerDialect.render_limit_offset(25, 50),
            " OFFSET 50 ROWS FETCH NEXT 25 ROWS ONLY"
        );
    }

    #[test]
    fn sqlserver_has_no_row_limit_suffix() {
        // TOP 必须插在 SELECT 列表之前，不能后缀追加。
        assert_eq!(SqlServerDialect.render_row_limit_suffix(100), None);
    }
}


#[cfg(test)]
mod oracle_dialect_tests {
    use super::{OracleDialect, SqlDialect as _};
    use crate::ObjectPath;

    #[test]
    fn quotes_oracle_identifiers_and_keeps_case() {
        assert_eq!(OracleDialect.quote_identifier("Emp\"loyee"), "\"Emp\"\"loyee\"");
        assert_eq!(OracleDialect.quote_identifier("EMPLOYEES"), "\"EMPLOYEES\"");
    }

    #[test]
    fn qualifies_oracle_objects_with_owner_only() {
        let path = ObjectPath {
            connection_id: crate::ConnectionId(1),
            database: Some("FREEPDB1".into()),
            schema: Some("HR".into()),
            name: "EMPLOYEES".into(),
            kind: crate::ObjectKind::Table,
        };
        assert_eq!(OracleDialect.qualified_object_name(&path), "\"HR\".\"EMPLOYEES\"");
    }

    #[test]
    fn renders_oracle_offset_fetch_pagination() {
        assert_eq!(
            OracleDialect.render_limit_offset(25, 50),
            " OFFSET 50 ROWS FETCH NEXT 25 ROWS ONLY"
        );
    }

    #[test]
    fn oracle_row_limit_suffix_uses_fetch_first() {
        assert_eq!(
            OracleDialect.render_row_limit_suffix(100).as_deref(),
            Some(" FETCH FIRST 100 ROWS ONLY")
        );
    }
}
