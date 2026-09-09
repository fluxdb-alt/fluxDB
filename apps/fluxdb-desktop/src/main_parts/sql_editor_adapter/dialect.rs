// sql_editor_adapter/dialect.rs —— SQL 方言定义与静态关键字表。
//
// 本文件定义 `SqlDialect` 枚举与 `KeywordDef` 关键字定义，以及各方言的
// 关键字列表、注释标记等纯逻辑能力；不依赖 GPUI 或数据库。

use fluxdb_core::DatabaseKind;

/// 单条关键字定义：词形 + 补全类型。
#[derive(Clone, Copy, Debug)]
pub struct KeywordDef {
    /// 关键字 / 内置函数名（小写）。
    pub word: &'static str,
    /// 补全类型：关键字 / 函数。
    pub kind: CompletionKind,
}

impl KeywordDef {
    const fn kw(word: &'static str) -> Self {
        Self {
            word,
            kind: CompletionKind::Keyword,
        }
    }

    const fn func(word: &'static str) -> Self {
        Self {
            word,
            kind: CompletionKind::Function,
        }
    }
}

/// 支持的 SQL 方言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlDialect {
    /// MySQL / MariaDB。
    Mysql,
    #[allow(dead_code)] // 为方言契约保留，当前宿主仅连接 MySQL/SQLite。
    /// PostgreSQL。
    Postgres,
    /// SQLite。
    Sqlite,
    #[allow(dead_code)] // 同上，保留 SQL Server 方言契约。
    /// SQL Server。
    SqlServer,
}

impl SqlDialect {
    /// fluxdb-editor-core 的 language_id。
    #[allow(dead_code)] // 供语法 provider 使用，当前语法高亮未接入，保留。
    pub fn language_id(&self) -> &'static str {
        match self {
            SqlDialect::Mysql => "sql_mysql",
            SqlDialect::Postgres => "sql_postgres",
            SqlDialect::Sqlite => "sql_sqlite",
            SqlDialect::SqlServer => "sql_sqlserver",
        }
    }

    /// 方言中文名（用于补全项 detail 展示）。
    pub fn name(&self) -> &'static str {
        match self {
            SqlDialect::Mysql => "MySQL",
            SqlDialect::Postgres => "PostgreSQL",
            SqlDialect::Sqlite => "SQLite",
            SqlDialect::SqlServer => "SQL Server",
        }
    }

    /// 行注释标记。
    #[allow(dead_code)] // 供语法 provider 使用，当前语法高亮未接入，保留。
    pub fn line_comment(&self) -> Option<&'static str> {
        Some(match self {
            SqlDialect::Mysql => "--",
            SqlDialect::Postgres => "--",
            SqlDialect::Sqlite => "--",
            SqlDialect::SqlServer => "--",
        })
    }

    /// 块注释标记 (start, end)。
    #[allow(dead_code)] // 供语法 provider 使用，当前语法高亮未接入，保留。
    pub fn block_comment(&self) -> Option<(&'static str, &'static str)> {
        Some(("/*", "*/"))
    }

    /// 该方言的关键字 / 函数表。
    pub fn keywords(&self) -> &'static [KeywordDef] {
        match self {
            SqlDialect::Mysql => MYSQL_KEYWORDS,
            SqlDialect::Postgres => POSTGRES_KEYWORDS,
            SqlDialect::Sqlite => SQLITE_KEYWORDS,
            SqlDialect::SqlServer => SQLSERVER_KEYWORDS,
        }
    }

    /// 从数据库连接类型映射到 SQL 方言。
    ///
    /// 查询编辑页只对 SQL 类连接可用；Redis / MongoDB 非 SQL，映射为 MySQL 兜底
    /// （实际不会被 SQL 编辑器使用）。
    pub fn from_database_kind(kind: DatabaseKind) -> Self {
        match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => SqlDialect::Mysql,
            DatabaseKind::Sqlite => SqlDialect::Sqlite,
            DatabaseKind::MongoDb | DatabaseKind::Redis => SqlDialect::Mysql,
        }
    }

    /// 判断某个词是否为该方言的关键字 / 函数（不区分大小写）。
    pub fn is_keyword(&self, word: &str) -> bool {
        let lower = word.to_ascii_lowercase();
        self.keywords()
            .iter()
            .any(|def| def.word == lower)
    }
}

