fn data_filter_rule_sql(rule: &DataFilterRule) -> Option<String> {
    if !rule.enabled {
        return None;
    }
    let field = rule.field.as_deref()?;
    let field = sql_quote_ident(field);
    let normalized_values = rule
        .values
        .iter()
        .map(|value| normalized_data_filter_value(value))
        .collect::<Vec<_>>();
    let values = normalized_values
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let first_value = values.first().copied().unwrap_or("?");
    let first = sql_quote_literal(first_value);

    let sql = match rule.operator {
        DataFilterOperator::Eq => data_filter_multi_value_sql(&field, " = ", " OR ", &values),
        DataFilterOperator::Ne => data_filter_multi_value_sql(&field, " != ", " AND ", &values),
        DataFilterOperator::Lt => format!("{field} < {first}"),
        DataFilterOperator::Le => format!("{field} <= {first}"),
        DataFilterOperator::Gt => format!("{field} > {first}"),
        DataFilterOperator::Ge => format!("{field} >= {first}"),
        DataFilterOperator::Contains => {
            data_filter_like_sql(&field, &values, |value| format!("%{value}%"), false, "OR")
        }
        DataFilterOperator::NotContains => {
            data_filter_like_sql(&field, &values, |value| format!("%{value}%"), true, "AND")
        }
        DataFilterOperator::StartsWith => {
            data_filter_like_sql(&field, &values, |value| format!("{value}%"), false, "OR")
        }
        DataFilterOperator::NotStartsWith => {
            data_filter_like_sql(&field, &values, |value| format!("{value}%"), true, "AND")
        }
        DataFilterOperator::EndsWith => {
            data_filter_like_sql(&field, &values, |value| format!("%{value}"), false, "OR")
        }
        DataFilterOperator::NotEndsWith => {
            data_filter_like_sql(&field, &values, |value| format!("%{value}"), true, "AND")
        }
        DataFilterOperator::IsNull => format!("{field} IS NULL"),
        DataFilterOperator::IsNotNull => format!("{field} IS NOT NULL"),
        DataFilterOperator::IsEmpty => format!("{field} = ''"),
        DataFilterOperator::IsNotEmpty => format!("{field} != ''"),
        DataFilterOperator::Between => {
            let start = values.first().copied().unwrap_or("?");
            let end = values.get(1).copied().unwrap_or("?");
            format!(
                "{field} BETWEEN {} AND {}",
                sql_quote_literal(start),
                sql_quote_literal(end)
            )
        }
        DataFilterOperator::NotBetween => {
            let start = values.first().copied().unwrap_or("?");
            let end = values.get(1).copied().unwrap_or("?");
            format!(
                "{field} NOT BETWEEN {} AND {}",
                sql_quote_literal(start),
                sql_quote_literal(end)
            )
        }
        DataFilterOperator::InList => {
            format!("{field} IN ({})", sql_literal_list(&values))
        }
        DataFilterOperator::NotInList => {
            format!("{field} NOT IN ({})", sql_literal_list(&values))
        }
    };
    Some(sql)
}

fn data_filter_multi_value_sql(
    column: &str,
    op: &str,
    joiner: &str,
    values: &[&str],
) -> String {
    let values = if values.is_empty() {
        vec!["?"]
    } else {
        values.to_vec()
    };
    let parts = values
        .iter()
        .map(|value| format!("{column}{op}{}", sql_quote_literal(value)))
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        format!("({})", parts.join(format!(" {joiner} ").as_str()))
    } else {
        parts.into_iter().next().unwrap_or_default()
    }
}

fn data_filter_rules_sql(rules: &[DataFilterRule]) -> String {
    let mut clauses = Vec::new();
    let mut index = 0;
    while index < rules.len() {
        let rule = &rules[index];
        if rule.grouped {
            let mut group_clauses = Vec::new();
            while index < rules.len() && rules[index].grouped {
                if let Some(sql) = data_filter_rule_sql(&rules[index]) {
                    group_clauses.push(sql);
                }
                index += 1;
            }
            if !group_clauses.is_empty() {
                clauses.push(format!("({})", group_clauses.join(" AND ")));
            }
        } else {
            if let Some(sql) = data_filter_rule_sql(rule) {
                clauses.push(sql);
            }
            index += 1;
        }
    }
    clauses.join(" AND ")
}

