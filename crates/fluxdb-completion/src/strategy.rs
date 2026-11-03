//! 槽位策略：每个槽位决定候选从哪来、怎么排序。
//!
//! 与旧实现的根本差别：候选集合由语法槽位决定，而不是由光标前字符串决定。

use std::collections::{HashMap, HashSet};

use crate::dialect::CompletionDialect;
use crate::locate::{ClauseSlot, ExprScope};
use crate::metadata::{
    ColumnMeta, ForeignKeyMeta, MetadataSource, RoutineKind, RoutineMeta, TableKey, TableKind,
};
use crate::model::{Candidate, CandidateKind, WordContext};
use crate::symbols::{RelationSymbol, Symbols};

/// 槽位策略需要的全部输入。
pub(crate) struct SlotContext<'a> {
    pub(crate) dialect: &'a dyn CompletionDialect,
    pub(crate) meta: &'a dyn MetadataSource,
    pub(crate) symbols: &'a Symbols,
    pub(crate) word: &'a WordContext,
    pub(crate) database: Option<&'a str>,
    pub(crate) schema: Option<&'a str>,
}

/// 排序基础分：越大越靠前。
const SCORE_FK_COLUMN: i32 = 120;
const SCORE_ON_SNIPPET: i32 = 110;
const SCORE_COLUMN: i32 = 100;
/// 作用域内的 CTE / 派生表：比物理表更可能是本次要写的关系。
const SCORE_LOCAL_RELATION: i32 = 96;
const SCORE_ALIAS: i32 = 95;
const SCORE_TABLE_RELATED: i32 = 95;
const SCORE_TABLE: i32 = 90;
const SCORE_SCHEMA: i32 = 85;
const SCORE_FUNCTION: i32 = 60;
const SCORE_KEYWORD: i32 = 50;

/// 按槽位产出候选。
pub(crate) fn slot_candidates(slot: &ClauseSlot, ctx: &SlotContext<'_>) -> Vec<Candidate> {
    match slot {
        ClauseSlot::StatementStart => keyword_candidates(ctx),
        ClauseSlot::FromTable { join } => table_candidates(*join, ctx),
        ClauseSlot::TableAlias => alias_position_candidates(ctx),
        ClauseSlot::Expression { scope, qualifier } => {
            expression_candidates(*scope, qualifier.as_deref(), ctx)
        }
        ClauseSlot::JoinOnLeft {
            joined,
            left_tables,
        } => join_on_left_candidates(joined, left_tables, ctx),
        ClauseSlot::JoinOnRight {
            left_qualifier,
            left_column,
            joined,
            left_tables,
        } => join_on_right_candidates(
            left_qualifier.as_deref(),
            left_column,
            joined,
            left_tables,
            ctx,
        ),
        ClauseSlot::InsertColumn {
            table,
            already_listed,
        } => unlisted_columns(table, already_listed, ctx),
        ClauseSlot::UpdateAssignLhs { table, already_set } => {
            unlisted_columns(table, already_set, ctx)
        }
        ClauseSlot::UpdateAssignValue => update_value_candidates(ctx),
        ClauseSlot::JoinUsing => join_on_left_from_symbols(ctx),
        ClauseSlot::LexicalFallback { prev_keyword } => {
            lexical_candidates(prev_keyword.as_deref(), ctx)
        }
    }
}

/// 语句起始：关键字 + 语句级 snippet。
fn keyword_candidates(ctx: &SlotContext<'_>) -> Vec<Candidate> {
    ctx.dialect
        .keywords()
        .iter()
        .filter(|keyword| {
            matches!(
                **keyword,
                "SELECT"
                    | "INSERT INTO"
                    | "UPDATE"
                    | "DELETE FROM"
                    | "CREATE"
                    | "ALTER"
                    | "DROP"
                    | "SHOW"
                    | "EXPLAIN"
                    | "DESCRIBE"
                    | "WITH"
                    | "CALL"
                    | "TRUNCATE"
            )
        })
        .map(|keyword| {
            Candidate::new(*keyword, *keyword, CandidateKind::Keyword)
                .with_sort_text(format!("{SCORE_KEYWORD}:{keyword}"))
        })
        .collect()
}

