pub fn format_sql_text_for_dialect(sql: &str, dialect: DatabaseKind) -> String {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return sql.to_string();
    }

    let protected = protect_digit_prefixed_identifiers(trimmed, dialect);
    let formatted = restore_protected_identifiers(
        &format_sql_with_options(&protected.sql, dialect),
        &protected.identifiers,
    );
    format_sql_statements(
        &formatted,
        matches!(dialect, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb),
    )
}

fn format_sql_statements(sql: &str, align_mysql_columns: bool) -> String {
    let statements = sql_text_statement_ranges(sql)
        .into_iter()
        .map(|range| format_sql_statement(sql[range].trim(), align_mysql_columns))
        .filter(|statement| !statement.is_empty())
        .collect::<Vec<_>>();

    if statements.is_empty() {
        sql.to_string()
    } else {
        statements.join("\n")
    }
}

fn format_sql_statement(sql: &str, align_mysql_columns: bool) -> String {
    if starts_with_create_table(sql) {
        format_create_table_sql(sql, align_mysql_columns)
    } else {
        sql.to_string()
    }
}

fn starts_with_create_table(sql: &str) -> bool {
    let sql = sql.trim_start();
    starts_with_sql_keyword(sql, "create")
        && starts_with_sql_keyword(sql["create".len()..].trim_start(), "table")
}

struct ProtectedSqlIdentifiers {
    sql: String,
    identifiers: Vec<String>,
}

fn protect_digit_prefixed_identifiers(sql: &str, dialect: DatabaseKind) -> ProtectedSqlIdentifiers {
    // SQL Server 的方括号标识符内部允许空格；`sqlformat` 会把它改写成 `[ ID ]`，
    // 使 `[ID]` 与列名 `ID` 不再相等。先整体占位，格式化后再原样还原。
    let protect_brackets = dialect == DatabaseKind::SqlServer;
    if !matches!(dialect, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb) && !protect_brackets {
        return ProtectedSqlIdentifiers {
            sql: sql.to_string(),
            identifiers: Vec::new(),
        };
    }

    let mut protected = String::with_capacity(sql.len());
    let mut identifiers = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\'' | '"' | '`' => copy_quoted_sql_from_indices(&mut protected, &mut chars, ch),
            '[' if protect_brackets => {
                let start = index;
                let mut end = index + 1;
                let mut closed = false;
                while let Some((next_index, next_ch)) = chars.next() {
                    end = next_index + next_ch.len_utf8();
                    if next_ch == ']' {
                        if matches!(chars.peek(), Some((_, ']'))) {
                            chars.next();
                            end += 1;
                        } else {
                            closed = true;
                            break;
                        }
                    }
                }
                if !closed {
                    // 未闭合的方括号交给 formatter 原样处理，避免吞掉后续 SQL。
                    protected.push_str(&sql[start..end]);
                    continue;
                }
                let identifier = &sql[start..end];
                let placeholder = format!("__GDBSQLFMTIDENT{}__", identifiers.len());
                identifiers.push(identifier.to_string());
                protected.push_str(&placeholder);
            }
            '-' if matches!(chars.peek(), Some((_, '-'))) => {
                protected.push(ch);
                protected.push(chars.next().unwrap().1);
                copy_until_newline_from_indices(&mut protected, &mut chars);
            }
            '#' => {
                protected.push(ch);
                copy_until_newline_from_indices(&mut protected, &mut chars);
            }
            '/' if matches!(chars.peek(), Some((_, '*'))) => {
                protected.push(ch);
                protected.push(chars.next().unwrap().1);
                copy_until_block_comment_end_from_indices(&mut protected, &mut chars);
            }
            _ if ch.is_ascii_digit() => {
                let mut end = index + ch.len_utf8();
                while let Some((next_index, next_ch)) = chars.peek().copied() {
                    if !is_sql_word_char(next_ch) {
                        break;
                    }
                    chars.next();
                    end = next_index + next_ch.len_utf8();
                }
                let token = &sql[index..end];
                if is_digit_prefixed_identifier(token) {
                    let placeholder = format!("__GDBSQLFMTIDENT{}__", identifiers.len());
                    identifiers.push(token.to_string());
                    protected.push_str(&placeholder);
                } else {
                    protected.push_str(token);
                }
            }
            _ => protected.push(ch),
        }
    }

    ProtectedSqlIdentifiers {
        sql: protected,
        identifiers,
    }
}

fn restore_protected_identifiers(sql: &str, identifiers: &[String]) -> String {
    let mut restored = sql.to_string();
    for (index, identifier) in identifiers.iter().enumerate() {
        restored = restored.replace(&format!("__GDBSQLFMTIDENT{index}__"), identifier);
    }
    restored
}