fn data_filter_rules_sql_pretty(rules: &[DataFilterRule]) -> String {
    let mut lines = Vec::new();
    let mut index = 0;
    while index < rules.len() {
        if rules[index].grouped {
            let group_start = index;
            let mut group_lines = Vec::new();
            while index < rules.len() && rules[index].grouped {
                if let Some(sql) = data_filter_rule_sql(&rules[index]) {
                    group_lines.push(sql);
                }
                index += 1;
            }
            if !group_lines.is_empty() {
                lines.push(if group_start == 0 {
                    "(".to_string()
                } else {
                    "AND (".to_string()
                });
                for (group_index, sql) in group_lines.iter().enumerate() {
                    let suffix = if group_index + 1 < group_lines.len() {
                        " AND"
                    } else {
                        ""
                    };
                    lines.push(format!("  {sql}{suffix}"));
                }
                lines.push(")".to_string());
            }
        } else {
            if let Some(sql) = data_filter_rule_sql(&rules[index]) {
                lines.push(if lines.is_empty() {
                    sql
                } else {
                    format!("AND {sql}")
                });
            }
            index += 1;
        }
    }
    lines.join("\n")
}

fn parse_data_filter_rule_text(text: &str) -> Option<DataFilterRule> {
    let raw_text = trim_start_matches_case_insensitive(text.trim(), "WHERE").trim();
    let text = trim_wrapping_parentheses(raw_text).trim();
    if text.is_empty() {
        return None;
    }

    let rule = parse_data_filter_multi_value_rule(text, " OR ", DataFilterOperator::Eq)
        .or_else(|| parse_data_filter_multi_value_rule(text, " AND ", DataFilterOperator::Ne))
        .or_else(|| parse_data_filter_multi_value_rule(
            text,
            " OR ",
            DataFilterOperator::Contains,
        ))
        .or_else(|| parse_data_filter_multi_value_rule(
            text,
            " AND ",
            DataFilterOperator::NotContains,
        ))
        .or_else(|| parse_data_filter_multi_value_rule(
            text,
            " OR ",
            DataFilterOperator::StartsWith,
        ))
        .or_else(|| parse_data_filter_multi_value_rule(
            text,
            " AND ",
            DataFilterOperator::NotStartsWith,
        ))
        .or_else(|| parse_data_filter_multi_value_rule(text, " OR ", DataFilterOperator::EndsWith))
        .or_else(|| parse_data_filter_multi_value_rule(
            text,
            " AND ",
            DataFilterOperator::NotEndsWith,
        ))
        .or_else(|| parse_data_filter_single_rule(text))?;

    if data_filter_rule_text_signature(&rule) == sql_fragment_signature(text) {
        Some(rule)
    } else {
        None
    }
}

fn parse_data_filter_rules_text(text: &str) -> Option<Vec<DataFilterRule>> {
    let normalized = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let text = trim_start_matches_case_insensitive(normalized.trim(), "WHERE").trim();
    if text.is_empty() {
        return Some(Vec::new());
    }

    if let Some(rule) = parse_data_filter_rule_text(text) {
        return Some(vec![rule]);
    }

    let mut rules = Vec::new();
    for part in split_top_level_sql_and(text) {
        let part = part.trim();
        if has_wrapping_parentheses(part) {
            let inner = trim_wrapping_parentheses(part);
            let inner_parts = split_top_level_sql_and(inner);
            if inner_parts.len() > 1 {
                for inner_part in inner_parts {
                    let mut rule = parse_data_filter_rule_text(inner_part)?;
                    rule.grouped = true;
                    rules.push(rule);
                }
            } else if let Some(rule) = parse_data_filter_rule_text(inner) {
                rules.push(rule);
            }
        } else {
            let rule = parse_data_filter_rule_text(part)?;
            rules.push(rule);
        }
    }
    Some(rules)
}

fn split_top_level_sql_and(text: &str) -> Vec<&str> {
    split_top_level_sql_parts(text, "AND", true)
}

fn split_top_level_sql_or(text: &str) -> Vec<&str> {
    split_top_level_sql_parts(text, "OR", false)
}

fn split_top_level_sql_parts<'a>(
    text: &'a str,
    keyword: &str,
    skip_between: bool,
) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let needle = format!(" {keyword} ");
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut index = 0usize;
    let mut in_single_quote = false;
    let mut in_backtick = false;
    let mut between_pending = false;

    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        match ch {
            '\'' if !in_backtick => {
                if in_single_quote && text[index + 1..].starts_with('\'') {
                    index += 2;
                    continue;
                }
                in_single_quote = !in_single_quote;
                index += ch.len_utf8();
                continue;
            }
            '`' if !in_single_quote => {
                if in_backtick && text[index + 1..].starts_with('`') {
                    index += 2;
                    continue;
                }
                in_backtick = !in_backtick;
                index += ch.len_utf8();
                continue;
            }
            '(' if !in_single_quote && !in_backtick => depth += 1,
            ')' if !in_single_quote && !in_backtick => depth = (depth - 1).max(0),
            _ => {}
        }

        if depth == 0 && !in_single_quote && !in_backtick {
            if skip_between && is_sql_word_at(text, index, "BETWEEN") {
                between_pending = true;
            }
            if text[index..].starts_with(needle.as_str()) {
                if between_pending {
                    between_pending = false;
                    index += needle.len();
                    continue;
                }
                let part = text[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                index += needle.len();
                start = index;
                continue;
            }
        }
        index += ch.len_utf8();
    }

    let part = text[start..].trim();
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