/// FROM / JOIN 后的表名。
fn table_candidates(join: bool, ctx: &SlotContext<'_>) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    // JOIN 槽位不硬限定，但已有关系的外键目标表加分。
    let related = if join {
        related_tables(ctx)
    } else {
        Vec::new()
    };

    // 限定符是 schema 时只出该 schema 的表。
    let qualifier_schema = ctx
        .word
        .qualifier
        .as_deref()
        .and_then(|qualifier| qualifier.rsplit('.').next());

    // 作用域内的 CTE / 派生表：比物理表更可能是本次要写的关系，排前面。
    if qualifier_schema.is_none() {
        for binding in ctx.symbols.virtual_bindings() {
            let name = &binding.name;
            candidates.push(
                Candidate::new(name.clone(), name.clone(), CandidateKind::Table)
                    .with_sort_text(format!("{SCORE_LOCAL_RELATION}:{name}")),
            );
        }
    }

    let tables = match qualifier_schema {
        Some(schema) => ctx.meta.tables(Some(schema)),
        None => ctx.meta.tables_for_bare_name(ctx.schema),
    };
    for table in tables {
        let schema = table
            .schema
            .clone()
            .or_else(|| ctx.schema.map(str::to_string));
        // 展示名始终是表名（已敲 schema 前缀时由替换区间保留前缀）；
        // 达梦没有可靠的默认 schema，插入文本显式带上 schema 限定。
        let label = table.name.clone();
        let insert_text = match (&qualifier_schema, &schema) {
            (None, Some(schema)) if ctx.dialect.qualify_bare_table() => {
                format!("{schema}.{}", table.name)
            }
            _ => table.name.clone(),
        };
        let kind = match table.kind {
            TableKind::Table => CandidateKind::Table,
            TableKind::View => CandidateKind::View,
        };
        let score = if related
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&table.name))
        {
            SCORE_TABLE_RELATED
        } else {
            SCORE_TABLE
        };
        let mut candidate = Candidate::new(label, insert_text, kind)
            .with_sort_text(format!("{score}:{}", table.name));
        if let Some(schema) = schema {
            candidate = candidate.with_schema(schema);
        }
        if let Some(comment) = table.comment {
            candidate = candidate.with_detail(comment);
        }
        candidates.push(candidate);
    }

    // 无 schema 前缀时也允许补 schema 名（PG 跨 schema 常用）。
    if qualifier_schema.is_none() {
        for schema in ctx.meta.schemas() {
            // 插入 `schema.`，让用户接着输入表名。
            candidates.push(
                Candidate::new(schema.clone(), format!("{schema}."), CandidateKind::Schema)
                    .with_sort_text(format!("{SCORE_SCHEMA}:{schema}")),
            );
        }
    }

    for keyword in ctx.dialect.keywords() {
        if matches!(
            *keyword,
            "JOIN" | "LEFT JOIN" | "RIGHT JOIN" | "INNER JOIN" | "WHERE" | "AS"
        ) {
            candidates.push(
                Candidate::new(*keyword, *keyword, CandidateKind::Keyword)
                    .with_sort_text(format!("{SCORE_KEYWORD}:{keyword}")),
            );
        }
    }

    candidates
}

/// 已有关系的外键指向的表名。
fn related_tables(ctx: &SlotContext<'_>) -> Vec<String> {
    let mut names = Vec::new();
    for relation in &ctx.symbols.relations {
        let key = relation.key.with_context(ctx.database, ctx.schema);
        for fk in ctx.meta.foreign_keys(&key) {
            names.push(fk.ref_table);
        }
    }
    names
}