fn is_digit_prefixed_identifier(token: &str) -> bool {
    token.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && token
            .chars()
            .any(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && token.parse::<f64>().is_err()
}

pub fn compress_sql_text(sql: &str) -> String {
    let mut compressed = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut pending_space = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' | '`' => {
                push_pending_sql_space(&mut compressed, &mut pending_space);
                copy_quoted_sql(&mut compressed, &mut chars, ch);
            }
            '-' if matches!(chars.peek(), Some('-')) => {
                chars.next();
                push_pending_sql_space(&mut compressed, &mut pending_space);
                compressed.push_str("/*");
                copy_line_comment_as_block(&mut compressed, &mut chars);
                compressed.push_str("*/");
                pending_space = true;
            }
            '#' => {
                push_pending_sql_space(&mut compressed, &mut pending_space);
                compressed.push_str("/*");
                copy_line_comment_as_block(&mut compressed, &mut chars);
                compressed.push_str("*/");
                pending_space = true;
            }
            '/' if matches!(chars.peek(), Some('*')) => {
                chars.next();
                push_pending_sql_space(&mut compressed, &mut pending_space);
                compressed.push_str("/*");
                copy_until_block_comment_end(&mut compressed, &mut chars);
                pending_space = true;
            }
            _ if ch.is_whitespace() => pending_space = !compressed.is_empty(),
            _ => {
                push_pending_sql_space(&mut compressed, &mut pending_space);
                compressed.push(ch);
            }
        }
    }

    compressed.trim().to_string()
}

/// 组装实际下发执行的 SQL：去掉注释、按分句追加默认行数上限。
///
/// `db_kind` 决定双引号语义：MySQL/TiDB（以及历史行为一致的 SQLite）默认把 `"` 当字符串
/// 定界符，用户用 `"..."` 写字符串时归一化为 `'...'`；**PostgreSQL 的 `"` 是标识符引用**
/// （`"public"."orders"`、`"amount"`），绝不能改写，否则合法语句被换成单引号字符串导致
/// 语法错误（connection 里输入双引号、实际下发变单引号即此 bug）。Oracle 的 `"` 同样是
/// 标识符引用，且保留大小写，更不能归一。
fn sql_text_for_execution(sql: &str, default_limit: u64, db_kind: DatabaseKind) -> String {
    let pipeline = if matches!(
        db_kind,
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb | DatabaseKind::Sqlite
    ) {
        normalize_double_quoted_sql_strings(sql)
    } else {
        // PG/Oracle/Redis/Mongo：如实保留 `"`，不做字符串归一。
        sql.to_string()
    };
    apply_default_select_limit(
        &strip_sql_comments(&pipeline, db_kind),
        default_limit,
        db_kind,
    )
}

/// 去掉注释。
///
/// Oracle 有两处特例，按其它方言处理会静默改语义：
/// - `/*+ … */` 是优化器 hint，去掉后语句仍合法、执行计划却变了，必须原样保留；
/// - `#` 不是注释起始符，按 MySQL 规则剥到行尾会把合法 SQL 吃掉。
fn strip_sql_comments(sql: &str, db_kind: DatabaseKind) -> String {
    let is_oracle = db_kind == DatabaseKind::Oracle;
    let mut stripped = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' | '`' => copy_quoted_sql(&mut stripped, &mut chars, ch),
            '[' => copy_bracket_identifier(&mut stripped, &mut chars),
            '-' if matches!(chars.peek(), Some('-')) => {
                chars.next();
                stripped.push(' ');
                strip_until_newline(&mut stripped, &mut chars);
            }
            '#' if !is_oracle => {
                stripped.push(' ');
                strip_until_newline(&mut stripped, &mut chars);
            }
            '/' if matches!(chars.peek(), Some('*')) => {
                chars.next();
                if is_oracle && matches!(chars.peek(), Some('+')) {
                    stripped.push_str("/*");
                    copy_until_block_comment_end(&mut stripped, &mut chars);
                } else {
                    stripped.push(' ');
                    strip_block_comment(&mut stripped, &mut chars);
                }
            }
            _ => stripped.push(ch),
        }
    }

    stripped
}

fn apply_default_select_limit(sql: &str, default_limit: u64, db_kind: DatabaseKind) -> String {
    let dialect = fluxdb_core::sql_dialect(db_kind);
    let mut output = String::with_capacity(sql.len() + 16);
    let mut start = 0;
    for (end, separator_len) in sql_statement_ranges(sql) {
        let statement = sql[start..end].trim();
        let statement = match dialect.render_row_limit_suffix(default_limit) {
            // SQL Server：`TOP` 在列位置，只能单独插入，且不能与 OFFSET/FETCH 混用。
            None => apply_sqlserver_top_limit(statement, default_limit),
            Some(suffix) => limit_select_statement(
                &normalize_select_limit_before_order(statement),
                default_limit,
                &suffix,
                dialect.row_limit_allows_locking_clause(),
            ),
        };
        if !statement.is_empty() {
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(&statement);
            if separator_len > 0 {
                output.push(';');
            }
        }
        start = end + separator_len;
    }
    output
}

/// 给 T-SQL 的 SELECT 加 `TOP (n)` 上限。
///
/// 与 `LIMIT` 不同，TOP 必须紧跟在 `SELECT [ALL|DISTINCT]` 之后；已经有
/// `OFFSET/FETCH` 的语句不能再加 TOP（T-SQL 禁止同用），保持原样交给服务端。
/// 已有 TOP 且大于上限时改写为上限，避免模型写 `TOP 100000` 绕过结果有界。
fn apply_sqlserver_top_limit(statement: &str, page_size: u64) -> String {
    if page_size == 0 || statement.is_empty() {
        return statement.to_string();
    }
    let Some(select_start) = find_top_level_sql_keyword(statement, "select") else {
        return statement.to_string();
    };
    // 已有 OFFSET/FETCH 分页时不再叠加 TOP。
    if has_top_level_sql_keyword(statement, "offset") || has_top_level_sql_keyword(statement, "fetch")
    {
        return statement.to_string();
    }
    let header_end = find_top_level_sql_keyword(statement, "from")
        .or_else(|| find_top_level_sql_keyword(statement, "where"))
        .unwrap_or(statement.len());
    let header = &statement[select_start + "select".len()..header_end];

    let mut cursor = select_start + "select".len();
    let mut rest = &statement[cursor..];
    for keyword in ["all", "distinct"] {
        let trimmed = rest.trim_start();
        if ascii_keyword_at(trimmed, 0, keyword) {
            cursor = statement.len() - trimmed.len() + keyword.len();
            rest = &statement[cursor..];
            break;
        }
    }
    let trimmed = rest.trim_start();
    if ascii_keyword_at(trimmed, 0, "top") {
        // 解析 `TOP n` / `TOP (n)`；表达式形式（变量/子查询）不猜测，原样保留。
        let top_start = statement.len() - trimmed.len();
        let after_top = &trimmed["top".len()..];
        let value_start = after_top
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(index, _)| index);
        let Some(value_start) = value_start else {
            return statement.to_string();
        };
        let value_text = &after_top[value_start..];
        let (count, count_end) = if let Some(inner) = value_text.strip_prefix('(') {
            let Some(close) = inner.find(')') else {
                return statement.to_string();
            };
            // +2：跳过开括号与闭括号本身。
            (inner[..close].trim(), value_start + close + 2)
        } else {
            let end = value_text
                .find(|ch: char| !ch.is_ascii_digit())
                .unwrap_or(value_text.len());
            (&value_text[..end], value_start + end)
        };
        let Ok(count) = count.parse::<u64>() else {
            return statement.to_string();
        };
        if count <= page_size {
            return statement.to_string();
        }
        let value_abs_start = top_start + "top".len() + value_start;
        let value_abs_end = top_start + "top".len() + count_end;
        let mut capped = String::with_capacity(statement.len() + 4);
        capped.push_str(&statement[..value_abs_start]);
        capped.push_str(&format!("({page_size})"));
        capped.push_str(&statement[value_abs_end..]);
        return capped;
    }

    // 没有 TOP：插到 SELECT [ALL|DISTINCT] 之后。
    let _ = header;
    let mut limited = String::with_capacity(statement.len() + 12);
    limited.push_str(&statement[..cursor]);
    limited.push_str(" TOP (");
    limited.push_str(&page_size.to_string());
    limited.push(')');
    limited.push_str(&statement[cursor..]);
    limited
}

fn normalize_select_limit_before_order(statement: &str) -> String {
    if !sql_selects_rows(statement) {
        return statement.to_string();
    }

    let Some(limit_start) = find_top_level_sql_keyword(statement, "limit") else {
        return statement.to_string();
    };
    let Some(order_start) = find_top_level_order_by(statement) else {
        return statement.to_string();
    };
    if limit_start > order_start {
        return statement.to_string();
    }

    let before_limit = statement[..limit_start].trim_end();
    let limit_clause = statement[limit_start..order_start].trim();
    let order_clause = statement[order_start..].trim();
    format!("{before_limit} {order_clause} {limit_clause}")
}

fn sql_statement_ranges(sql: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\'' | '"' | '`' => skip_quoted_sql(&mut chars, ch),
            // SQL Server 方括号标识符里的 `from`/`where` 不是关键字。
            '[' => skip_bracket_identifier(&mut chars),
            '-' if matches!(chars.peek(), Some((_, '-'))) => skip_line_comment(&mut chars),
            '#' => skip_line_comment(&mut chars),
            '/' if matches!(chars.peek(), Some((_, '*'))) => skip_block_comment(&mut chars),
            // PG dollar-quote `$tag$...$tag$` / `$$...$$`：体内分号/引号属函数体，不得切分。
            '$' => {
                if let Some(tag) = dollar_quote_opener_tag(&mut chars) {
                    skip_dollar_quote(&mut chars, &tag);
                }
            }
            ';' | '；' => ranges.push((index, ch.len_utf8())),
            _ => {}
        }
    }
    ranges.push((sql.len(), 0));
    ranges
}

