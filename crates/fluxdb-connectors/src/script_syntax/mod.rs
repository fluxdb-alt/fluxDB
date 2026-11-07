//! 客户端脚本边界规则：把一段脚本文本切成可执行语句。
//!
//! 分两层，各管一件事：
//! 1. 词法交给 `sqlparser::dialect::Dialect`。替代引号 `q'…'` / `nq'…'`、`#` 行注释、
//!    `\'` 转义、`$$…$$`、`[…]`、`"…"` 的差异都由方言词法器处理，本层不重复实现。
//! 2. 边界由 [`ScriptSyntax`] 按方言回答：语句头属于哪种结束条件、哪些词开启/关闭一层块、
//!    有没有独占一行的批分隔符。遍历只写一份，方言只提供判定。
//!
//! PL/SQL 包体是唯一无法纯靠词法判定的形状：包体的初始化段 `BEGIN` 与包本身共用同一个
//! `END`，深度永远回不到 0。这类语句按 SQL*Plus 的写法要求独占一行的 `/` 收尾——
//! 与 Flyway 的 `PLSQL_DELIMITER` 同口径。
//!
//! 分词失败时 [`split`] 返回 `None`，由调用方退回手写扫描器——例如 Oracle 条件编译
//! `$IF $$x` 会被词法器当成美元引号。这类输入原本就切不干净，回退保持历史行为。

use fluxdb_core::DatabaseKind;
use sqlparser::dialect::{
    Dialect as SqlParserDialect, GenericDialect, MsSqlDialect, MySqlDialect, OracleDialect,
    PostgreSqlDialect, SQLiteDialect,
};
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};

mod oracle;
mod sqlserver;

use oracle::ORACLE_SYNTAX;
use sqlserver::SQLSERVER_SYNTAX;

/// 语句头决定的结束条件。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum StatementForm {
    /// 普通 SQL：深度为 0 时遇到 `;` 即结束。
    #[default]
    Sql,
    /// 块结构：块真正开启之前一律不分句（`DECLARE` 声明段、过程 `IS/AS` 之后的声明段），
    /// 开启之后等深度归零。
    Block {
        /// 语句头本身占一层块（`CREATE PACKAGE [BODY]`、`CREATE TYPE BODY`）。
        /// 单个过程/函数不占：第一层由过程体里的 `BEGIN` 贡献。
        ///
        /// 包体置位后深度回不到 0（初始化段与包共用 `END`），因此必须以独占一行的
        /// 批分隔符或输入结尾收尾。
        header_opens: bool,
    },
}

/// 一个方言的脚本边界规则。只表达判定，不负责遍历——
/// 遍历、括号深度、字节偏移换算由 [`split`] 统一处理。
pub(crate) trait ScriptSyntax: Sync {
    /// 语句头关键字（已转大写，`CREATE [OR REPLACE] [NON]EDITIONABLE` 前缀已剥离）
    /// 决定该语句的结束条件。
    fn statement_form(&self, _head: &[String]) -> StatementForm {
        StatementForm::Sql
    }

    /// 该词是否开启一层块。`prev`/`next` 是相邻的有意义词（已大写），
    /// 用于区分同名不同义：`GOTO end_loop` 的标签、`BEGIN TRAN` 的事务、`INTO case` 的标识符。
    ///
    /// `form` 是当前语句的结束条件：同一个 `BEGIN` 在不同语句里含义不同——
    /// 触发器体里的 `BEGIN` 是块，而 PostgreSQL/SQLite/MySQL 顶层单独的 `BEGIN`
    /// 是开启事务（没有配对的 `END`），把它当块会让后面所有语句被吞掉。
    fn opens_block(
        &self,
        _form: StatementForm,
        _word: &str,
        _prev: Option<&str>,
        _next: Option<&str>,
    ) -> bool {
        false
    }

    /// 该词是否关闭一层块。
    fn closes_block(&self, _word: &str) -> bool {
        false
    }

    /// 独占一行才生效的批分隔符（Oracle `/`、SQL Server `GO`）。
    fn batch_separator(&self) -> Option<&'static str> {
        None
    }
}