/// 表达式位置：限定符命中关系时出列，否则出语句内全部关系的列。
fn expression_candidates(
    scope: ExprScope,
    qualifier: Option<&str>,
    ctx: &SlotContext<'_>,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    // ORDER BY / GROUP BY 可以直接引用投影别名。
    if qualifier.is_none() && matches!(scope, ExprScope::OrderBy | ExprScope::GroupBy) {
        candidates.extend(ctx.symbols.output_aliases.iter().map(|alias| {
            Candidate::new(alias.clone(), alias.clone(), CandidateKind::Column)
                .with_sort_text(format!("{SCORE_ALIAS}:{alias}"))
                .without_identifier_quotes()
        }));
    }

    // CTE / 派生表遮蔽同名物理表，优先按虚拟列的投影推导。
    if let Some(qualifier) = qualifier
        && let Some(columns) = ctx.symbols.qualifier_virtual_columns(qualifier)
    {
        return virtual_column_candidates(&columns);
    }
    if qualifier.is_none() {
        candidates.extend(unqualified_column_candidates(ctx));
    }

    let tables = match qualifier {
        Some(qualifier) => {
            let resolved = ctx.symbols.resolve_qualifier(qualifier);
            if resolved.is_empty() {
                // 限定符不是已知关系：可能是 schema 名 → 出该 schema 的表。
                if let Some(schema) = ctx
                    .meta
                    .schemas()
                    .into_iter()
                    .find(|schema| schema.eq_ignore_ascii_case(qualifier))
                {
                    let mut candidates: Vec<Candidate> = ctx
                        .meta
                        .tables(Some(&schema))
                        .into_iter()
                        .map(|table| {
                            let kind = match table.kind {
                                TableKind::Table => CandidateKind::Table,
                                TableKind::View => CandidateKind::View,
                            };
                            Candidate::new(
                                format!("{schema}.{}", table.name),
                                table.name.clone(),
                                kind,
                            )
                            .with_schema(schema.clone())
                            .with_sort_text(format!("{SCORE_TABLE}:{}", table.name))
                        })
                        .collect();
                    // 库限定的 UDF（`completion_lab.lab_discount(`）也要能补出来。
                    candidates.extend(
                        ctx.meta
                            .routines(Some(&schema))
                            .into_iter()
                            .filter(|routine| routine.kind == RoutineKind::Function)
                            .map(|routine| routine_candidate(&routine)),
                    );
                    return candidates;
                }
                // 限定符无法解析：返回空集合（IDE 行为，不再兜底全列）。
                return Vec::new();
            }
            resolved
        }
        None => Vec::new(),
    };

    for key in tables {
        // AST 里只有名字，库/schema 上下文由连接补全。
        let key = key.with_context(ctx.database, ctx.schema);
        for column in ctx.meta.columns(&key) {
            candidates.push(column_candidate(&column, &key, None, SCORE_COLUMN));
        }
    }

    if qualifier.is_none() {
        candidates.extend(function_candidates(ctx));
        // 谓词表达式里的字面量常量。
        if matches!(
            scope,
            ExprScope::Where | ExprScope::Having | ExprScope::JoinOn
        ) {
            for literal in ["NULL", "TRUE", "FALSE"] {
                candidates.push(
                    Candidate::new(literal, literal, CandidateKind::Keyword)
                        .with_sort_text(format!("{SCORE_KEYWORD}:{literal}")),
                );
            }
        }
        // 子句关键字：正在打字（`SELECT * fro|`）时全部关键字都进候选，靠前缀过滤收窄；
        // 空前缀只给当前子句相关的一小撮，否则每次弹层都被 60+ 个 DDL 关键字淹没。
        for keyword in expression_keywords(scope, ctx) {
            candidates.push(
                Candidate::new(keyword, keyword, CandidateKind::Keyword)
                    .with_sort_text(format!("{SCORE_KEYWORD}:{keyword}")),
            );
        }
    }

    candidates
}

/// 内置函数 + 用户自定义函数（过程只在 CALL 场景出现，不混入表达式候选）。
fn function_candidates(ctx: &SlotContext<'_>) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = ctx
        .dialect
        .functions()
        .iter()
        .map(|function| {
            Candidate::new(*function, format!("{function}()"), CandidateKind::Function)
                .with_sort_text(format!("{SCORE_FUNCTION}:{function}"))
        })
        .collect();
    for routine in ctx.meta.routines(ctx.schema) {
        if routine.kind == RoutineKind::Function {
            candidates.push(routine_candidate(&routine));
        }
    }
    candidates
}