fn skip_bracket_identifier(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    while let Some((_, ch)) = chars.next() {
        if ch == ']' {
            if matches!(chars.peek(), Some((_, ']'))) {
                chars.next();
            } else {
                break;
            }
        }
    }
}

fn copy_bracket_identifier(
    output: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    output.push('[');
    while let Some(ch) = chars.next() {
        output.push(ch);
        if ch == ']' {
            if matches!(chars.peek(), Some(']')) {
                output.push(chars.next().unwrap());
            } else {
                break;
            }
        }
    }
}

pub fn sql_text_statement_ranges(sql: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (end, separator_len) in sql_statement_ranges(sql) {
        let raw_end = end + separator_len;
        let mut range = start..raw_end;
        trim_sql_statement_range(sql, &mut range);
        if range.start < range.end {
            ranges.push(range);
        }
        start = raw_end;
    }
    ranges
}

fn trim_sql_statement_range(sql: &str, range: &mut std::ops::Range<usize>) {
    while range.start < range.end
        && sql[range.start..range.end]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    {
        range.start += sql[range.start..range.end]
            .chars()
            .next()
            .unwrap()
            .len_utf8();
    }
    while range.end > range.start
        && sql[range.start..range.end]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        range.end -= sql[range.start..range.end]
            .chars()
            .next_back()
            .unwrap()
            .len_utf8();
    }
}

/// 对单个 SELECT 应用默认上限（封顶值）。`limit_suffix` 由方言给出（` LIMIT n` /
/// ` FETCH FIRST n ROWS ONLY`）。
///
/// - `page_size == 0` 表示「不限制」：不追加，也不强制改写用户已写的上限子句。
/// - 无上限子句且 `page_size > 0`：追加 `limit_suffix`。
/// - 已有 `LIMIT` 且 count `> page_size`：强制改写为 `LIMIT {page_size}`（封顶）；
///   count `<= page_size` 保持不动。
/// - 已有 `FETCH`/`OFFSET`（Oracle/SQL Server 语法）：不做解析改写，原样放过——
///   猜错 count 的代价是改写用户语句，比不封顶更危险。
/// - 有锁定子句且方言不允许共存（Oracle）：原样放过，不能为了封顶让语句直接报错。
fn limit_select_statement(
    statement: &str,
    page_size: u64,
    limit_suffix: &str,
    allows_locking_clause: bool,
) -> String {
    if !sql_selects_rows(statement) || page_size == 0 {
        return statement.to_string();
    }
    if let Some(limit_start) = find_top_level_sql_keyword(statement, "limit") {
        // 已有 LIMIT：仅当 count 超过封顶值时改写，否则保持原样。
        return cap_existing_limit(statement, limit_start, page_size)
            .unwrap_or_else(|| statement.to_string());
    }
    if has_top_level_sql_keyword(statement, "fetch") || has_top_level_sql_keyword(statement, "offset")
    {
        return statement.to_string();
    }
    // 锁定子句（`FOR UPDATE` / `FOR SHARE`）在所有相关方言里都排在最后，
    // 上限子句必须插在它前面，否则是语法错误。
    let locking_start = locking_clause_start(statement);
    if locking_start.is_some() && !allows_locking_clause {
        return statement.to_string();
    }
    let insert_at = locking_start.unwrap_or(statement.len());
    let head = statement[..insert_at].trim_end();
    let tail = statement[insert_at..].trim();
    if tail.is_empty() {
        format!("{head}{limit_suffix}")
    } else {
        format!("{head}{limit_suffix} {tail}")
    }
}

/// 顶层行锁子句 `FOR UPDATE` / `FOR SHARE` / `FOR NO KEY UPDATE` / `FOR KEY SHARE` 的起点。
fn locking_clause_start(statement: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(offset) = find_top_level_sql_keyword(&statement[search_from..], "for") {
        let for_start = search_from + offset;
        let rest = statement[for_start + "for".len()..].trim_start();
        for keyword in ["update", "share", "no", "key"] {
            if ascii_keyword_at(rest, 0, keyword) {
                return Some(for_start);
            }
        }
        search_from = for_start + "for".len();
    }
    None
}