/// 没有专属规则的方言。唯一需要处理的是 `CREATE TRIGGER … BEGIN … END;`：
/// MySQL 与 SQLite 的触发器体是块，体内分号不能切。原先这条靠
/// `create_trigger_statement_needs_more` 的字符串启发式，现在收进同一套判定。
///
/// 只对 `CREATE TRIGGER` 生效：PostgreSQL 的触发器是 `EXECUTE FUNCTION`、达梦同 Oracle，
/// 都不会命中这个语句头；即便命中，也只影响那一条语句的切分，不波及其它 SQL。
struct DefaultSyntax;

impl ScriptSyntax for DefaultSyntax {
    fn statement_form(&self, head: &[String]) -> StatementForm {
        let words: Vec<&str> = head.iter().map(String::as_str).collect();
        let rest = match words.split_first() {
            Some((&"CREATE", rest)) => rest,
            _ => &words[..],
        };
        match rest {
            ["TRIGGER", ..] | ["TEMP", "TRIGGER", ..] | ["TEMPORARY", "TRIGGER", ..] => {
                StatementForm::Block {
                    header_opens: false,
                }
            }
            _ => StatementForm::Sql,
        }
    }

    fn opens_block(
        &self,
        form: StatementForm,
        word: &str,
        _prev: Option<&str>,
        _next: Option<&str>,
    ) -> bool {
        // 只有触发器体这种「语句头已判定为块」的语句里，`BEGIN` 才是块开启。
        // 顶层单独的 `BEGIN;` 是开启事务（PostgreSQL/SQLite/MySQL 都如此），
        // 它没有配对的 `END`，当成块会让后续语句全部被吞。
        form == (StatementForm::Block {
            header_opens: false,
        }) && word == "BEGIN"
    }

    fn closes_block(&self, word: &str) -> bool {
        word == "END"
    }
}

fn syntax_for(kind: DatabaseKind) -> &'static dyn ScriptSyntax {
    static DEFAULT_SYNTAX: DefaultSyntax = DefaultSyntax;
    match kind {
        // 达梦的 PL/SQL 与 Oracle 同源：匿名块、过程/函数/触发器体、`DECLARE`
        // 声明段、独占一行的 `/` 都需要同样的块边界，否则块体会被 `;` 切碎。
        DatabaseKind::Oracle | DatabaseKind::Dameng => &ORACLE_SYNTAX,
        DatabaseKind::SqlServer => &SQLSERVER_SYNTAX,
        _ => &DEFAULT_SYNTAX,
    }
}

fn dialect_for(kind: DatabaseKind) -> &'static dyn SqlParserDialect {
    static GENERIC: GenericDialect = GenericDialect {};
    static ORACLE: OracleDialect = OracleDialect {};
    static POSTGRES: PostgreSqlDialect = PostgreSqlDialect {};
    static MYSQL: MySqlDialect = MySqlDialect {};
    static SQLITE: SQLiteDialect = SQLiteDialect {};
    static MSSQL: MsSqlDialect = MsSqlDialect {};
    match kind {
        DatabaseKind::Oracle => &ORACLE,
        DatabaseKind::Postgres | DatabaseKind::Kingbase => &POSTGRES,
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => &MYSQL,
        DatabaseKind::Sqlite => &SQLITE,
        DatabaseKind::SqlServer => &MSSQL,
        // 达梦的分号、引号、反斜杠语义都按 SQL 标准，Generic 比 PostgreSQL 更贴：
        // 它既不认 `#` 行注释，也不把反斜杠当转义。
        DatabaseKind::Dameng | DatabaseKind::MongoDb | DatabaseKind::Redis => &GENERIC,
    }
}

/// 按方言切分脚本。分词失败返回 `None`，调用方据此回退到历史扫描器。
pub(crate) fn split(sql: &str, kind: DatabaseKind) -> Option<Vec<String>> {
    statement_ranges(sql, kind).map(|ranges| {
        ranges
            .into_iter()
            .map(|range| sql[range].trim().to_string())
            .collect()
    })
}