/// 投影位置的关键字：`SELECT DISTINCT ...` / 收尾的 FROM。`CASE ... END` 属表达式通用集。
const SELECT_ITEM_KEYWORDS: &[&str] = &["ALL", "DISTINCT", "AS", "FROM", "INTO"];
/// 谓词位置（WHERE / HAVING）的后续子句。连接符与运算符见 [`PREDICATE_OPERATOR_KEYWORDS`]。
const PREDICATE_KEYWORDS: &[&str] = &[
    "GROUP BY",
    "HAVING",
    "ORDER BY",
    "LIMIT",
    "OFFSET",
    "UNION",
    "UNION ALL",
    "INTERSECT",
    "EXCEPT",
    "WINDOW",
    "RETURNING",
];
/// ON 条件里能续写的连接与后续子句。运算符见 [`PREDICATE_OPERATOR_KEYWORDS`]。
const JOIN_ON_KEYWORDS: &[&str] = &[
    "JOIN",
    "LEFT JOIN",
    "INNER JOIN",
    "RIGHT JOIN",
    "WHERE",
    "GROUP BY",
    "ORDER BY",
    "LIMIT",
    "OFFSET",
];
const GROUP_BY_KEYWORDS: &[&str] = &[
    "HAVING",
    "ORDER BY",
    "LIMIT",
    "OFFSET",
    "UNION",
    "UNION ALL",
    "WINDOW",
];
const ORDER_BY_KEYWORDS: &[&str] = &["ASC", "DESC", "LIMIT", "OFFSET", "UNION", "UNION ALL"];
/// UPDATE ... SET 赋值右侧之后：只能接 WHERE 或 RETURNING。
const SET_KEYWORDS: &[&str] = &["WHERE", "RETURNING"];

/// 光标前是否已敲了半截词。
fn is_typing(ctx: &SlotContext<'_>) -> bool {
    !ctx.word.word_part.trim().is_empty()
}

/// 谓词里的连接符与运算符：只在 WHERE / HAVING / ON 中合法。
const PREDICATE_OPERATOR_KEYWORDS: &[&str] = &[
    "AND",
    "OR",
    "NOT",
    "IN",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "IS NULL",
    "IS NOT NULL",
    "EXISTS",
];

/// 任何表达式里都合法的结构关键字与字面量常量。
const EXPRESSION_ATOM_KEYWORDS: &[&str] = &[
    "CASE", "WHEN", "THEN", "ELSE", "END", "NULL", "TRUE", "FALSE",
];