/// 仅当用户已写 LIMIT 的单行 count 超过 page_size 时，返回重写后的字符串
/// `... LIMIT {page_size}`；否则返回 `None`（保持原 statement）。
///
/// `limit_start` 是顶层 `limit` 关键字的字节起始位置（来自 `find_top_level_sql_keyword`）。
fn cap_existing_limit(statement: &str, limit_start: usize, page_size: u64) -> Option<String> {
    // ponytail: 只解析字面整数 count（`LIMIT n` / `LIMIT offset, n` / `LIMIT n OFFSET m`），
    // 不做表达式求值；SQL 表达式 limit 属罕见场景，忽略封顶。
    let rest = &statement[limit_start + "limit".len()..];
    // 状态机：digit_start = count 数字起点；逗号表示 `LIMIT offset, count`（count 重新起）。
    let mut digit_start: Option<usize> = None;
    let mut seen_digit = false;
    for (index, ch) in rest.char_indices() {
        match ch {
            _ if ch.is_ascii_digit() => {
                digit_start.get_or_insert(index);
                seen_digit = true;
            }
            ' ' | '\t' | '\r' | '\n' => {}
            ',' => {
                // `LIMIT offset, count`：逗号后是真正的 count
                digit_start = None;
                seen_digit = false;
            }
            _ => {
                // 非数字非空白（如 OFFSET / subquery / ;）→ count 结束
                break;
            }
        }
    }
    let Some(start) = digit_start.filter(|_| seen_digit) else {
        return None;
    };
    let count_text = rest[start..].trim();
    let count_text = count_text
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    let Ok(count) = count_text.parse::<u64>() else {
        return None;
    };
    if count <= page_size {
        return None;
    }
    // 重写 [limit_start .. limit_start+limit.len()+start+count_text.len()] 为新的 LIMIT 子句
    let end = limit_start + "limit".len() + start + count_text.len();
    Some(format!(
        "{} LIMIT {page_size}{}",
        statement[..limit_start].trim_end(),
        &statement[end..]
    ))
}

/// 该语句是否**以返回行集结束**，从而可以安全追加行数上限。
///
/// 只看 `SELECT` 开头会漏掉 `WITH ... SELECT`：CTE 写法在过去完全拿不到默认上限，
/// 一条 `WITH ... SELECT * FROM 大表` 会把整表拉进内存。
///
/// `WITH` 后面也可能是写入（`WITH ... DELETE/UPDATE/MERGE`）。给写入加 `LIMIT`
/// 会静默截断影响行数，所以只认「顶层第一个查询/写入关键字是 select」。
/// CTE 定义体在括号里（depth > 0），天然被 `find_top_level_sql_keyword` 跳过。
fn sql_selects_rows(statement: &str) -> bool {
    if starts_with_sql_keyword(statement, "select") {
        return true;
    }
    if !starts_with_sql_keyword(statement, "with") {
        return false;
    }
    ["select", "insert", "update", "delete", "merge"]
        .iter()
        .filter_map(|keyword| {
            find_top_level_sql_keyword(statement, keyword).map(|at| (at, *keyword))
        })
        // `FOR UPDATE` 之类排在 SELECT 之后，取最早出现者即主语句类型。
        .min_by_key(|(at, _)| *at)
        .is_some_and(|(_, keyword)| keyword == "select")
}

fn starts_with_sql_keyword(statement: &str, keyword: &str) -> bool {
    let statement = statement.trim_start();
    statement
        .get(..keyword.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(keyword))
        && statement
            .get(keyword.len()..)
            .and_then(|rest| rest.chars().next())
            .is_none_or(|ch| !is_sql_word_char(ch))
}

fn has_top_level_sql_keyword(statement: &str, keyword: &str) -> bool {
    find_top_level_sql_keyword(statement, keyword).is_some()
}

fn find_top_level_order_by(statement: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(order_start) = find_top_level_sql_keyword(&statement[search_from..], "order") {
        let order_start = search_from + order_start;
        let by_start = statement[order_start + "order".len()..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(index, _)| order_start + "order".len() + index);
        if by_start.is_some_and(|index| {
            statement
                .get(index..index + "by".len())
                .is_some_and(|value| value.eq_ignore_ascii_case("by"))
                && statement
                    .as_bytes()
                    .get(index + "by".len())
                    .is_none_or(|byte| !is_sql_word_byte(*byte))
        }) {
            return Some(order_start);
        }
        search_from = order_start + "order".len();
    }
    None
}

fn find_top_level_sql_keyword(statement: &str, keyword: &str) -> Option<usize> {
    let bytes = statement.as_bytes();
    let mut depth = 0usize;
    let mut chars = statement.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\'' | '"' | '`' => skip_quoted_sql(&mut chars, ch),
            '[' => skip_bracket_identifier(&mut chars),
            '-' if matches!(chars.peek(), Some((_, '-'))) => skip_line_comment(&mut chars),
            '#' => skip_line_comment(&mut chars),
            '/' if matches!(chars.peek(), Some((_, '*'))) => skip_block_comment(&mut chars),
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && ch.eq_ignore_ascii_case(&keyword.chars().next().unwrap()) => {
                let end = index + keyword.len();
                if statement
                    .get(index..end)
                    .is_some_and(|value| value.eq_ignore_ascii_case(keyword))
                    && (index == 0 || !is_sql_word_byte(bytes[index - 1]))
                    && bytes.get(end).is_none_or(|byte| !is_sql_word_byte(*byte))
                {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn skip_quoted_sql(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, quote: char) {
    while let Some((_, ch)) = chars.next() {
        if ch == quote {
            if matches!(chars.peek(), Some((_, next)) if *next == quote) {
                chars.next();
            } else {
                break;
            }
        }
    }
}

fn skip_line_comment(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    for (_, ch) in chars.by_ref() {
        if ch == '\n' {
            break;
        }
    }
}

fn skip_block_comment(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    chars.next();
    let mut previous = '\0';
    for (_, ch) in chars.by_ref() {
        if previous == '*' && ch == '/' {
            break;
        }
        previous = ch;
    }
}

/// 识别 PG dollar-quote 开启符 `$$` 或 `$tag$`，识别成功则消费到闭合 `$` 并返回标签；
/// 识别失败返回 `None`。标签须为字母/下划线开头，仅字母数字下划线（`$1` 参数、`$name` 不计入）。
fn dollar_quote_opener_tag(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Option<String> {
    let mut tag = String::new();
    // 空标签 `$$`。
    if matches!(chars.peek(), Some((_, '$'))) {
        chars.next();
        return Some(tag);
    }
    // 非空标签须以字母或下划线开头。
    match chars.peek() {
        Some((_, c)) if c.is_ascii_alphabetic() || *c == '_' => {}
        _ => return None,
    }
    while let Some((_, c)) = chars.peek() {
        if c.is_ascii_alphanumeric() || *c == '_' {
            tag.push(*c);
            chars.next();
        } else {
            break;
        }
    }
    if matches!(chars.peek(), Some((_, '$'))) {
        chars.next();
        Some(tag)
    } else {
        None
    }
}

/// 跳过 dollar-quote 体内直到匹配的结束 `$tag$`。标签体不含 `;`/`；`，
/// 故不匹配处的消费不影响分号切分。
fn skip_dollar_quote(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, tag: &str) {
    while let Some((_, c)) = chars.peek().copied() {
        if c != '$' {
            chars.next();
            continue;
        }
        // 试匹配 `$` + tag + `$`。
        chars.next(); // 消费起始 `$`
        let mut tag_chars = tag.chars();
        let mut matched = true;
        for tc in tag_chars.by_ref() {
            match chars.peek() {
                Some((_, c2)) if *c2 == tc => {
                    chars.next();
                }
                _ => {
                    matched = false;
                    break;
                }
            }
        }
        if matched && matches!(chars.peek(), Some((_, '$'))) {
            chars.next(); // 消费结束 `$`
            return;
        }
        // 未匹配：继续扫描（已消费的 tag/`$` 字符不含分号，不破坏切分）。
    }
}

fn is_sql_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_sql_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn push_pending_sql_space(compressed: &mut String, pending_space: &mut bool) {
    if *pending_space && !compressed.ends_with(' ') {
        compressed.push(' ');
    }
    *pending_space = false;
}

fn copy_line_comment_as_block(
    compressed: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    for ch in chars.by_ref() {
        if ch == '\n' {
            break;
        }
        compressed.push(ch);
    }
}

fn normalize_double_quoted_sql_strings(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '`' => copy_quoted_sql(&mut normalized, &mut chars, ch),
            '"' => copy_double_quoted_sql_string(&mut normalized, &mut chars),
            '-' if matches!(chars.peek(), Some('-')) => {
                normalized.push(ch);
                normalized.push(chars.next().unwrap());
                copy_until_newline(&mut normalized, &mut chars);
            }
            '/' if matches!(chars.peek(), Some('*')) => {
                normalized.push(ch);
                normalized.push(chars.next().unwrap());
                copy_until_block_comment_end(&mut normalized, &mut chars);
            }
            '#' => {
                normalized.push(ch);
                copy_until_newline(&mut normalized, &mut chars);
            }
            _ => normalized.push(ch),
        }
    }

    normalized
}

fn copy_double_quoted_sql_string(
    normalized: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    normalized.push('\'');
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if matches!(chars.peek(), Some('"')) {
                    chars.next();
                    normalized.push('"');
                } else {
                    normalized.push('\'');
                    break;
                }
            }
            '\'' => normalized.push_str("''"),
            _ => normalized.push(ch),
        }
    }
}

