// sql_editor_adapter/dialect.rs —— SQL 方言定义与静态关键字表。
//
// 本文件定义 `SqlDialect` 枚举与 `KeywordDef` 关键字定义，以及各方言的
// 关键字列表、注释标记等纯逻辑能力；不依赖 GPUI 或数据库。

/// 单条关键字定义：词形 + 补全类型。
#[derive(Clone, Copy, Debug)]
pub struct KeywordDef {
    /// 关键字 / 内置函数名。手写表用小写，从补全引擎生成的表保持大写原样，
    /// 因此匹配一律走 [`KeywordDef::matches`]，不要直接比较字符串。
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

    /// 与已小写化的词比对（大小写不敏感，兼容大写词表）。
    pub fn matches(&self, word_lower: &str) -> bool {
        self.word.eq_ignore_ascii_case(word_lower)
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
    /// Oracle。语法树仍由 tree-sitter-sequel 解析（它没有 PL/SQL 语法），
    /// 但关键字表、补全方言与标识符引用按 Oracle 走。
    Oracle,
    /// 达梦。PL/SQL 与 Oracle 同源，关键字表共用；分页/`TOP` 差异走补全引擎的词表。
    Dameng,
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
            SqlDialect::Oracle => "sql_oracle",
            SqlDialect::Dameng => "sql_dameng",
        }
    }

    /// 方言中文名（用于补全项 detail 展示）。
    pub fn name(&self) -> &'static str {
        match self {
            SqlDialect::Mysql => "MySQL",
            SqlDialect::Postgres => "PostgreSQL",
            SqlDialect::Sqlite => "SQLite",
            SqlDialect::SqlServer => "SQL Server",
            SqlDialect::Oracle => "Oracle",
            SqlDialect::Dameng => "Dameng",
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
            SqlDialect::Oracle => "--",
            SqlDialect::Dameng => "--",
        })
    }

    /// 块注释标记 (start, end)。
    #[allow(dead_code)] // 供语法 provider 使用，当前语法高亮未接入，保留。
    pub fn block_comment(&self) -> Option<(&'static str, &'static str)> {
        Some(("/*", "*/"))
    }

    /// 该方言的关键字 / 函数表。
    ///
    /// Oracle / 达梦没有手抄一份：直接取补全引擎的方言词表（[`fluxdb_completion`]），
    /// 保证「高亮认的词」与「补全给的词」永远一致。多词条目（如 `LEFT JOIN`）
    /// 词法器匹配不到单 token，建表时滤掉。
    pub fn keywords(&self) -> &'static [KeywordDef] {
        match self {
            SqlDialect::Mysql => MYSQL_KEYWORDS,
            SqlDialect::Postgres => POSTGRES_KEYWORDS,
            SqlDialect::Sqlite => SQLITE_KEYWORDS,
            SqlDialect::SqlServer => SQLSERVER_KEYWORDS,
            SqlDialect::Oracle => ORACLE_TABLE.as_slice(),
            SqlDialect::Dameng => DAMENG_TABLE.as_slice(),
        }
    }

    /// 从数据库连接类型映射到 SQL 方言。
    ///
    /// 查询编辑页只对 SQL 类连接可用；Redis / MongoDB 非 SQL，映射为 MySQL 兜底
    /// （实际不会被 SQL 编辑器使用）。
    pub fn from_database_kind(kind: DatabaseKind) -> Self {
        match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => SqlDialect::Mysql,
            DatabaseKind::Sqlite => SqlDialect::Sqlite,
            DatabaseKind::Postgres | DatabaseKind::Kingbase => SqlDialect::Postgres,
            DatabaseKind::Dameng => SqlDialect::Dameng,
            DatabaseKind::Oracle => SqlDialect::Oracle,
            DatabaseKind::SqlServer => SqlDialect::SqlServer,
            DatabaseKind::MongoDb | DatabaseKind::Redis => SqlDialect::Mysql,
        }
    }

    /// 反向映射：编辑器方言对应的数据库类型。
    ///
    /// 与 [`SqlDialect::from_database_kind`] 互逆（TiDB 归到 MySQL：两者语法一致，
    /// 该枚举不区分）。
    pub fn database_kind(&self) -> DatabaseKind {
        match self {
            SqlDialect::Mysql => DatabaseKind::MySql,
            SqlDialect::Postgres => DatabaseKind::Postgres,
            SqlDialect::Sqlite => DatabaseKind::Sqlite,
            SqlDialect::SqlServer => DatabaseKind::SqlServer,
            SqlDialect::Oracle => DatabaseKind::Oracle,
            SqlDialect::Dameng => DatabaseKind::Dameng,
        }
    }

    /// 判断某个词是否为该方言的关键字 / 函数（不区分大小写）。
    pub fn is_keyword(&self, word: &str) -> bool {
        let lower = word.to_ascii_lowercase();
        self.keywords().iter().any(|def| def.matches(&lower))
    }
}

