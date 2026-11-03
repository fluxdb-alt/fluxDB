//! 引擎编排：词法 → 容错解析 → 符号表 → 槽位 → 候选 → 过滤排序。

use crate::dialect::CompletionDialect;
use crate::locate::{lexical_slot, locate_slot};
use crate::metadata::MetadataSource;
use crate::model::{Candidate, CandidateKind, CompletionOutput, CompletionRequest, WordContext};
use crate::parse::{LineColIndex, parse_tolerant};
use crate::strategy::{SlotContext, slot_candidates};
use crate::symbols::{Symbols, collect};
use crate::word::{in_literal_or_comment, statement_range_at, word_context};

/// 候选上限：弹层只消费靠前的一批，多余的既拖慢跨层序列化也没人会翻。
const MAX_CANDIDATES: usize = 200;

/// 补全主入口：输入窗口文本与光标，输出替换区间与候选。
pub fn complete(
    request: &CompletionRequest<'_>,
    dialect: &dyn CompletionDialect,
    meta: &dyn MetadataSource,
) -> CompletionOutput {
    let text = request.text;
    let cursor = clamp_to_boundary(text, request.cursor);
    let parser_dialect = dialect.parser_dialect();

    let (statement_start, statement_end) = statement_range_at(text, cursor, parser_dialect);
    // 语句区间按去空白后的边界给出；光标可能落在尾部空白里，必须把窗口扩到光标。
    let statement_end = statement_end.max(cursor);
    let statement = &text[statement_start..statement_end];
    let cursor_in_statement = cursor.saturating_sub(statement_start);

    // 词法上下文按「光标所在语句」算：前一个关键字必须落在同一语句内，
    // 否则 `SELECT 1; sel|` 会拿上一条语句的 SELECT 当成当前上下文。
    let word = word_context(statement, cursor_in_statement, parser_dialect);
    // 字符串 / 注释里不是 SQL 语法位置，候选只会是噪声。
    if in_literal_or_comment(statement, cursor_in_statement) {
        return CompletionOutput {
            replace_start: word.prefix_start + statement_start,
            replace_end: word.replace_end + statement_start,
            items: Vec::new(),
        };
    }

    let index = LineColIndex::new(statement);
    // 掩码区间：把光标处的残词换成等长占位标识符，保住 FROM 等后文子句。
    let mask = Some((word.replace_start, word.replace_end));
    let parsed = parse_tolerant(statement, mask, cursor_in_statement, parser_dialect);
    // 词起点之前只有空白 ⇒ 语句起点。按词起点而非光标算：`sel|` 也是语句起点，
    // 按光标算会被当成「表达式里打了一半」，把 SELECT 这类起始关键字挡掉。
    let prefix_is_blank = statement[..word.replace_start.min(statement.len())]
        .trim()
        .is_empty();

    // 作用域链按光标投影：内层同名别名遮蔽外层。span 与光标同属解析后的文本坐标
    // （插入式兜底就发生在光标处，光标之前不受偏移影响）。
    let symbols = match &parsed {
        Some(parsed) => collect(&parsed.statements, &index).visible_at(cursor_in_statement),
        None => Symbols::default(),
    };

    let prefix = &statement[..cursor_in_statement.min(statement.len())];
    let slot = match &parsed {
        Some(parsed) => locate_slot(
            parsed,
            &index,
            cursor_in_statement,
            &word,
            prefix_is_blank,
            prefix,
        ),
        None => lexical_slot(&word, prefix_is_blank, prefix),
    };

    let ctx = SlotContext {
        dialect,
        meta,
        symbols: &symbols,
        word: &word,
        database: request.database,
        schema: request.schema,
    };

    let mut items = slot_candidates(&slot, &ctx);
    // 光标后已经写了调用括号（`lab_dis|(amount)`）：只补函数名，别再插入 `()`，
    // 否则采纳后变成 `lab_discount()(amount)`。
    if statement
        .get(word.replace_end..)
        .is_some_and(|suffix| suffix.trim_start().starts_with('('))
    {
        for item in items
            .iter_mut()
            .filter(|item| item.kind == CandidateKind::Function)
        {
            if let Some(name) = item.insert_text.strip_suffix("()") {
                item.insert_text = name.to_string();
            }
        }
    }
    retain_matching(&mut items, &word);
    let filter = word.word_part.trim().to_ascii_lowercase();
    items.sort_by_key(|left| sort_key(left, &filter));
    dedupe(&mut items);
    items.truncate(MAX_CANDIDATES);

    CompletionOutput {
        replace_start: word.prefix_start + statement_start,
        replace_end: word.replace_end + statement_start,
        items,
    }
}