fn copy_quoted_sql(
    normalized: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    quote: char,
) {
    normalized.push(quote);
    while let Some(ch) = chars.next() {
        normalized.push(ch);
        if ch == quote {
            if matches!(chars.peek(), Some(next) if *next == quote) {
                normalized.push(chars.next().unwrap());
            } else {
                break;
            }
        }
    }
}

fn copy_until_newline(
    normalized: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    for ch in chars.by_ref() {
        normalized.push(ch);
        if ch == '\n' {
            break;
        }
    }
}

fn copy_until_block_comment_end(
    normalized: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    let mut previous = '\0';
    for ch in chars.by_ref() {
        normalized.push(ch);
        if previous == '*' && ch == '/' {
            break;
        }
        previous = ch;
    }
}

fn strip_until_newline(output: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for ch in chars.by_ref() {
        if ch == '\n' {
            output.push(ch);
            break;
        }
    }
}

fn strip_block_comment(output: &mut String, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let mut previous = '\0';
    for ch in chars.by_ref() {
        if ch == '\n' {
            output.push(ch);
        }
        if previous == '*' && ch == '/' {
            break;
        }
        previous = ch;
    }
}

fn copy_quoted_sql_from_indices(
    output: &mut String,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    quote: char,
) {
    output.push(quote);
    while let Some((_, ch)) = chars.next() {
        output.push(ch);
        if ch == quote {
            if matches!(chars.peek(), Some((_, next)) if *next == quote) {
                output.push(chars.next().unwrap().1);
            } else {
                break;
            }
        }
    }
}

fn copy_until_newline_from_indices(
    output: &mut String,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) {
    for (_, ch) in chars.by_ref() {
        output.push(ch);
        if ch == '\n' {
            break;
        }
    }
}

fn copy_until_block_comment_end_from_indices(
    output: &mut String,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) {
    let mut previous = '\0';
    for (_, ch) in chars.by_ref() {
        output.push(ch);
        if previous == '*' && ch == '/' {
            break;
        }
        previous = ch;
    }
}

fn format_sql_with_options(sql: &str, dialect: DatabaseKind) -> String {
    sqlformat::format(
        sql,
        &QueryParams::None,
        &FormatOptions {
            indent: Indent::Spaces(2),
            uppercase: Some(true),
            dialect: match dialect {
                DatabaseKind::MySql
                | DatabaseKind::TiDb
                | DatabaseKind::MariaDb
                | DatabaseKind::Sqlite
                | DatabaseKind::MongoDb
                | DatabaseKind::Redis
                | DatabaseKind::Postgres
                | DatabaseKind::Dameng
                | DatabaseKind::SqlServer
                | DatabaseKind::Oracle => Dialect::Generic,
            },
            ..FormatOptions::default()
        },
    )
}

fn format_create_table_sql(sql: &str, align_mysql_columns: bool) -> String {
    let Some(open_index) = find_unquoted_char(sql, '(') else {
        return uppercase_sql_keywords(sql);
    };
    let Some(close_index) = matching_close_paren(sql, open_index) else {
        return uppercase_sql_keywords(sql);
    };

    let prefix = uppercase_sql_keywords(&collapse_sql_whitespace(sql[..open_index].trim()));
    let body = &sql[open_index + 1..close_index];
    let suffix = sql[close_index + 1..].trim();
    let mut items = split_top_level_commas(body)
        .into_iter()
        .map(|item| uppercase_sql_keywords(&collapse_sql_whitespace(item.trim())))
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    if items.is_empty() {
        return uppercase_sql_keywords(sql);
    }

    if align_mysql_columns {
        align_mysql_create_table_columns(&mut items);
    }

    let mut formatted = format!("{prefix} (\n");
    for (index, item) in items.iter().enumerate() {
        formatted.push_str("  ");
        formatted.push_str(item);
        if index + 1 < items.len() {
            formatted.push(',');
        }
        formatted.push('\n');
    }
    formatted.push(')');
    if !suffix.is_empty() {
        if !suffix.starts_with(';') {
            formatted.push(' ');
        }
        formatted.push_str(&uppercase_sql_keywords(suffix));
    }
    formatted
}

struct MysqlColumnParts {
    name: String,
    column_type: String,
    attrs: String,
}