/// 按方言切分脚本，返回每条语句的字节区间（已去掉首尾空白）。
///
/// 编辑器侧需要区间而不是整串（code lens、折叠、执行状态装饰都按偏移工作），
/// 与 [`split`] 共用同一套遍历，避免两处判定漂移。
///
/// 切片**不含**分隔符：普通 SQL 去掉 `;`，Oracle `/` 与 SQL Server `GO` 整行去掉；
/// 但块语句（`BEGIN … END;`）保留结尾分号——少了它就是语法错误。
pub fn statement_ranges(sql: &str, kind: DatabaseKind) -> Option<Vec<std::ops::Range<usize>>> {
    let tokens = Tokenizer::new(dialect_for(kind), sql)
        .tokenize_with_location()
        .ok()?;
    Some(split_tokens(sql, syntax_for(kind), &tokens))
}

fn split_tokens(
    sql: &str,
    syntax: &dyn ScriptSyntax,
    tokens: &[TokenWithSpan],
) -> Vec<std::ops::Range<usize>> {
    let lines = line_starts(sql);
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut state = StatementState::default();

    for (index, token) in tokens.iter().enumerate() {
        let token_start = byte_offset(sql, &lines, token.span.start);
        let token_end = byte_offset(sql, &lines, token.span.end);
        let prev = neighbour_word(tokens, index, false);
        let next = neighbour_word(tokens, index, true);

        // SQL Server 的 `GO` 是普通词，Oracle 的 `/` 被词法器归成除号，两者都要在这里拦。
        let separator_candidate = match &token.token {
            Token::Word(word) => Some(word.value.as_str()),
            Token::Div => Some("/"),
            _ => None,
        };
        if let Some(candidate) = separator_candidate
            && is_batch_separator(syntax, sql, token_start, token_end, candidate)
        {
            // 批分隔符优先于一切：包体的初始化段可能让深度收不回 0，
            // 独占一行的 `/`（或 `GO`）是作者给出的明确收尾信号。
            push_statement(sql, start, token_start, &mut statements);
            // 分隔符独占一行：切到该行行尾，换行本身不属于下一条语句。
            let (_, line_end) = line_bounds(sql, token_start);
            start = sql[line_end..]
                .strip_prefix('\n')
                .map_or(line_end, |_| line_end + 1);
            state = StatementState::default();
            continue;
        }

        state.begin_if_needed(syntax, tokens, index);

        match &token.token {
            Token::Whitespace(_) => {}
            Token::LParen => state.parens += 1,
            Token::RParen => state.parens = state.parens.saturating_sub(1),
            Token::Word(word) => {
                state.visit_word(syntax, &word.value, prev.as_deref(), next.as_deref())
            }
            Token::SemiColon => {
                state.visit_separator(sql, token_start, 1, &mut statements, &mut start)
            }
            // 全角分号不是标准 SQL，但中文输入法下常见；词法器把它当未知字符原样返回。
            Token::Char('；') => state.visit_separator(
                sql,
                token_start,
                '；'.len_utf8(),
                &mut statements,
                &mut start,
            ),
            _ => {}
        }
    }
    push_statement(sql, start, sql.len(), &mut statements);
    statements
}

/// 一条语句在遍历过程中的状态。
#[derive(Default)]
struct StatementState {
    form: StatementForm,
    started: bool,
    /// 已开启、尚未关闭的块。记录的不是计数而是**开启词**：
    /// `END IF` 的 `IF` 要靠它判断「这是结束符尾巴」而不是新块。
    openers: Vec<String>,
    /// 刚被 `END` 关掉的那一层的开启词。
    last_closed: Option<String>,
    /// 块是否至少开启过一次。`DECLARE` 的声明段、过程 `IS/AS` 之后的声明段都在块开启前，
    /// 这些 `;` 不能切。
    block_seen: bool,
    parens: usize,
}

impl StatementState {
    /// 语句的第一个非空白 token 到达时确定结束条件。
    fn begin_if_needed(
        &mut self,
        syntax: &dyn ScriptSyntax,
        tokens: &[TokenWithSpan],
        index: usize,
    ) {
        if self.started || matches!(tokens[index].token, Token::Whitespace(_)) {
            return;
        }
        self.started = true;
        self.form = syntax.statement_form(&statement_head(tokens, index));
        if self.form == (StatementForm::Block { header_opens: true }) {
            // 语句头占一层：包/类型体本身。深度因此回不到 0，收尾依赖批分隔符或结尾。
            self.openers.push(String::new());
            self.block_seen = true;
        }
    }

