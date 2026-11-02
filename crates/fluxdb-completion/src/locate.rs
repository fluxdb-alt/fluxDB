//! 槽位定位：把光标位置映射到「当前该补什么」的语法槽位。
//!
//! 这是整套引擎的决策输入——补什么由槽位决定，而不是由光标前的字符串决定。

use core::ops::ControlFlow;
use sqlparser::ast::{
    Expr, GroupByExpr, JoinConstraint, JoinOperator, NamedWindowDefinition, NamedWindowExpr,
    OrderByKind, Query, Select, SelectItem, Spanned, TableFactor, Visit, Visitor,
};

use crate::metadata::TableKey;
use crate::model::WordContext;
use crate::parse::{LineColIndex, TolerantParse};
use crate::symbols::object_name_parts;

/// 表达式所在子句，决定候选范围（投影/谓词/排序各有侧重）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprScope {
    SelectItem,
    Where,
    Having,
    JoinOn,
    GroupBy,
    OrderBy,
    /// UPDATE ... SET 赋值右侧（`SET a = 1 |`）。
    Set,
}

/// 光标所处的语法槽位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClauseSlot {
    /// 语句起点/分号后：允许 SELECT/INSERT/UPDATE/WITH 等起始关键字。
    StatementStart,
    /// 表达式位置；`qualifier` 为 `u.` 形式的限定符。
    Expression {
        scope: ExprScope,
        qualifier: Option<String>,
    },
    /// FROM / JOIN 后的表名位置。
    FromTable { join: bool },
    /// FROM / JOIN 表名之后：别名或后续子句位置。
    TableAlias,
    /// JOIN ... ON 条件起点：`joined` 为本次 JOIN 的表，`left_tables` 为其左侧已出现的关系。
    JoinOnLeft {
        joined: TableKey,
        left_tables: Vec<TableKey>,
    },
    /// JOIN ... ON 等号右侧：`left_column` 为左侧已确定的列（`u.id` 中的 `id`）。
    /// 右侧可以是本次 JOIN 的表，也可以回指左侧表，因此携带整条 JOIN 链的表集合。
    JoinOnRight {
        left_qualifier: Option<String>,
        left_column: String,
        joined: TableKey,
        left_tables: Vec<TableKey>,
    },
    /// INSERT 的列清单内部；`already_listed` 为光标前已写的列。
    InsertColumn {
        table: TableKey,
        already_listed: Vec<String>,
    },
    /// UPDATE ... SET 的赋值左侧：只给该表尚未赋值的列。
    UpdateAssignLhs {
        table: TableKey,
        already_set: Vec<String>,
    },
    /// UPDATE ... SET col = 的值位置。
    UpdateAssignValue,
    /// JOIN ... USING ( 的列清单内部：只给两侧都有的关联列。
    JoinUsing,
    /// 兜底：AST 不可用，按最近关键字近似。
    LexicalFallback { prev_keyword: Option<String> },
}