#[derive(Clone)]
struct MysqlColumnAlignedParts {
    name: String,
    column_type: String,
    character_set: String,
    collate: String,
    nullability: String,
    default_value: String,
    on_update: String,
    auto_increment: String,
    other: String,
    comment: String,
}

fn align_mysql_create_table_columns(items: &mut [String]) {
    let columns = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            parse_mysql_column_def(item).map(|column| (index, alignable_mysql_column(column)))
        })
        .collect::<Vec<_>>();
    if columns.len() < 2 {
        return;
    }

    let name_width = columns
        .iter()
        .map(|(_, column)| column.name.len())
        .max()
        .unwrap_or(0);
    let type_width = columns
        .iter()
        .map(|(_, column)| column.column_type.len())
        .max()
        .unwrap_or(0);
    let character_set_width = mysql_attr_width(&columns, |column| &column.character_set);
    let collate_width = mysql_attr_width(&columns, |column| &column.collate);
    let nullability_width = mysql_attr_width(&columns, |column| &column.nullability);
    let default_width = mysql_attr_width(&columns, |column| &column.default_value);
    let on_update_width = mysql_attr_width(&columns, |column| &column.on_update);
    let auto_increment_width = mysql_attr_width(&columns, |column| &column.auto_increment);
    let other_width = mysql_attr_width(&columns, |column| &column.other);

    for (index, column) in columns {
        let mut item = format!(
            "{}  {}",
            pad_ascii(&column.name, name_width),
            pad_ascii(&column.column_type, type_width)
        );
        push_mysql_aligned_attr(&mut item, &column.character_set, character_set_width);
        push_mysql_aligned_attr(&mut item, &column.collate, collate_width);
        push_mysql_aligned_attr(&mut item, &column.nullability, nullability_width);
        push_mysql_aligned_attr(&mut item, &column.default_value, default_width);
        push_mysql_aligned_attr(&mut item, &column.on_update, on_update_width);
        push_mysql_aligned_attr(&mut item, &column.auto_increment, auto_increment_width);
        push_mysql_aligned_attr(&mut item, &column.other, other_width);
        if !column.comment.is_empty() {
            item.push_str("  ");
            item.push_str(&column.comment);
        }
        trim_trailing_ascii_spaces(&mut item);
        items[index] = item;
    }
}

fn alignable_mysql_column(column: MysqlColumnParts) -> MysqlColumnAlignedParts {
    let mut aligned = MysqlColumnAlignedParts {
        name: column.name,
        column_type: column.column_type,
        character_set: String::new(),
        collate: String::new(),
        nullability: String::new(),
        default_value: String::new(),
        on_update: String::new(),
        auto_increment: String::new(),
        other: String::new(),
        comment: String::new(),
    };

    for clause in split_mysql_attr_clauses(&column.attrs) {
        let upper = clause.to_ascii_uppercase();
        if upper.starts_with("CHARACTER SET") {
            aligned.character_set = clause;
        } else if upper.starts_with("COLLATE") {
            aligned.collate = clause;
        } else if upper.starts_with("NOT NULL") || upper == "NULL" {
            aligned.nullability = clause;
        } else if upper.starts_with("DEFAULT") {
            aligned.default_value = clause;
        } else if upper.starts_with("ON UPDATE") {
            aligned.on_update = clause;
        } else if upper.starts_with("AUTO_INCREMENT") {
            aligned.auto_increment = clause;
        } else if upper.starts_with("COMMENT") {
            aligned.comment = clause;
        } else if aligned.other.is_empty() {
            aligned.other = clause;
        } else {
            aligned.other.push(' ');
            aligned.other.push_str(&clause);
        }
    }

    aligned
}

fn mysql_attr_width(
    columns: &[(usize, MysqlColumnAlignedParts)],
    attr: impl Fn(&MysqlColumnAlignedParts) -> &String,
) -> usize {
    columns
        .iter()
        .map(|(_, column)| attr(column).len())
        .max()
        .unwrap_or(0)
}

fn push_mysql_aligned_attr(item: &mut String, attr: &str, width: usize) {
    if width == 0 {
        return;
    }
    item.push_str("  ");
    item.push_str(&pad_ascii(attr, width));
}

fn trim_trailing_ascii_spaces(value: &mut String) {
    while value.ends_with(' ') {
        value.pop();
    }
}

fn parse_mysql_column_def(item: &str) -> Option<MysqlColumnParts> {
    if !item.starts_with('`') {
        return None;
    }

    let name_end = mysql_quoted_identifier_end(item)?;
    let name = &item[..name_end];
    let rest = item[name_end..].trim_start();
    if rest.is_empty() {
        return None;
    }

    let attr_index = mysql_column_attr_index(rest);
    let (column_type, attrs) = if let Some(index) = attr_index {
        (rest[..index].trim_end(), rest[index..].trim_start())
    } else {
        (rest.trim_end(), "")
    };

    if column_type.is_empty() {
        return None;
    }

    Some(MysqlColumnParts {
        name: name.to_string(),
        column_type: column_type.to_string(),
        attrs: attrs.to_string(),
    })
}

fn mysql_quoted_identifier_end(value: &str) -> Option<usize> {
    let mut chars = value.char_indices().skip(1).peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '`' {
            if matches!(chars.peek(), Some((_, '`'))) {
                chars.next();
            } else {
                return Some(index + ch.len_utf8());
            }
        }
    }
    None
}

fn mysql_column_attr_index(rest: &str) -> Option<usize> {
    let mut quote = None;
    let mut depth = 0usize;

    for (index, ch) in rest.char_indices() {
        if let Some(current_quote) = quote {
            if ch == current_quote {
                quote = None;
            }
            continue;
        }

        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
        } else if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
        } else if ch.is_whitespace() && depth == 0 && mysql_attr_starts(rest[index..].trim_start())
        {
            return Some(index);
        }
    }

    None
}

fn split_mysql_attr_clauses(attrs: &str) -> Vec<String> {
    let attrs = attrs.trim();
    if attrs.is_empty() {
        return Vec::new();
    }

    let mut starts = vec![0usize];
    let mut kinds = vec![mysql_attr_kind_at(attrs, 0).unwrap_or("OTHER")];
    let mut quote = None;
    let mut depth = 0usize;

    for (index, ch) in attrs.char_indices().skip(1) {
        if let Some(current_quote) = quote {
            if ch == current_quote {
                quote = None;
            }
            continue;
        }

        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
            continue;
        }
        if ch == '(' {
            depth += 1;
            continue;
        }
        if ch == ')' {
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth != 0 || !attrs[..index].ends_with(char::is_whitespace) {
            continue;
        }

        let Some(kind) = mysql_attr_kind_at(attrs, index) else {
            continue;
        };
        if kind == "NULL" && kinds.last().is_some_and(|kind| *kind == "DEFAULT") {
            continue;
        }
        starts.push(index);
        kinds.push(kind);
    }

    starts
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(attrs.len());
            attrs[start..end].trim().to_string()
        })
        .filter(|clause| !clause.is_empty())
        .collect()
}