fn parse_data_filter_multi_value_rule(
    text: &str,
    joiner: &str,
    operator: DataFilterOperator,
) -> Option<DataFilterRule> {
    let parts = if joiner == " OR " {
        split_top_level_sql_or(text)
    } else {
        split_top_level_sql_and(text)
    };
    if parts.len() <= 1 {
        return None;
    }

    let mut field = None;
    let mut values = BTreeSet::new();
    for part in parts {
        let (part_field, part_operator, value) = parse_data_filter_simple_piece(part)?;
        if part_operator != operator {
            return None;
        }
        match field.as_deref() {
            Some(existing) if existing != part_field.as_str() => return None,
            None => field = Some(part_field),
            _ => {}
        }
        if !value.is_empty() && value != "?" {
            values.insert(value);
        }
    }

    Some(DataFilterRule {
        enabled: true,
        field,
        operator,
        values,
        grouped: false,
    })
}

fn parse_data_filter_single_rule(text: &str) -> Option<DataFilterRule> {
    parse_data_filter_null_rule(text)
        .or_else(|| parse_data_filter_between_rule(text))
        .or_else(|| parse_data_filter_in_rule(text))
        .or_else(|| parse_data_filter_like_rule(text))
        .or_else(|| parse_data_filter_comparison_rule(text))
}

fn parse_data_filter_simple_piece(text: &str) -> Option<(String, DataFilterOperator, String)> {
    parse_data_filter_like_piece(text).or_else(|| parse_data_filter_comparison_piece(text))
}

fn parse_data_filter_null_rule(text: &str) -> Option<DataFilterRule> {
    if let Some(field) = trim_suffix_case_insensitive(text, " IS NOT NULL") {
        return Some(DataFilterRule {
            enabled: true,
            field: Some(parse_sql_identifier(field)?),
            operator: DataFilterOperator::IsNotNull,
            values: BTreeSet::new(),
            grouped: false,
        });
    }
    if let Some(field) = trim_suffix_case_insensitive(text, " IS NULL") {
        return Some(DataFilterRule {
            enabled: true,
            field: Some(parse_sql_identifier(field)?),
            operator: DataFilterOperator::IsNull,
            values: BTreeSet::new(),
            grouped: false,
        });
    }
    if let Some(field) = trim_suffix_case_insensitive(text, " IS NOT EMPTY") {
        return Some(DataFilterRule {
            enabled: true,
            field: Some(parse_sql_identifier(field)?),
            operator: DataFilterOperator::IsNotEmpty,
            values: BTreeSet::new(),
            grouped: false,
        });
    }
    if let Some(field) = trim_suffix_case_insensitive(text, " IS EMPTY") {
        return Some(DataFilterRule {
            enabled: true,
            field: Some(parse_sql_identifier(field)?),
            operator: DataFilterOperator::IsEmpty,
            values: BTreeSet::new(),
            grouped: false,
        });
    }

    if let Some((field, value)) = split_once_case_insensitive(text, " != ") {
        let value = parse_sql_literal(value.trim()).unwrap_or_else(|| value.trim().to_string());
        if value.is_empty() {
            return Some(DataFilterRule {
                enabled: true,
                field: Some(parse_sql_identifier(field)?),
                operator: DataFilterOperator::IsNotEmpty,
                values: BTreeSet::new(),
                grouped: false,
            });
        }
    }
    if let Some((field, value)) = split_once_case_insensitive(text, " = ") {
        let value = parse_sql_literal(value.trim()).unwrap_or_else(|| value.trim().to_string());
        if value.is_empty() {
            return Some(DataFilterRule {
                enabled: true,
                field: Some(parse_sql_identifier(field)?),
                operator: DataFilterOperator::IsEmpty,
                values: BTreeSet::new(),
                grouped: false,
            });
        }
    }

    None
}