/// Oracle 关键字 / 函数表，由补全引擎的 `ORACLE` 词表生成。
static ORACLE_TABLE: std::sync::LazyLock<Vec<KeywordDef>> =
    std::sync::LazyLock::new(|| completion_keyword_defs(&fluxdb_completion::ORACLE));

/// 达梦关键字 / 函数表，由补全引擎的 `DAMENG` 词表生成。
static DAMENG_TABLE: std::sync::LazyLock<Vec<KeywordDef>> =
    std::sync::LazyLock::new(|| completion_keyword_defs(&fluxdb_completion::DAMENG));

/// 把补全方言的关键字 / 函数词表转成 [`KeywordDef`]。
///
/// 直接借用补全引擎的 `'static` 词条，不再另抄一份（两处词表迟早分叉）。
/// 多词条目（`LEFT JOIN`、`IS NOT NULL`）在补全候选里照常出现，词法器则单个
/// token 匹配不到，因此两边都不受影响。
fn completion_keyword_defs(
    dialect: &'static dyn fluxdb_completion::CompletionDialect,
) -> Vec<KeywordDef> {
    dialect
        .keywords()
        .iter()
        .map(|keyword| KeywordDef::kw(keyword))
        .chain(
            dialect
                .functions()
                .iter()
                .map(|function| KeywordDef::func(function)),
        )
        .collect()
}

/// 数据页筛选条件输入使用的关键字表。
///
/// 达梦复用 PostgreSQL 方言作为通用 SQL 编辑能力，但筛选补全需要补上
/// 达梦常用函数与伪列；MongoDB/Redis 没有 SQL 条件，返回空表。
pub fn data_filter_keywords(kind: DatabaseKind) -> Vec<KeywordDef> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => MYSQL_KEYWORDS.to_vec(),
        DatabaseKind::Postgres | DatabaseKind::Kingbase => POSTGRES_KEYWORDS.to_vec(),
        DatabaseKind::Sqlite => SQLITE_KEYWORDS.to_vec(),
        DatabaseKind::SqlServer => SQLSERVER_KEYWORDS.to_vec(),
        DatabaseKind::Dameng => {
            let mut keywords = DAMENG_FILTER_KEYWORDS.to_vec();
            keywords.extend(
                POSTGRES_KEYWORDS
                    .iter()
                    .copied()
                    .filter(|keyword| {
                        !DAMENG_FILTER_KEYWORDS
                            .iter()
                            .any(|specific| specific.word == keyword.word)
                    }),
            );
            keywords
        }
        // Oracle 不合并 PostgreSQL 词表：PG 的 `LIMIT` 在 Oracle 不存在，
        // 补进候选会让人写出无法执行的筛选条件。
        DatabaseKind::Oracle => ORACLE_FILTER_KEYWORDS.to_vec(),
        DatabaseKind::MongoDb | DatabaseKind::Redis => Vec::new(),
    }
}