/// MySQL 关键字 / 内置函数表。
static MYSQL_KEYWORDS: &[KeywordDef] = &[
    KeywordDef::kw("select"),
    KeywordDef::kw("from"),
    KeywordDef::kw("where"),
    KeywordDef::kw("and"),
    KeywordDef::kw("or"),
    KeywordDef::kw("not"),
    KeywordDef::kw("insert"),
    KeywordDef::kw("into"),
    KeywordDef::kw("values"),
    KeywordDef::kw("update"),
    KeywordDef::kw("set"),
    KeywordDef::kw("delete"),
    KeywordDef::kw("create"),
    KeywordDef::kw("table"),
    KeywordDef::kw("database"),
    KeywordDef::kw("schema"),
    KeywordDef::kw("alter"),
    KeywordDef::kw("drop"),
    KeywordDef::kw("index"),
    KeywordDef::kw("view"),
    KeywordDef::kw("procedure"),
    KeywordDef::kw("function"),
    KeywordDef::kw("trigger"),
    KeywordDef::kw("join"),
    KeywordDef::kw("inner"),
    KeywordDef::kw("left"),
    KeywordDef::kw("right"),
    KeywordDef::kw("outer"),
    KeywordDef::kw("on"),
    KeywordDef::kw("as"),
    KeywordDef::kw("group"),
    KeywordDef::kw("by"),
    KeywordDef::kw("order"),
    KeywordDef::kw("having"),
    KeywordDef::kw("limit"),
    KeywordDef::kw("offset"),
    KeywordDef::kw("union"),
    KeywordDef::kw("all"),
    KeywordDef::kw("distinct"),
    KeywordDef::kw("case"),
    KeywordDef::kw("when"),
    KeywordDef::kw("then"),
    KeywordDef::kw("else"),
    KeywordDef::kw("end"),
    KeywordDef::kw("in"),
    KeywordDef::kw("is"),
    KeywordDef::kw("null"),
    KeywordDef::kw("like"),
    KeywordDef::kw("between"),
    KeywordDef::kw("exists"),
    KeywordDef::kw("primary"),
    KeywordDef::kw("key"),
    KeywordDef::kw("foreign"),
    KeywordDef::kw("references"),
    KeywordDef::kw("default"),
    KeywordDef::kw("unique"),
    KeywordDef::kw("constraint"),
    KeywordDef::kw("check"),
    KeywordDef::kw("asc"),
    KeywordDef::kw("desc"),
    KeywordDef::kw("into"),
    KeywordDef::kw("use"),
    KeywordDef::kw("show"),
    KeywordDef::kw("describe"),
    KeywordDef::kw("explain"),
    KeywordDef::kw("if"),
    KeywordDef::kw("then"),
    KeywordDef::kw("begin"),
    KeywordDef::kw("commit"),
    KeywordDef::kw("rollback"),
    KeywordDef::kw("with"),
    KeywordDef::kw("char"),
    KeywordDef::kw("character"),
    KeywordDef::kw("varchar"),
    KeywordDef::kw("text"),
    KeywordDef::kw("tinyint"),
    KeywordDef::kw("smallint"),
    KeywordDef::kw("mediumint"),
    KeywordDef::kw("int"),
    KeywordDef::kw("bigint"),
    KeywordDef::kw("decimal"),
    KeywordDef::kw("float"),
    KeywordDef::kw("double"),
    KeywordDef::kw("boolean"),
    KeywordDef::kw("date"),
    KeywordDef::kw("time"),
    KeywordDef::kw("datetime"),
    KeywordDef::kw("timestamp"),
    KeywordDef::kw("json"),
    KeywordDef::kw("blob"),
    KeywordDef::kw("enum"),
    KeywordDef::kw("unsigned"),
    KeywordDef::kw("auto_increment"),
    KeywordDef::kw("comment"),
    KeywordDef::kw("charset"),
    KeywordDef::kw("collate"),
    KeywordDef::kw("engine"),
    KeywordDef::func("count"),
    KeywordDef::func("sum"),
    KeywordDef::func("avg"),
    KeywordDef::func("min"),
    KeywordDef::func("max"),
    KeywordDef::func("concat"),
    KeywordDef::func("lower"),
    KeywordDef::func("upper"),
    KeywordDef::func("length"),
    KeywordDef::func("now"),
    KeywordDef::func("date"),
    KeywordDef::func("ifnull"),
    KeywordDef::func("coalesce"),
];