/// 表达式位置的关键字候选 = 本子句合法集 ∪ 表达式通用集，再按方言过滤。
///
/// 打字中不再拿整张方言词表做前缀放开：`INDEX` / `IF EXISTS`（DDL）与 `INNER JOIN`
/// （子句级）在表达式里不合法，`WHERE i|` 补出它们纯属噪声。`GROUP B|` → `GROUP BY`
/// 这类续写仍成立，因为续写子句本来就在对应子句的合法集里。
fn expression_keywords(scope: ExprScope, ctx: &SlotContext<'_>) -> Vec<&'static str> {
    let contextual: &[&str] = match scope {
        ExprScope::SelectItem => SELECT_ITEM_KEYWORDS,
        ExprScope::Where | ExprScope::Having => PREDICATE_KEYWORDS,
        ExprScope::JoinOn => JOIN_ON_KEYWORDS,
        ExprScope::GroupBy => GROUP_BY_KEYWORDS,
        ExprScope::OrderBy => ORDER_BY_KEYWORDS,
        ExprScope::Set => SET_KEYWORDS,
    };
    let operators: &[&str] = match scope {
        ExprScope::Where | ExprScope::Having | ExprScope::JoinOn => PREDICATE_OPERATOR_KEYWORDS,
        _ => &[],
    };
    // 方言没有的关键字（如 MySQL 无 ILIKE）不进候选。
    contextual
        .iter()
        .chain(operators)
        .chain(EXPRESSION_ATOM_KEYWORDS)
        .copied()
        .filter(|keyword| ctx.dialect.keywords().contains(keyword))
        .collect()
}

/// 列候选。`qualifier` 非空时展示与插入都用 `限定符.列名`：无限定符的同名列会被去重
/// 合并成一条，用户没法选到另一张表的那一列。
fn column_candidate(
    column: &ColumnMeta,
    key: &TableKey,
    qualifier: Option<&str>,
    score: i32,
) -> Candidate {
    let label = match qualifier {
        Some(qualifier) => format!("{qualifier}.{}", column.name),
        None => column.name.clone(),
    };
    let mut candidate = Candidate::new(label.clone(), label.clone(), CandidateKind::Column)
        .with_sort_text(format!("{score}:{label}"));
    if let Some(type_name) = &column.type_name {
        candidate = candidate.with_detail(type_name.clone());
    }
    if let Some(schema) = &key.schema {
        candidate = candidate.with_schema(schema.clone());
    }
    candidate
}

/// 无限定符位置的列：同名跨多个来源时自动带限定符。
fn unqualified_column_candidates(ctx: &SlotContext<'_>) -> Vec<Candidate> {
    let ambiguous = ambiguous_column_names(ctx);
    let is_ambiguous = |name: &str| ambiguous.contains(&name.to_ascii_lowercase());

    let mut candidates = Vec::new();
    for relation in &ctx.symbols.relations {
        let key = relation.key.with_context(ctx.database, ctx.schema);
        let qualifier = relation_name(relation);
        for column in ctx.meta.columns(&key) {
            let qualifier = is_ambiguous(&column.name).then_some(qualifier);
            candidates.push(column_candidate(&column, &key, qualifier, SCORE_COLUMN));
        }
    }
    for binding in ctx.symbols.virtual_bindings() {
        for column in &binding.columns {
            let label = if is_ambiguous(column) {
                format!("{}.{column}", binding.name)
            } else {
                column.clone()
            };
            candidates.push(
                Candidate::new(label.clone(), label.clone(), CandidateKind::Column)
                    .with_sort_text(format!("{SCORE_COLUMN}:{label}"))
                    .without_identifier_quotes(),
            );
        }
    }
    candidates
}

/// 在作用域内出现于多个来源的列名（小写）。
///
/// 按「限定符名集合」判重而不是出现次数：同一条 SQL 里自连接、或相关子查询里重复
/// 出现的同一张表，限定符名相同，加限定符也消不了歧义。
fn ambiguous_column_names(ctx: &SlotContext<'_>) -> HashSet<String> {
    let mut owners: HashMap<String, HashSet<String>> = HashMap::new();
    for relation in &ctx.symbols.relations {
        let key = relation.key.with_context(ctx.database, ctx.schema);
        let owner = relation_name(relation).to_ascii_lowercase();
        for column in ctx.meta.columns(&key) {
            owners
                .entry(column.name.to_ascii_lowercase())
                .or_default()
                .insert(owner.clone());
        }
    }
    for binding in ctx.symbols.virtual_bindings() {
        for column in &binding.columns {
            owners
                .entry(column.to_ascii_lowercase())
                .or_default()
                .insert(binding.name.to_ascii_lowercase());
        }
    }
    owners
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(column, _)| column)
        .collect()
}

/// 表名之后（别名/后续子句）：续接关键字 + 由外键推导的 ON 片段。
fn alias_position_candidates(ctx: &SlotContext<'_>) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for keyword in [
        "AS",
        "JOIN",
        "LEFT JOIN",
        "INNER JOIN",
        "WHERE",
        "GROUP BY",
        "ORDER BY",
        "LIMIT",
    ] {
        if ctx.dialect.keywords().contains(&keyword) {
            candidates.push(
                Candidate::new(keyword, keyword, CandidateKind::Keyword)
                    .with_sort_text(format!("{SCORE_KEYWORD}:{keyword}")),
            );
        }
    }

    // 最近的关系是本次 JOIN 的表，其余为左侧；有外键时给出可直接落地的 ON 条件。
    let relations = &ctx.symbols.relations;
    if relations.len() >= 2 {
        let joined = &relations[relations.len() - 1];
        let joined_key = joined.key.with_context(ctx.database, ctx.schema);
        for left in &relations[..relations.len() - 1] {
            let left_key = left.key.with_context(ctx.database, ctx.schema);
            for (left_column, joined_column) in fk_pairs(&left_key, &joined_key, ctx) {
                let text = format!(
                    "ON {}.{} = {}.{}",
                    relation_name(left),
                    left_column,
                    relation_name(joined),
                    joined_column
                );
                candidates.push(
                    Candidate::new(text.clone(), text.clone(), CandidateKind::Snippet)
                        .with_sort_text(format!("{SCORE_ON_SNIPPET}:{text}")),
                );
            }
        }
    }
    candidates
}