fn parse_data_filter_between_rule(text: &str) -> Option<DataFilterRule> {
    for (needle, operator) in [
        (" NOT BETWEEN ", DataFilterOperator::NotBetween),
        (" BETWEEN ", DataFilterOperator::Between),
    ] {
        if let Some((field, rest)) = split_once_case_insensitive(text, needle) {
            let parts = split_top_level_sql_and(rest);
            if parts.len() != 2 {
                return None;
            }
            let field = parse_sql_identifier(field)?;
            let mut values = BTreeSet::new();
            for part in parts {
                let value = parse_sql_literal(part.trim()).unwrap_or_else(|| part.trim().to_string());
                if !value.is_empty() {
                    values.insert(value);
                }
            }
            return Some(DataFilterRule {
                enabled: true,
                field: Some(field),
                operator,
                values,
                grouped: false,
            });
        }
    }
    None
}

fn parse_data_filter_in_rule(text: &str) -> Option<DataFilterRule> {
    for (needle, operator) in [(" NOT IN ", DataFilterOperator::NotInList), (" IN ", DataFilterOperator::InList)] {
        if let Some((field, rest)) = split_once_case_insensitive(text, needle) {
            let rest = rest.trim();
            let rest = rest.strip_prefix('(')?.strip_suffix(')')?;
            let field = parse_sql_identifier(field)?;
            let mut values = BTreeSet::new();
            for part in split_top_level_sql_list(rest) {
                let value = parse_sql_literal(part.trim()).unwrap_or_else(|| part.trim().to_string());
                if !value.is_empty() {
                    values.insert(value);
                }
            }
            return Some(DataFilterRule {
                enabled: true,
                field: Some(field),
                operator,
                values,
                grouped: false,
            });
        }
    }
    None
}

fn parse_data_filter_like_rule(text: &str) -> Option<DataFilterRule> {
    let (field, operator, values) = parse_data_filter_like_piece(text)?;
    let mut set = BTreeSet::new();
    if !values.is_empty() && values != "?" {
        set.insert(values);
    }
    Some(DataFilterRule {
        enabled: true,
        field: Some(field),
        operator,
        values: set,
        grouped: false,
    })
}

fn parse_data_filter_comparison_rule(text: &str) -> Option<DataFilterRule> {
    let (field, operator, values) = parse_data_filter_comparison_piece(text)?;
    let mut set = BTreeSet::new();
    if !values.is_empty() && values != "?" {
        set.insert(values);
    }
    Some(DataFilterRule {
        enabled: true,
        field: Some(field),
        operator,
        values: set,
        grouped: false,
    })
}

fn parse_data_filter_like_piece(text: &str) -> Option<(String, DataFilterOperator, String)> {
    for (needle, operator) in [
        (" NOT LIKE ", DataFilterOperator::NotContains),
        (" LIKE ", DataFilterOperator::Contains),
    ] {
        if let Some((field, value)) = split_once_case_insensitive(text, needle) {
            let field = parse_sql_identifier(field)?;
            let value = parse_sql_literal(value.trim()).unwrap_or_else(|| value.trim().to_string());
            let (operator, value) = normalize_like_operator(operator, value);
            return Some((field, operator, value));
        }
    }
    None
}

fn parse_data_filter_comparison_piece(text: &str) -> Option<(String, DataFilterOperator, String)> {
    for (needle, operator) in [
        (" != ", DataFilterOperator::Ne),
        (" >= ", DataFilterOperator::Ge),
        (" <= ", DataFilterOperator::Le),
        (" = ", DataFilterOperator::Eq),
        (" > ", DataFilterOperator::Gt),
        (" < ", DataFilterOperator::Lt),
    ] {
        if let Some((field, value)) = split_once_case_insensitive(text, needle) {
            let field = parse_sql_identifier(field)?;
            let value = parse_sql_literal(value.trim()).unwrap_or_else(|| value.trim().to_string());
            return Some((field, operator, value));
        }
    }
    None
}

fn normalize_like_operator(
    operator: DataFilterOperator,
    value: String,
) -> (DataFilterOperator, String) {
    match operator {
        DataFilterOperator::Contains if value.starts_with('%') && value.ends_with('%') => {
            (DataFilterOperator::Contains, trim_like_percent(&value))
        }
        DataFilterOperator::Contains if value.ends_with('%') => {
            (DataFilterOperator::StartsWith, trim_like_percent(&value))
        }
        DataFilterOperator::Contains if value.starts_with('%') => {
            (DataFilterOperator::EndsWith, trim_like_percent(&value))
        }
        DataFilterOperator::NotContains if value.starts_with('%') && value.ends_with('%') => {
            (DataFilterOperator::NotContains, trim_like_percent(&value))
        }
        DataFilterOperator::NotContains if value.ends_with('%') => {
            (DataFilterOperator::NotStartsWith, trim_like_percent(&value))
        }
        DataFilterOperator::NotContains if value.starts_with('%') => {
            (DataFilterOperator::NotEndsWith, trim_like_percent(&value))
        }
        _ => (operator, value),
    }
}