/// 定位光标槽位。
///
/// `cursor` 为语句内相对偏移；`parsed.parsed_len` 之后的区域属于被截断的部分，
/// 走「最后一个语法位置 + 词法上下文」推断。
/// `prefix_is_blank` 表示光标前只有空白——此时是语句起点。
pub(crate) fn locate_slot(
    parsed: &TolerantParse,
    index: &LineColIndex,
    cursor: usize,
    word: &WordContext,
    prefix_is_blank: bool,
    statement_prefix: &str,
) -> ClauseSlot {
    let mut locator = SlotLocator {
        cursor,
        index,
        parsed,
        best: None,
    };
    for statement in &parsed.statements {
        let _ = statement.visit(&mut locator);
    }

    // 光标越出解析范围时 AST 位置不可信（尾部被截断），直接走词法近似。
    let ast_hit = locator.best.take().filter(|_| cursor <= parsed.parsed_len);

    if let Some((_, slot)) = ast_hit {
        // AST 不记录 `u.` 这种半截输入，限定符统一由词法层补。
        return normalize(
            match slot {
                ClauseSlot::Expression { scope, .. } => ClauseSlot::Expression {
                    scope,
                    qualifier: word.qualifier.clone(),
                },
                other => other,
            },
            word,
            statement_prefix,
        );
    }

    // 光标落在语法节点之间的空白处或被截断的尾部：按最近关键字近似。
    if prefix_is_blank && word.prev_keyword.is_none() {
        return ClauseSlot::StatementStart;
    }
    // 未完成的 INSERT/UPDATE 拿不到 AST，按前缀词法识别列清单与赋值位置。
    if let Some(slot) = dml_slot(statement_prefix) {
        return slot;
    }
    if let Some(slot) = slot_from_prev_keyword(word) {
        return normalize(slot, word, statement_prefix);
    }
    if word.qualifier.is_some() {
        return ClauseSlot::Expression {
            scope: ExprScope::SelectItem,
            qualifier: word.qualifier.clone(),
        };
    }
    ClauseSlot::LexicalFallback {
        prev_keyword: word.prev_keyword.clone(),
    }
}

/// 表名后、光标前还有标识符 token（别名或已完成表名）时，位置不再是表名槽。
fn is_after_relation_name(word: &WordContext) -> bool {
    let Some(last) = word.prev_words.last() else {
        return false;
    };
    word.prev_keyword
        .as_deref()
        .is_none_or(|keyword| !last.eq_ignore_ascii_case(keyword))
}

/// 光标前的词是不是紧跟逗号（`FROM a, |` / `FROM a, b|`）——那是新表名位置，不是别名位置。
fn follows_comma(statement_prefix: &str, word: &WordContext) -> bool {
    let head = statement_prefix
        .strip_suffix(word.word_part.as_str())
        .unwrap_or(statement_prefix);
    head.trim_end().ends_with(',')
}

fn normalize(slot: ClauseSlot, word: &WordContext, statement_prefix: &str) -> ClauseSlot {
    match slot {
        ClauseSlot::FromTable { join } if is_after_relation_name(word) => {
            if follows_comma(statement_prefix, word) {
                ClauseSlot::FromTable { join }
            } else {
                ClauseSlot::TableAlias
            }
        }
        other => other,
    }
}

/// 未完成的 INSERT / UPDATE 在解析阶梯里拿不到 AST（sqlparser 要求完整语句），
/// 这里按光标前的前缀文本识别列清单与赋值位置。
fn dml_slot(prefix: &str) -> Option<ClauseSlot> {
    insert_column_slot(prefix).or_else(|| update_slot(prefix))
}

fn insert_column_slot(prefix: &str) -> Option<ClauseSlot> {
    if !leading_keyword(prefix, "INSERT") || find_keyword(prefix, "VALUES").is_some() {
        return None;
    }
    // 取光标前最后一个未闭合的括号：那就是列清单。
    let open = prefix.rfind('(')?;
    if prefix[open..].contains(')') {
        return None;
    }
    let table = table_ref_after_keyword(&prefix[..open], "INTO")?;
    // 最后一段是光标正在输入的列，不算「已写」。
    let mut segments: Vec<&str> = prefix[open + 1..].split(',').collect();
    segments.pop();
    let already_listed = segments
        .iter()
        .filter_map(|segment| first_identifier(segment))
        .collect();
    Some(ClauseSlot::InsertColumn {
        table,
        already_listed,
    })
}

fn update_slot(prefix: &str) -> Option<ClauseSlot> {
    if !leading_keyword(prefix, "UPDATE") {
        return None;
    }
    let set_at = find_keyword(prefix, "SET")?;
    let table = table_ref_after_keyword(&prefix[..set_at], "UPDATE")?;
    let segments: Vec<&str> = prefix[set_at + "SET".len()..].split(',').collect();
    let current = segments.last()?.trim();
    if current.contains('=') {
        return Some(ClauseSlot::UpdateAssignValue);
    }
    let already_set = segments[..segments.len() - 1]
        .iter()
        .filter_map(|segment| segment.split('=').next().and_then(first_identifier))
        .collect();
    Some(ClauseSlot::UpdateAssignLhs { table, already_set })
}