/// 光标对齐到字符边界，避免切片 panic。
fn clamp_to_boundary(text: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

/// 按已输入内容过滤。引号标识符大小写敏感且按前缀匹配；否则走子序列匹配。
fn retain_matching(items: &mut Vec<Candidate>, word: &WordContext) {
    let filter = word.word_part.trim();
    if filter.is_empty() {
        return;
    }
    items.retain(|item| {
        let haystack = item.filter_text.as_deref().unwrap_or(&item.label);
        if word.quoted {
            haystack.starts_with(filter)
        } else {
            fuzzy_score(haystack, filter).is_some()
        }
    });
}

/// 模糊匹配打分；`None` 表示不匹配（硬过滤）。
///
/// 与 DBeaver 的 `TextUtils.fuzzyScore` 同构：命中 +1、首位命中 +4、命中落在词边界
/// （前一个字符不是字母数字，含 `_`）+2、连续命中序列按 4→8→16 翻倍。贪心从左到右
/// 取首个命中，不做最优对齐——打分只用于同层内排序，不值当为它上 DP。
pub fn fuzzy_score(value: &str, filter: &str) -> Option<i32> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = value.to_ascii_lowercase().chars().collect();
    let mut score = 0i32;
    let mut cursor = 0usize;
    let mut last_hit: Option<usize> = None;
    let mut streak = 0u32;
    for needle in filter.to_ascii_lowercase().chars() {
        let hit = cursor
            + haystack[cursor.min(haystack.len())..]
                .iter()
                .position(|&value_char| value_char == needle)?;
        score += 1;
        if hit == 0 {
            score += 4;
        } else if !haystack[hit - 1].is_alphanumeric() {
            score += 2;
        }
        // 连续命中：第 n 个连续字符给 4×2^(n-1)，封顶 16（再长就没有区分度了）。
        if last_hit == Some(hit.wrapping_sub(1)) && hit > 0 {
            streak += 1;
            score += 4 << (streak - 1).min(2);
        } else {
            streak = 0;
        }
        last_hit = Some(hit);
        cursor = hit + 1;
    }
    Some(score)
}

/// 排序键：语义分优先，其次匹配度（前缀 > 子序列），最后按标签。
fn sort_key(item: &Candidate, filter: &str) -> (i32, i32, String) {
    let (score, label) = item
        .sort_text
        .as_deref()
        .and_then(|sort_text| sort_text.split_once(':'))
        .and_then(|(score, label)| score.parse::<i32>().ok().map(|score| (score, label)))
        .unwrap_or((0, item.label.as_str()));
    // 同层内按匹配质量排：前缀 / 词边界命中 > 中段子序列命中。
    let match_score = fuzzy_score(label, filter).unwrap_or(0);
    (-score, -match_score, label.to_string())
}