/// PostgreSQL 关键字 / 内置函数表。
static POSTGRES_KEYWORDS: &[KeywordDef] = &[
    KeywordDef::kw("select"),
    KeywordDef::kw("from"),
    KeywordDef::kw("where"),
    KeywordDef::kw("and"),
    KeywordDef::kw("or"),
    KeywordDef::kw("not"),
    KeywordDef::kw("insert"),
    KeywordDef::kw("into"),
    KeywordDef::kw("values"),
    KeywordDef::kw("update"),
    KeywordDef::kw("set"),
    KeywordDef::kw("delete"),
    KeywordDef::kw("create"),
    KeywordDef::kw("table"),
    KeywordDef::kw("database"),
    KeywordDef::kw("schema"),
    KeywordDef::kw("alter"),
    KeywordDef::kw("drop"),
    KeywordDef::kw("index"),
    KeywordDef::kw("view"),
    KeywordDef::kw("sequence"),
    KeywordDef::kw("trigger"),
    KeywordDef::kw("function"),
    KeywordDef::kw("returning"),
    KeywordDef::kw("join"),
    KeywordDef::kw("inner"),
    KeywordDef::kw("left"),
    KeywordDef::kw("right"),
    KeywordDef::kw("outer"),
    KeywordDef::kw("on"),
    KeywordDef::kw("as"),
    KeywordDef::kw("group"),
    KeywordDef::kw("by"),
    KeywordDef::kw("order"),
    KeywordDef::kw("having"),
    KeywordDef::kw("limit"),
    KeywordDef::kw("offset"),
    KeywordDef::kw("union"),
    KeywordDef::kw("all"),
    KeywordDef::kw("distinct"),
    KeywordDef::kw("case"),
    KeywordDef::kw("when"),
    KeywordDef::kw("then"),
    KeywordDef::kw("else"),
    KeywordDef::kw("end"),
    KeywordDef::kw("in"),
    KeywordDef::kw("is"),
    KeywordDef::kw("null"),
    KeywordDef::kw("like"),
    KeywordDef::kw("ilike"),
    KeywordDef::kw("between"),
    KeywordDef::kw("exists"),
    KeywordDef::kw("primary"),
    KeywordDef::kw("key"),
    KeywordDef::kw("foreign"),
    KeywordDef::kw("references"),
    KeywordDef::kw("default"),
    KeywordDef::kw("unique"),
    KeywordDef::kw("constraint"),
    KeywordDef::kw("check"),
    KeywordDef::kw("asc"),
    KeywordDef::kw("desc"),
    KeywordDef::kw("with"),
    KeywordDef::kw("as"),
    KeywordDef::kw("begin"),
    KeywordDef::kw("commit"),
    KeywordDef::kw("rollback"),
    KeywordDef::kw("using"),
    KeywordDef::kw("cast"),
    KeywordDef::func("count"),
    KeywordDef::func("sum"),
    KeywordDef::func("avg"),
    KeywordDef::func("min"),
    KeywordDef::func("max"),
    KeywordDef::func("concat"),
    KeywordDef::func("lower"),
    KeywordDef::func("upper"),
    KeywordDef::func("length"),
    KeywordDef::func("now"),
    KeywordDef::func("date_trunc"),
    KeywordDef::func("coalesce"),
];

/// SQLite 关键字 / 内置函数表。
static SQLITE_KEYWORDS: &[KeywordDef] = &[
    KeywordDef::kw("select"),
    KeywordDef::kw("from"),
    KeywordDef::kw("where"),
    KeywordDef::kw("and"),
    KeywordDef::kw("or"),
    KeywordDef::kw("not"),
    KeywordDef::kw("insert"),
    KeywordDef::kw("into"),
    KeywordDef::kw("values"),
    KeywordDef::kw("update"),
    KeywordDef::kw("set"),
    KeywordDef::kw("delete"),
    KeywordDef::kw("create"),
    KeywordDef::kw("table"),
    KeywordDef::kw("database"),
    KeywordDef::kw("index"),
    KeywordDef::kw("view"),
    KeywordDef::kw("trigger"),
    KeywordDef::kw("join"),
    KeywordDef::kw("inner"),
    KeywordDef::kw("left"),
    KeywordDef::kw("right"),
    KeywordDef::kw("outer"),
    KeywordDef::kw("on"),
    KeywordDef::kw("as"),
    KeywordDef::kw("group"),
    KeywordDef::kw("by"),
    KeywordDef::kw("order"),
    KeywordDef::kw("having"),
    KeywordDef::kw("limit"),
    KeywordDef::kw("offset"),
    KeywordDef::kw("union"),
    KeywordDef::kw("all"),
    KeywordDef::kw("distinct"),
    KeywordDef::kw("case"),
    KeywordDef::kw("when"),
    KeywordDef::kw("then"),
    KeywordDef::kw("else"),
    KeywordDef::kw("end"),
    KeywordDef::kw("in"),
    KeywordDef::kw("is"),
    KeywordDef::kw("null"),
    KeywordDef::kw("like"),
    KeywordDef::kw("between"),
    KeywordDef::kw("exists"),
    KeywordDef::kw("primary"),
    KeywordDef::kw("key"),
    KeywordDef::kw("foreign"),
    KeywordDef::kw("references"),
    KeywordDef::kw("default"),
    KeywordDef::kw("unique"),
    KeywordDef::kw("constraint"),
    KeywordDef::kw("check"),
    KeywordDef::kw("asc"),
    KeywordDef::kw("desc"),
    KeywordDef::kw("with"),
    KeywordDef::kw("as"),
    KeywordDef::kw("begin"),
    KeywordDef::kw("commit"),
    KeywordDef::kw("rollback"),
    KeywordDef::func("count"),
    KeywordDef::func("sum"),
    KeywordDef::func("avg"),
    KeywordDef::func("min"),
    KeywordDef::func("max"),
    KeywordDef::func("lower"),
    KeywordDef::func("upper"),
    KeywordDef::func("length"),
    KeywordDef::func("abs"),
    KeywordDef::func("ifnull"),
    KeywordDef::func("coalesce"),
];