/// `keyword` 之后的表引用（支持 `schema.table` 与引号标识符）。
fn table_ref_after_keyword(text: &str, keyword: &str) -> Option<TableKey> {
    let at = find_keyword(text, keyword)?;
    let name = take_qualified_name(&text[at + keyword.len()..])?;
    let parts: Vec<String> = name.split('.').map(|part| unquote(part.trim())).collect();
    Some(TableKey {
        database: None,
        schema: parts
            .len()
            .checked_sub(2)
            .and_then(|i| parts.get(i))
            .cloned(),
        table: parts.last()?.clone(),
    })
}

/// 取开头的限定名：要么是引号标识符，要么读到空白/括号/分号为止。
fn take_qualified_name(text: &str) -> Option<String> {
    let text = text.trim_start();
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    let closer = match first {
        '"' | '`' => Some(first),
        '[' => Some(']'),
        _ => None,
    };
    if let Some(closer) = closer {
        let end = text[first.len_utf8()..].find(closer)? + first.len_utf8();
        return Some(text[first.len_utf8()..end].to_string());
    }
    let token: String = text
        .chars()
        .take_while(|ch| !ch.is_whitespace() && !matches!(ch, '(' | ')' | ';'))
        .collect();
    (!token.is_empty()).then_some(token)
}

fn unquote(value: &str) -> String {
    value
        .trim_matches(|ch| matches!(ch, '"' | '`' | '[' | ']'))
        .to_string()
}

fn leading_keyword(text: &str, keyword: &str) -> bool {
    let word: String = text
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    word.eq_ignore_ascii_case(keyword)
}

/// 按单词边界查找关键字，返回其字节起点。
fn find_keyword(text: &str, keyword: &str) -> Option<usize> {
    let upper = text.to_ascii_uppercase();
    let mut from = 0;
    while let Some(relative) = upper[from..].find(keyword) {
        let start = from + relative;
        let end = start + keyword.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
        if before_ok && after_ok {
            return Some(start);
        }
        from = end;
    }
    None
}

/// 段首标识符（去掉逗号/空白/引号）。
fn first_identifier(segment: &str) -> Option<String> {
    let segment = segment.trim();
    let first = segment.chars().next()?;
    if matches!(first, '"' | '`' | '[') {
        let closer = if first == '[' { ']' } else { first };
        let end = segment[first.len_utf8()..].find(closer)? + first.len_utf8();
        return Some(segment[first.len_utf8()..end].to_string());
    }
    let ident: String = segment
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '$'))
        .collect();
    (!ident.is_empty()).then_some(ident)
}

/// `prev_keyword` 之前紧邻的词是否为 `expected`（用于区分 GROUP BY / ORDER BY）。
fn is_preceded_by(word: &WordContext, expected: &str) -> bool {
    word.prev_words
        .iter()
        .rev()
        .nth(1)
        .is_some_and(|previous| previous.eq_ignore_ascii_case(expected))
}