fn split_top_level_sql_list(text: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    let mut in_single_quote = false;
    let mut in_backtick = false;

    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        match ch {
            '\'' if !in_backtick => {
                if in_single_quote && text[index + 1..].starts_with('\'') {
                    index += 2;
                    continue;
                }
                in_single_quote = !in_single_quote;
                index += ch.len_utf8();
                continue;
            }
            '`' if !in_single_quote => {
                if in_backtick && text[index + 1..].starts_with('`') {
                    index += 2;
                    continue;
                }
                in_backtick = !in_backtick;
                index += ch.len_utf8();
                continue;
            }
            ',' if !in_single_quote && !in_backtick => {
                let item = text[start..index].trim();
                if !item.is_empty() {
                    items.push(item);
                }
                index += ch.len_utf8();
                start = index;
                continue;
            }
            _ => {}
        }
        index += ch.len_utf8();
    }

    let item = text[start..].trim();
    if !item.is_empty() {
        items.push(item);
    }
    items
}

fn trim_start_matches_case_insensitive<'a>(text: &'a str, prefix: &str) -> &'a str {
    if text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix) {
        &text[prefix.len()..]
    } else {
        text
    }
}

fn trim_suffix_case_insensitive<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    if text.len() >= suffix.len() && text[text.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    {
        Some(&text[..text.len() - suffix.len()])
    } else {
        None
    }
}

fn is_sql_word_at(text: &str, index: usize, word: &str) -> bool {
    let end = index + word.len();
    let Some(slice) = text.get(index..end) else {
        return false;
    };
    if !slice.eq_ignore_ascii_case(word) {
        return false;
    }
    if index > 0 {
        let prev = text.as_bytes()[index - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return false;
        }
    }
    if end < text.len() {
        let next = text.as_bytes()[end];
        if next.is_ascii_alphanumeric() || next == b'_' {
            return false;
        }
    }
    true
}

fn data_filter_rule_text_signature(rule: &DataFilterRule) -> String {
    data_filter_rule_sql(rule)
        .map(|sql| sql_fragment_signature(trim_wrapping_parentheses(sql.as_str())))
        .unwrap_or_default()
}

fn sql_fragment_signature(text: &str) -> String {
    let mut out = String::new();
    let mut index = 0usize;
    let mut in_single_quote = false;
    let mut in_backtick = false;

    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        match ch {
            '\'' if !in_backtick => {
                in_single_quote = !in_single_quote;
                out.push(ch);
            }
            '`' if !in_single_quote => {
                in_backtick = !in_backtick;
            }
            _ if in_single_quote || in_backtick => {
                out.push(ch);
            }
            _ if ch.is_whitespace() => {}
            _ => out.extend(ch.to_uppercase()),
        }
        index += ch.len_utf8();
    }

    out
}

fn parse_data_sort_rules_text(text: &str) -> Option<Vec<DataSortRule>> {
    let text = trim_start_matches_case_insensitive(text.trim(), "ORDER BY").trim();
    if text.is_empty() {
        return Some(Vec::new());
    }

    let mut rules = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (field, direction) = split_last_whitespace(part).unwrap_or((part, "ASC"));
        let field = parse_sql_identifier(field)?;
        let ascending = !direction.eq_ignore_ascii_case("DESC");
        rules.push(DataSortRule {
            enabled: true,
            field,
            ascending,
        });
    }
    Some(rules)
}

fn parse_data_editor_sql_text(text: &str) -> Option<ParsedDataEditorSqlText> {
    let text = text.trim().trim_end_matches(';').trim();
    if !text
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SELECT"))
    {
        return None;
    }
    if find_top_level_sql_keyword(text, "JOIN").is_some()
        || find_top_level_sql_keyword(text, "GROUP BY").is_some()
        || find_top_level_sql_keyword(text, "HAVING").is_some()
    {
        return None;
    }

    let where_start = find_top_level_sql_keyword(text, "WHERE");
    let order_start = find_top_level_sql_keyword(text, "ORDER BY");
    let limit_start = find_top_level_sql_keyword(text, "LIMIT");
    let offset_start = find_top_level_sql_keyword(text, "OFFSET");

    let filter_text = where_start
        .map(|start| {
            let end = [order_start, limit_start, offset_start]
                .into_iter()
                .flatten()
                .filter(|end| *end > start)
                .min()
                .unwrap_or(text.len());
            text[start + "WHERE".len()..end].trim().to_string()
        })
        .unwrap_or_default();
    let sort_text = order_start
        .map(|start| {
            let end = [limit_start, offset_start]
                .into_iter()
                .flatten()
                .filter(|end| *end > start)
                .min()
                .unwrap_or(text.len());
            text[start + "ORDER BY".len()..end].trim().to_string()
        })
        .unwrap_or_default();
    let limit = limit_start.and_then(|start| {
        text[start + "LIMIT".len()..]
            .split_whitespace()
            .next()
            .and_then(|limit| limit.parse::<u64>().ok())
    });

    parse_data_filter_rules_text(&filter_text)?;
    parse_data_sort_rules_text(&sort_text)?;

    Some(ParsedDataEditorSqlText {
        filter_text,
        sort_text,
        limit,
    })
}