/// SQL Server 关键字 / 内置函数表。
static SQLSERVER_KEYWORDS: &[KeywordDef] = &[
    KeywordDef::kw("select"),
    KeywordDef::kw("from"),
    KeywordDef::kw("where"),
    KeywordDef::kw("and"),
    KeywordDef::kw("or"),
    KeywordDef::kw("not"),
    KeywordDef::kw("insert"),
    KeywordDef::kw("into"),
    KeywordDef::kw("values"),
    KeywordDef::kw("update"),
    KeywordDef::kw("set"),
    KeywordDef::kw("delete"),
    KeywordDef::kw("create"),
    KeywordDef::kw("table"),
    KeywordDef::kw("database"),
    KeywordDef::kw("schema"),
    KeywordDef::kw("alter"),
    KeywordDef::kw("drop"),
    KeywordDef::kw("index"),
    KeywordDef::kw("view"),
    KeywordDef::kw("procedure"),
    KeywordDef::kw("function"),
    KeywordDef::kw("trigger"),
    KeywordDef::kw("join"),
    KeywordDef::kw("inner"),
    KeywordDef::kw("left"),
    KeywordDef::kw("right"),
    KeywordDef::kw("outer"),
    KeywordDef::kw("on"),
    KeywordDef::kw("as"),
    KeywordDef::kw("group"),
    KeywordDef::kw("by"),
    KeywordDef::kw("order"),
    KeywordDef::kw("having"),
    KeywordDef::kw("top"),
    KeywordDef::kw("union"),
    KeywordDef::kw("all"),
    KeywordDef::kw("distinct"),
    KeywordDef::kw("case"),
    KeywordDef::kw("when"),
    KeywordDef::kw("then"),
    KeywordDef::kw("else"),
    KeywordDef::kw("end"),
    KeywordDef::kw("in"),
    KeywordDef::kw("is"),
    KeywordDef::kw("null"),
    KeywordDef::kw("like"),
    KeywordDef::kw("between"),
    KeywordDef::kw("exists"),
    KeywordDef::kw("primary"),
    KeywordDef::kw("key"),
    KeywordDef::kw("foreign"),
    KeywordDef::kw("references"),
    KeywordDef::kw("default"),
    KeywordDef::kw("unique"),
    KeywordDef::kw("constraint"),
    KeywordDef::kw("check"),
    KeywordDef::kw("asc"),
    KeywordDef::kw("desc"),
    KeywordDef::kw("go"),
    KeywordDef::kw("with"),
    KeywordDef::kw("begin"),
    KeywordDef::kw("commit"),
    KeywordDef::kw("rollback"),
    KeywordDef::kw("declare"),
    KeywordDef::kw("print"),
    KeywordDef::func("count"),
    KeywordDef::func("sum"),
    KeywordDef::func("avg"),
    KeywordDef::func("min"),
    KeywordDef::func("max"),
    KeywordDef::func("concat"),
    KeywordDef::func("lower"),
    KeywordDef::func("upper"),
    KeywordDef::func("len"),
    KeywordDef::func("getdate"),
    KeywordDef::func("isnull"),
    KeywordDef::func("coalesce"),
];