/// 按最近关键字推断槽位（AST 不可用或光标在节点间隙时的近似）。
fn slot_from_prev_keyword(word: &WordContext) -> Option<ClauseSlot> {
    let keyword = word.prev_keyword.as_deref()?;
    let slot = match keyword {
        "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE" => ClauseSlot::FromTable {
            join: matches!(keyword, "JOIN"),
        },
        "ON" => ClauseSlot::Expression {
            scope: ExprScope::JoinOn,
            qualifier: word.qualifier.clone(),
        },
        // `WHERE x IN (` / `= ANY (`：括号里是值位置，按谓词表达式给候选。
        "WHERE" | "IN" => ClauseSlot::Expression {
            scope: ExprScope::Where,
            qualifier: word.qualifier.clone(),
        },
        // `JOIN t USING (`：括号里只能是两侧都有的关联列。
        "USING" => ClauseSlot::JoinUsing,
        "HAVING" => ClauseSlot::Expression {
            scope: ExprScope::Having,
            qualifier: word.qualifier.clone(),
        },
        "BY" => ClauseSlot::Expression {
            scope: if is_preceded_by(word, "GROUP") {
                ExprScope::GroupBy
            } else {
                ExprScope::OrderBy
            },
            qualifier: word.qualifier.clone(),
        },
        "SELECT" | "DISTINCT" | "AND" | "OR" | "NOT" | "SET" | "AS" => ClauseSlot::Expression {
            scope: ExprScope::SelectItem,
            qualifier: word.qualifier.clone(),
        },
        _ => return None,
    };
    Some(slot)
}

/// 遍历 AST，记录包含光标的最小节点及其槽位。
struct SlotLocator<'a> {
    cursor: usize,
    index: &'a LineColIndex,
    /// 解析结果，用于把解析文本偏移回映射到原语句偏移。
    parsed: &'a TolerantParse,
    best: Option<(usize, ClauseSlot)>,
}

impl SlotLocator<'_> {
    /// 解析文本偏移回映射到原语句偏移。
    fn mapped_start(&self, span: sqlparser::tokenizer::Span) -> usize {
        self.parsed.map_offset(self.index.offset(span.start))
    }

    /// JOIN ON 条件的槽位：光标在比较运算符右侧 → 精确到配对列，否则条件起点。
    fn join_slot(&self, expr: &Expr, joined: &TableKey, left_tables: &[TableKey]) -> ClauseSlot {
        if let Expr::BinaryOp { left, right, .. } = expr
            && self.cursor >= self.mapped_start(right.span())
        {
            let (left_qualifier, left_column) = column_ref(left);
            return ClauseSlot::JoinOnRight {
                left_qualifier,
                left_column,
                joined: joined.clone(),
                left_tables: left_tables.to_vec(),
            };
        }
        ClauseSlot::JoinOnLeft {
            joined: joined.clone(),
            left_tables: left_tables.to_vec(),
        }
    }

    /// 记录候选；同跨度时保留先记录者（更内层的节点先被访问）。
    fn record(&mut self, span: sqlparser::tokenizer::Span, slot: ClauseSlot) {
        let start = self.parsed.map_offset(self.index.offset(span.start));
        let end = self.parsed.map_offset(self.index.end_offset(span.end));
        // 插入的占位符经过偏移回映射后是零宽跨度，仍要参与定位。
        if self.cursor < start || self.cursor > end {
            return;
        }
        let width = end - start;
        if self
            .best
            .as_ref()
            .is_none_or(|(best_width, _)| width < *best_width)
        {
            self.best = Some((width, slot));
        }
    }
}