fn sql_text_selection_offset(
    text: &str,
    bounds: Option<&Bounds<Pixels>>,
    position: Point<Pixels>,
) -> usize {
    let Some(bounds) = bounds else {
        return 0;
    };
    let char_index = if position.x <= bounds.left() {
        0
    } else if position.x >= bounds.right() {
        sql_char_count(text)
    } else {
        ((position.x - bounds.left()) / sql_selection_char_width()).floor() as usize
    };
    byte_index_for_char_index(text, char_index)
}

fn byte_index_for_char_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn sql_char_count(text: &str) -> usize {
    text.chars().count()
}

fn sql_prefix_char_count(text: &str, byte_index: usize) -> usize {
    text.char_indices()
        .take_while(|(index, _)| *index < byte_index.min(text.len()))
        .count()
}

fn sql_selection_char_width() -> Pixels {
    px(7.3)
}

fn find_top_level_sql_keyword(text: &str, keyword: &str) -> Option<usize> {
    let upper_text = text.to_ascii_uppercase();
    let upper_keyword = keyword.to_ascii_uppercase();
    let mut depth = 0i32;
    let mut in_single_quote = false;
    let mut in_backtick = false;
    let mut index = 0usize;

    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        match ch {
            '\'' if !in_backtick => {
                if in_single_quote && text[index + 1..].starts_with('\'') {
                    index += 2;
                    continue;
                }
                in_single_quote = !in_single_quote;
                index += ch.len_utf8();
                continue;
            }
            '`' if !in_single_quote => {
                if in_backtick && text[index + 1..].starts_with('`') {
                    index += 2;
                    continue;
                }
                in_backtick = !in_backtick;
                index += ch.len_utf8();
                continue;
            }
            '(' if !in_single_quote && !in_backtick => depth += 1,
            ')' if !in_single_quote && !in_backtick => depth = (depth - 1).max(0),
            _ => {}
        }

        if depth == 0
            && !in_single_quote
            && !in_backtick
            && upper_text[index..].starts_with(&upper_keyword)
            && sql_keyword_boundary(text, index, keyword.len())
        {
            return Some(index);
        }
        index += ch.len_utf8();
    }
    None
}

fn sql_keyword_boundary(text: &str, start: usize, len: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[start + len..].chars().next();
    !before.is_some_and(is_sql_identifier_char) && !after.is_some_and(is_sql_identifier_char)
}

fn is_sql_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '`'
}

fn data_sort_rules_after_header_sort(
    current: &[DataSortRule],
    field: String,
    direction: Option<DataTableSortDirection>,
) -> Vec<DataSortRule> {
    let mut rules = current.to_vec();
    match direction {
        Some(direction) => {
            let ascending = direction == DataTableSortDirection::Ascending;
            if let Some(rule) = rules.iter_mut().find(|rule| rule.field == field) {
                rule.enabled = true;
                rule.ascending = ascending;
            } else {
                rules.push(DataSortRule {
                    enabled: true,
                    field,
                    ascending,
                });
            }
        }
        None => {
            rules.retain(|rule| rule.field != field);
        }
    }
    rules
}