/// 同 label + kind 去重，保留先出现者（高分在前）。
fn dedupe(items: &mut Vec<Candidate>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| {
        seen.insert((
            item.label.to_ascii_lowercase(),
            item.kind as u8,
            item.schema.clone(),
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect;
    use crate::metadata::{ColumnMeta, ForeignKeyMeta, TableKind, TableMeta};
    use std::collections::HashMap;

    /// 两张表：users(id, name) / orders(id, user_id)，orders.user_id → users.id。
    struct MockMeta {
        columns: HashMap<String, Vec<ColumnMeta>>,
        with_fk: bool,
    }

    impl MockMeta {
        fn new() -> Self {
            Self::build(true)
        }

        fn without_fk() -> Self {
            Self::build(false)
        }

        fn build(with_fk: bool) -> Self {
            let mut columns = HashMap::new();
            columns.insert(
                "users".to_string(),
                vec![column("users", "id", true), column("users", "name", false)],
            );
            columns.insert(
                "orders".to_string(),
                vec![
                    column("orders", "id", true),
                    column("orders", "user_id", false),
                ],
            );
            Self { columns, with_fk }
        }
    }

    fn column(table: &str, name: &str, primary_key: bool) -> ColumnMeta {
        ColumnMeta {
            table: table.to_string(),
            name: name.to_string(),
            type_name: Some("int".to_string()),
            nullable: true,
            primary_key,
            comment: None,
        }
    }

    impl MetadataSource for MockMeta {
        fn tables(&self, _schema: Option<&str>) -> Vec<TableMeta> {
            vec![
                TableMeta {
                    schema: Some("public".to_string()),
                    name: "users".to_string(),
                    kind: TableKind::Table,
                    comment: None,
                },
                TableMeta {
                    schema: Some("public".to_string()),
                    name: "orders".to_string(),
                    kind: TableKind::Table,
                    comment: None,
                },
            ]
        }

        fn schemas(&self) -> Vec<String> {
            vec!["public".to_string()]
        }

        fn columns(&self, key: &crate::metadata::TableKey) -> Vec<ColumnMeta> {
            self.columns
                .get(&key.table.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default()
        }

        fn foreign_keys(&self, key: &crate::metadata::TableKey) -> Vec<ForeignKeyMeta> {
            if !self.with_fk || !key.table.eq_ignore_ascii_case("orders") {
                return Vec::new();
            }
            vec![ForeignKeyMeta {
                columns: vec!["user_id".to_string()],
                ref_schema: Some("public".to_string()),
                ref_table: "users".to_string(),
                ref_columns: vec!["id".to_string()],
            }]
        }
    }

    /// 用 `|` 标记光标位置跑一次补全。
    fn labels(text: &str) -> Vec<String> {
        labels_with(&MockMeta::new(), text)
    }

    fn labels_with(meta: &MockMeta, text: &str) -> Vec<String> {
        let cursor = text.find('|').expect("用例必须用 | 标记光标");
        let sql = text.replace('|', "");
        let request = CompletionRequest::new(&sql, cursor);
        let output = complete(&request, &dialect::POSTGRES, meta);
        output.items.into_iter().map(|item| item.label).collect()
    }

    fn replace_range(text: &str) -> (usize, usize) {
        let cursor = text.find('|').expect("用例必须用 | 标记光标");
        let sql = text.replace('|', "");
        let request = CompletionRequest::new(&sql, cursor);
        let output = complete(&request, &dialect::POSTGRES, &MockMeta::new());
        (output.replace_start, output.replace_end)
    }

    fn labels_for(dialect: &dyn crate::dialect::CompletionDialect, text: &str) -> Vec<String> {
        candidates_for(dialect, text)
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    fn candidates_for(
        dialect: &dyn crate::dialect::CompletionDialect,
        text: &str,
    ) -> Vec<Candidate> {
        let cursor = text.find('|').expect("用例必须用 | 标记光标");
        let sql = text.replace('|', "");
        complete(
            &CompletionRequest::new(&sql, cursor),
            dialect,
            &MockMeta::new(),
        )
        .items
    }

    fn candidates_with(
        dialect: &dyn crate::dialect::CompletionDialect,
        text: &str,
    ) -> Vec<Candidate> {
        let cursor = text.find('|').expect("用例必须用 | 标记光标");
        let sql = text.replace('|', "");
        complete(
            &CompletionRequest::new(&sql, cursor),
            dialect,
            &MockMeta::new(),
        )
        .items
    }

    #[test]
    fn schema_qualified_dialects_insert_qualified_bare_tables() {
        for dialect in [
            &dialect::POSTGRES as &dyn crate::dialect::CompletionDialect,
            &dialect::DAMENG,
        ] {
            let items = candidates_with(dialect, "SELECT * FROM |");
            let users = items
                .iter()
                .find(|item| item.label == "users")
                .expect("应补出 users 表");
            assert_eq!(users.insert_text, "public.users");
        }
    }

    #[test]
    fn sqlserver_closed_bracket_qualifier_lists_only_schema_tables() {
        // 图一：`[app].` 后应只补该 schema 的表；不能把整库表/schema 再列一遍。
        let items = candidates_with(&dialect::SQLSERVER, "SELECT * FROM [public].|");
        assert!(
            items.iter().any(|item| item.label == "users"),
            "{:?}",
            items.iter().map(|item| &item.label).collect::<Vec<_>>()
        );
        assert!(
            items
                .iter()
                .all(|item| item.kind != crate::model::CandidateKind::Schema),
            "already-qualified schema must not be suggested again"
        );
        let users = items
            .iter()
            .find(|item| item.label == "users")
            .expect("users candidate");
        // 替换区间只覆盖已输入的表名部分，`[public].` 原样保留。
        assert_eq!(users.insert_text, "users");
    }

    #[test]
    fn sqlserver_bracket_qualifier_keeps_schema_when_partially_typed() {
        // 闭合方括号 + 已输入部分表名：仍按 schema 过滤。
        let items = candidates_with(&dialect::SQLSERVER, "SELECT * FROM [public].us|");
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["users"], "{labels:?}");
    }

    #[test]
    fn schema_candidate_inserts_trailing_dot() {
        let items = candidates_with(&dialect::POSTGRES, "SELECT * FROM |");
        let schema = items
            .iter()
            .find(|item| item.label == "public")
            .expect("应补出 public schema");
        assert_eq!(schema.insert_text, "public.");
    }

    #[test]
    fn postgres_prefix_matching_still_finds_schema_qualified_table() {
        let items = candidates_with(&dialect::POSTGRES, "SELECT * FROM us|");
        let users = items
            .iter()
            .find(|item| item.label == "users")
            .expect("应补出 users 表");
        assert_eq!(users.insert_text, "public.users");
    }

    #[test]
    fn quoted_schema_prefix_lists_tables() {
        let items = candidates_with(&dialect::POSTGRES, r#"SELECT * FROM "public".|"#);
        assert!(
            items.iter().any(|item| item.label == "users"),
            "候选：{items:?}"
        );
    }

    /// 空前缀下不能把 DDL 关键字灌进表达式候选（IDE 手感的核心）。
    #[test]
    fn empty_prefix_keeps_expression_keywords_contextual() {
        for text in [
            "SELECT * FROM users u WHERE u.id = |",
            "SELECT * FROM users u ORDER BY |",
        ] {
            let items = labels(text);
            for noise in ["CREATE", "DROP", "ALTER", "TRUNCATE", "DATABASE"] {
                assert!(
                    !items.contains(&noise.to_string()),
                    "{text} 候选：{items:?}"
                );
            }
        }
    }

    /// 各方言的上下文关键字表都要过滤出实际存在的项，不能整片落空。
    #[test]
    fn contextual_keywords_exist_in_every_dialect() {
        for dialect in [
            &dialect::MYSQL as &dyn crate::dialect::CompletionDialect,
            &dialect::DAMENG,
            &dialect::SQLITE,
        ] {
            let items = candidates_with(dialect, "SELECT * FROM users u WHERE u.id = |");
            for keyword in ["AND", "OR", "LIKE", "IS NULL", "GROUP BY"] {
                assert!(
                    items.iter().any(|item| item.label == keyword),
                    "缺少 {keyword}，候选：{items:?}"
                );
            }
        }
    }

    /// 打字中前缀命中的关键字照旧可补：`AND|` → AND。
    #[test]
    fn typed_prefix_keeps_matching_keywords() {
        let items = labels("SELECT * FROM users u WHERE u.id > 1 AND|");
        assert!(items.contains(&"AND".to_string()), "候选：{items:?}");
    }

    /// FROM 列表里的逗号后是新表名位置，不是别名位置。
    #[test]
    fn comma_in_from_starts_new_table_slot() {
        let items = labels("SELECT * FROM users u, |");
        assert!(items.contains(&"orders".to_string()), "候选：{items:?}");
        assert!(items.contains(&"users".to_string()), "候选：{items:?}");
        // 别名位置（无逗号）仍然只给续接关键字。
        let items = labels("SELECT * FROM users u |");
        assert!(!items.contains(&"orders".to_string()), "候选：{items:?}");
        assert!(items.contains(&"WHERE".to_string()), "候选：{items:?}");
    }

    /// 赋值右侧也是表达式，函数要能补出来。
    #[test]
    fn update_set_value_suggests_functions() {
        let items = labels("UPDATE users SET name = COU|");
        assert!(items.contains(&"COUNT".to_string()), "候选：{items:?}");
    }

    #[test]
    fn suggests_columns_for_bare_projection() {
        let items = labels("SELECT | FROM users");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn suggests_columns_for_qualified_projection() {
        let items = labels("SELECT u.| FROM users u");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn unknown_qualifier_yields_no_columns() {
        let items = labels("SELECT nope.| FROM users u");
        assert!(items.is_empty(), "候选应为空，实际：{items:?}");
    }

    #[test]
    fn suggests_tables_after_from() {
        let items = labels("SELECT * FROM |");
        assert!(items.contains(&"users".to_string()), "候选：{items:?}");
        assert!(items.contains(&"orders".to_string()), "候选：{items:?}");
    }

    /// 作用域内的 CTE / 派生表进 FROM 候选，且排在物理表前面。
    #[test]
    fn cte_and_derived_tables_appear_after_from() {
        let items = labels("WITH c AS (SELECT id FROM users) SELECT * FROM |");
        assert_eq!(
            items.first().map(String::as_str),
            Some("c"),
            "候选：{items:?}"
        );
        assert!(items.contains(&"users".to_string()), "候选：{items:?}");

        let items = labels("SELECT * FROM (SELECT id FROM users) t, |");
        assert_eq!(
            items.first().map(String::as_str),
            Some("t"),
            "候选：{items:?}"
        );
    }

    /// 同名跨多表的列自动带限定符，否则去重后只剩一条，另一张表的列选不到。
    #[test]
    fn ambiguous_columns_are_qualified() {
        let items = labels("SELECT | FROM users u JOIN orders o ON u.id = o.user_id");
        assert!(items.contains(&"u.id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"o.id".to_string()), "候选：{items:?}");
        assert!(!items.contains(&"id".to_string()), "候选：{items:?}");
        // 不歧义的列不加限定符。
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    /// 打字中只放开前缀命中的关键字，不再把整张 DDL 词表灌进弹层。
    #[test]
    fn typing_does_not_dump_ddl_keywords() {
        let items = labels("SELECT * FROM users u WHERE u.id = 1 AND c|");
        for noise in [
            "CREATE",
            "TRUNCATE",
            "CALL",
            "SELECT",
            "DISTINCT",
            "REFERENCES",
        ] {
            assert!(!items.contains(&noise.to_string()), "候选：{items:?}");
        }
        assert!(items.contains(&"CASE".to_string()), "候选：{items:?}");
    }

    /// 收敛不能收敛掉跨子句续写：`GROUP B|` 仍要补出 `GROUP BY`。
    #[test]
    fn multiword_keyword_matches_by_last_word() {
        let items = labels("SELECT * FROM users u GROUP B|");
        assert!(items.contains(&"GROUP BY".to_string()), "候选：{items:?}");
    }

    /// 打字中只在本子句的合法后继集里做前缀放开：DDL 与跨子句关键字不出现在表达式里。
    #[test]
    fn expression_slot_only_offers_clause_legal_keywords() {
        for text in [
            "SELECT * FROM users u WHERE i|",
            "SELECT * FROM users u ORDER BY i|",
            "SELECT * FROM users u GROUP BY i|",
        ] {
            let items = labels(text);
            for noise in [
                "INDEX",
                "IF EXISTS",
                "IF NOT EXISTS",
                "INNER JOIN",
                "LEFT JOIN",
            ] {
                assert!(
                    !items.contains(&noise.to_string()),
                    "{text} 候选：{items:?}"
                );
            }
        }
        // 谓词里合法的仍要在。
        let items = labels("SELECT * FROM users u WHERE i|");
        for legal in ["IN", "IS NULL", "IS NOT NULL"] {
            assert!(items.contains(&legal.to_string()), "候选：{items:?}");
        }
        // ORDER BY 里 `IN` / `IS NULL` 不合法，续写子句合法。
        let items = labels("SELECT * FROM users u ORDER BY i|");
        assert!(items.contains(&"LIMIT".to_string()), "候选：{items:?}");
        assert!(!items.contains(&"IS NULL".to_string()), "候选：{items:?}");
    }

    /// `IN (` / `USING (` 不再是死胡同。
    #[test]
    fn parenthesized_slots_offer_columns() {
        let items = labels("SELECT * FROM users u WHERE u.id IN (|");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");

        // USING 只给两侧的关联列（外键配对），不是全列。
        let items = labels("SELECT * FROM users u JOIN orders o USING (|");
        assert!(items.contains(&"user_id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(!items.contains(&"name".to_string()), "候选：{items:?}");
    }

    /// 赋值右侧之后能接 WHERE（此前是空候选）。
    #[test]
    fn update_value_slot_continues_with_where() {
        let items = labels("UPDATE users SET name = 1 WH|");
        assert!(items.contains(&"WHERE".to_string()), "候选：{items:?}");
    }

    /// 模糊打分：前缀与词边界优先于中段子序列，不匹配直接判否。
    #[test]
    fn fuzzy_score_ranks_prefix_and_word_boundary_first() {
        assert_eq!(fuzzy_score("id", "zzz"), None);
        // 首位命中得分高于中段命中。
        assert!(fuzzy_score("name", "na") > fuzzy_score("user_name", "na"));
        // 词边界命中（`_` 之后）高于词中命中。
        assert!(fuzzy_score("user_id", "id") > fuzzy_score("valid", "id"));
        // 空过滤词视为全匹配。
        assert_eq!(fuzzy_score("anything", ""), Some(0));
    }

    /// 字符串 / 注释里不是 SQL 语法位置。
    #[test]
    fn no_completion_inside_string_or_comment() {
        assert!(labels("SELECT * FROM users u -- 待办：补 JOIN |").is_empty());
        assert!(labels("SELECT * FROM users u WHERE u.name = 'abc |").is_empty());
        // 已闭合的字符串之后照常补全。
        assert!(!labels("SELECT * FROM users u WHERE u.name = 'abc' |").is_empty());
    }

    #[test]
    fn suggests_statement_starters_on_empty_input() {
        let items = labels("|");
        assert!(items.contains(&"SELECT".to_string()), "候选：{items:?}");
    }

    /// 起始关键字打了一半也要补：`sel|` 是语句起点，不是「表达式里打了一半」。
    #[test]
    fn suggests_statement_starters_while_typing_at_statement_start() {
        let items = labels("sel|");
        assert_eq!(
            items.first().map(String::as_str),
            Some("SELECT"),
            "候选：{items:?}"
        );
        let items = labels("ins|");
        assert!(
            items.contains(&"INSERT INTO".to_string()),
            "候选：{items:?}"
        );
    }

    /// 前一个关键字只在同一语句内算：分号后的新语句不该继承上一条的上下文。
    #[test]
    fn previous_keyword_does_not_leak_across_statements() {
        let items = labels("SELECT 1; sel|");
        assert!(items.contains(&"SELECT".to_string()), "候选：{items:?}");
    }

    /// 表达式位置也要能补子句关键字：`SELECT * fro|` → FROM。
    #[test]
    fn suggests_clause_keywords_in_expression_scope() {
        let items = labels("SELECT * fro|");
        assert!(items.contains(&"FROM".to_string()), "候选：{items:?}");
        let items = labels("SELECT * FROM t WHERE x = 1 AND|");
        assert!(items.contains(&"AND".to_string()), "候选：{items:?}");
    }

    /// 已闭合的引号标识符之后，别名位置的关键字仍可补全。
    #[test]
    fn suggests_join_keyword_after_quoted_identifier() {
        let items = labels("SELECT * from `3d_user` u left|");
        assert!(items.contains(&"LEFT JOIN".to_string()), "候选：{items:?}");
    }

    #[test]
    fn filters_by_partial_word() {
        let items = labels("SELECT na| FROM users");
        // 子序列匹配允许 name/name 之外的函数名命中，但列必须优先且 id 被滤掉。
        assert_eq!(
            items.first().map(String::as_str),
            Some("name"),
            "候选：{items:?}"
        );
        assert!(!items.contains(&"id".to_string()), "候选：{items:?}");
    }

    #[test]
    fn qualified_insert_text_excludes_qualifier() {
        let cursor = "SELECT u.na| FROM users u".find('|').unwrap();
        let sql = "SELECT u.na FROM users u".to_string();
        let output = complete(
            &CompletionRequest::new(&sql, cursor),
            &dialect::POSTGRES,
            &MockMeta::new(),
        );
        let name = output
            .items
            .iter()
            .find(|item| item.label == "name")
            .expect("应补出 name 列");
        assert_eq!(name.insert_text, "name");
        // 只替换词本身，保留 `u.` 限定符。
        assert_eq!(&sql[output.replace_start..output.replace_end], "na");
    }

    /// 光标后已有调用括号时，函数候选只补名字，不能插入重复的 `()`。
    #[test]
    fn function_insert_text_keeps_existing_call_parens() {
        let cursor = "SELECT COUNT|(id) FROM users".find('|').unwrap();
        let sql = "SELECT COUNT(id) FROM users".to_string();
        let output = complete(
            &CompletionRequest::new(&sql, cursor),
            &dialect::POSTGRES,
            &MockMeta::new(),
        );
        let count = output
            .items
            .iter()
            .find(|item| item.label == "COUNT")
            .expect("应补出 COUNT");
        assert_eq!(count.insert_text, "COUNT");
    }

    #[test]
    fn replaces_partial_word_range() {
        assert_eq!(replace_range("SELECT na| FROM users"), (7, 9));
    }

    #[test]
    fn replaces_partial_word_after_multiline_statement() {
        assert_eq!(replace_range("SELECT 1;\n\n\nsel|"), (12, 15));
        assert_eq!(
            replace_range("SELECT * FROM \"fluxdb_pr46_acceptance\".\"pg_acceptance\";\n\n\nsel|"),
            (58, 61)
        );
    }

    #[test]
    fn suggests_join_table_after_join_keyword() {
        let items = labels("SELECT * FROM users u JOIN |");
        assert!(items.contains(&"orders".to_string()), "候选：{items:?}");
    }

    #[test]
    fn join_on_left_limits_to_foreign_key_columns() {
        // users 只有 id 参与外键（name 被排除）。
        let items = labels("SELECT * FROM users u JOIN orders o ON u.|");
        assert_eq!(items, vec!["id".to_string()], "候选：{items:?}");
    }

    #[test]
    fn join_on_left_without_qualifier_offers_both_sides() {
        let items = labels("SELECT * FROM users u JOIN orders o ON |");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"user_id".to_string()), "候选：{items:?}");
        assert!(!items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn join_on_right_limits_to_paired_column() {
        let items = labels("SELECT * FROM users u JOIN orders o ON u.id = o.|");
        assert_eq!(items, vec!["user_id".to_string()], "候选：{items:?}");
    }

    #[test]
    fn join_on_right_supports_reversed_declaration() {
        let items = labels("SELECT * FROM users u JOIN orders o ON o.user_id = u.|");
        assert_eq!(items, vec!["id".to_string()], "候选：{items:?}");
    }

    #[test]
    fn join_on_falls_back_to_all_columns_without_fk() {
        let items = labels_with(
            &MockMeta::without_fk(),
            "SELECT * FROM users u JOIN orders o ON u.|",
        );
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn suggests_on_snippet_after_join_table() {
        let items = labels("SELECT * FROM users u JOIN orders o |");
        assert!(
            items.contains(&"ON u.id = o.user_id".to_string()),
            "候选：{items:?}"
        );
    }

    #[test]
    fn order_by_suggests_output_alias() {
        let items = labels("SELECT u.name AS n FROM users u ORDER BY |");
        assert!(items.contains(&"n".to_string()), "候选：{items:?}");
    }

    #[test]
    fn insert_column_list_suggests_table_columns() {
        let items = labels("INSERT INTO users (|");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn insert_column_list_excludes_written_columns() {
        let items = labels("INSERT INTO users (id, |");
        assert!(!items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn update_assignments_suggest_unassigned_columns() {
        let items = labels("UPDATE users SET name = 'a', |");
        assert!(!items.contains(&"name".to_string()), "候选：{items:?}");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
    }

    #[test]
    fn cte_columns_are_visible_through_alias() {
        let items = labels("WITH c AS (SELECT id FROM users) SELECT c.|");
        assert_eq!(items, vec!["id".to_string()], "候选：{items:?}");
    }

    /// CTE 在 FROM 里起别名后，别名仍要能解析到 CTE 的投影列。
    #[test]
    fn cte_columns_are_visible_through_from_alias() {
        let items = labels(
            "WITH item_totals AS (SELECT id, total FROM order_items) SELECT x.| FROM item_totals x",
        );
        assert_eq!(
            items,
            vec!["id".to_string(), "total".to_string()],
            "候选：{items:?}"
        );
    }

    /// CTE 的 FROM 别名不是关系名，不能出现在 FROM 候选里。
    #[test]
    fn cte_from_alias_is_not_a_from_candidate() {
        let items = labels(
            "WITH item_totals AS (SELECT id FROM order_items) SELECT * FROM item_totals x JOIN |",
        );
        assert!(!items.contains(&"x".to_string()), "候选：{items:?}");
        assert!(
            items.contains(&"item_totals".to_string()),
            "候选：{items:?}"
        );
    }

    /// 集合运算的派生表列名取首分支（SQL 标准）。
    #[test]
    fn set_operation_derived_columns_come_from_first_branch() {
        let items = labels(
            "SELECT u.| FROM (SELECT id AS entity_id FROM users UNION ALL SELECT id FROM orders) u",
        );
        assert!(items.contains(&"entity_id".to_string()), "候选：{items:?}");
    }

    /// 命名窗口定义体里的表达式也要能补列。
    #[test]
    fn named_window_definition_suggests_columns() {
        let items =
            labels("SELECT 1 FROM orders o WINDOW w AS (PARTITION BY o.id ORDER BY o.user_|)");
        assert!(items.contains(&"user_id".to_string()), "候选：{items:?}");
    }

    /// CTE 内的命名窗口定义同样要能补列。
    #[test]
    fn named_window_inside_cte_suggests_columns() {
        let items = labels(
            "WITH running AS (SELECT o.tenant_id,o.order_id,SUM(o.user_id) OVER w AS total \
             FROM orders o WHERE o.tenant_id=1 \
             WINDOW w AS (PARTITION BY o.tenant_id ORDER BY o.user_|,o.order_id \
             ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)) \
             SELECT r.order_id FROM running r",
        );
        assert!(items.contains(&"user_id".to_string()), "候选：{items:?}");
    }

    /// JSON_TABLE 的 COLUMNS 清单是别名唯一可见的列来源。
    #[test]
    fn json_table_columns_are_visible_through_alias() {
        let items = labels_for(
            &crate::dialect::MYSQL,
            "SELECT jt.ta| FROM users u JOIN JSON_TABLE(u.name, '$[*]' COLUMNS (tag VARCHAR(30) PATH '$')) AS jt ON TRUE",
        );
        assert!(items.contains(&"tag".to_string()), "候选：{items:?}");
    }

    #[test]
    fn derived_table_columns_are_visible_through_alias() {
        let items = labels("SELECT * FROM (SELECT id, name FROM users) t WHERE t.|");
        assert!(items.contains(&"id".to_string()), "候选：{items:?}");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
    }

    #[test]
    fn nested_subquery_sees_outer_relation() {
        let items = labels("SELECT * FROM users u WHERE u.id IN (SELECT | FROM orders)");
        assert!(items.contains(&"name".to_string()), "候选：{items:?}");
        assert!(items.contains(&"user_id".to_string()), "候选：{items:?}");
    }

    #[test]
    fn where_does_not_suggest_output_alias() {
        let items = labels("SELECT u.name AS n FROM users u WHERE |");
        assert!(!items.contains(&"n".to_string()), "候选：{items:?}");
    }

    /// 补全按方言出词：Oracle / 达梦 / PostgreSQL 各自的词表必须真的不同，
    /// 否则「接上 ORACLE 实现」只是换了个名字。
    #[test]
    fn candidates_follow_the_requested_dialect() {
        let oracle = labels_for(&dialect::ORACLE, "SELECT * FROM users WHERE |");
        let postgres = labels_for(&dialect::POSTGRES, "SELECT * FROM users WHERE |");
        // Oracle 有 MINUS/ROWNUM 语义下的函数与伪列，PostgreSQL 有 LIMIT/ILIKE/RETURNING。
        assert!(
            oracle.contains(&"NVL".to_string()),
            "Oracle 应补 NVL：{oracle:?}"
        );
        assert!(oracle.contains(&"DECODE".to_string()), "Oracle 应补 DECODE");
        assert!(
            !oracle.contains(&"LIMIT".to_string()),
            "Oracle 没有 LIMIT：{oracle:?}"
        );
        assert!(!oracle.contains(&"ILIKE".to_string()), "Oracle 没有 ILIKE");
        assert!(
            postgres.contains(&"LIMIT".to_string()),
            "PG 应有 LIMIT：{postgres:?}"
        );
        assert!(!postgres.contains(&"NVL".to_string()), "PG 没有 NVL");
    }

    /// `qualify_bare_table` 的差异要落到 `insert_text` 上，而不是只留在 trait 里。
    #[test]
    fn bare_table_insert_text_follows_dialect_qualification() {
        let oracle = candidates_for(&dialect::ORACLE, "SELECT * FROM |");
        let users = oracle
            .iter()
            .find(|item| item.label == "users")
            .expect("Oracle 应补 users");
        assert_eq!(users.insert_text, "public.users");

        let mysql = candidates_for(&dialect::MYSQL, "SELECT * FROM |");
        let users = mysql
            .iter()
            .find(|item| item.label == "users")
            .expect("MySQL 应补 users");
        assert_eq!(users.insert_text, "users", "MySQL 裸表名不带 schema 限定");
    }

    /// 标识符引用字符由方言决定：函数名为包装成 `()` 插入，表名按方言引号。
    #[test]
    fn identifier_quote_follows_dialect() {
        assert_eq!(dialect::MYSQL.identifier_quote(), '`');
        assert_eq!(dialect::ORACLE.identifier_quote(), '"');
        // SQL Server 用方括号，`quote_ident` 覆写成 `[x]`。
        assert_eq!(dialect::SQLSERVER.identifier_quote(), '[');
        assert_eq!(dialect::SQLSERVER.quote_ident("order"), "[order]");
        assert_eq!(dialect::MYSQL.quote_ident("order"), "`order`");
        assert_eq!(dialect::ORACLE.quote_ident("order"), "\"order\"");
    }
}