/// ON 条件起点：有外键元数据时只给关联列，否则回退两侧全列。
fn join_on_left_candidates(
    joined: &TableKey,
    left_tables: &[TableKey],
    ctx: &SlotContext<'_>,
) -> Vec<Candidate> {
    let joined = joined.with_context(ctx.database, ctx.schema);
    let lefts: Vec<TableKey> = left_tables
        .iter()
        .map(|key| key.with_context(ctx.database, ctx.schema))
        .collect();
    let allowed = match filter_tables(
        lefts
            .iter()
            .chain(std::iter::once(&joined))
            .cloned()
            .collect(),
        ctx,
    ) {
        Some(allowed) => allowed,
        None => return Vec::new(),
    };
    let is_allowed = |key: &TableKey| allowed.iter().any(|candidate| same_table(candidate, key));

    let mut candidates = Vec::new();
    for left in &lefts {
        for (left_column, joined_column) in fk_pairs(left, &joined, ctx) {
            if is_allowed(left) {
                candidates.push(named_column(left, &left_column, SCORE_FK_COLUMN, ctx));
            }
            if is_allowed(&joined) {
                candidates.push(named_column(&joined, &joined_column, SCORE_FK_COLUMN, ctx));
            }
        }
    }
    if !candidates.is_empty() {
        return candidates;
    }

    // 无外键：两侧全列。
    allowed
        .iter()
        .flat_map(|key| columns_of(key, ctx, SCORE_COLUMN))
        .collect()
}

/// ON 条件等号右侧：只给与左侧列配对的列。
///
/// 右侧既可能是本次 JOIN 的表（`ON u.id = o.`），也可能回指左侧表
/// （`ON o.user_id = u.`），因此按整条 JOIN 链的表集合找配对列。
fn join_on_right_candidates(
    left_qualifier: Option<&str>,
    left_column: &str,
    joined: &TableKey,
    left_tables: &[TableKey],
    ctx: &SlotContext<'_>,
) -> Vec<Candidate> {
    let tables: Vec<TableKey> = left_tables
        .iter()
        .chain(std::iter::once(joined))
        .map(|key| key.with_context(ctx.database, ctx.schema))
        .collect();
    let Some(allowed) = filter_tables(tables, ctx) else {
        return Vec::new();
    };
    let left_key = left_qualifier
        .and_then(|qualifier| ctx.symbols.resolve_qualifier(qualifier).into_iter().next())
        .map(|key| key.with_context(ctx.database, ctx.schema));

    let mut candidates = Vec::new();
    if let Some(left_key) = left_key {
        for table in &allowed {
            if same_table(table, &left_key) {
                continue;
            }
            for column in paired_columns(&left_key, left_column, table, ctx) {
                candidates.push(named_column(table, &column, SCORE_FK_COLUMN, ctx));
            }
        }
        if !candidates.is_empty() {
            return candidates;
        }
    }

    // 左侧列无法定位或无外键配对：回退到目标表的全列。
    allowed
        .iter()
        .flat_map(|key| columns_of(key, ctx, SCORE_COLUMN))
        .collect()
}

/// 限定符存在时把表集合收窄到限定符命中的表；限定符无法解析时返回 `None`。
fn filter_tables(tables: Vec<TableKey>, ctx: &SlotContext<'_>) -> Option<Vec<TableKey>> {
    let Some(qualifier) = ctx.word.qualifier.as_deref() else {
        return Some(tables);
    };
    let resolved = ctx.symbols.resolve_qualifier(qualifier);
    if resolved.is_empty() {
        return None;
    }
    Some(
        tables
            .into_iter()
            .filter(|table| resolved.iter().any(|key| same_table(key, table)))
            .collect(),
    )
}