fn mysql_attr_kind_at(value: &str, index: usize) -> Option<&'static str> {
    let rest = value.get(index..)?.trim_start();
    for keyword in [
        "CHARACTER SET",
        "AUTO_INCREMENT",
        "ON UPDATE",
        "NOT NULL",
        "PRIMARY KEY",
        "REFERENCES",
        "COLLATE",
        "DEFAULT",
        "COMMENT",
        "UNIQUE",
        "CHECK",
        "GENERATED",
        "NULL",
    ] {
        if keyword == "NULL"
            && value[..index]
                .trim_end()
                .to_ascii_uppercase()
                .ends_with("NOT")
        {
            continue;
        }
        if rest
            .get(..keyword.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(keyword))
            && rest
                .as_bytes()
                .get(keyword.len())
                .is_none_or(|byte| byte.is_ascii_whitespace())
        {
            return Some(keyword);
        }
    }
    None
}

fn mysql_attr_starts(value: &str) -> bool {
    mysql_attr_kind_at(value, 0).is_some()
}

fn pad_ascii(value: &str, width: usize) -> String {
    format!("{value:<width$}")
}

fn collapse_sql_whitespace(sql: &str) -> String {
    let mut collapsed = String::with_capacity(sql.len());
    let mut in_quote = None;
    let mut previous_space = false;

    for ch in sql.chars() {
        if let Some(quote) = in_quote {
            collapsed.push(ch);
            if ch == quote {
                in_quote = None;
            }
            previous_space = false;
            continue;
        }

        if matches!(ch, '\'' | '"' | '`') {
            in_quote = Some(ch);
            collapsed.push(ch);
            previous_space = false;
        } else if ch.is_whitespace() {
            if !previous_space {
                collapsed.push(' ');
                previous_space = true;
            }
        } else {
            collapsed.push(ch);
            previous_space = false;
        }
    }

    collapsed.trim().to_string()
}