fn data_sort_rules_text(rules: &[DataSortRule]) -> String {
    rules
        .iter()
        .filter(|rule| rule.enabled)
        .map(|rule| {
            format!(
                "{} {}",
                sql_quote_ident(&rule.field),
                if rule.ascending { "ASC" } else { "DESC" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn next_app_message(
    current: Option<&AppMessage>,
    text: impl Into<String>,
    kind: AppMessageKind,
) -> AppMessage {
    AppMessage {
        id: current.map(|message| message.id + 1).unwrap_or(1),
        text: text.into(),
        kind,
    }
}

fn trim_wrapping_parentheses(text: &str) -> &str {
    let text = text.trim();
    if has_wrapping_parentheses(text) {
        text[1..text.len() - 1].trim()
    } else {
        text
    }
}

fn has_wrapping_parentheses(text: &str) -> bool {
    let text = text.trim();
    text.starts_with('(') && text.ends_with(')') && text.len() > 2
}

fn split_once_case_insensitive<'a>(text: &'a str, needle: &str) -> Option<(&'a str, &'a str)> {
    let index = text
        .to_ascii_uppercase()
        .find(&needle.to_ascii_uppercase())?;
    Some((&text[..index], &text[index + needle.len()..]))
}

fn split_last_whitespace(text: &str) -> Option<(&str, &str)> {
    let index = text.rfind(char::is_whitespace)?;
    Some((&text[..index], text[index..].trim()))
}

fn parse_sql_identifier(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with('`') && text.ends_with('`') && text.len() >= 2 {
        Some(text[1..text.len() - 1].replace("``", "`"))
    } else if text.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
        Some(text.to_string())
    } else {
        None
    }
}

fn parse_sql_literal(text: &str) -> Option<String> {
    let text = text.trim();
    if text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2 {
        Some(text[1..text.len() - 1].replace("''", "'"))
    } else if text == "?" {
        Some(text.to_string())
    } else {
        None
    }
}

fn trim_like_percent(value: &str) -> String {
    value
        .trim_start_matches('%')
        .trim_end_matches('%')
        .to_string()
}

fn data_filter_like_sql(
    field: &str,
    values: &[&str],
    pattern: impl Fn(&str) -> String,
    negative: bool,
    joiner: &str,
) -> String {
    let values = if values.is_empty() {
        vec!["?"]
    } else {
        values.to_vec()
    };
    let operator = if negative { "NOT LIKE" } else { "LIKE" };
    values
        .iter()
        .map(|value| {
            format!(
                "{field} {operator} {}",
                sql_quote_literal(pattern(value).as_str())
            )
        })
        .collect::<Vec<_>>()
        .join(format!(" {joiner} ").as_str())
}

fn sql_qualified_object_name(object: &ObjectPath) -> String {
    let mut parts = Vec::new();
    if let Some(database) = object.database.as_deref() {
        parts.push(sql_quote_ident(database));
    }
    if let Some(schema) = object.schema.as_deref() {
        parts.push(sql_quote_ident(schema));
    }
    parts.push(sql_quote_ident(&object.name));
    parts.join(".")
}

fn sql_quote_ident(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn sql_quote_literal(value: &str) -> String {
    if value == "?" {
        "?".to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn sql_literal_list(values: &[&str]) -> String {
    if values.is_empty() {
        return "?".to_string();
    }
    values
        .iter()
        .map(|value| sql_quote_literal(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn data_change_statement_count(changes: &DataChangeSet) -> usize {
    changes.inserts.len() + changes.updates.len() + changes.deletes.len()
}

fn data_change_item_count(changes: &DataChangeSet) -> usize {
    changes.dirty_cell_count() + changes.deletes.len()
}

fn data_change_sql_preview(page: &DataPage, changes: &DataChangeSet) -> String {
    let table = sql_qualified_object_name(&changes.object);
    let mut statements = Vec::new();

    for update in &changes.updates {
        if update.cells.is_empty() {
            continue;
        }
        let assignments = update
            .cells
            .iter()
            .map(|cell| {
                format!(
                    "{} = {}",
                    sql_quote_ident(&cell.column),
                    sql_preview_cell_literal(&cell.value)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let where_clause = row_identity_sql(&update.identity);
        statements.push(format!(
            "UPDATE {table} SET {assignments} WHERE {where_clause};"
        ));
    }

    for row in &changes.inserts {
        let insert_values = page
            .columns
            .iter()
            .zip(row.values.iter())
            .filter(|(_, value)| !matches!(value, CellValue::Null))
            .collect::<Vec<_>>();
        if insert_values.is_empty() {
            statements.push(format!("INSERT INTO {table} DEFAULT VALUES;"));
            continue;
        }
        let column_names = insert_values
            .iter()
            .map(|(column, _)| sql_quote_ident(&column.name))
            .collect::<Vec<_>>();
        let values = insert_values
            .iter()
            .map(|(_, value)| sql_preview_cell_literal(value))
            .collect::<Vec<_>>();
        statements.push(format!(
            "INSERT INTO {table} ({}) VALUES ({});",
            column_names.join(", "),
            values.join(", ")
        ));
    }

    for identity in &changes.deletes {
        statements.push(format!(
            "DELETE FROM {table} WHERE {};",
            row_identity_sql(identity)
        ));
    }

    if statements.is_empty() {
        "-- 暂无待提交 SQL".to_string()
    } else {
        statements.join("\n")
    }
}

fn row_fields_for_page(page: &DataPage, row: usize) -> Option<Vec<RowFieldSnapshot>> {
    let row = page.rows.get(row)?;
    Some(
        page.columns
            .iter()
            .enumerate()
            .map(|(index, column)| RowFieldSnapshot {
                index: index + 1,
                name: column.name.clone(),
                type_name: column
                    .type_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                primary_key: column.primary_key,
                comment: column.comment.clone(),
                value: row.values.get(index).cloned().unwrap_or(CellValue::Null),
            })
            .collect(),
    )
}

fn row_json_text(fields: &[RowFieldSnapshot]) -> String {
    serde_json::to_string_pretty(&row_json_value(fields)).unwrap_or_default()
}

fn row_json_array_text(rows: &[&[RowFieldSnapshot]]) -> String {
    serde_json::to_string_pretty(&serde_json::Value::Array(
        rows.iter().map(|fields| row_json_value(fields)).collect(),
    ))
    .unwrap_or_default()
}

fn row_json_value(fields: &[RowFieldSnapshot]) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    for field in fields {
        object.insert(field.name.clone(), cell_value_json(&field.value));
    }
    serde_json::Value::Object(object)
}

fn cell_value_json(value: &CellValue) -> serde_json::Value {
    match value {
        CellValue::Null => serde_json::Value::Null,
        CellValue::Bool(value) => serde_json::Value::Bool(*value),
        CellValue::I64(value) => serde_json::json!(value),
        CellValue::F64(value) => serde_json::json!(value),
        CellValue::Text(value) | CellValue::Json(value) => serde_json::Value::String(value.clone()),
        CellValue::Bytes(value) => {
            serde_json::Value::String(format!("(BLOB) {} bytes", value.len()))
        }
        CellValue::BinarySummary(_) => serde_json::Value::String(value.display_label()),
    }
}

fn row_tsv_text(fields: &[RowFieldSnapshot]) -> String {
    row_tsv_rows_text(&[fields])
}

fn row_tsv_rows_text(rows: &[&[RowFieldSnapshot]]) -> String {
    let Some(fields) = rows.first() else {
        return String::new();
    };
    let header = fields
        .iter()
        .map(|field| tsv_cell(field.name.as_str()))
        .collect::<Vec<_>>()
        .join("\t");
    let values = rows
        .iter()
        .map(|fields| {
            fields
                .iter()
                .map(|field| tsv_cell(cell_value_label(&field.value).as_str()))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header}\n{values}")
}

fn tsv_cell(value: &str) -> String {
    value
        .replace('\t', " ")
        .replace('\r', " ")
        .replace('\n', " ")
}

fn row_insert_sql(
    object: &ObjectPath,
    fields: &[RowFieldSnapshot],
    skip_primary_key: bool,
) -> String {
    let values = fields
        .iter()
        .filter(|field| !(skip_primary_key && field.primary_key))
        .filter(|field| !matches!(field.value, CellValue::Null))
        .collect::<Vec<_>>();
    let table = sql_qualified_object_name(object);
    if values.is_empty() {
        return format!("INSERT INTO {table} DEFAULT VALUES;");
    }
    let columns = values
        .iter()
        .map(|field| sql_quote_ident(&field.name))
        .collect::<Vec<_>>()
        .join(", ");
    let literals = values
        .iter()
        .map(|field| sql_preview_cell_literal(&field.value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("INSERT INTO {table} ({columns}) VALUES ({literals});")
}

fn row_update_sql(object: &ObjectPath, fields: &[RowFieldSnapshot]) -> String {
    let assignments = fields
        .iter()
        .filter(|field| !field.primary_key)
        .map(|field| {
            format!(
                "{} = {}",
                sql_quote_ident(&field.name),
                sql_preview_cell_literal(&field.value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let identity = fields
        .iter()
        .filter(|field| field.primary_key)
        .map(|field| {
            format!(
                "{} = {}",
                sql_quote_ident(&field.name),
                sql_preview_cell_literal(&field.value)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let where_clause = if identity.is_empty() {
        "1 = 0"
    } else {
        identity.as_str()
    };
    format!(
        "UPDATE {} SET {} WHERE {};",
        sql_qualified_object_name(object),
        assignments,
        where_clause
    )
}

fn row_identity_sql(identity: &RowIdentity) -> String {
    if identity.values.is_empty() {
        return "1 = 0".to_string();
    }
    identity
        .values
        .iter()
        .map(|(column, value)| {
            format!(
                "{} = {}",
                sql_quote_ident(column),
                sql_preview_cell_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn sql_preview_cell_literal(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(true) => "TRUE".to_string(),
        CellValue::Bool(false) => "FALSE".to_string(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => sql_quote_literal(value),
        CellValue::BinarySummary(_) => "NULL".to_string(),
        CellValue::Bytes(value) => {
            let hex = value
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>();
            format!("X'{hex}'")
        }
    }
}