    fn visit_word(
        &mut self,
        syntax: &dyn ScriptSyntax,
        word: &str,
        prev: Option<&str>,
        next: Option<&str>,
    ) {
        let upper = word.to_ascii_uppercase();
        // `END IF` / `END LOOP` / `END CASE` 的尾巴：所属块已在 `END` 处关闭，这里不能再开一层。
        // 只认与「刚关闭的开启词」同名的尾巴，这样 `... CASE ... END` 之后真正的 `LOOP`
        // 不会被误吞。
        if self.last_closed.as_deref() == Some(upper.as_str()) {
            self.last_closed = None;
            return;
        }
        self.last_closed = None;

        if !self.started {
            return;
        }
        if syntax.closes_block(&upper) {
            if let Some(opener) = self.openers.pop() {
                self.block_seen = true;
                self.last_closed = Some(opener);
            }
            return;
        }
        if syntax.opens_block(self.form, &upper, prev, next) {
            self.openers.push(upper);
            self.block_seen = true;
        }
    }

    fn visit_separator(
        &mut self,
        sql: &str,
        at: usize,
        len: usize,
        statements: &mut Vec<std::ops::Range<usize>>,
        start: &mut usize,
    ) {
        // 块语句在块真正开启前不分句（DECLARE 声明段、过程 IS/AS 之后的声明段）；
        // 开启之后，深度未归零说明还在块内。
        if self.form != StatementForm::Sql && (!self.block_seen || !self.openers.is_empty()) {
            return;
        }
        if !self.openers.is_empty() {
            return;
        }
        // 普通 SQL 的分号只是分隔符，丢掉不影响语义（沿用历史行为）；
        // 块语句结尾的 `;` 是 PL/SQL / T-SQL 语法的一部分，`BEGIN … END` 少了它就是语法错误。
        let end = match self.form {
            StatementForm::Sql => at,
            StatementForm::Block { .. } => at + len,
        };
        push_statement(sql, *start, end, statements);
        *start = at + len;
        // 下一条语句重新判定结束条件，否则 `SELECT ...; BEGIN ...; END;` 会把第二段
        // 沿用第一条的 Sql 规则，块内的 `;` 被当成分句。
        *self = Self::default();
    }
}

/// 取语句开头的关键字，最多 [`HEAD_WORDS`] 个。`CREATE [OR REPLACE] [NON]EDITIONABLE`
/// 这类前缀由各方言的 `statement_form` 自己跳过，本函数只负责原样给出词序列。
fn statement_head(tokens: &[TokenWithSpan], from: usize) -> Vec<String> {
    tokens[from..]
        .iter()
        .take_while(|token| !matches!(token.token, Token::SemiColon))
        .filter_map(|token| match &token.token {
            Token::Word(word) => Some(word.value.to_ascii_uppercase()),
            _ => None,
        })
        .take(HEAD_WORDS)
        .collect()
}

const HEAD_WORDS: usize = 8;

/// 相邻的有意义词。注释与空白不算；标点返回空串占位，让调用方能把
/// 「前面是标点」和「前面没有东西」区分开。
fn neighbour_word(tokens: &[TokenWithSpan], index: usize, forward: bool) -> Option<String> {
    let nearby = |token: &TokenWithSpan| match &token.token {
        Token::Whitespace(_) => None,
        Token::Word(word) => Some(word.value.to_ascii_uppercase()),
        _ => Some(String::new()),
    };
    if forward {
        tokens[index + 1..].iter().find_map(nearby)
    } else {
        tokens[..index].iter().rev().find_map(nearby)
    }
}

