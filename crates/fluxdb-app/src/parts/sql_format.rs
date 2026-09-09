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
        matches!(dialect, DatabaseKind::MySql | DatabaseKind::TiDb),
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
    if !matches!(dialect, DatabaseKind::MySql | DatabaseKind::TiDb) {
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
    token
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
        && token.chars().any(|ch| ch == '_' || ch.is_ascii_alphabetic())
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

fn sql_text_for_execution(sql: &str) -> String {
    apply_default_select_limit(&strip_sql_comments(&normalize_double_quoted_sql_strings(sql)))
}

fn strip_sql_comments(sql: &str) -> String {
    let mut stripped = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' | '`' => copy_quoted_sql(&mut stripped, &mut chars, ch),
            '-' if matches!(chars.peek(), Some('-')) => {
                chars.next();
                stripped.push(' ');
                strip_until_newline(&mut stripped, &mut chars);
            }
            '#' => {
                stripped.push(' ');
                strip_until_newline(&mut stripped, &mut chars);
            }
            '/' if matches!(chars.peek(), Some('*')) => {
                chars.next();
                stripped.push(' ');
                strip_block_comment(&mut stripped, &mut chars);
            }
            _ => stripped.push(ch),
        }
    }

    stripped
}

fn apply_default_select_limit(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len() + 16);
    let mut start = 0;
    for (end, separator_len) in sql_statement_ranges(sql) {
        let statement = normalize_select_limit_before_order(sql[start..end].trim());
        if !statement.is_empty() {
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(&limit_select_statement(&statement));
            if separator_len > 0 {
                output.push(';');
            }
        }
        start = end + separator_len;
    }
    output
}

fn normalize_select_limit_before_order(statement: &str) -> String {
    if !starts_with_sql_keyword(statement, "select") {
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
            '-' if matches!(chars.peek(), Some((_, '-'))) => skip_line_comment(&mut chars),
            '#' => skip_line_comment(&mut chars),
            '/' if matches!(chars.peek(), Some((_, '*'))) => skip_block_comment(&mut chars),
            ';' | '；' => ranges.push((index, ch.len_utf8())),
            _ => {}
        }
    }
    ranges.push((sql.len(), 0));
    ranges
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
        range.start += sql[range.start..range.end].chars().next().unwrap().len_utf8();
    }
    while range.end > range.start
        && sql[range.start..range.end]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        range.end -= sql[range.start..range.end].chars().next_back().unwrap().len_utf8();
    }
}

fn limit_select_statement(statement: &str) -> String {
    if starts_with_sql_keyword(statement, "select") && !has_top_level_sql_keyword(statement, "limit")
    {
        format!("{statement} LIMIT 100")
    } else {
        statement.to_string()
    }
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

fn skip_quoted_sql(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    quote: char,
) {
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

fn strip_until_newline(
    output: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    for ch in chars.by_ref() {
        if ch == '\n' {
            output.push(ch);
            break;
        }
    }
}

fn strip_block_comment(
    output: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
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
                | DatabaseKind::Sqlite
                | DatabaseKind::MongoDb
                | DatabaseKind::Redis => Dialect::Generic,
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