/// 两表之间的外键配对列，按 `(左表列, 右表列)` 返回；双向声明都认。
fn fk_pairs(left: &TableKey, right: &TableKey, ctx: &SlotContext<'_>) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    // 右表持有指向左表的外键。
    for fk in ctx.meta.foreign_keys(right) {
        if fk_ref_matches(&fk, left) {
            pairs.extend(
                fk.columns
                    .iter()
                    .zip(&fk.ref_columns)
                    .map(|(column, ref_column)| (ref_column.clone(), column.clone())),
            );
        }
    }
    // 左表持有指向右表的外键。
    for fk in ctx.meta.foreign_keys(left) {
        if fk_ref_matches(&fk, right) {
            pairs.extend(
                fk.columns
                    .iter()
                    .zip(&fk.ref_columns)
                    .map(|(column, ref_column)| (column.clone(), ref_column.clone())),
            );
        }
    }
    pairs
}

/// 与 `left_column` 配对的、属于 `joined` 的列。
fn paired_columns(
    left: &TableKey,
    left_column: &str,
    joined: &TableKey,
    ctx: &SlotContext<'_>,
) -> Vec<String> {
    let mut columns = Vec::new();
    for fk in ctx.meta.foreign_keys(joined) {
        if fk_ref_matches(&fk, left) {
            for (column, ref_column) in fk.columns.iter().zip(&fk.ref_columns) {
                if ref_column.eq_ignore_ascii_case(left_column) {
                    columns.push(column.clone());
                }
            }
        }
    }
    for fk in ctx.meta.foreign_keys(left) {
        if fk_ref_matches(&fk, joined) {
            for (column, ref_column) in fk.columns.iter().zip(&fk.ref_columns) {
                if column.eq_ignore_ascii_case(left_column) {
                    columns.push(ref_column.clone());
                }
            }
        }
    }
    columns
}

fn fk_ref_matches(fk: &ForeignKeyMeta, target: &TableKey) -> bool {
    fk.ref_table.eq_ignore_ascii_case(&target.table)
        && schema_compatible(fk.ref_schema.as_deref(), target.schema.as_deref())
}

/// AST 里的表名通常没有 schema；只有两侧都已知时才比较。
fn schema_compatible(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => true,
    }
}

fn same_table(left: &TableKey, right: &TableKey) -> bool {
    left.table.eq_ignore_ascii_case(&right.table)
        && schema_compatible(left.schema.as_deref(), right.schema.as_deref())
}

fn routine_candidate(routine: &RoutineMeta) -> Candidate {
    let label = match routine.signature.as_deref() {
        Some(signature) if !signature.is_empty() => format!("{}({signature})", routine.name),
        _ => routine.name.clone(),
    };
    let detail = match routine.signature.as_deref() {
        Some(signature) if !signature.is_empty() => format!("function({signature})"),
        _ => "function".to_string(),
    };
    let mut candidate = Candidate::new(
        label,
        format!("{}()", routine.name),
        CandidateKind::Function,
    )
    .with_detail(detail)
    .with_sort_text(format!("{SCORE_FUNCTION}:{}", routine.name));
    if let Some(schema) = &routine.schema {
        candidate = candidate.with_schema(schema.clone());
    }
    candidate
}

/// DML 列位置：排除光标前已经写过的列。
fn unlisted_columns(
    table: &TableKey,
    excluded: &[String],
    ctx: &SlotContext<'_>,
) -> Vec<Candidate> {
    let key = table.with_context(ctx.database, ctx.schema);
    columns_of(&key, ctx, SCORE_COLUMN)
        .into_iter()
        .filter(|candidate| {
            !excluded
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&candidate.label))
        })
        .collect()
}

/// UPDATE 赋值右侧：常量字面量（值枚举需要列值采样，暂不提供）。
fn update_value_candidates(ctx: &SlotContext<'_>) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = ["DEFAULT", "NULL", "TRUE", "FALSE"]
        .into_iter()
        .map(|literal| {
            Candidate::new(literal, literal, CandidateKind::Keyword)
                .with_sort_text(format!("{SCORE_KEYWORD}:{literal}"))
        })
        .collect();
    // 赋值右侧也是表达式：`SET name = COU|` 要能补函数。
    candidates.extend(function_candidates(ctx));
    // 值之后就是 WHERE：`SET name = 1 wh|` 不能什么都不给。
    for keyword in expression_keywords(ExprScope::Set, ctx) {
        candidates.push(
            Candidate::new(keyword, keyword, CandidateKind::Keyword)
                .with_sort_text(format!("{SCORE_KEYWORD}:{keyword}")),
        );
    }
    candidates
}