/// 条件补全里展示的数据库名称；达梦不能显示成 PostgreSQL。
pub fn data_filter_dialect_name(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "MySQL",
        DatabaseKind::TiDb => "TiDB",
        DatabaseKind::MariaDb => "MariaDB",
        DatabaseKind::Sqlite => "SQLite",
        DatabaseKind::Postgres => "PostgreSQL",
        DatabaseKind::Kingbase => "Kingbase",
        DatabaseKind::Dameng => "Dameng",
        DatabaseKind::SqlServer => "SQL Server",
        DatabaseKind::Oracle => "Oracle",
        DatabaseKind::MongoDb => "MongoDB",
        DatabaseKind::Redis => "Redis",
    }
}

static DAMENG_FILTER_KEYWORDS: &[KeywordDef] = &[
    KeywordDef::kw("rownum"),
    KeywordDef::kw("connect"),
    KeywordDef::kw("start"),
    KeywordDef::kw("prior"),
    KeywordDef::func("sysdate"),
    KeywordDef::func("nvl"),
    KeywordDef::func("decode"),
    KeywordDef::func("to_char"),
    KeywordDef::func("to_date"),
    KeywordDef::func("to_number"),
    KeywordDef::func("instr"),
    KeywordDef::func("substr"),
    KeywordDef::func("trunc"),
    KeywordDef::func("listagg"),
];

/// Oracle 筛选补全词表：只放 Oracle 真实存在的伪列与函数。
static ORACLE_FILTER_KEYWORDS: &[KeywordDef] = &[
    KeywordDef::kw("rownum"),
    KeywordDef::kw("connect"),
    KeywordDef::kw("start"),
    KeywordDef::kw("prior"),
    KeywordDef::kw("nulls"),
    KeywordDef::func("sysdate"),
    KeywordDef::func("systimestamp"),
    KeywordDef::func("nvl"),
    KeywordDef::func("nvl2"),
    KeywordDef::func("decode"),
    KeywordDef::func("to_char"),
    KeywordDef::func("to_date"),
    KeywordDef::func("to_number"),
    KeywordDef::func("instr"),
    KeywordDef::func("substr"),
    KeywordDef::func("trunc"),
    KeywordDef::func("listagg"),
    KeywordDef::func("coalesce"),
];

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

#[cfg(test)]
mod dialect_oracle_tests {
    use super::*;

    /// Oracle / 达梦不能落回 PostgreSQL 方言：那会让 `/` 分隔符、PL/SQL 块边界
    /// 和 `LIMIT`（Oracle 不存在）一起漏进来。映射必须一一对应。
    #[test]
    fn oracle_and_dameng_keep_their_own_dialect() {
        for (kind, expected) in [
            (DatabaseKind::Oracle, SqlDialect::Oracle),
            (DatabaseKind::Dameng, SqlDialect::Dameng),
        ] {
            let dialect = SqlDialect::from_database_kind(kind);
            assert_eq!(dialect, expected, "{kind:?}");
            assert_eq!(dialect.database_kind(), kind, "{kind:?} 往返不能失真");
        }
    }

    /// 词表取自补全引擎，PL/SQL 词必须能高亮；Oracle 没有 `LIMIT`，不能借 PG 的词。
    #[test]
    fn oracle_keywords_carry_plsql_and_drop_limit() {
        let dialect = SqlDialect::Oracle;
        for word in ["begin", "declare", "procedure", "loop", "exception", "nextval"] {
            assert!(dialect.is_keyword(word), "{word} 应被 Oracle 词表收录");
        }
        assert!(!dialect.is_keyword("limit"), "Oracle 没有 LIMIT");
        assert!(dialect.is_keyword("rownum"));
    }

    /// 高亮路径按词表分类：函数名要给 `function` 颜色，不能被当成关键字。
    #[test]
    fn oracle_functions_are_classified_as_functions() {
        let is_function = |word: &str| {
            SqlDialect::Oracle
                .keywords()
                .iter()
                .any(|def| def.matches(word) && def.kind == CompletionKind::Function)
        };
        assert!(is_function("nvl"));
        assert!(is_function("to_char"));
        assert!(!is_function("begin"));
    }
}