fn find_unquoted_char(sql: &str, target: char) -> Option<usize> {
    let mut quote = None;
    for (index, ch) in sql.char_indices() {
        if let Some(current_quote) = quote {
            if ch == current_quote {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
        } else if ch == target {
            return Some(index);
        }
    }
    None
}

fn matching_close_paren(sql: &str, open_index: usize) -> Option<usize> {
    let mut quote = None;
    let mut depth = 0usize;
    for (index, ch) in sql.char_indices().filter(|(index, _)| *index >= open_index) {
        if let Some(current_quote) = quote {
            if ch == current_quote {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
        } else if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn split_top_level_commas(sql: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quote = None;
    let mut depth = 0usize;
    let mut start = 0usize;

    for (index, ch) in sql.char_indices() {
        if let Some(current_quote) = quote {
            if ch == current_quote {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
        } else if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
        } else if ch == ',' && depth == 0 {
            parts.push(&sql[start..index]);
            start = index + ch.len_utf8();
        }
    }
    parts.push(&sql[start..]);
    parts
}

fn uppercase_sql_keywords(sql: &str) -> String {
    let mut formatted = String::with_capacity(sql.len());
    let mut token = String::new();
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        if matches!(ch, '\'' | '"' | '`') {
            flush_sql_token(&mut formatted, &mut token);
            formatted.push(ch);
            for quoted in chars.by_ref() {
                formatted.push(quoted);
                if quoted == ch {
                    break;
                }
            }
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            token.push(ch);
        } else {
            flush_sql_token(&mut formatted, &mut token);
            formatted.push(ch);
        }
    }
    flush_sql_token(&mut formatted, &mut token);
    formatted
}

fn flush_sql_token(formatted: &mut String, token: &mut String) {
    if token.is_empty() {
        return;
    }
    if is_sql_keyword(token) {
        formatted.push_str(&token.to_ascii_uppercase());
    } else {
        formatted.push_str(token);
    }
    token.clear();
}

fn is_sql_keyword(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "add"
            | "alter"
            | "as"
            | "bigint"
            | "boolean"
            | "btree"
            | "cascade"
            | "check"
            | "charset"
            | "comment"
            | "constraint"
            | "create"
            | "current_timestamp"
            | "date"
            | "datetime"
            | "decimal"
            | "default"
            | "delete"
            | "double"
            | "engine"
            | "exists"
            | "float"
            | "foreign"
            | "from"
            | "index"
            | "insert"
            | "integer"
            | "int"
            | "into"
            | "key"
            | "not"
            | "null"
            | "numeric"
            | "on"
            | "primary"
            | "real"
            | "references"
            | "select"
            | "set"
            | "smallint"
            | "table"
            | "text"
            | "time"
            | "timestamp"
            | "unique"
            | "update"
            | "using"
            | "values"
            | "varchar"
            | "where"
    )
}

#[cfg(test)]
mod sqlserver_limit_tests {
    use super::sql_text_for_execution;
    use fluxdb_core::DatabaseKind;

    #[test]
    fn sqlserver_appends_top_and_never_uses_limit() {
        let sql =
            sql_text_for_execution("SELECT [id] FROM [dbo].[orders]", 101, DatabaseKind::SqlServer);
        assert_eq!(sql, "SELECT TOP (101) [id] FROM [dbo].[orders]");
        assert!(!sql.to_ascii_uppercase().contains("LIMIT"));
    }

    #[test]
    fn sqlserver_keeps_smaller_top_and_caps_larger_top() {
        assert_eq!(
            sql_text_for_execution("SELECT TOP 5 * FROM [dbo].[t]", 100, DatabaseKind::SqlServer),
            "SELECT TOP 5 * FROM [dbo].[t]"
        );
        assert_eq!(
            sql_text_for_execution(
                "SELECT TOP (5000) * FROM [dbo].[t]",
                100,
                DatabaseKind::SqlServer
            ),
            "SELECT TOP (100) * FROM [dbo].[t]"
        );
    }

    #[test]
    fn sqlserver_top_goes_after_distinct() {
        assert_eq!(
            sql_text_for_execution(
                "SELECT DISTINCT [name] FROM [dbo].[t]",
                10,
                DatabaseKind::SqlServer
            ),
            "SELECT DISTINCT TOP (10) [name] FROM [dbo].[t]"
        );
    }

    #[test]
    fn sqlserver_does_not_stack_top_on_offset_fetch() {
        let sql = "SELECT * FROM [dbo].[t] ORDER BY [id] OFFSET 0 ROWS FETCH NEXT 5 ROWS ONLY";
        assert_eq!(
            sql_text_for_execution(sql, 100, DatabaseKind::SqlServer),
            sql
        );
    }

    #[test]
    fn sqlserver_bracket_identifiers_are_not_keywords() {
        // `[from]` / `[where]` 是列名，不能被当成子句边界。
        assert_eq!(
            sql_text_for_execution(
                "SELECT [from], [where] FROM [dbo].[t]",
                3,
                DatabaseKind::SqlServer
            ),
            "SELECT TOP (3) [from], [where] FROM [dbo].[t]"
        );
    }
}

#[cfg(test)]
mod oracle_limit_tests {
    use super::sql_text_for_execution;
    use fluxdb_core::DatabaseKind;

    #[test]
    fn oracle_appends_fetch_first_and_never_uses_limit() {
        let sql = sql_text_for_execution("SELECT * FROM \"HR\".\"EMPLOYEES\"", 101, DatabaseKind::Oracle);
        assert_eq!(
            sql,
            "SELECT * FROM \"HR\".\"EMPLOYEES\" FETCH FIRST 101 ROWS ONLY"
        );
        assert!(!sql.to_ascii_uppercase().contains("LIMIT"));
    }

    #[test]
    fn oracle_keeps_existing_row_limiting_clause() {
        let sql = "SELECT * FROM employees ORDER BY id OFFSET 10 ROWS FETCH NEXT 5 ROWS ONLY";
        assert_eq!(
            sql_text_for_execution(sql, 100, DatabaseKind::Oracle),
            sql
        );
    }

    #[test]
    fn oracle_keeps_optimizer_hint() {
        let sql = sql_text_for_execution(
            "SELECT /*+ INDEX(e emp_idx) */ id FROM employees",
            100,
            DatabaseKind::Oracle,
        );
        assert_eq!(
            sql,
            "SELECT /*+ INDEX(e emp_idx) */ id FROM employees FETCH FIRST 100 ROWS ONLY"
        );
    }

    #[test]
    fn oracle_treats_hash_as_identifier_char_not_comment() {
        let sql = sql_text_for_execution("SELECT emp# FROM employees", 10, DatabaseKind::Oracle);
        assert!(sql.contains("emp#"), "Oracle 的 # 不是注释起始符: {sql}");
    }

    #[test]
    fn row_limit_goes_before_for_update() {
        // MySQL/PG 要求行数上限排在锁定子句之前，否则语法错误。
        assert_eq!(
            sql_text_for_execution("SELECT * FROM t FOR UPDATE", 10, DatabaseKind::MySql),
            "SELECT * FROM t LIMIT 10 FOR UPDATE"
        );
        assert_eq!(
            sql_text_for_execution("SELECT * FROM t FOR SHARE", 10, DatabaseKind::Postgres),
            "SELECT * FROM t LIMIT 10 FOR SHARE"
        );
    }

    #[test]
    fn oracle_skips_row_limit_when_for_update_present() {
        // Oracle 的 row_limiting_clause 与 for_update_clause 互斥（ORA-02014），
        // 封顶会让语句直接报错，只能放过。
        let sql = "SELECT * FROM employees FOR UPDATE";
        assert_eq!(
            sql_text_for_execution(sql, 10, DatabaseKind::Oracle),
            sql
        );
    }

    #[test]
    fn zero_page_size_does_not_add_row_limit() {
        assert_eq!(
            sql_text_for_execution("SELECT * FROM employees", 0, DatabaseKind::Oracle),
            "SELECT * FROM employees"
        );
    }
}

#[cfg(test)]
mod cte_row_limit_tests {
    use super::sql_text_for_execution;
    use fluxdb_core::DatabaseKind;

    /// CTE 过去完全绕过默认上限：`WITH ... SELECT` 拿不到任何行数限制，
    /// 一条这样的查询就能把整表拉进内存。
    #[test]
    fn cte_select_gets_row_limit_on_every_dialect() {
        let sql = "WITH x AS (SELECT 1 AS n FROM dual) SELECT * FROM x";
        assert_eq!(
            sql_text_for_execution(sql, 100, DatabaseKind::Oracle),
            "WITH x AS (SELECT 1 AS n FROM dual) SELECT * FROM x FETCH FIRST 100 ROWS ONLY"
        );
        assert_eq!(
            sql_text_for_execution(sql, 100, DatabaseKind::Postgres),
            "WITH x AS (SELECT 1 AS n FROM dual) SELECT * FROM x LIMIT 100"
        );
        assert_eq!(
            sql_text_for_execution(sql, 100, DatabaseKind::MySql),
            "WITH x AS (SELECT 1 AS n FROM dual) SELECT * FROM x LIMIT 100"
        );
        assert_eq!(
            sql_text_for_execution(
                "WITH x AS (SELECT [id] FROM [dbo].[t]) SELECT * FROM x",
                100,
                DatabaseKind::SqlServer
            ),
            "WITH x AS (SELECT [id] FROM [dbo].[t]) SELECT TOP (100) * FROM x"
        );
    }

    /// `WITH ... DELETE/UPDATE` 是写入：加上限会静默截断影响行数。
    #[test]
    fn cte_write_statements_get_no_row_limit() {
        for (kind, sql) in [
            (
                DatabaseKind::Postgres,
                "WITH gone AS (SELECT id FROM t) DELETE FROM t WHERE id IN (SELECT id FROM gone)",
            ),
            (
                DatabaseKind::Oracle,
                "WITH src AS (SELECT id FROM t) UPDATE t SET flag = 1 WHERE id IN (SELECT id FROM src)",
            ),
        ] {
            assert_eq!(sql_text_for_execution(sql, 100, kind), sql, "{kind:?}");
        }
    }

    /// CTE 体里的 `SELECT` 在括号内，不能被当成主语句类型。
    #[test]
    fn select_inside_cte_body_does_not_unlock_the_limit_alone() {
        let sql = "WITH moved AS (SELECT * FROM t) INSERT INTO archive SELECT * FROM moved";
        assert_eq!(
            sql_text_for_execution(sql, 100, DatabaseKind::Postgres),
            sql
        );
    }
}