impl Visitor for SlotLocator<'_> {
    type Break = ();

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        for item in &select.projection {
            if let SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } = item
                && contains_subquery(expr)
            {
                continue;
            }
            self.record(
                item_span(item),
                ClauseSlot::Expression {
                    scope: ExprScope::SelectItem,
                    qualifier: None,
                },
            );
        }
        // `WINDOW w AS (PARTITION BY ... ORDER BY ...)`：定义体也是表达式位置，
        // 不记录就会落到词法兜底而给不出候选。
        for NamedWindowDefinition(_, window) in &select.named_window {
            let NamedWindowExpr::WindowSpec(spec) = window else {
                continue;
            };
            for expr in &spec.partition_by {
                self.record(
                    expr.span(),
                    ClauseSlot::Expression {
                        // 分区表达式不能引用投影别名，按谓词作用域处理。
                        scope: ExprScope::Where,
                        qualifier: None,
                    },
                );
            }
            for expr in &spec.order_by {
                self.record(
                    expr.expr.span(),
                    ClauseSlot::Expression {
                        scope: ExprScope::OrderBy,
                        qualifier: None,
                    },
                );
            }
        }
        for table in &select.from {
            self.record(table.relation.span(), ClauseSlot::FromTable { join: false });
            // 逐个 JOIN 累积左侧关系，ON 槽位用它限定候选。
            let mut left_tables: Vec<TableKey> =
                relation_key(&table.relation).into_iter().collect();
            for join in &table.joins {
                self.record(join.relation.span(), ClauseSlot::FromTable { join: true });
                let joined = relation_key(&join.relation);
                if let (Some(joined), Some(expr)) =
                    (joined.as_ref(), join_on_expr(&join.join_operator))
                {
                    let slot = self.join_slot(expr, joined, &left_tables);
                    self.record(expr.span(), slot);
                }
                left_tables.extend(joined);
            }
        }
        if let Some(selection) = &select.selection
            && !contains_subquery(selection)
        {
            self.record(
                selection.span(),
                ClauseSlot::Expression {
                    scope: ExprScope::Where,
                    qualifier: None,
                },
            );
        }
        if let Some(having) = &select.having {
            self.record(
                having.span(),
                ClauseSlot::Expression {
                    scope: ExprScope::Having,
                    qualifier: None,
                },
            );
        }
        match &select.group_by {
            GroupByExpr::Expressions(exprs, _) => {
                for expr in exprs {
                    self.record(
                        expr.span(),
                        ClauseSlot::Expression {
                            scope: ExprScope::GroupBy,
                            qualifier: None,
                        },
                    );
                }
            }
            GroupByExpr::All(_) => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        // ORDER BY 挂在 Query 上（sqlparser 0.62）。
        if let Some(order_by) = &query.order_by
            && let OrderByKind::Expressions(exprs) = &order_by.kind
        {
            for expr in exprs {
                self.record(
                    expr.expr.span(),
                    ClauseSlot::Expression {
                        scope: ExprScope::OrderBy,
                        qualifier: None,
                    },
                );
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<Self::Break> {
        if let TableFactor::Derived { subquery, .. } = table_factor {
            // 派生表内部的槽位由内层 Query 的 Select 记录；这里只在光标落在
            // 别名位置时补一个 TableAlias。
            let _ = subquery;
        }
        ControlFlow::Continue(())
    }
}

/// 该表达式是否含子查询——含子查询时其内部 Select 会记录更精确的槽位。
fn contains_subquery(expr: &Expr) -> bool {
    let mut finder = SubqueryFinder { found: false };
    let _ = expr.visit(&mut finder);
    finder.found
}

struct SubqueryFinder {
    found: bool,
}

impl Visitor for SubqueryFinder {
    type Break = ();

    fn pre_visit_query(&mut self, _query: &sqlparser::ast::Query) -> ControlFlow<Self::Break> {
        self.found = true;
        ControlFlow::Break(())
    }
}

fn item_span(item: &SelectItem) -> sqlparser::tokenizer::Span {
    item.span()
}

/// 表因子对应的表引用；派生表/表函数没有可查元数据的表名。
fn relation_key(table_factor: &TableFactor) -> Option<TableKey> {
    let TableFactor::Table { name, .. } = table_factor else {
        return None;
    };
    let parts = object_name_parts(name);
    Some(TableKey {
        database: None,
        schema: parts
            .len()
            .checked_sub(2)
            .and_then(|i| parts.get(i))
            .cloned(),
        table: parts.last()?.clone(),
    })
}

/// JOIN 操作符上的 `ON` 表达式。
fn join_on_expr(operator: &JoinOperator) -> Option<&Expr> {
    let constraint = match operator {
        JoinOperator::Join(constraint)
        | JoinOperator::Inner(constraint)
        | JoinOperator::Left(constraint)
        | JoinOperator::LeftOuter(constraint)
        | JoinOperator::Right(constraint)
        | JoinOperator::RightOuter(constraint)
        | JoinOperator::FullOuter(constraint)
        | JoinOperator::CrossJoin(constraint)
        | JoinOperator::Semi(constraint)
        | JoinOperator::LeftSemi(constraint)
        | JoinOperator::RightSemi(constraint)
        | JoinOperator::Anti(constraint)
        | JoinOperator::LeftAnti(constraint)
        | JoinOperator::RightAnti(constraint) => constraint,
        JoinOperator::AsOf { constraint, .. } => constraint,
        _ => return None,
    };
    match constraint {
        JoinConstraint::On(expr) => Some(expr),
        _ => None,
    }
}

/// 从表达式取出 `(限定符, 列名)`；非列引用时限定符为空、列名为原文本。
fn column_ref(expr: &Expr) -> (Option<String>, String) {
    match expr {
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => (
            Some(parts[parts.len() - 2].value.clone()),
            parts[parts.len() - 1].value.clone(),
        ),
        Expr::Identifier(ident) => (None, ident.value.clone()),
        other => (None, other.to_string()),
    }
}

/// 语句级兜底：语句完全无法解析时按词法推断。
pub(crate) fn lexical_slot(
    word: &WordContext,
    prefix_is_blank: bool,
    statement_prefix: &str,
) -> ClauseSlot {
    if prefix_is_blank && word.prev_keyword.is_none() {
        return ClauseSlot::StatementStart;
    }
    if let Some(slot) = dml_slot(statement_prefix) {
        return slot;
    }
    normalize(
        slot_from_prev_keyword(word).unwrap_or(ClauseSlot::LexicalFallback {
            prev_keyword: word.prev_keyword.clone(),
        }),
        word,
        statement_prefix,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_tolerant;
    use crate::word::word_context;
    use sqlparser::dialect::GenericDialect;

    fn slot(text: &str) -> ClauseSlot {
        let cursor = text.find('|').expect("用例必须用 | 标记光标");
        let sql = text.replace('|', "");
        let dialect = GenericDialect {};
        let word = word_context(&sql, cursor, &dialect);
        let prefix_is_blank = sql[..word.replace_start].trim().is_empty();
        let mask = (word.replace_start < word.replace_end)
            .then_some((word.replace_start, word.replace_end));
        let parsed = match parse_tolerant(&sql, mask, cursor, &dialect) {
            Some(parsed) => parsed,
            None => return lexical_slot(&word, prefix_is_blank, &sql[..cursor]),
        };
        let index = LineColIndex::new(&sql);
        locate_slot(
            &parsed,
            &index,
            cursor,
            &word,
            prefix_is_blank,
            &sql[..cursor],
        )
    }

    fn expr_scope(text: &str) -> ExprScope {
        match slot(text) {
            ClauseSlot::Expression { scope, .. } => scope,
            other => panic!("期望表达式槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_table_after_from() {
        assert_eq!(
            slot("SELECT * FROM |"),
            ClauseSlot::FromTable { join: false }
        );
    }

    #[test]
    fn locates_table_after_join() {
        assert_eq!(
            slot("SELECT * FROM users u JOIN |"),
            ClauseSlot::FromTable { join: true }
        );
    }

    #[test]
    fn locates_projection_expression() {
        assert_eq!(expr_scope("SELECT | FROM users"), ExprScope::SelectItem);
    }

    #[test]
    fn locates_qualified_projection() {
        match slot("SELECT u.| FROM users u") {
            ClauseSlot::Expression { scope, qualifier } => {
                assert_eq!(scope, ExprScope::SelectItem);
                assert_eq!(qualifier.as_deref(), Some("u"));
            }
            other => panic!("期望表达式槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_where_expression() {
        assert_eq!(expr_scope("SELECT * FROM users WHERE |"), ExprScope::Where);
    }

    #[test]
    fn locates_join_on_start_as_left_slot() {
        match slot("SELECT * FROM users u JOIN orders o ON |") {
            ClauseSlot::JoinOnLeft { joined, .. } => {
                assert_eq!(joined.table, "orders");
            }
            other => panic!("期望 ON 起点槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_order_by_expression() {
        assert_eq!(
            expr_scope("SELECT * FROM users ORDER BY |"),
            ExprScope::OrderBy
        );
    }

    #[test]
    fn locates_join_on_left_with_joined_table() {
        match slot("SELECT * FROM users u JOIN orders o ON u.|") {
            ClauseSlot::JoinOnLeft {
                joined,
                left_tables,
            } => {
                assert_eq!(joined.table, "orders");
                assert_eq!(
                    left_tables
                        .iter()
                        .map(|key| key.table.as_str())
                        .collect::<Vec<_>>(),
                    vec!["users"]
                );
            }
            other => panic!("期望 ON 左侧槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_join_on_right_with_left_column() {
        match slot("SELECT * FROM users u JOIN orders o ON u.id = o.|") {
            ClauseSlot::JoinOnRight {
                left_qualifier,
                left_column,
                joined,
                ..
            } => {
                assert_eq!(left_qualifier.as_deref(), Some("u"));
                assert_eq!(left_column, "id");
                assert_eq!(joined.table, "orders");
            }
            other => panic!("期望 ON 右侧槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_join_on_right_when_left_is_parent_column() {
        match slot("SELECT * FROM users u JOIN orders o ON o.user_id = u.|") {
            ClauseSlot::JoinOnRight {
                left_qualifier,
                left_column,
                joined,
                left_tables,
            } => {
                // 左侧列属于本次 JOIN 的表；右侧回到 users。
                assert_eq!(left_qualifier.as_deref(), Some("o"));
                assert_eq!(left_column, "user_id");
                assert_eq!(joined.table, "orders");
                assert_eq!(
                    left_tables
                        .iter()
                        .map(|key| key.table.as_str())
                        .collect::<Vec<_>>(),
                    vec!["users"]
                );
            }
            other => panic!("期望 ON 右侧槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_alias_position_after_join_table() {
        assert_eq!(
            slot("SELECT * FROM users u JOIN orders o |"),
            ClauseSlot::TableAlias
        );
    }

    #[test]
    fn locates_group_by_expression() {
        assert_eq!(
            expr_scope("SELECT name FROM users GROUP BY |"),
            ExprScope::GroupBy
        );
    }

    #[test]
    fn locates_insert_column_list() {
        match slot("INSERT INTO users (|") {
            ClauseSlot::InsertColumn {
                table,
                already_listed,
            } => {
                assert_eq!(table.table, "users");
                assert!(already_listed.is_empty());
            }
            other => panic!("期望 INSERT 列槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn insert_column_list_excludes_written_columns() {
        match slot("INSERT INTO users (id, |") {
            ClauseSlot::InsertColumn { already_listed, .. } => {
                assert_eq!(already_listed, vec!["id".to_string()])
            }
            other => panic!("期望 INSERT 列槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_update_assign_lhs() {
        match slot("UPDATE users SET |") {
            ClauseSlot::UpdateAssignLhs { table, already_set } => {
                assert_eq!(table.table, "users");
                assert!(already_set.is_empty());
            }
            other => panic!("期望 UPDATE 赋值左侧槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn update_assign_lhs_excludes_assigned_columns() {
        match slot("UPDATE users SET name = 'a', |") {
            ClauseSlot::UpdateAssignLhs { already_set, .. } => {
                assert_eq!(already_set, vec!["name".to_string()]);
            }
            other => panic!("期望 UPDATE 赋值左侧槽位，实际 {other:?}"),
        }
    }

    #[test]
    fn locates_update_assign_value() {
        assert_eq!(
            slot("UPDATE users SET name = |"),
            ClauseSlot::UpdateAssignValue
        );
    }

    #[test]
    fn falls_back_on_unparsable_text() {
        match slot("@@@ |") {
            ClauseSlot::LexicalFallback { .. } => {}
            other => panic!("期望词法兜底，实际 {other:?}"),
        }
    }
}