fn virtual_column_candidates(columns: &[String]) -> Vec<Candidate> {
    columns
        .iter()
        .map(|column| {
            Candidate::new(column.clone(), column.clone(), CandidateKind::Column)
                .with_sort_text(format!("{SCORE_COLUMN}:{column}"))
                .without_identifier_quotes()
        })
        .collect()
}

fn columns_of(key: &TableKey, ctx: &SlotContext<'_>, score: i32) -> Vec<Candidate> {
    ctx.meta
        .columns(key)
        .iter()
        .map(|column| column_candidate(column, key, None, score))
        .collect()
}

/// 只知道列名时构造候选；元数据里能查到就复用完整列信息。
fn named_column(key: &TableKey, name: &str, score: i32, ctx: &SlotContext<'_>) -> Candidate {
    ctx.meta
        .columns(key)
        .into_iter()
        .find(|column| column.name.eq_ignore_ascii_case(name))
        .map(|column| column_candidate(&column, key, None, score))
        .unwrap_or_else(|| {
            Candidate::new(name, name, CandidateKind::Column)
                .with_sort_text(format!("{score}:{name}"))
        })
}

fn relation_name(relation: &RelationSymbol) -> &str {
    relation.alias.as_deref().unwrap_or(&relation.key.table)
}

/// AST 不可用但符号表已有多个关系时，仍按 JOIN 硬限定。
fn join_on_left_from_symbols(ctx: &SlotContext<'_>) -> Vec<Candidate> {
    let relations = &ctx.symbols.relations;
    let Some((joined, lefts)) = relations.split_last() else {
        return expression_candidates(ExprScope::JoinOn, ctx.word.qualifier.as_deref(), ctx);
    };
    if lefts.is_empty() {
        return expression_candidates(ExprScope::JoinOn, ctx.word.qualifier.as_deref(), ctx);
    }
    let left_tables: Vec<TableKey> = lefts
        .iter()
        .map(|relation| relation.key.with_context(ctx.database, ctx.schema))
        .collect();
    let joined = joined.key.with_context(ctx.database, ctx.schema);
    join_on_left_candidates(&joined, &left_tables, ctx)
}

/// 词法兜底的关键字：子句未知，取谓词 + 投影两套合法集的并集。
fn fallback_keywords(ctx: &SlotContext<'_>) -> Vec<&'static str> {
    let mut keywords = expression_keywords(ExprScope::Where, ctx);
    for keyword in expression_keywords(ExprScope::SelectItem, ctx) {
        if !keywords.contains(&keyword) {
            keywords.push(keyword);
        }
    }
    keywords
}

/// 词法兜底：AST 完全不可用时的近似。
fn lexical_candidates(prev_keyword: Option<&str>, ctx: &SlotContext<'_>) -> Vec<Candidate> {
    match prev_keyword {
        Some("FROM") | Some("JOIN") | Some("INTO") | Some("UPDATE") | Some("TABLE") => {
            table_candidates(prev_keyword == Some("JOIN"), ctx)
        }
        Some("ON") => join_on_left_from_symbols(ctx),
        Some("WHERE") | Some("HAVING") => {
            expression_candidates(ExprScope::Where, ctx.word.qualifier.as_deref(), ctx)
        }
        Some("SELECT") | Some("AND") | Some("OR") | Some("SET") | Some("BY") => {
            expression_candidates(ExprScope::SelectItem, ctx.word.qualifier.as_deref(), ctx)
        }
        // 关键字识别不出子句（残句）：只在打字中给关键字，空前缀宁可不弹。
        _ if is_typing(ctx) => fallback_keywords(ctx)
            .into_iter()
            .map(|keyword| {
                Candidate::new(keyword, keyword, CandidateKind::Keyword)
                    .with_sort_text(format!("{SCORE_KEYWORD}:{keyword}"))
            })
            .collect(),
        _ => Vec::new(),
    }
}