fn is_batch_separator(
    syntax: &dyn ScriptSyntax,
    sql: &str,
    start: usize,
    end: usize,
    word: &str,
) -> bool {
    let Some(separator) = syntax.batch_separator() else {
        return false;
    };
    if !word.eq_ignore_ascii_case(separator) {
        return false;
    }
    let (line_start, line_end) = line_bounds(sql, start);
    end <= line_end
        && sql[line_start..line_end]
            .trim()
            .eq_ignore_ascii_case(separator)
}

fn line_starts(sql: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (index, byte) in sql.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn line_bounds(sql: &str, byte: usize) -> (usize, usize) {
    let start = sql[..byte].rfind('\n').map_or(0, |index| index + 1);
    let end = sql[byte..]
        .find('\n')
        .map_or(sql.len(), |index| byte + index);
    (start, end)
}

/// 词法器给的是「第几行第几列」，列按字符计；切片要的是字节偏移。
fn byte_offset(sql: &str, lines: &[usize], location: Location) -> usize {
    let line = (location.line as usize).saturating_sub(1);
    let Some(&line_start) = lines.get(line) else {
        return sql.len();
    };
    let line_end = lines.get(line + 1).copied().unwrap_or(sql.len());
    let column = (location.column as usize).saturating_sub(1);
    sql[line_start..line_end]
        .char_indices()
        .nth(column)
        .map_or(line_end, |(offset, _)| line_start + offset)
}

fn push_statement(
    sql: &str,
    start: usize,
    end: usize,
    statements: &mut Vec<std::ops::Range<usize>>,
) {
    let end = end.min(sql.len());
    if start >= end {
        return;
    }
    // 只裁首尾空白，保留区间表达能力（编辑器按偏移取文本 / 画装饰）。
    let text = &sql[start..end];
    let trimmed_start = text.len() - text.trim_start().len();
    let trimmed_end = text.trim_end().len();
    if trimmed_start >= trimmed_end {
        return;
    }
    statements.push(start + trimmed_start..start + trimmed_end);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::DatabaseKind;

    fn split_of(sql: &str, kind: DatabaseKind) -> Vec<String> {
        split(sql, kind).expect("词法器不应失败")
    }

    fn oracle(sql: &str) -> Vec<String> {
        split_of(sql, DatabaseKind::Oracle)
    }

    /// 本问题的原始症状：`q'[…]'` 里的 `;` 被当成分句，PL/SQL 块的 `;` 也被切走。
    #[test]
    fn oracle_alternative_quoting_keeps_semicolons() {
        assert_eq!(
            oracle("select q'[it's a;test]' from dual; select 1 from dual"),
            vec!["select q'[it's a;test]' from dual", "select 1 from dual"]
        );
        assert_eq!(
            oracle("select q'{a;b}', q'<c;d>', q'!e;f!', nq'[g;h]' from dual; select 1 from dual"),
            vec![
                "select q'{a;b}', q'<c;d>', q'!e;f!', nq'[g;h]' from dual",
                "select 1 from dual"
            ]
        );
    }

    #[test]
    fn oracle_anonymous_block_is_one_statement() {
        assert_eq!(oracle("BEGIN NULL; END;"), vec!["BEGIN NULL; END;"]);
        assert_eq!(
            oracle("BEGIN NULL; END; select 1 from dual"),
            vec!["BEGIN NULL; END;", "select 1 from dual"]
        );
    }

    /// 达梦的 PL/SQL 同源：匿名块、声明段、嵌套控制流都不能按 `;` 切碎。
    ///
    /// 这条曾经会失败——达梦落在 `DefaultSyntax`，只认 `CREATE TRIGGER`，
    /// `BEGIN NULL; END;` 被切成两条，编辑器里按 Run 会把块拆开执行。
    #[test]
    fn dameng_blocks_follow_oracle_boundaries() {
        for sql in [
            "BEGIN NULL; END;",
            "DECLARE\n  x NUMBER;\nBEGIN\n  x := 1;\nEND;",
            "BEGIN IF 1 = 1 THEN NULL; END IF; END;",
            "BEGIN FOR i IN 1..3 LOOP NULL; END LOOP; END;",
        ] {
            assert_eq!(
                split_of(sql, DatabaseKind::Dameng),
                vec![sql.to_string()],
                "{sql}"
            );
        }
        assert_eq!(
            split_of("BEGIN NULL; END; select 1;", DatabaseKind::Dameng),
            vec!["BEGIN NULL; END;", "select 1"]
        );
    }

    #[test]
    fn oracle_declare_section_is_not_split() {
        assert_eq!(
            oracle("DECLARE\n  x NUMBER;\n  y VARCHAR2(10);\nBEGIN\n  x := 1;\nEND;"),
            vec!["DECLARE\n  x NUMBER;\n  y VARCHAR2(10);\nBEGIN\n  x := 1;\nEND;"]
        );
    }

    #[test]
    fn oracle_nested_control_flow_pairs() {
        for sql in [
            "BEGIN IF 1 = 1 THEN NULL; END IF; END;",
            "BEGIN FOR i IN 1..3 LOOP NULL; END LOOP; END;",
            "BEGIN CASE WHEN 1 = 1 THEN NULL; END CASE; END;",
            "BEGIN WHILE 1 = 1 LOOP IF 1 = 1 THEN NULL; END IF; END LOOP; END;",
        ] {
            assert_eq!(oracle(sql), vec![sql.to_string()], "{sql}");
        }
    }

    /// `SELECT CASE … END` 的 `END` 不属于任何开启词，不能就此把语句切断或错配。
    #[test]
    fn oracle_case_expression_inside_block() {
        let sql = "BEGIN\n  SELECT CASE WHEN 1 = 1 THEN 'a' END INTO v FROM dual;\nEND;";
        assert_eq!(oracle(sql), vec![sql.to_string()]);
    }

    /// `GOTO end_loop` 的标签、`INTO case` 的标识符都不能开块。
    #[test]
    fn oracle_labels_and_identifiers_do_not_open_blocks() {
        let sql = "BEGIN GOTO end_loop; <<end_loop>> SELECT 1 INTO case FROM t; END;";
        assert_eq!(oracle(sql), vec![sql.to_string()]);
    }

    #[test]
    fn oracle_procedure_declaration_suffix_is_not_a_boundary() {
        let sql =
            "CREATE OR REPLACE FUNCTION f RETURN NUMBER DETERMINISTIC IS\nBEGIN\n  RETURN 1;\nEND;";
        assert_eq!(oracle(sql), vec![sql.to_string()]);
    }

    /// 包体按 SQL*Plus 口径，靠独占一行的 `/` 收尾。
    #[test]
    fn oracle_package_body_ends_at_slash_line() {
        let sql = "CREATE OR REPLACE PACKAGE BODY p AS\n  PROCEDURE a IS BEGIN NULL; END;\nBEGIN\n  NULL;\nEND;\n/\nselect 1 from dual";
        assert_eq!(
            oracle(sql),
            vec![
                "CREATE OR REPLACE PACKAGE BODY p AS\n  PROCEDURE a IS BEGIN NULL; END;\nBEGIN\n  NULL;\nEND;",
                "select 1 from dual"
            ]
        );
    }

    /// 反斜杠在 Oracle 里不是转义符，`'C:\'` 就是完整字符串。
    #[test]
    fn oracle_backslash_is_not_an_escape() {
        assert_eq!(
            oracle(r"select 'C:\' as v from dual; select 1 from dual"),
            vec![r"select 'C:\' as v from dual", "select 1 from dual"]
        );
    }

    /// `#` 在 Oracle 里是合法标识符字符，不是行注释。
    #[test]
    fn oracle_sharp_is_part_of_identifier() {
        assert_eq!(
            oracle("select emp# from t; select 1 from dual"),
            vec!["select emp# from t", "select 1 from dual"]
        );
    }

    /// 词法器失败时返回 None，调用方回退历史扫描器，不会更差。
    #[test]
    fn untokenizable_input_reports_fallback() {
        let sql = "create or replace package body p as begin $IF $$x $THEN null; $END end;";
        assert!(split(sql, DatabaseKind::Oracle).is_none());
    }

    #[test]
    fn plain_dialects_keep_current_behaviour() {
        assert_eq!(
            split_of("select 1; select 2", DatabaseKind::MySql),
            vec!["select 1", "select 2"]
        );
        // MySQL 的 `#` 仍是行注释，`;` 在注释里不切。
        assert_eq!(
            split_of("select 1 # c ;\n; select 2", DatabaseKind::MySql),
            vec!["select 1 # c ;", "select 2"]
        );
        // MySQL 的 `\'` 是转义，字符串不提前收尾。
        assert_eq!(
            split_of(r"select 'a\'b'; select 1", DatabaseKind::MySql),
            vec![r"select 'a\'b'", "select 1"]
        );
    }

    /// 顶层单独的 `BEGIN;` 是开启事务，不是块起点——把它当块会让后面所有语句一起被吞。
    /// 只有语句头已判定为块（触发器体等）时，`BEGIN` 才开启深度。
    ///
    /// 达梦不在这个集合里：它同 Oracle，`BEGIN` 只能是匿名块的起点、必须配 `END;`，
    /// 没有「`BEGIN;` 开启事务」这种写法（事务由首个 DML 隐式开启，`COMMIT` 收尾）。
    #[test]
    fn bare_begin_opens_a_transaction_not_a_block() {
        for kind in [
            DatabaseKind::Postgres,
            DatabaseKind::Sqlite,
            DatabaseKind::MySql,
        ] {
            assert_eq!(
                split_of("BEGIN;\nINSERT INTO t VALUES (1);\nCOMMIT;", kind),
                vec!["BEGIN", "INSERT INTO t VALUES (1)", "COMMIT"],
                "{kind:?}"
            );
        }
        assert_eq!(
            split_of("BEGIN TRAN;\nselect 1;\nCOMMIT;", DatabaseKind::SqlServer),
            vec!["BEGIN TRAN", "select 1", "COMMIT"]
        );
    }

    #[test]
    fn postgres_dollar_quoting_and_create_trigger() {
        assert_eq!(
            split_of(
                "do $$ begin perform 1; end $$; select 1",
                DatabaseKind::Postgres
            ),
            vec!["do $$ begin perform 1; end $$", "select 1"]
        );
        let trigger =
            "CREATE TRIGGER t AFTER INSERT ON a FOR EACH ROW BEGIN UPDATE b SET c = 1; END;";
        assert_eq!(
            split_of(&format!("{trigger} select 1"), DatabaseKind::Postgres),
            vec![trigger, "select 1"]
        );
    }

    #[test]
    fn sqlserver_go_separates_batches() {
        assert_eq!(
            split_of("select 1\ngo\nselect 2", DatabaseKind::SqlServer),
            vec!["select 1", "select 2"]
        );
        // `GO` 必须独占一行，行内出现的仍按标识符处理。
        assert_eq!(
            split_of("select go from t; select 1", DatabaseKind::SqlServer),
            vec!["select go from t", "select 1"]
        );
    }

    #[test]
    fn sqlserver_begin_transaction_does_not_open_a_block() {
        assert_eq!(
            split_of(
                "BEGIN TRANSACTION; insert into t values (1); COMMIT;",
                DatabaseKind::SqlServer
            ),
            vec!["BEGIN TRANSACTION", "insert into t values (1)", "COMMIT"]
        );
    }

    #[test]
    fn sqlserver_begin_end_block_stays_whole() {
        let sql = "BEGIN\n  insert into t values (1);\n  insert into t values (2);\nEND";
        assert_eq!(
            split_of(sql, DatabaseKind::SqlServer),
            vec![sql.to_string()]
        );
    }

    #[test]
    fn statements_report_their_own_form() {
        // 第一条是普通 SQL，第二条是块：状态必须逐条重置，否则块内的 `;` 会被切走。
        assert_eq!(
            oracle("select 1 from dual; BEGIN NULL; END; select 2 from dual"),
            vec![
                "select 1 from dual",
                "BEGIN NULL; END;",
                "select 2 from dual"
            ]
        );
    }

    #[test]
    fn fullwidth_semicolon_still_splits() {
        assert_eq!(
            split_of(
                "select 1 from dual；select 2 from dual",
                DatabaseKind::Oracle
            ),
            vec!["select 1 from dual", "select 2 from dual"]
        );
    }
}
