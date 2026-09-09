/// 下一步意图：根据光标前后的 SQL 判断「接下来最可能出现的构造」。
///
/// 这独立于 `CompletionExpectation`（它只描述当前应补哪类对象）。意图驱动
/// expected-token 表达式候选与全局排序，是「先判断意图，再生成候选」的第一步。
/// 只保留在 fluxdb-app，不进入通用编辑器（见 design §6.2）。
// 大部分分支尚未被对象 provider 正式采用（仅 expected-token 覆盖其中一部分），
// 但属于设计 §6.2 的完整意图集合，测试中会构造多数分支，故保留并抑制 dead-code 提示。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NextAction {
    /// 语句头关键字 / snippet。
    StatementKeyword,
    /// SELECT 投影表达式（列 / 函数 / 别名）。
    SelectExpression,
    /// FROM / JOIN 的关系槽位（表 / schema / CTE）。
    FromRelation,
    /// 表后的 alias 或后续子句（JOIN / WHERE ...）。
    RelationAlias,
    /// JOIN 关系槽位。
    JoinRelation,
    /// JOIN 关联条件（ON / USING）或 ON 一侧的列。
    JoinCondition,
    /// 谓词操作符（`=`, `<>`, `IN`, `BETWEEN` ...）。
    PredicateOperator,
    /// 谓词右侧的取值（字面量 / 参数 / 可推断的枚举）。
    PredicateValue,
    /// GROUP BY 表达式。
    GroupByExpression,
    /// ORDER BY 表达式 / 排序方向。
    OrderByExpression,
    /// 函数 / 窗口函数参数位置。
    FunctionArgument,
    /// INSERT 列列表。
    InsertColumn,
    /// INSERT VALUES 取值。
    InsertValue,
    /// UPDATE 赋值（`SET col = <value>`）。
    UpdateAssignment,
    /// 无法确定下一步构造；保守降级，不做激进候选。
    Unknown,
}

/// 下一步意图结果（design §6.2）。只参与 fluxdb-app 内部的候选过滤与排序，
/// 不暴露给通用编辑器。
#[derive(Clone, Debug, PartialEq)]
struct SqlIntent {
    action: NextAction,
    /// 方言 parser 或 fallback scanner 给出的期望关键字 / 符号。
    expected_tokens: Vec<&'static str>,
    /// 0..1；低置信度只做保守候选。
    confidence: f32,
    /// debug / tooltip 可用的解释。
    reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionExpectation {
    /// 需要补 SQL 关键字（例如 SELECT 语句投影结束后的 FROM）。
    Keyword,
    /// SELECT 投影列表结束，优先补 FROM，不查询字段 metadata。
    FromClause,
    /// 需要补表或 schema。
    Table,
    /// 需要补列或表达式中的字段。
    Column,
    /// 需要补函数。
    Function,
    /// 需要补存储过程。
    Procedure,
    /// 需要补触发器。
    Trigger,
}

#[derive(Clone, Debug, PartialEq)]
struct SqlCompletionContext {
    expectation: CompletionExpectation,
    prefix: String,
    qualifier: Option<String>,
    /// 光标前的完整限定路径，不含正在输入的 identifier。
    /// 例如 `u.` 为 [`u`]，`sales.public.` 为 [`sales`, `public`]。
    qualifier_path: Vec<String>,
    replace_start: usize,
    replace_end: usize,
    quoted_identifier: bool,
    suggest_keywords: bool,
    suggest_tables: bool,
    suggest_schemas: bool,
    suggest_columns: bool,
    suggest_functions: bool,
    suggest_procedures: bool,
    suggest_triggers: bool,
    /// ORDER BY / GROUP BY 上下文时是否提升当前 SELECT 别名候选（P2.11）。
    suggest_select_aliases: bool,
    /// 当前语句 SELECT 列表中的显式别名（去重，保持出现顺序）。
    select_aliases: Vec<String>,
    /// 当前查询可见的 CTE 输出列（CTE 名称按小写存储）。
    cte_columns: BTreeMap<String, Vec<String>>,
    /// 当前查询可见的派生表输出列（别名按小写存储）。`(SELECT ...) t` 的 `t.` 走这里。
    derived_columns: BTreeMap<String, Vec<String>>,
    /// 星号展开目标（P2.12）：`t.*` 光标在 `*` 后 → Some(Some(t))，裸 `*` → Some(None)，否则 None。
    star_expansion: Option<Option<String>>,
    /// 下一步意图（P1/T013）：光标前后 SQL 推断出的构造，驱动 expected-token 与全局排序。
    intent: SqlIntent,
    /// JOIN 关键字后且已有引用表时，基于真实外键生成 JOIN 提示（P2.13）。
    suggest_join_keys: bool,
    referenced_tables: Vec<ReferencedTable>,
    /// CREATE TABLE 列定义/表选项上下文，补充 MySQL 数据类型和约束关键字。
    create_table_context: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferencedTable {
    database: Option<String>,
    name: String,
    alias: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletionColumnTarget {
    database: Option<String>,
    table: String,
    /// 表的别名（如有）。P2.10 重复列消歧时优先用别名作为限定前缀。
    alias: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SqlDdlImpact {
    tables: BTreeSet<String>,
    database_wide: bool,
}

/// 当前光标作用域的轻量 symbol table。
///
/// 这不是完整 SQL AST；它只保存补全真正消费的三类符号，避免 CTE、别名和
/// 表引用在同一次按键中被分别扫描。外层 CTE 会在嵌套子查询中继承，局部
/// CTE 同名时覆盖外层定义。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SqlScope {
    referenced_tables: Vec<ReferencedTable>,
    select_aliases: Vec<String>,
    cte_columns: BTreeMap<String, Vec<String>>,
    /// 派生表别名 → 其子查询 SELECT 输出列名（`(subquery) t` → t 的可见列）。
    /// 与 CTE 列同构：限定到该别名时，补全列来自此处而非底层表 metadata。
    derived_columns: BTreeMap<String, Vec<String>>,
}

/// 跨 crate 复用的 SQL 作用域符号快照；不携带 connector 或 UI 状态。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SqlScopeTable {
    pub database: Option<String>,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SqlScopeSymbols {
    pub referenced_tables: Vec<SqlScopeTable>,
    pub select_aliases: Vec<String>,
    pub cte_columns: BTreeMap<String, Vec<String>>,
    /// 派生表别名 → 输出列名（T082）。与 cte_columns 同构。
    pub derived_columns: BTreeMap<String, Vec<String>>,
    /// AST successfully parsed the statement; callers may disable heuristic checks.
    pub ast_parsed: bool,
    pub function_names: Vec<String>,
    pub cast_types: Vec<String>,
    pub qualified_columns: Vec<(String, String)>,
    pub unqualified_columns: Vec<String>,
    pub relation_names: Vec<String>,
}

/// 提取单个 SQL statement 的轻量作用域符号，供补全和语义诊断共用。
pub fn sql_scope_symbols(sql: &str, dialect: DatabaseKind) -> SqlScopeSymbols {
    let provider = sql_completion_dialect(dialect);
    let scope = build_sql_scope(provider, sql, 0, sql);
    let ast_symbols = ast_semantic_symbols(sql, dialect);
    let (ast_parsed, function_names, cast_types, qualified_columns, unqualified_columns, relation_names) =
        ast_symbols
            .map(|symbols| {
                (
                    true,
                    symbols.function_names,
                    symbols.cast_types,
                    symbols.qualified_columns,
                    symbols.unqualified_columns,
                    symbols.relation_names,
                )
            })
            .unwrap_or_default();
    SqlScopeSymbols {
        referenced_tables: scope
            .referenced_tables
            .into_iter()
            .map(|table| SqlScopeTable {
                database: table.database,
                name: table.name,
                alias: table.alias,
            })
            .collect(),
        select_aliases: scope.select_aliases,
        cte_columns: scope.cte_columns,
        derived_columns: scope.derived_columns,
        ast_parsed,
        function_names,
        cast_types,
        qualified_columns,
        unqualified_columns,
        relation_names,
    }
}

#[derive(Default)]
struct AstSemanticSymbols {
    function_names: Vec<String>,
    cast_types: Vec<String>,
    qualified_columns: Vec<(String, String)>,
    unqualified_columns: Vec<String>,
    relation_names: Vec<String>,
}

fn ast_semantic_symbols(sql: &str, dialect: DatabaseKind) -> Option<AstSemanticSymbols> {
    let statements = match dialect {
        DatabaseKind::MySql | DatabaseKind::TiDb => Parser::parse_sql(&MySqlDialect {}, sql).ok()?,
        _ => Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, sql).ok()?,
    };
    let mut symbols = AstSemanticSymbols::default();
    let _ = visit_expressions(&statements, |expr| {
        match expr {
            Expr::Function(function) => {
                if let Some(name) = object_name_text(&function.name) {
                    symbols.function_names.push(name.to_ascii_lowercase());
                }
            }
            Expr::Cast { data_type, .. } => {
                symbols.cast_types.push(data_type.to_string().to_ascii_lowercase());
            }
            Expr::CompoundIdentifier(parts) if parts.len() >= 2 => {
                if let (Some(qualifier), Some(column)) = (parts.first(), parts.last()) {
                    symbols
                        .qualified_columns
                        .push((qualifier.value.to_ascii_lowercase(), column.value.to_ascii_lowercase()));
                }
            }
            Expr::Identifier(identifier) => {
                symbols
                    .unqualified_columns
                    .push(identifier.value.to_ascii_lowercase());
            }
            _ => {}
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    let _ = visit_relations(&statements, |relation| {
        if let Some(name) = object_name_text(relation) {
            symbols.relation_names.push(name.to_ascii_lowercase());
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    symbols.function_names.sort_unstable();
    symbols.function_names.dedup();
    symbols.cast_types.sort_unstable();
    symbols.cast_types.dedup();
    symbols.qualified_columns.sort_unstable();
    symbols.qualified_columns.dedup();
    symbols.unqualified_columns.sort_unstable();
    symbols.unqualified_columns.dedup();
    symbols.relation_names.sort_unstable();
    symbols.relation_names.dedup();
    Some(symbols)
}

fn object_name_text(name: &ObjectName) -> Option<String> {
    name.0.iter().rev().find_map(|part| match part {
        ObjectNamePart::Identifier(identifier) => Some(identifier.value.clone()),
        ObjectNamePart::Function(_) => None,
    })
}

const COMPLETION_CONTEXT_WINDOW_BYTES: usize = 64 * 1024;

fn sql_completion_context(sql: &str, cursor: usize, dialect: DatabaseKind) -> SqlCompletionContext {
    let dialect = sql_completion_dialect(dialect);
    let cursor = cursor.min(sql.len());
    // 补全只需要当前 statement 和光标前的局部语境。窗口上限避免 1MB 文档每次按键
    // 都复制/扫描全文；最近的分号仍作为语句边界，保证多 statement 语义不串线。
    let context_start = completion_context_start(sql, cursor);
    let context_end = completion_context_end(sql, cursor);
    let context_sql = &sql[context_start..context_end];
    let before = &context_sql[..cursor - context_start];
    let trailing = trailing_identifier(before);
    let replace_start = context_start + trailing.start;
    let replace_end = if trailing.quoted_identifier
        && sql.as_bytes().get(cursor).is_some_and(|byte| *byte == b'`')
    {
        cursor + 1
    } else {
        cursor
    };
    let prefix = trailing.prefix;
    let qualifier = trailing.qualifier;
    let qualifier_path = trailing.qualifier_path;
    let (scope_start, scope_end) = current_sql_scope_range(context_sql, cursor - context_start);
    let scope = &context_sql[scope_start..scope_end];
    let before_token = context_sql[scope_start..trailing.start.min(scope_end)].trim_end();
    let before_lower = before_token.to_ascii_lowercase();
    let scope_model = build_sql_scope(dialect, context_sql, scope_start, scope);
    let referenced_tables = scope_model.referenced_tables;
    let from_clause_context = is_projection_from_context(&before_lower);
    let create_table_context = is_create_table_context(&before_lower);
    let suggest_procedures = !from_clause_context
        && dialect.supports_procedures()
        && is_call_context(&before_lower);
    let suggest_triggers = !from_clause_context
        && dialect.supports_triggers()
        && is_trigger_context(&before_lower);
    let qualifier_matches_table = qualifier.as_ref().is_some_and(|qualifier| {
        referenced_tables.iter().any(|table| {
            table.name.eq_ignore_ascii_case(qualifier)
                || table
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
        })
    });
    let suggest_tables = !suggest_procedures
        && !suggest_triggers
        && !from_clause_context
        && (is_table_context(&before_lower) || (qualifier.is_some() && !qualifier_matches_table));
    let suggest_columns = !from_clause_context
        && (qualifier_matches_table
        || is_column_context(&before_lower)
        || (!referenced_tables.is_empty()
            && (is_insert_column_list_context(&before_lower) || is_update_set_context(&before_lower))));
    let has_qualifier = qualifier.is_some();
    let suggest_functions =
        !from_clause_context
            && !suggest_procedures
            && !suggest_triggers
            && !suggest_tables
            && !has_qualifier;
    let suggest_keywords =
        !suggest_procedures && !suggest_triggers && !suggest_tables && !has_qualifier;
    let expectation = if from_clause_context {
        CompletionExpectation::FromClause
    } else if suggest_procedures {
        CompletionExpectation::Procedure
    } else if suggest_triggers {
        CompletionExpectation::Trigger
    } else if suggest_tables {
        CompletionExpectation::Table
    } else if suggest_columns {
        CompletionExpectation::Column
    } else if suggest_functions {
        CompletionExpectation::Function
    } else {
        CompletionExpectation::Keyword
    };

    // P2.11：提取当前 SELECT 别名，仅在 ORDER BY / GROUP BY / HAVING 上下文提升为候选
    // （T042：方言允许处，别名在排序/聚合过滤子句可用）。
    let select_aliases = scope_model.select_aliases;
    let suggest_select_aliases = is_alias_boost_context(&before_lower) && !select_aliases.is_empty();
    let cte_columns = scope_model.cte_columns;
    let derived_columns = scope_model.derived_columns;

    // P2.12：星号展开（`SELECT *` / `SELECT t.*`），仅当有引用表可提供列时启用。
    let star_expansion = star_expansion_target(before).filter(|_| !referenced_tables.is_empty());

    // P2.13：仅对真实外键生成高优先级 JOIN 提示；需已有关联表且刚输入 JOIN 关键字。
    let suggest_join_keys = is_join_context(&before_lower) && !referenced_tables.is_empty();

    // 下一步意图（T013）：独立于 assertion 的对象选择，描述「接下来最可能出现的构造」。
    // 对象上下文（列/表/函数等）仍由现有 provider 处理，这里只覆盖能稳定判断的「下一步」。
    let intent = infer_sql_intent(&before_lower, qualifier.is_some(), &referenced_tables);

    SqlCompletionContext {
        expectation,
        prefix,
        qualifier,
        qualifier_path,
        replace_start,
        replace_end,
        quoted_identifier: trailing.quoted_identifier,
        suggest_keywords,
        suggest_tables,
        // 仅在表上下文且无 qualifier（尚未进入某 schema）时给出 schema 候选；
        // 一旦带 qualifier（如 `FROM db.`）视为已限定 schema，不再重复建议。
        suggest_schemas: suggest_tables && !has_qualifier,
        suggest_columns,
        suggest_functions,
        suggest_procedures,
        suggest_triggers,
        suggest_select_aliases,
        select_aliases,
        cte_columns,
        derived_columns,
        star_expansion,
        intent,
        suggest_join_keys,
        referenced_tables,
        create_table_context,
    }
}

/// 推断下一步意图（T013）。规则只覆盖能稳定判断的「下一步构造」；无法判断时降级为
/// `Unknown`，交给现有对象 provider 保守处理，不制造激进候选。
fn infer_sql_intent(
    before_lower: &str,
    has_qualifier: bool,
    referenced_tables: &[ReferencedTable],
) -> SqlIntent {
    // 限定列（`o.`）之后：JOIN ON 右侧 / 任意 qualified 列。
    if has_qualifier {
        return SqlIntent {
            action: NextAction::JoinCondition,
            expected_tokens: Vec::new(),
            confidence: 0.9,
            reason: "qualified column: prefer target table columns",
        };
    }

    // 谓词取值：`WHERE age = |` / `> |` / `LIKE |` / `IN (` 之后。
    if predicate_value_context(before_lower) {
        return SqlIntent {
            action: NextAction::PredicateValue,
            expected_tokens: vec!["NULL", "TRUE", "FALSE"],
            confidence: 0.9,
            reason: "predicate RHS: value / parameter",
        };
    }

    // 谓词操作符：`WHERE age |` 或 `AND age |`，上一个是列引用且尚未消费操作符。
    if predicate_operator_context(before_lower) {
        return SqlIntent {
            action: NextAction::PredicateOperator,
            expected_tokens: vec![
                "=", "<>", "!=", ">", "<", ">=", "<=", "IN", "BETWEEN", "LIKE", "IS NULL",
                "IS NOT NULL",
            ],
            confidence: 0.85,
            reason: "predicate after column: operator",
        };
    }

    // 表后：`FROM users |` / `JOIN orders |` → alias 或后续子句。
    if relation_alias_context(before_lower, referenced_tables) {
        return SqlIntent {
            action: NextAction::RelationAlias,
            expected_tokens: vec!["AS", "JOIN", "WHERE", "GROUP BY", "ORDER BY", "LIMIT"],
            confidence: 0.8,
            reason: "after relation: alias / next clause",
        };
    }

    // ORDER BY / GROUP BY 方向与表达式：只要光标位于 order/group 列表内（其后未进入
    // 其它子句）即判定，`ORDER BY age |` 也命中，而不仅是 `ORDER BY |`。
    if let Some(order_group) = order_group_intent(before_lower) {
        return SqlIntent {
            action: order_group,
            expected_tokens: vec!["ASC", "DESC"],
            confidence: 0.8,
            reason: "order/group by: expression or direction",
        };
    }

    // INSERT VALUES 取值。
    if insert_value_context(before_lower) {
        return SqlIntent {
            action: NextAction::InsertValue,
            expected_tokens: vec!["NULL", "DEFAULT"],
            confidence: 0.8,
            reason: "INSERT VALUES: value",
        };
    }

    // INSERT 列列表：`INSERT INTO t (|`，尚未进入 VALUES。
    if insert_column_context(before_lower) {
        return SqlIntent {
            action: NextAction::InsertColumn,
            expected_tokens: Vec::new(),
            confidence: 0.8,
            reason: "INSERT column list: target columns",
        };
    }

    // UPDATE 赋值列列表：`UPDATE t SET |` 或 `SET col1, |`。
    if update_assignment_context(before_lower) {
        return SqlIntent {
            action: NextAction::UpdateAssignment,
            expected_tokens: vec![",", "="],
            confidence: 0.8,
            reason: "UPDATE SET: assignment columns",
        };
    }

    // 语句头 / SELECT 投影等对象上下文：保持 Unknown，由现有 provider 处理。
    SqlIntent {
        action: NextAction::Unknown,
        expected_tokens: Vec::new(),
        confidence: 0.5,
        reason: "unknown / object context: conservative",
    }
}

/// `WHERE col = |` / `> |` / `LIKE |` / `IN (` / `IS [NOT] |` 之后 → 取值。
fn predicate_value_context(before_lower: &str) -> bool {
    let trimmed = before_lower.trim_end();
    ["=", "<>", "!=", ">", "<", ">=", "<=", " like", " in ", " between ", " in(",
     " is", " is not"]
        .iter()
        .any(|marker| trimmed.ends_with(marker))
}

/// 提取比较/取值上下文左操作数列（`on a.x = |` / `where price >= |`）→ 左列的
/// （限定符, 列名）。T063 用它解析已知比较列类型以构建类型感知提升。
fn comparison_left_operand(before: &str) -> Option<(Option<String>, String)> {
    let trimmed = before.trim_end();
    let bytes = trimmed.as_bytes();
    // 逐字符回溯找最后一个比较操作符（= < >，含 != <= >= 变体）。
    let mut op_end = None;
    let mut pos = trimmed.len();
    while pos > 0 {
        let ch = trimmed[..pos].chars().next_back().unwrap();
        let char_len = ch.len_utf8();
        if matches!(ch, '=' | '<' | '>') {
            op_end = Some(pos - char_len);
            break;
        }
        pos -= char_len;
    }
    let op_end = op_end?;
    // 操作符可能多字符（`>=`/`<=`/`!=`）：从找到的操作符字符向左扩展完整操作符区间，
    // 作为左操作数切片的右边界。
    let mut op_start = op_end;
    while op_start > 0 && matches!(bytes[op_start - 1], b'=' | b'<' | b'>' | b'!') {
        op_start -= 1;
    }
    // 操作符左侧可间隔空白（`col = op` 而非 `col=op`）。跳过空白后回溯连续标识符。
    let mut name_end = op_start;
    while name_end > 0 && bytes[name_end - 1].is_ascii_whitespace() {
        name_end -= 1;
    }
    while name_end > 0 && is_sql_ident_char(bytes[name_end - 1] as char) {
        name_end -= 1;
    }
    if name_end == op_start {
        return None;
    }
    let name = trimmed[name_end..op_start].trim_end();
    // 纯数字左操作数（如 `WHERE 5 = |`）不是列引用，不参与类型提升。
    if name.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let mut qualifier = None;
    if name_end > 0 && bytes[name_end - 1] == b'.' {
        let q_end = name_end - 1;
        let mut q_start = q_end;
        while q_start > 0 && is_sql_ident_char(bytes[q_start - 1] as char) {
            q_start -= 1;
        }
        if q_start < q_end {
            qualifier = Some(trimmed[q_start..q_end].to_string());
        }
    }
    Some((qualifier, name.to_string()))
}

/// `WHERE col |`（col 之后尚未消费操作符）→ 操作符。
/// `WHERE col |`（col 之后尚未消费操作符）→ 操作符。
/// 保守策略：末 token 必须是「明显不是关键字」的标识符，且其前缀是 WHERE/AND/OR/ON/HAVING 开口。
/// 这样 `FROM users |` 不会误判为操作符（末尾是表名但前缀是 FROM），`WHERE age |` 则正确命中。
fn predicate_operator_context(before_lower: &str) -> bool {
    let trimmed = before_lower.trim_end();
    // 尾部是操作符或取值关键字时已交由 PredicateValue 处理。
    if predicate_value_context(before_lower) {
        return false;
    }
    // 取最后一个 identifier 作为候选列；去掉它后的前缀必须是某个比较开口。
    let Some(column_end) = trimmed.rfind(|ch: char| is_sql_ident_char(ch)) else {
        return false;
    };
    let mut column_start = column_end;
    for (idx, ch) in trimmed[..=column_end].char_indices().rev() {
        if is_sql_ident_char(ch) {
            column_start = idx;
        } else {
            break;
        }
    }
    let column = &trimmed[column_start..=column_end];
    if column.is_empty()
        || SQL_COMPLETION_KEYWORDS
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(column))
    {
        return false;
    }
    let prefix = trimmed[..column_start].trim_end();
    // 比较开口必须是 WHERE/HAVING/ON/AND/OR 之一，排除 FROM/JOIN/UPDATE/INTO 等关系开口。
    ["where", "and", "or", "on", "having"]
        .iter()
        .any(|kw| prefix.ends_with(kw) || prefix.ends_with(&format!("{kw} ")))
}

/// 表/视图/CTE 之后（`FROM users |`）→ alias 或后续子句。
fn relation_alias_context(before_lower: &str, referenced_tables: &[ReferencedTable]) -> bool {
    let trimmed = before_lower.trim_end();
    let Some(column_end) = trimmed.rfind(|ch: char| is_sql_ident_char(ch)) else {
        return false;
    };
    let mut column_start = column_end;
    for (idx, ch) in trimmed[..=column_end].char_indices().rev() {
        if is_sql_ident_char(ch) {
            column_start = idx;
        } else {
            break;
        }
    }
    let last = &trimmed[column_start..=column_end];
    // 末 token 必须恰好是当前作用域的一张表/视图/CTE 名。
    if !referenced_tables
        .iter()
        .any(|table| table.name.eq_ignore_ascii_case(last))
    {
        return false;
    }
    // 其前面必须是 FROM/JOIN/UPDATE/INTO 关系开口，且后接 alias 的不是 `,`（多表）。
    let prefix = trimmed[..column_start].trim_end();
    ["from", "join", "update", "into", "delete from"]
        .iter()
        .any(|kw| prefix.ends_with(kw) || prefix.ends_with(&format!("{kw} ")))
}

/// `INSERT INTO t (...) VALUES (` 内 → 取值。
fn insert_value_context(before_lower: &str) -> bool {
    if !starts_with_sql_keyword(before_lower, "insert") {
        return false;
    }
    let Some(values_start) = find_top_level_sql_keyword(before_lower, "values") else {
        return false;
    };
    let after_values = &before_lower[values_start + "values".len()..];
    after_values.trim_start().starts_with('(')
}

/// `INSERT INTO t (|` 内、未进入 VALUES → 列列表。
fn insert_column_context(before_lower: &str) -> bool {
    if !starts_with_sql_keyword(before_lower, "insert") {
        return false;
    }
    // 已进入 VALUES 阶段则不是列列表（交由 insert_value_context 判定）。
    if find_top_level_sql_keyword(before_lower, "values").is_some() {
        return false;
    }
    // 存在未闭合左括号：光标处于列列表或 VALUES 括号内。
    let mut depth = 0usize;
    for ch in before_lower.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth > 0
}

/// `UPDATE t SET |` 或 `SET col1, |` → 赋值列列表。
fn update_assignment_context(before_lower: &str) -> bool {
    if !starts_with_sql_keyword(before_lower, "update") {
        return false;
    }
    let Some(set_start) = find_top_level_sql_keyword(before_lower, "set") else {
        return false;
    };
    let tail = before_lower[set_start + "set".len()..].trim_start();
    // 已进入取值（`col = |`，由 predicate_value_context 判定）或其它子句则不判定为列列表。
    if tail.contains('=') {
        return false;
    }
    !["where", "order by", "limit", "returning"]
        .iter()
        .any(|kw| find_top_level_sql_keyword(tail, kw).is_some())
}

fn build_sql_scope(
    dialect: &dyn SqlCompletionDialect,
    context_sql: &str,
    scope_start: usize,
    scope: &str,
) -> SqlScope {
    let mut cte_columns = BTreeMap::new();

    // 嵌套查询只传入自身文本时会丢掉外层 WITH。外层定义先放入 map，
    // 再放入当前 scope 的定义，天然实现 SQL 的 shadowing 规则。
    if scope_start > 0 {
        if let Some(with_start) = find_top_level_sql_keyword(&context_sql[..scope_start], "with") {
            cte_columns.extend(extract_cte_columns_fallback(&context_sql[with_start..]));
        }
    }
    cte_columns.extend(extract_cte_columns_fallback(scope));

    let mut referenced_tables = dialect.referenced_tables(scope);
    // sqlparser 的 FROM 收集不会进入 EXISTS/IN 等表达式子查询；只补入
    // fallback 中明确指向已知 CTE 的别名，避免把普通启发式表扫描带回热路径。
    for table in extract_referenced_tables(scope) {
        if table.alias.is_some()
            && cte_columns.contains_key(&table.name.to_ascii_lowercase())
            && !referenced_tables.iter().any(|known| {
                known.name.eq_ignore_ascii_case(&table.name)
                    && known.alias.as_deref() == table.alias.as_deref()
            })
        {
            referenced_tables.push(table);
        }
    }

    // T082：解析当前 scope 内的派生表输出列（`(subquery) t`）。派生列是局部符号，
    // 只从当前 scope 提取，不继承外层（外层派生表在本行后已失效，避免污染）。
    let derived_columns = extract_derived_columns(scope, dialect);

    SqlScope {
        referenced_tables,
        select_aliases: extract_select_aliases(scope),
        cte_columns,
        derived_columns,
    }
}

fn completion_context_start(sql: &str, cursor: usize) -> usize {
    let window_start = cursor.saturating_sub(COMPLETION_CONTEXT_WINDOW_BYTES);
    let window = &sql[window_start..cursor];
    let statement_start = sql_statement_ranges(window)
        .iter()
        .rev()
        .find_map(|(end, separator_len)| {
            (*separator_len > 0).then_some(window_start + end + separator_len)
        })
        .unwrap_or(window_start);
    statement_start.min(cursor)
}

fn completion_context_end(sql: &str, cursor: usize) -> usize {
    let cursor = cursor.min(sql.len());
    let window_end = cursor.saturating_add(COMPLETION_CONTEXT_WINDOW_BYTES).min(sql.len());
    let window = &sql[cursor..window_end];
    let separator_end = sql_statement_ranges(window)
        .into_iter()
        .find_map(|(end, separator_len)| {
            (separator_len > 0).then_some(cursor + end + separator_len)
        });
    separator_end.unwrap_or(window_end)
}

/// 检测星号展开目标：光标紧跟在 `*` 之后时，`t.*` → Some(Some(t))，裸 `*` → Some(None)；否则 None。
/// 候选由当前已加载列去重生成，仅提供显式 snippet，不自动改写用户文本（P2.12）。
fn star_expansion_target(before: &str) -> Option<Option<String>> {
    let trimmed = before.trim_end();
    if !trimmed.ends_with('*') {
        return None;
    }
    let before_star = trimmed[..trimmed.len() - 1].trim_end();
    // 限定星号（`o.` / `sales.orders.`）：取最后一个限定段作为目标。
    if before_star.ends_with('.') {
        let qual_part = &before_star[..before_star.len() - 1];
        let last_seg = qual_part
            .split(|ch: char| ch.is_whitespace())
            .last()
            .unwrap_or("");
        if !last_seg.is_empty() && last_seg.chars().all(|ch| is_sql_ident_char(ch) || ch == '.') {
            return Some(Some(last_seg.to_string()));
        }
        return Some(None);
    }
    Some(None)
}

fn is_order_group_context(before_lower: &str) -> bool {
    ["order by", "group by"]
        .iter()
        .any(|keyword| {
            before_lower.ends_with(keyword) || before_lower.ends_with(&format!("{keyword} "))
        })
}

/// SELECT 别名可见的排序/聚合过滤子句（ORDER BY / GROUP BY / HAVING）起点（T042）。
fn is_alias_boost_context(before_lower: &str) -> bool {
    if is_order_group_context(before_lower) {
        return true;
    }
    let trimmed = before_lower.trim_end();
    trimmed.ends_with("having") || trimmed.ends_with("having ")
}

/// 检测是否刚输入 JOIN 关键字（`JOIN` / `LEFT JOIN` 等），用于 FK JOIN 提示（P2.13）。
fn is_join_context(before_lower: &str) -> bool {
    let before_lower = before_lower.trim_end();
    [
        "join",
        "left join",
        "right join",
        "inner join",
        "outer join",
        "cross join",
        "full join",
        "full outer join",
    ]
    .iter()
    .any(|keyword| before_lower.ends_with(keyword))
}

/// 判断光标是否位于 ORDER BY / GROUP BY 列表内，并给出对应意图。
/// 比 `is_order_group_context`（仅 `order by ` 结尾）更通用：光标后已有表达式
/// （`ORDER BY age |`)也命中；若其后已进入其它子句（LIMIT/UNION 等）则不再判定。
fn order_group_intent(before_lower: &str) -> Option<NextAction> {
    let clauses = ["order by", "group by"];
    let actions = [NextAction::OrderByExpression, NextAction::GroupByExpression];

    // 找到 within `before_lower` 顶层最后一次出现的 order/group by 位置。
    let mut found: Option<(usize, NextAction)> = None;
    for (keyword, action) in clauses.iter().zip(actions) {
        let mut search_from = 0;
        while let Some(index) = find_top_level_sql_keyword(&before_lower[search_from..], keyword) {
            found = Some((search_from + index, action));
            search_from += index + keyword.len();
        }
    }
    let (start, action) = found?;

    // 其后若已是其它子句开头则不在 order/group 列表内（避免把 `ORDER BY x LIMIT |` 判成表达式）。
    let tail = &before_lower[start + "order by".len()..];
    let continued_clause = ["limit", "offset", "union", "intersect", "having", "set", "values"]
        .iter()
        .any(|kw| find_top_level_sql_keyword(tail, kw).is_some());
    if continued_clause {
        return None;
    }
    Some(action)
}

/// 提取 scope 中最后一个顶层 SELECT 列表里的显式别名（`expr AS alias` 或 MySQL 尾随裸别名）。
/// 仅供 ORDER BY / GROUP BY 上下文提升别名候选（P2.11），失败时返回空列表，保持保守。
fn extract_select_aliases(scope: &str) -> Vec<String> {
    let Some(select_start) = last_select_keyword(scope) else {
        return Vec::new();
    };
    let select_list = &scope[select_start + "select".len()..];
    // 选择列表在上层 (FROM/WHERE/GROUP/ORDER/HAVING/LIMIT/JOIN/UNION) 关键字前结束。
    let end = ["from", "where", "group", "order", "having", "limit", "join", "union"]
        .iter()
        .filter_map(|keyword| find_top_level_sql_keyword(select_list, keyword))
        .min()
        .unwrap_or(select_list.len());
    let list = &select_list[..end];

    let mut aliases = Vec::new();
    let mut seen = BTreeSet::new();
    for item in split_select_items(list) {
        if let Some(alias) = select_item_alias(item) {
            let key = alias.to_ascii_lowercase();
            if seen.insert(key) {
                aliases.push(alias);
            }
        }
    }
    aliases
}

/// scope 中最后一个顶层 `select` 的位置；`WITH` 场景取最外层主查询的 SELECT。
fn last_select_keyword(scope: &str) -> Option<usize> {
    let mut last = None;
    let mut search_from = 0usize;
    while let Some(relative) = find_top_level_sql_keyword(&scope[search_from..], "select") {
        let index = search_from + relative;
        last = Some(index);
        search_from = index + "select".len();
    }
    last
}

/// 在顶层（括号、字符串、注释之外）按英文逗号切分 SELECT 列表项。
fn split_select_items(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut chars = list.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\'' | '"' | '`' => skip_quoted_sql(&mut chars, ch),
            '-' if matches!(chars.peek(), Some((_, '-'))) => skip_line_comment(&mut chars),
            '#' => skip_line_comment(&mut chars),
            '/' if matches!(chars.peek(), Some((_, '*'))) => skip_block_comment(&mut chars),
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&list[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&list[start..]);
    parts
}

/// 单个 SELECT 项的别名：`AS alias` 优先；否则若表达式不是单纯列引用则取 MySQL 尾随裸标识符。
fn select_item_alias(item: &str) -> Option<String> {
    if let Some(relative) = find_top_level_sql_keyword(item, "as") {
        let after = item[relative + "as".len()..].trim();
        return trailing_single_identifier(after);
    }
    let tokens: Vec<&str> = item.split_whitespace().collect();
    // 单纯一个标识符（如 `name`、`o.customer_id`）视为普通列引用而非别名。
    if tokens.len() < 2 {
        return None;
    }
    let last = tokens.last()?;
    last.chars()
        .all(is_sql_ident_char)
        .then(|| last.to_string())
}

fn trailing_single_identifier(s: &str) -> Option<String> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.len() != 1 {
        return None;
    }
    normalize_sql_identifier(tokens[0])
}

fn normalize_sql_identifier(token: &str) -> Option<String> {
    let token = token.trim();
    let bytes = token.as_bytes();
    if bytes.len() >= 2
        && matches!((bytes[0], bytes[bytes.len() - 1]), (b'`', b'`') | (b'"', b'"'))
    {
        let quote = bytes[0] as char;
        return Some(
            token[1..token.len() - 1]
                .replace(&format!("{quote}{quote}"), &quote.to_string()),
        );
    }
    token.chars().all(is_sql_ident_char).then(|| token.to_string())
}

fn read_sql_identifier_at(text: &str, start: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let first = *bytes.get(start)?;
    if matches!(first, b'`' | b'"') {
        let mut index = start + 1;
        while index < bytes.len() {
            if bytes[index] == first {
                if bytes.get(index + 1) == Some(&first) {
                    index += 2;
                    continue;
                }
                let raw = &text[start..=index];
                return normalize_sql_identifier(raw).map(|name| (name, index + 1));
            }
            index += 1;
        }
        return None;
    }
    if !(first.is_ascii_alphanumeric() || matches!(first, b'_' | b'$')) {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'$'))
    {
        end += 1;
    }
    Some((text[start..end].to_ascii_lowercase(), end))
}

/// 提升 ORDER BY / GROUP BY 上下文中的 SELECT 别名候选（P2.11）。
fn select_alias_completion_items(aliases: &[String], prefix: &str) -> Vec<QueryCompletionItem> {
    aliases
        .iter()
        .filter(|alias| matches_completion_prefix(alias, prefix))
        .map(|alias| QueryCompletionItem {
            label: alias.clone(),
            insert_text: alias.clone(),
            kind: QueryCompletionKind::Column,
            detail: Some("select alias".to_string()),
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
})
        .collect()
}

/// 基于真实外键生成 JOIN snippet 候选（P2.13）。仅处理真实 FK，不做名称启发式。
/// `root_prefix` 为 FROM 侧表的限定前缀（别名或表名），owner 列取自外键列。
fn fk_join_completion_items(
    root_prefix: &str,
    foreign_keys: &[ForeignKeyInfo],
    kind: DatabaseKind,
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    if foreign_keys.is_empty() {
        return Vec::new();
    }
    // 引号按「裸保留字 ∪ 短语关键字」判定：SQL_COMPLETION_KEYWORDS 多为短语
    // （ORDER BY/GROUP BY），须补 SQL_JOIN_STOP_WORDS 里的裸保留字（order/group/on 等），
    // 否则名为 order/group 的表或别名不会加反引号（T060）。
    let reserved = |word: &str| {
        SQL_COMPLETION_KEYWORDS
            .iter()
            .chain(SQL_JOIN_STOP_WORDS.iter())
            .any(|keyword| keyword.eq_ignore_ascii_case(word))
    };
    let quote = |word: &str| quote_identifier(word, kind, &reserved);
    let root = quote(root_prefix);

    let mut grouped: BTreeMap<String, Vec<&ForeignKeyInfo>> = BTreeMap::new();
    for fk in foreign_keys {
        grouped.entry(fk.name.clone()).or_default().push(fk);
    }

    let mut items = Vec::new();
    for fks in grouped.into_values() {
        let fk = fks[0];
        let ref_table = match &fk.ref_schema {
            Some(schema) if !schema.is_empty() => {
                format!("{}.{}", quote(schema), quote(&fk.ref_table))
            }
            _ => quote(&fk.ref_table),
        };
        let on_clause = fks
            .iter()
            .map(|fk| {
                format!(
                    "{root}.{} = {ref_table}.{}",
                    quote(&fk.column),
                    quote(&fk.ref_column)
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        let label = format!("JOIN {ref_table} ON {on_clause}");
        items.push(QueryCompletionItem {
            label,
            insert_text: format!("JOIN {ref_table} ON {on_clause}"),
            kind: QueryCompletionKind::Snippet,
            detail: Some(format!("外键关联 {on_clause}")),
            documentation: None,
            filter_text: Some(fk.ref_table.clone()),
            sort_text: None,
            insert_text_format: InsertTextFormat::PlainText,
        });
    }
    // 过滤：以参考表名匹配前缀。
    items.retain(|item| {
        item.filter_text
            .as_deref()
            .is_some_and(|text| matches_completion_prefix(text, prefix))
    });
    items
}

trait SqlCompletionDialect: Sync {
    fn referenced_tables(&self, sql: &str) -> Vec<ReferencedTable>;

    fn keywords(&self) -> &'static [&'static str] {
        SQL_COMPLETION_KEYWORDS
    }

    fn functions(&self) -> &'static [&'static str] {
        SQL_COMPLETION_FUNCTIONS
    }

    fn supports_procedures(&self) -> bool {
        false
    }

    fn supports_triggers(&self) -> bool {
        false
    }
}

struct MySqlCompletionDialect;
struct SqliteCompletionDialect;
struct GenericCompletionDialect;

impl SqlCompletionDialect for MySqlCompletionDialect {
    fn referenced_tables(&self, sql: &str) -> Vec<ReferencedTable> {
        referenced_tables_from_statements(sql, true)
    }

    fn supports_procedures(&self) -> bool {
        true
    }

    fn supports_triggers(&self) -> bool {
        true
    }
}

impl SqlCompletionDialect for SqliteCompletionDialect {
    fn referenced_tables(&self, sql: &str) -> Vec<ReferencedTable> {
        referenced_tables_from_statements(sql, false)
    }

    fn keywords(&self) -> &'static [&'static str] {
        SQLITE_COMPLETION_KEYWORDS
    }

    fn functions(&self) -> &'static [&'static str] {
        SQLITE_COMPLETION_FUNCTIONS
    }

    fn supports_triggers(&self) -> bool {
        true
    }
}

impl SqlCompletionDialect for GenericCompletionDialect {
    fn referenced_tables(&self, sql: &str) -> Vec<ReferencedTable> {
        referenced_tables_from_statements(sql, false)
    }
}

/// T083：统一的 referenced table 提取——优先走尾部容忍的 AST 解析收集真实关系表；
/// 仅在解析失败（连尾部裁剪都无法产出语句）时才回退到纯 token scanner。
/// 避免 `where t.` 这类光标不完整输入让整句 AST 失败、从而回退回含 `select alias=id` 噪声的扫描器。
fn referenced_tables_from_statements(
    sql: &str,
    mysql: bool,
) -> Vec<ReferencedTable> {
    if let Some(statements) = parse_statements_tolerant(sql, mysql) {
        let mut tables = Vec::new();
        for statement in &statements {
            collect_statement_tables(statement, &mut tables);
        }
        if !tables.is_empty() {
            return dedupe_referenced_tables(tables);
        }
    }
    extract_referenced_tables(sql)
}

static MYSQL_COMPLETION_DIALECT: MySqlCompletionDialect = MySqlCompletionDialect;
static SQLITE_COMPLETION_DIALECT: SqliteCompletionDialect = SqliteCompletionDialect;
static GENERIC_COMPLETION_DIALECT: GenericCompletionDialect = GenericCompletionDialect;

fn sql_completion_dialect(dialect: DatabaseKind) -> &'static dyn SqlCompletionDialect {
    match dialect {
        DatabaseKind::MySql | DatabaseKind::TiDb => &MYSQL_COMPLETION_DIALECT,
        DatabaseKind::Sqlite => &SQLITE_COMPLETION_DIALECT,
        DatabaseKind::MongoDb | DatabaseKind::Redis => &GENERIC_COMPLETION_DIALECT,
    }
}

fn collect_statement_tables(statement: &Statement, tables: &mut Vec<ReferencedTable>) {
    let cte_sources = BTreeMap::new();
    match statement {
        Statement::Query(query) => collect_query_tables(query, tables),
        Statement::Insert(insert) => {
            match &insert.table {
                TableObject::TableName(name) => {
                    if let Some(table) = referenced_table_from_object_name(name, None) {
                        tables.push(table);
                    }
                }
                TableObject::TableQuery(query) => collect_query_tables(query, tables),
                TableObject::TableFunction(_) => {}
            }
            if let Some(source) = &insert.source {
                collect_query_tables(source, tables);
            }
        }
        Statement::Update(update) => {
            collect_table_with_joins(&update.table, tables, &cte_sources);
            if let Some(from) = &update.from {
                match from {
                    UpdateTableFromKind::BeforeSet(from) | UpdateTableFromKind::AfterSet(from) => {
                        collect_table_with_joins_list(from, tables, &cte_sources)
                    }
                }
            }
        }
        Statement::Delete(delete) => {
            match &delete.from {
                FromTable::WithFromKeyword(from) | FromTable::WithoutKeyword(from) => {
                    collect_table_with_joins_list(from, tables, &cte_sources)
                }
            }
            if let Some(using) = &delete.using {
                collect_table_with_joins_list(using, tables, &cte_sources);
            }
        }
        Statement::Directory { source, .. } => collect_query_tables(source, tables),
        _ => {}
    }
}

fn collect_query_tables(query: &SqlAstQuery, tables: &mut Vec<ReferencedTable>) {
    collect_query_tables_with_ctes(query, tables, &BTreeMap::new());
}

fn extract_cte_columns_fallback(sql: &str) -> BTreeMap<String, Vec<String>> {
    let trimmed = sql.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("with ") && lower != "with" {
        return BTreeMap::new();
    }
    let bytes = trimmed.as_bytes();
    let mut position = 4usize;
    let mut result = BTreeMap::new();
    loop {
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        let Some((name, name_end)) = read_sql_identifier_at(trimmed, position) else {
            break;
        };
        position = name_end;
        if name.eq_ignore_ascii_case("recursive") {
            continue;
        }
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        let mut output = Vec::new();
        if bytes.get(position) == Some(&b'(') {
            if let Some(close) = find_matching_sql_paren(trimmed, position) {
                output = split_select_items(&trimmed[position + 1..close])
                    .into_iter()
                    .filter_map(|item| trailing_single_identifier(item.trim()))
                    .collect();
                position = close + 1;
            }
        }
        let Some(as_offset) = find_top_level_sql_keyword(&trimmed[position..], "as") else {
            break;
        };
        position += as_offset + 2;
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        if bytes.get(position) != Some(&b'(') {
            break;
        }
        let Some(close) = find_matching_sql_paren(trimmed, position) else { break };
        if output.is_empty() {
            output = infer_cte_select_columns(&trimmed[position + 1..close]);
        }
        if !output.is_empty() {
            result.insert(name, output);
        }
        position = close + 1;
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        if bytes.get(position) != Some(&b',') {
            break;
        }
        position += 1;
    }
    result
}

fn infer_cte_select_columns(sql: &str) -> Vec<String> {
    let Some(select_start) = last_select_keyword(sql) else { return Vec::new() };
    let projection = &sql[select_start + 6..];
    let end = ["from", "where", "group", "order", "having", "limit"]
        .iter()
        .filter_map(|keyword| find_top_level_sql_keyword(projection, keyword))
        .min()
        .unwrap_or(projection.len());
    split_select_items(&projection[..end])
        .into_iter()
        .filter_map(|item| {
            select_item_alias(item).or_else(|| {
                let token = item.trim().split('.').last()?.trim();
                token.chars().all(is_sql_ident_char).then(|| token.to_string())
            })
        })
        .collect()
}

fn find_matching_sql_paren(sql: &str, open: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    for index in open..bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == delimiter {
                quote = None;
            }
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
        } else if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn collect_query_tables_with_ctes(
    query: &SqlAstQuery,
    tables: &mut Vec<ReferencedTable>,
    parent_ctes: &BTreeMap<String, ReferencedTable>,
) {
    let mut cte_sources = parent_ctes.clone();
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            collect_query_tables_with_ctes(&cte.query, tables, parent_ctes);
            if let Some(source) = single_query_source_table(&cte.query) {
                cte_sources.insert(cte.alias.name.value.to_ascii_lowercase(), source);
            }
        }
    }
    collect_set_expr_tables(&query.body, tables, &cte_sources);
}

fn collect_set_expr_tables(
    expr: &SetExpr,
    tables: &mut Vec<ReferencedTable>,
    cte_sources: &BTreeMap<String, ReferencedTable>,
) {
    match expr {
        SetExpr::Select(select) => collect_table_with_joins_list(&select.from, tables, cte_sources),
        SetExpr::Query(query) => collect_query_tables_with_ctes(query, tables, cte_sources),
        SetExpr::SetOperation { left, right, .. } => {
            collect_set_expr_tables(left, tables, cte_sources);
            collect_set_expr_tables(right, tables, cte_sources);
        }
        SetExpr::Insert(statement)
        | SetExpr::Update(statement)
        | SetExpr::Delete(statement)
        | SetExpr::Merge(statement) => collect_statement_tables(statement, tables),
        SetExpr::Values(_) | SetExpr::Table(_) => {}
    }
}

fn collect_table_with_joins_list(
    tables_with_joins: &[TableWithJoins],
    tables: &mut Vec<ReferencedTable>,
    cte_sources: &BTreeMap<String, ReferencedTable>,
) {
    for table_with_joins in tables_with_joins {
        collect_table_with_joins(table_with_joins, tables, cte_sources);
    }
}

fn collect_table_with_joins(
    table_with_joins: &TableWithJoins,
    tables: &mut Vec<ReferencedTable>,
    cte_sources: &BTreeMap<String, ReferencedTable>,
) {
    collect_table_factor(&table_with_joins.relation, tables, cte_sources);
    for join in &table_with_joins.joins {
        collect_table_factor(&join.relation, tables, cte_sources);
    }
}

fn collect_table_factor(
    table_factor: &TableFactor,
    tables: &mut Vec<ReferencedTable>,
    cte_sources: &BTreeMap<String, ReferencedTable>,
) {
    match table_factor {
        TableFactor::Table { name, alias, .. } => {
            let cte_source = object_name_identifier_parts(name)
                .first()
                .filter(|_| name.0.len() == 1)
                .and_then(|name| cte_sources.get(&name.to_ascii_lowercase()));
            if let Some(table) = cte_source
                .cloned()
                .map(|source| referenced_table_with_alias(source, alias.as_ref(), name))
                .or_else(|| referenced_table_from_object_name(name, alias.as_ref()))
            {
                tables.push(table);
            }
        }
        TableFactor::Derived {
            subquery, alias, ..
        } => {
            collect_query_tables_with_ctes(subquery, tables, cte_sources);
            if let Some(alias) = alias
                && let Some(source) = single_query_source_table(subquery)
            {
                tables.push(referenced_table_with_alias(source, Some(alias), &ObjectName(vec![])));
            }
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => collect_table_with_joins(table_with_joins, tables, cte_sources),
        TableFactor::Pivot { table, .. } | TableFactor::Unpivot { table, .. } => {
            collect_table_factor(table, tables, cte_sources)
        }
        _ => {}
    }
}

fn referenced_table_from_object_name(
    name: &ObjectName,
    alias: Option<&TableAlias>,
) -> Option<ReferencedTable> {
    let parts = object_name_identifier_parts(name);
    let name = parts.last()?.clone();
    let database = (parts.len() > 1).then(|| parts[..parts.len() - 1].join("."));
    Some(ReferencedTable {
        database,
        name,
        alias: alias.map(|alias| alias.name.value.clone()),
    })
}

fn referenced_table_with_alias(
    mut table: ReferencedTable,
    alias: Option<&TableAlias>,
    fallback_name: &ObjectName,
) -> ReferencedTable {
    table.alias = alias
        .map(|alias| alias.name.value.clone())
        .or_else(|| object_name_identifier_parts(fallback_name).first().cloned());
    table
}

fn single_query_source_table(query: &SqlAstQuery) -> Option<ReferencedTable> {
    let mut tables = Vec::new();
    collect_query_tables(query, &mut tables);
    let mut unique = BTreeMap::new();
    for mut table in tables {
        table.alias = None;
        unique.insert(
            (
                table.database.as_ref().map(|value| value.to_ascii_lowercase()),
                table.name.to_ascii_lowercase(),
            ),
            table,
        );
    }
    (unique.len() == 1).then(|| unique.into_values().next().unwrap())
}

fn object_name_identifier_parts(name: &ObjectName) -> Vec<String> {
    name.0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(identifier) => Some(identifier.value.clone()),
            ObjectNamePart::Function(_) => None,
        })
        .collect()
}

fn current_sql_statement_range(sql: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(sql.len());
    let mut start = 0;
    for (end, separator_len) in sql_statement_ranges(sql) {
        if cursor <= end + separator_len {
            return (start, end);
        }
        start = end + separator_len;
    }
    (start, sql.len())
}

fn current_sql_scope_range(sql: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(sql.len());
    let (statement_start, statement_end) = current_sql_statement_range(sql, cursor);
    let statement = &sql[statement_start..statement_end];
    let statement_cursor = cursor.saturating_sub(statement_start).min(statement.len());

    for open in sql_open_parentheses_before_cursor(statement, statement_cursor)
        .into_iter()
        .rev()
    {
        let inner_before_cursor = &statement[open + 1..statement_cursor];
        if !starts_with_select_or_with(inner_before_cursor) {
            continue;
        }
        let close = matching_sql_parenthesis(statement, open).unwrap_or(statement_end - statement_start);
        return (statement_start + open + 1, statement_start + close);
    }

    (statement_start, statement_end)
}

fn sql_open_parentheses_before_cursor(sql: &str, cursor: usize) -> Vec<usize> {
    let mut opens = Vec::new();
    let mut chars = sql[..cursor.min(sql.len())].char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\'' | '"' | '`' => skip_quoted_sql(&mut chars, ch),
            '-' if matches!(chars.peek(), Some((_, '-'))) => skip_line_comment(&mut chars),
            '#' => skip_line_comment(&mut chars),
            '/' if matches!(chars.peek(), Some((_, '*'))) => skip_block_comment(&mut chars),
            '(' => opens.push(index),
            ')' => {
                opens.pop();
            }
            _ => {}
        }
    }
    opens
}

fn matching_sql_parenthesis(sql: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = sql[open..].char_indices().peekable();
    while let Some((relative_index, ch)) = chars.next() {
        let index = open + relative_index;
        match ch {
            '\'' | '"' | '`' => skip_quoted_sql(&mut chars, ch),
            '-' if matches!(chars.peek(), Some((_, '-'))) => skip_line_comment(&mut chars),
            '#' => skip_line_comment(&mut chars),
            '/' if matches!(chars.peek(), Some((_, '*'))) => skip_block_comment(&mut chars),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn starts_with_select_or_with(sql: &str) -> bool {
    let sql = sql.trim_start();
    starts_with_sql_keyword(sql, "select") || starts_with_sql_keyword(sql, "with")
}

fn dedupe_referenced_tables(tables: Vec<ReferencedTable>) -> Vec<ReferencedTable> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for table in tables {
        let key = (
            table.database.as_ref().map(|value| value.to_ascii_lowercase()),
            table.name.to_ascii_lowercase(),
            table.alias.as_ref().map(|value| value.to_ascii_lowercase()),
        );
        if seen.insert(key) {
            deduped.push(table);
        }
    }
    deduped
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrailingIdentifier {
    prefix: String,
    qualifier: Option<String>,
    qualifier_path: Vec<String>,
    start: usize,
    quoted_identifier: bool,
}

fn trailing_identifier(before: &str) -> TrailingIdentifier {
    let mut prefix_start = before.len();
    for (index, ch) in before.char_indices().rev() {
        if is_sql_ident_char(ch) {
            prefix_start = index;
        } else {
            break;
        }
    }
    let prefix = before[prefix_start..].to_string();
    let quoted_identifier = prefix_start > 0 && before.as_bytes()[prefix_start - 1] == b'`';
    // 将左反引号纳入替换范围：既能恢复 FROM/JOIN 上下文，也避免采纳候选后留下旧引号。
    let start = prefix_start - usize::from(quoted_identifier);
    let qualifier_path = trailing_qualifier_path(before, start);
    if let Some(qualifier) = qualifier_path.last() {
        return TrailingIdentifier {
            prefix,
            qualifier: Some(qualifier.clone()),
            qualifier_path,
            start,
            quoted_identifier,
        };
    }
    TrailingIdentifier {
        prefix,
        qualifier: None,
        qualifier_path,
        start,
        quoted_identifier,
    }
}

/// 从当前 identifier 起点向左读取连续的 `a.b.c.` 限定路径。
/// 同时识别紧邻点号的反引号/双引号段，避免带空格或保留字的名称丢失 namespace。
fn trailing_qualifier_path(before: &str, start: usize) -> Vec<String> {
    if start == 0 || !before[..start].ends_with('.') {
        return Vec::new();
    }

    let mut cursor = start;
    let mut parts = Vec::new();
    while cursor > 0 && before.as_bytes()[cursor - 1] == b'.' {
        let segment_end = cursor - 1;
        let (segment_start, segment) = if let Some(quote) = before[..segment_end]
            .chars()
            .next_back()
            .filter(|quote| matches!(quote, '`' | '"'))
        {
            let quoted_end = segment_end - quote.len_utf8();
            let quote_byte = quote as u8;
            let bytes = before.as_bytes();
            let mut opening = None;
            let mut search_end = quoted_end;
            while search_end > 0 {
                let candidate = search_end - 1;
                if bytes[candidate] != quote_byte {
                    search_end = candidate;
                    continue;
                }
                if candidate > 0 && bytes[candidate - 1] == quote_byte {
                    search_end = candidate - 1;
                    continue;
                }
                opening = Some(candidate);
                break;
            }
            let Some(opening) = opening else {
                break;
            };
            let value = before[opening + quote.len_utf8()..quoted_end]
                .replace(&format!("{quote}{quote}"), &quote.to_string());
            (opening, value)
        } else {
            let mut segment_start = segment_end;
            for (index, ch) in before[..segment_end].char_indices().rev() {
                if is_sql_ident_char(ch) {
                    segment_start = index;
                } else {
                    break;
                }
            }
            if segment_start == segment_end {
                break;
            }
            (segment_start, before[segment_start..segment_end].to_string())
        };
        if segment.is_empty() {
            break;
        }
        parts.push(segment);
        cursor = segment_start;
    }
    parts.reverse();
    parts
}

fn is_sql_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

/// 光标所处 SQL 函数调用的签名信息（T062）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlCallSignature {
    /// 函数名（schema 限定只取末段）。
    pub name: String,
    /// schema 限定前缀（如 `sales.concat` 的 `sales`）。
    pub qualifier: Option<String>,
    /// 当前正在编辑的参数下标。
    pub active_parameter: usize,
}

/// 推断光标所在 SQL 函数调用（T062）。
///
/// SQL 侧函数名识别独立于通用编辑器：fluxdb-app 不依赖 fluxdb-editor-core，通用编辑器的
/// `signature_at`/`active_parameter_index`（P1.9）用于非 SQL 文件。此处按 SQL 语义
/// 复刻同一算法——识别最内层包围光标的 `func(` 括号并给出 active 参数：
/// - 函数名必须紧邻开括号（`if (` 等控制流不判定为调用）；
/// - 支持嵌套调用（取最内层）与 schema 限定（`schema.func` 取末段名 + 前缀）；
/// - active 参数按括号内深度 0 的逗号计数，忽略字符串/引用字面量内的逗号；
/// - 逆向字符边界扫描与字符串扫描均按 char 边界推进，兼容多字节文本。
pub fn sql_signature_at(query: &str, cursor: usize) -> Option<SqlCallSignature> {
    // 编辑器 offset 始终落在 char 边界；此处保守地把非边界光标回退到前一边界。
    // 显式追加 query.len() 哨兵，避免字符串末尾光标（= len）被误落到最后一个字符起点。
    let cursor = query
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(query.len()))
        .filter(|&i| i <= cursor)
        .last()
        .unwrap_or(0);
    let before = &query[..cursor];

    // 逆向找包围光标的最近（未闭合）开括号。
    let mut depth = 0i32;
    let mut open = None;
    for (i, ch) in before.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    open = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let open = open?;
    let (name, qualifier) = read_call_identifier(before, open)?;
    let inside = &query[open + 1..cursor];
    let active_parameter = sql_active_parameter_index(inside);
    Some(SqlCallSignature {
        name,
        qualifier,
        active_parameter,
    })
}

/// 读开括号左侧紧邻的调用名（含可选 schema 限定）。括号前无紧邻标识符（如 `if (`）返回 None。
fn read_call_identifier(before: &str, open: usize) -> Option<(String, Option<String>)> {
    let bytes = before.as_bytes();
    let mut name_start = open;
    while name_start > 0 && is_sql_ident_char(bytes[name_start - 1] as char) {
        name_start -= 1;
    }
    if name_start == open {
        return None;
    }
    let name = before[name_start..open].to_string();
    let mut qualifier = None;
    if name_start > 0 && bytes[name_start - 1] == b'.' {
        let q_end = name_start - 1;
        let mut q_start = q_end;
        while q_start > 0 && is_sql_ident_char(bytes[q_start - 1] as char) {
            q_start -= 1;
        }
        if q_start < q_end {
            qualifier = Some(before[q_start..q_end].to_string());
        }
    }
    Some((name, qualifier))
}

/// 计算某函数调用括号内当前 active 参数下标：统计深度 0（非嵌套括号）的逗号，
/// 忽略字符串/引用字面量（含转义）内的逗号。空文本或括号内无参数返回 0。
fn sql_active_parameter_index(inside: &str) -> usize {
    let mut depth = 0usize;
    let mut commas = 0usize;
    let mut chars = inside.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '\'' | '"' | '`' => skip_quoted_sql(&mut chars, ch),
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
    }
    commas
}

/// 类型族（T063）：把数据库 `type_name` 归并到少数可比较的族，用于比较/取值上下文的
/// 「兼容类型」适度提升。只做确定性归并，不做完整类型推断（T063 约束）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TypeFamily {
    Integer,
    Numeric,
    Text,
    DateTime,
    Boolean,
    Other,
}

/// 把 `type_name` 归并到类型族；未知/空返回 `Other`。
/// 判定基于关键字子串（大小写不敏感），保持确定性：
/// 数值族、整型族、时间族、文本/二进制族、布尔族。
fn sql_type_family(type_name: &str) -> TypeFamily {
    let lower = type_name.to_ascii_lowercase();
    if lower.contains("int") || lower.contains("serial") {
        TypeFamily::Integer
    } else if lower.contains("dec")
        || lower.contains("numeric")
        || lower.contains("float")
        || lower.contains("real")
        || lower.contains("double")
        || lower.contains("number")
        || lower.contains("money")
    {
        TypeFamily::Numeric
    } else if lower.contains("date") || lower.contains("time") || lower.contains("year") {
        TypeFamily::DateTime
    } else if lower.contains("bool") {
        TypeFamily::Boolean
    } else if lower == "char"
        || lower.contains("char")
        || lower.contains("text")
        || lower.contains("clob")
        || lower.contains("string")
        || lower.contains("binary")
        || lower.contains("blob")
        || lower.contains("bytea")
        || lower.contains("json")
        || lower.contains("uuid")
        || lower.contains("enum")
    {
        TypeFamily::Text
    } else {
        TypeFamily::Other
    }
}

/// 类型兼容（T063）：两个类型族一致即为「兼容类型」。调用方只在已知期望类型时用它
/// 构建 `type_match` 提升；无法判定（历来是 `Other`）时兼容恒 false，不产生偏好。
fn type_family_compatible(expected: TypeFamily, candidate: TypeFamily) -> bool {
    use TypeFamily::*;
    match (expected, candidate) {
        // 数值类互相兼容（INTEGER/NUMERIC 视为同一数值族，避免 int 列不命中 numeric 候选）。
        (Integer, Numeric) | (Numeric, Integer) | (Integer, Integer) | (Numeric, Numeric) => true,
        (a, b) if a == b => true,
        _ => false,
    }
}

/// 把 table context 中的限定路径映射到 completion index 的两级 key。
/// 单段路径沿用历史语义（按 database 查询），两段路径按 `database.schema` 查询。
fn completion_namespace_scope(
    context: &SqlCompletionContext,
    default_database: Option<&str>,
) -> (Option<String>, Option<String>) {
    match context.qualifier_path.as_slice() {
        [] => (default_database.map(str::to_string), None),
        [database] => (Some(database.clone()), None),
        [database, schema, ..] => (Some(database.clone()), Some(schema.clone())),
    }
}

fn is_call_context(before_lower: &str) -> bool {
    before_lower.ends_with("call") || before_lower.ends_with("call ")
}

fn is_trigger_context(before_lower: &str) -> bool {
    before_lower.ends_with("drop trigger") || before_lower.ends_with("drop trigger ")
}

fn is_table_context(before_lower: &str) -> bool {
    [
        "from",
        "join",
        "update",
        "into",
        "delete from",
        "create table",
        "alter table",
        "drop table",
        "truncate table",
    ]
        .iter()
        .any(|keyword| {
            before_lower.ends_with(keyword) || before_lower.ends_with(&format!("{keyword} "))
        })
}

fn is_create_table_context(before_lower: &str) -> bool {
    if !starts_with_sql_keyword(before_lower, "create")
        || find_top_level_sql_keyword(before_lower, "table").is_none()
    {
        return false;
    }
    let mut depth = 0usize;
    for ch in before_lower.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth > 0
}

fn is_column_context(before_lower: &str) -> bool {
    if ["select", "where", "and", "or", "order by", "group by", "having", "on"]
        .iter()
        .any(|keyword| {
            before_lower.ends_with(keyword) || before_lower.ends_with(&format!("{keyword} "))
        })
    {
        return true;
    }

    // 不完整表达式的 parser 可能无法构造 AST；光标紧跟操作符/逗号时仍应进入列候选。
    let trimmed = before_lower.trim_end();
    let mut token_start = trimmed.len();
    for (index, ch) in trimmed.char_indices().rev() {
        if is_sql_ident_char(ch) {
            token_start = index;
        } else {
            break;
        }
    }
    let expression_prefix = trimmed[..token_start].trim_end();
    if expression_prefix.ends_with('*') {
        let before_star = expression_prefix[..expression_prefix.len() - 1].trim_end();
        // SELECT * 后输入的 token 仍处于投影列表，不是乘法表达式的右值。
        // 排除该场景，避免在尚未写 FROM 时扫描整个数据库的列名。
        if before_star.ends_with("select")
            || before_star.ends_with("select distinct")
            || before_star.ends_with(',')
        {
            return false;
        }
    }
    matches!(
        expression_prefix.chars().next_back(),
        Some(',' | '(' | '=' | '+' | '-' | '*' | '/' | '%' | '<' | '>' | '!')
    )
}

/// 判断 SELECT 投影列表结束后的 FROM 槽位。
///
/// 这是 AST 解析失败或把尾部 identifier 解释成 alias 时的光标级补充：
/// `SELECT * F` 应优先给 `FROM`，而 `SELECT price * F` 仍属于表达式列上下文。
fn is_projection_from_context(before_lower: &str) -> bool {
    let Some(select_index) = before_lower.rfind("select") else {
        return false;
    };
    let projection = before_lower[select_index + "select".len()..].trim();
    if projection.is_empty() || projection.contains(" from ") {
        return false;
    }
    let Some(star_index) = projection.rfind('*') else {
        return false;
    };
    let before_star = projection[..star_index].trim_end();
    let after_star = projection[star_index + 1..].trim();
    after_star.is_empty()
        && (before_star.is_empty()
            || before_star.ends_with("distinct")
            || before_star.ends_with(','))
}

fn is_insert_column_list_context(before_lower: &str) -> bool {
    if !starts_with_sql_keyword(before_lower, "insert") {
        return false;
    }
    let Some(into_start) = find_top_level_sql_keyword(before_lower, "into") else {
        return false;
    };
    let after_into = before_lower[into_start + "into".len()..].trim_start();
    if ["values", "select", "set"]
        .iter()
        .any(|keyword| find_top_level_sql_keyword(after_into, keyword).is_some())
    {
        return false;
    }
    let Some(open) = after_into.find('(') else {
        return false;
    };
    !after_into[open + 1..].contains(')')
}

fn is_update_set_context(before_lower: &str) -> bool {
    if !starts_with_sql_keyword(before_lower, "update") {
        return false;
    }
    let Some(set_start) = find_top_level_sql_keyword(before_lower, "set") else {
        return false;
    };
    let after_set = &before_lower[set_start + "set".len()..];
    !["where", "order", "limit"]
        .iter()
        .any(|keyword| find_top_level_sql_keyword(after_set, keyword).is_some())
}

fn extract_referenced_tables(sql: &str) -> Vec<ReferencedTable> {
    let tokens = sql_identifier_tokens(sql);
    let mut tables = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].to_ascii_lowercase();
        let is_table_trigger = matches!(token.as_str(), "from" | "join" | "update" | "into")
            || (token == "delete"
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| next.eq_ignore_ascii_case("from")));
        if !is_table_trigger {
            index += 1;
            continue;
        }
        if token == "delete" {
            index += 1;
        }
        let Some(raw_name) = tokens.get(index + 1).cloned() else {
            break;
        };
        let (database, name) = split_qualified_table_name(&raw_name);
        let alias = match (tokens.get(index + 2), tokens.get(index + 3)) {
            (Some(as_token), Some(alias)) if as_token.eq_ignore_ascii_case("as") => {
                Some(alias.clone())
            }
            (Some(alias), _)
                if !SQL_JOIN_STOP_WORDS
                    .iter()
                    .any(|word| alias.eq_ignore_ascii_case(word)) =>
            {
                Some(alias.clone())
            }
            _ => None,
        };
        tables.push(ReferencedTable {
            database,
            name,
            alias,
        });
        index += 2;
    }
    tables
}

fn sql_identifier_tokens(sql: &str) -> Vec<String> {
    sql.split(|ch: char| !(is_sql_ident_char(ch) || ch == '.'))
        .filter(|token| !token.is_empty())
        .map(|token| token.trim_matches('.'))
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// 提取派生表别名 → 子查询 SELECT 输出列名（T082）。
///
/// 例如 `SELECT ... FROM (SELECT id, name FROM Product) t` → `{"t": ["id", "name"]}`。
/// 输出列名取显式别名（`ExprWithAlias`），否则取裸标识符（`UnnamedExpr(Identifier)`）；
/// 复合表达式（函数、运算等）无法确定列名时丢弃该列，交由底层表 metadata 兜底，避免伪造列。
/// 递归进入子查询自身，使嵌套派生表（derived-of-derived）也被收集。
fn extract_derived_columns(
    scope: &str,
    _dialect: &dyn SqlCompletionDialect,
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // 派生表语法 `(SELECT ...) alias` 在 GenericDialect 与 MySqlDialect 下一致，
    // 此处统一用 GenericDialect 解析即可，避免向 build_sql_scope 再传递 DatabaseKind。
    let Some(statements) = parse_statements_tolerant(scope, false) else {
        return out;
    };
    for statement in &statements {
        collect_statement_derived(statement, &mut out);
    }
    out
}

/// 容忍尾部不完整片段的语句解析（补全场景下光标处常有 `alias.` / 悬空关键字）。
///
/// 逐次剥掉尾部的 identifier / 点号 / 空白，直到 parser 能产出完整语句；
/// 只用于提取「已写完整部分」的符号（派生表来自前面的 FROM），剥掉的只是未完成的尾巴。
/// T083：同一份尾部恢复逻辑同时供派生表提取与 referenced_tables 使用，避免各自散落的字符串回退。
fn parse_statements_tolerant(sql: &str, mysql: bool) -> Option<Vec<Statement>> {
    let mut candidate = sql;
    loop {
        let parsed = if mysql {
            Parser::parse_sql(&MySqlDialect {}, candidate)
        } else {
            Parser::parse_sql(&sqlparser::dialect::GenericDialect {}, candidate)
        };
        if let Ok(statements) = parsed {
            return Some(statements);
        }
        // 去掉末尾一段 identifier / `.` / 空白；若一段都没剥掉（如尾部是不整除的非标识符字符），
        // 则退一格字节，保证循环必定收敛，避免死循环。
        let mut end = candidate.trim_end().len();
        while end > 0 {
            let ch = candidate.as_bytes()[end - 1];
            if (ch as char).is_ascii_alphanumeric() || ch == b'_' || ch == b'$' || ch == b'.' {
                end -= 1;
            } else {
                break;
            }
        }
        if end == candidate.len() {
            end = candidate.trim_end().len().saturating_sub(1);
        }
        // end 可能落在多字节字符中间（UTF-8）→ 回退到最近的字符边界，避免切片 panic。
        // 真实 column comment / 数据可能含 em dash 等多字节字（如 `—`），必须容错。
        candidate = &candidate[..candidate.floor_char_boundary(end)];
        if candidate.is_empty() {
            return None;
        }
    }
}

fn collect_statement_derived(statement: &Statement, out: &mut BTreeMap<String, Vec<String>>) {
    // 顶层 statement 常为 Query；CTE/子查询内的派生表由 collect_query_derived 递归覆盖。
    if let Statement::Query(query) = statement {
        collect_query_derived(query, out);
    }
}

fn collect_query_derived(query: &SqlAstQuery, out: &mut BTreeMap<String, Vec<String>>) {
    collect_derived_set_expr(&query.body, out);
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            collect_query_derived(&cte.query, out);
        }
    }
}

fn collect_derived_set_expr(expr: &SetExpr, out: &mut BTreeMap<String, Vec<String>>) {
    match expr {
        SetExpr::Select(select) => {
            for table_with_joins in &select.from {
                collect_table_factor_derived(&table_with_joins.relation, out);
                for join in &table_with_joins.joins {
                    collect_table_factor_derived(&join.relation, out);
                }
            }
        }
        SetExpr::Query(query) => collect_query_derived(query, out),
        SetExpr::SetOperation { left, right, .. } => {
            collect_derived_set_expr(left, out);
            collect_derived_set_expr(right, out);
        }
        _ => {}
    }
}

fn collect_table_factor_derived(table_factor: &TableFactor, out: &mut BTreeMap<String, Vec<String>>) {
    match table_factor {
        TableFactor::Derived {
            subquery, alias, ..
        } => {
            if let Some(alias) = alias {
                let columns = derived_projection_columns(&subquery.body);
                if !columns.is_empty() {
                    out.insert(alias.name.value.to_ascii_lowercase(), columns);
                }
            }
            collect_query_derived(subquery, out);
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            collect_table_factor_derived(&table_with_joins.relation, out);
            for join in &table_with_joins.joins {
                collect_table_factor_derived(&join.relation, out);
            }
        }
        _ => {}
    }
}

/// 派生表子查询的投影列名列表（保持 SELECT 出现顺序）。
fn derived_projection_columns(body: &SetExpr) -> Vec<String> {
    let SetExpr::Select(select) = body else {
        return Vec::new();
    };
    select
        .projection
        .iter()
        .filter_map(|item| match item {
            sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } => {
                Some(alias.value.clone())
            }
            sqlparser::ast::SelectItem::UnnamedExpr(Expr::Identifier(ident)) => {
                Some(ident.value.clone())
            }
            _ => None,
        })
        .collect()
}

fn split_qualified_table_name(name: &str) -> (Option<String>, String) {
    name.rsplit_once('.')
        .map(|(database, table)| (Some(database.to_string()), table.to_string()))
        .unwrap_or_else(|| (None, name.to_string()))
}

const SQL_JOIN_STOP_WORDS: &[&str] = &[
    "where", "join", "left", "right", "inner", "outer", "full", "cross", "on", "group", "order",
    "limit", "having", "union", "set", "values",
];

fn completion_column_tables(context: &SqlCompletionContext) -> Vec<CompletionColumnTarget> {
    if let Some(qualifier) = &context.qualifier {
        return context
            .referenced_tables
            .iter()
            .filter(|table| {
                table.name.eq_ignore_ascii_case(qualifier)
                    || table
                        .alias
                        .as_deref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
            })
            .map(|table| CompletionColumnTarget {
                database: table.database.clone(),
                table: table.name.clone(),
                alias: table.alias.clone(),
            })
            .collect();
    }

    context
        .referenced_tables
        .iter()
        .map(|table| CompletionColumnTarget {
            database: table.database.clone(),
            table: table.name.clone(),
            alias: table.alias.clone(),
        })
        .collect()
}

/// expected-token 候选（T014）：由下一步意图（T013）驱动，给出「接下来最可能出现的构造」。
/// 复用关键字构建器生成 Keyword 候选（操作符、取值关键字、子句关键字均视为关键字）。
fn expected_token_completion_items(
    intent: &SqlIntent,
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    if intent.expected_tokens.is_empty() {
        return Vec::new();
    }
    keyword_completion_items_for(&intent.expected_tokens, prefix)
}

/// 意图 → 候选类型的相关度打分（T021/T022）：值越大越贴合当前子句。
/// 用于在匹配 tier 相同（尤其空前缀）时，把「当前构造真正需要的对象」排到无关对象前，
/// 例如 WHERE 子句里列 > 表，FROM 子句里表 > 列。
fn intent_kind_relevance(action: NextAction, kind: QueryCompletionKind) -> u8 {
    let relational = matches!(
        kind,
        QueryCompletionKind::Table | QueryCompletionKind::View | QueryCompletionKind::Schema
    );
    let columnar = matches!(kind, QueryCompletionKind::Column | QueryCompletionKind::Function);
    match action {
        // 谓词侧：优先列/取值。
        NextAction::PredicateOperator | NextAction::PredicateValue => {
            if columnar { 3 } else { 1 }
        }
        // 关系侧：优先表/视图/schema。
        NextAction::RelationAlias | NextAction::FromRelation | NextAction::JoinRelation => {
            if relational { 3 } else { 1 }
        }
        // JOIN ON / 限定列：优先列。
        NextAction::JoinCondition => {
            if columnar { 3 } else { 1 }
        }
        // 排序/分组：列、函数、别名等同级优先，表次之。
        NextAction::OrderByExpression | NextAction::GroupByExpression => {
            if columnar { 2 } else if relational { 1 } else { 0 }
        }
        // 插入/赋值列列表：优先列。
        NextAction::InsertColumn | NextAction::UpdateAssignment => {
            if columnar { 3 } else { 1 }
        }
        // 未知构造：类型不设偏好，交给匹配度与稳定序。
        _ => 0,
    }
}

/// 全局排序（T020/T022）：对 App 层合并后的候选做统一重排，而不是各 provider 各自拼接。
/// 排序键（稳定 sort）：预期 token > 匹配 tier（exact>prefix>substring>fuzzy）> 意图相关度 > 类型匹配 > label。
/// - 预期 token（expected）是意图判定结果，恒置于最前。
/// - 相关度维度让空前缀（编辑层不重排）时当前子句需要的对象类型排在前。
/// - 类型匹配（T063）：仅当调用方给出 `type_match`（已知比较左值列类型时匹配同族列）才生效；
///   它只作等回收互相关度之后的次级排序，**永不过滤任何候选**，未给出则恒 false（与旧行为一致）。
/// - 其结果直接作为 `QueryCompletionResult.items` 返回：空前缀路径编辑器不重排，故此排序即最终顺序。
fn globally_rank_completion_items(
    mut items: Vec<QueryCompletionItem>,
    expected: Vec<QueryCompletionItem>,
    intent: &SqlIntent,
    prefix: &str,
    type_match: &dyn Fn(&QueryCompletionItem) -> bool,
    personal_score: &dyn Fn(&QueryCompletionItem) -> i32,
) -> Vec<QueryCompletionItem> {
    use std::cmp::Ordering;

    let expected_labels: std::collections::HashSet<String> = expected
        .iter()
        .map(|item| item.label.to_ascii_lowercase())
        .collect();
    let is_expected =
        |item: &QueryCompletionItem| expected_labels.contains(&item.label.to_ascii_lowercase());
    let match_tier = |item: &QueryCompletionItem| completion_match_rank(&item.label, prefix).1;
    let kind_relevance =
        |item: &QueryCompletionItem| intent_kind_relevance(intent.action, item.kind);
    // 空前缀时默认库 schema 作候选价值最低：置于 Table/View 之后，避免压过目标表
    // （T072 收敛 MRR 缺口「空 Schema 排表前」）。升序项：非 Schema（false）在前。
    let schema_penalty =
        |item: &QueryCompletionItem| prefix.is_empty() && item.kind == QueryCompletionKind::Schema;
    // 类型匹配升序排布：`(a==true,b==false) => Less`，即匹配项排前；全部不匹配时无影响。
    let type_boost = |item: &QueryCompletionItem| type_match(item) as u8;
    // 个性化加分降序排布：recency/frequency 高的候选（同确定性优先级下）排前。
    // 可关闭：关闭时调用方传恒 0 闭包，排序与确定性基线完全一致（T071 验收）。
    let personal = |item: &QueryCompletionItem| personal_score(item);

    // expected 先整体前置，保证意图候选中排在最前，即使与后续 provider 候选同名也以预期优先。
    items.splice(0..0, expected);
    items.sort_by(|a, b| match (is_expected(a), is_expected(b)) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => match_tier(a)
            .cmp(&match_tier(b))
            .then_with(|| kind_relevance(b).cmp(&kind_relevance(a)))
            .then_with(|| schema_penalty(a).cmp(&schema_penalty(b)))
            .then_with(|| type_boost(b).cmp(&type_boost(a)))
            .then_with(|| personal(b).cmp(&personal(a)))
            .then_with(|| a.label.to_ascii_lowercase().cmp(&b.label.to_ascii_lowercase())),
    });
    items
}

/// T071：可关闭的轻量个性化（recency/frequency 加分）。
///
/// 记录用户近期采纳哪些补全候选（仅 label 的匿名统计），在确定性语义分数
/// 之后对其小幅加分。安全：不记录完整 SQL、密码、DSN 或敏感值，只记
/// `label -> (count, last_seen_ts)`；默认 `enabled == false`（关闭），关闭时
/// `score` 恒 0，排序与确定性基线完全一致。
#[derive(Debug)]
pub struct RecencyFrequency {
    enabled: bool,
    /// label -> (采纳次数, 最近采纳时间戳秒)
    counts: std::collections::HashMap<String, (u32, u64)>,
}

impl RecencyFrequency {
    pub fn new() -> Self {
        Self {
            enabled: false,
            counts: std::collections::HashMap::new(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 记录一次候选采纳（仅匿名 label）。返回关闭时（禁用状态下）不记录。
    pub fn record_accept(&mut self, label: &str, ts: u64) {
        if !self.enabled {
            return;
        }
        let entry = self
            .counts
            .entry(label.to_ascii_lowercase())
            .or_insert((0, ts));
        entry.0 += 1;
        entry.1 = ts;
    }

    /// 个性化分数：frequency 为主、recency 衰减系数为次。关闭或未记录时 0。
    /// 设计为有采纳历史的对象获得确定性优先级之上的小幅加分，不改变过滤。
    pub fn score(&self, label: &str) -> i32 {
        if !self.enabled {
            return 0;
        }
        match self.counts.get(&label.to_ascii_lowercase()) {
            Some((count, last)) => {
                // 轻量 recency 加分：60s 内最近采纳的对象额外 +1。
                let now = now_ts();
                let recency = if *last + 60 >= now { 1 } else { 0 };
                i32::try_from(*count).unwrap_or(i32::MAX) + recency
            }
            None => 0,
        }
    }
}

impl Default for RecencyFrequency {
    fn default() -> Self {
        Self::new()
    }
}

/// 秒级时间戳。独立小函数便于单测注入；实际实现取系统时间（Unix 秒）。
/// ponytail: 直接用 SystemTime，测试通过相对 ts 差值断言，不注入时钟。
fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 排序解释摘要（T024）：给出候选 top-N 的（type:label），用于 debug/perf 日志
/// 回答「为什么这个候选排在前」，不输出完整 SQL，避免泄露敏感内容；仅在 debug 层输出。
fn completion_top_candidates(items: &[QueryCompletionItem], limit: usize) -> String {
    let mut out = String::new();
    for (index, item) in items.iter().take(limit).enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{:?}:{}", item.kind, item.label));
    }
    out
}

fn keyword_completion_items_for(
    keywords: &[&str],
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    let mut items = keywords
        .iter()
        .filter(|keyword| matches_completion_fuzzy(keyword, prefix))
        .map(|keyword| QueryCompletionItem {
            label: (*keyword).to_string(),
            insert_text: (*keyword).to_string(),
            kind: QueryCompletionKind::Keyword,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
})
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        completion_match_rank(&left.label, prefix)
            .cmp(&completion_match_rank(&right.label, prefix))
            .then_with(|| {
                left.label
                    .to_ascii_lowercase()
                    .cmp(&right.label.to_ascii_lowercase())
            })
    });
    items
}

#[allow(dead_code)]
fn function_completion_items(prefix: &str) -> Vec<QueryCompletionItem> {
    function_completion_items_for(SQL_COMPLETION_FUNCTIONS, prefix)
}

fn function_completion_items_for(
    functions: &[&str],
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    let mut items = functions
        .iter()
        .filter(|function| matches_completion_fuzzy(function, prefix))
        .map(|function| QueryCompletionItem {
            label: (*function).to_string(),
            insert_text: format!("{function}()"),
            kind: QueryCompletionKind::Function,
            detail: Some("built-in".to_string()),
            documentation: None,
            filter_text: None,
            sort_text: None,
            insert_text_format: InsertTextFormat::PlainText,
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        completion_match_rank(&left.label, prefix)
            .cmp(&completion_match_rank(&right.label, prefix))
            .then_with(|| {
                left.label
                    .to_ascii_lowercase()
                    .cmp(&right.label.to_ascii_lowercase())
            })
    });
    items
}

fn table_completion_items(tables: Vec<CompletionTable>, prefix: &str) -> Vec<QueryCompletionItem> {
    let mut tables = tables
        .into_iter()
        .filter(|table| matches_completion_fuzzy(&table.name, prefix))
        .collect::<Vec<_>>();
    tables.sort_by(|left, right| {
        completion_match_rank(&left.name, prefix)
            .cmp(&completion_match_rank(&right.name, prefix))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
    });

    tables
        .into_iter()
        .map(|table| {
            let kind = if table.kind == ObjectKind::View {
                QueryCompletionKind::View
            } else {
                QueryCompletionKind::Table
            };
            QueryCompletionItem {
                label: table.name.clone(),
                insert_text: table.name,
                kind,
                detail: table
                    .database
                    .or_else(|| Some(completion_kind_label(kind).to_string())),
                documentation: None,
                filter_text: None,
                sort_text: None,
                            ..Default::default()
}
        })
        .collect()
}

/// 将 routines 中指定 kind 的候选生成为补全项（P1.7）。
///
/// 函数无参数 metadata 时插入 `name()`，方便继续输入参数；procedure 插入裸名。
fn routine_completion_items(
    routines: Vec<CompletionRoutine>,
    kind: CompletionRoutineKind,
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    routines
        .into_iter()
        .filter_map(|routine| {
            (routine.kind == kind
                && matches_completion_prefix(&routine.name, prefix))
            .then(|| QueryCompletionItem {
                label: routine.name.clone(),
                insert_text: match kind {
                    CompletionRoutineKind::Function => format!("{}()", routine.name),
                    CompletionRoutineKind::Procedure => routine.name,
                },
                kind: match kind {
                    CompletionRoutineKind::Function => QueryCompletionKind::Function,
                    CompletionRoutineKind::Procedure => QueryCompletionKind::Procedure,
                },
                detail: match kind {
                    CompletionRoutineKind::Function => Some("function".to_string()),
                    CompletionRoutineKind::Procedure => Some("procedure".to_string()),
                },
                documentation: None,
                filter_text: None,
                sort_text: None,
                insert_text_format: InsertTextFormat::PlainText,
            })
        })
        .collect()
}

/// 星号展开（P2.12）的列数上限，避免生成超大候选。
const STAR_EXPANSION_COLUMN_LIMIT: usize = 200;

/// 静态 SQL 片段定义（P1.8）：无外部依赖、无 tabstop 解析，编辑器只做通用文本替换。
struct SqlSnippet {
    /// 触发前缀（小写），拼写该前缀时出现候选。
    trigger: &'static str,
    /// 插入的模板文本（普通文本，无占位符导航）。
    insert: &'static str,
    /// 展示标签。
    label: &'static str,
}

/// 预置静态 SQL 片段。临时占位标识（如 `table`、`col`）为字面文本，由用户手动编辑。
const SQL_SNIPPETS: &[SqlSnippet] = &[
    SqlSnippet {
        trigger: "select",
        insert: "SELECT * FROM ",
        label: "SELECT * FROM",
    },
    SqlSnippet {
        trigger: "insert",
        insert: "INSERT INTO table (col) VALUES (value)",
        label: "INSERT INTO table (col) VALUES (value)",
    },
    SqlSnippet {
        trigger: "update",
        insert: "UPDATE table SET col = value WHERE ",
        label: "UPDATE table SET col = value WHERE",
    },
    SqlSnippet {
        trigger: "delete",
        insert: "DELETE FROM table WHERE ",
        label: "DELETE FROM table WHERE",
    },
    SqlSnippet {
        trigger: "join",
        insert: "JOIN table ON table.id = other.id",
        label: "JOIN table ON table.id = other.id",
    },
    SqlSnippet {
        trigger: "where",
        insert: "WHERE ",
        label: "WHERE",
    },
];

/// 生成静态 SQL 片段候选（P1.8），仅在前缀命中时触发（非空前缀），
/// 避免在语句头部无输入时无差别弹出片段；filter_text 用 trigger 便于前缀匹配。
fn snippet_completion_items(prefix: &str) -> Vec<QueryCompletionItem> {
    if prefix.is_empty() {
        return Vec::new();
    }
    SQL_SNIPPETS
        .iter()
        .filter(|snippet| matches_completion_prefix(snippet.trigger, prefix))
        .map(|snippet| QueryCompletionItem {
            label: snippet.label.to_string(),
            insert_text: snippet.insert.to_string(),
            kind: QueryCompletionKind::Snippet,
            detail: Some("snippet".to_string()),
            documentation: None,
            filter_text: Some(snippet.trigger.to_string()),
            sort_text: None,
                    ..Default::default()
})
        .collect()
}

/// 判断标识符是否需要引号包裹（P1.6）：保留字、以数字开头、或含非标识符字符时。
///
/// `reserved` 由调用方提供该方言的保留字判定，避免在纯函数层引入关键字表耦合。
pub fn identifier_needs_quote(name: &str, reserved: impl Fn(&str) -> bool) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return true;
    }
    reserved(name)
}

/// 按方言给标识符加引号并对内部引号做转义（P1.6）。
///
/// 规则：MySQL/TiDB 用反引号并转义反引号；其余（SQLite/PostgreSQL/SQL Server）用双引号并转义双引号。
/// 普通安全标识符（非保留字、无特殊字符）原样返回，避免候选过度带引号。
pub fn quote_identifier(
    name: &str,
    kind: DatabaseKind,
    reserved: impl Fn(&str) -> bool,
) -> String {
    if !identifier_needs_quote(name, &reserved) {
        return name.to_string();
    }
    let (open, close) = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => ('`', '`'),
        _ => ('"', '"'),
    };
    // 方言内转义引号：MySQL 双反引号，标准 SQL 双引号。
    let escaped = if open == '`' {
        name.replace('`', "``")
    } else {
        name.replace('"', "\"\"")
    };
    format!("{open}{escaped}{close}")
}

/// 生成 schema（库/模式）候选，apply 文本为 `schema.`，便于继续输入表名（P1.5）。
///
/// schema 候选取「schema 名优先，其次 database 名」；按匹配优先级 + 字典序稳定排序。
fn schema_completion_items(
    schemas: Vec<(Option<String>, Option<String>)>,
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    let mut items = schemas
        .into_iter()
        .filter_map(|(database, schema)| schema.or(database))
        .filter(|name| matches_completion_fuzzy(name, prefix))
        .map(|name| QueryCompletionItem {
            label: name.clone(),
            insert_text: format!("{name}."),
            kind: QueryCompletionKind::Schema,
            detail: Some("schema".to_string()),
            documentation: None,
            filter_text: None,
            sort_text: None,
            insert_text_format: InsertTextFormat::PlainText,
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        completion_match_rank(&left.label, prefix)
            .cmp(&completion_match_rank(&right.label, prefix))
            .then_with(|| {
                left.label
                    .to_ascii_lowercase()
                    .cmp(&right.label.to_ascii_lowercase())
            })
    });
    items
}

#[allow(dead_code)]
fn column_completion_items(
    columns: Vec<CompletionColumn>,
    prefix: &str,
) -> Vec<QueryCompletionItem> {
    let mut columns = columns
        .into_iter()
        .filter(|column| matches_completion_fuzzy(&column.name, prefix))
        .collect::<Vec<_>>();
    columns.sort_by(|left, right| {
        completion_match_rank(&left.name, prefix)
            .cmp(&completion_match_rank(&right.name, prefix))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| {
                left.table
                    .to_ascii_lowercase()
                    .cmp(&right.table.to_ascii_lowercase())
            })
    });

    columns
        .into_iter()
        .map(|column| {
            let detail = column_completion_detail(&column);
            QueryCompletionItem {
                label: column.name.clone(),
                insert_text: column.name,
                kind: QueryCompletionKind::Column,
                detail,
                documentation: None,
                filter_text: None,
                sort_text: None,
                            ..Default::default()
}
        })
        .collect()
}

fn cte_column_completion_items(
    context: &SqlCompletionContext,
    prefix: &str,
    qualifier: Option<&str>,
) -> Vec<QueryCompletionItem> {
    context
        .cte_columns
        .iter()
        .filter(|(name, _)| qualifier.is_none_or(|qualifier| name.eq_ignore_ascii_case(qualifier)))
        .flat_map(|(cte, columns)| {
            columns.iter().filter_map(move |column| {
                matches_completion_fuzzy(column, prefix).then(|| QueryCompletionItem {
                    label: column.clone(),
                    insert_text: column.clone(),
                    kind: QueryCompletionKind::Column,
                    detail: Some(format!("CTE {cte}")),
                    documentation: None,
                    filter_text: None,
                    sort_text: None,
                    insert_text_format: InsertTextFormat::PlainText,
                })
            })
        })
        .collect()
}

/// T082：派生表别名的输出列候选（`(subquery) t` 的 `t.`）。与 CTE 列同构，
/// 限定到派生别名时命中；未限定（qualifier=None）不扩散，避免把任意子查询列漏给外层。
fn derived_column_completion_items(
    context: &SqlCompletionContext,
    prefix: &str,
    qualifier: Option<&str>,
) -> Vec<QueryCompletionItem> {
    context
        .derived_columns
        .iter()
        .filter(|(name, _)| qualifier.is_some_and(|qualifier| name.eq_ignore_ascii_case(qualifier)))
        .flat_map(|(alias, columns)| {
            columns.iter().filter_map(move |column| {
                matches_completion_fuzzy(column, prefix).then(|| QueryCompletionItem {
                    label: column.clone(),
                    insert_text: column.clone(),
                    kind: QueryCompletionKind::Column,
                    detail: Some(format!("derived {alias}")),
                    documentation: None,
                    filter_text: None,
                    sort_text: None,
                    insert_text_format: InsertTextFormat::PlainText,
                })
            })
        })
        .collect()
}

#[allow(dead_code)]
fn column_completion_detail(column: &CompletionColumn) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(type_name) = column.type_name.as_ref().filter(|value| !value.is_empty()) {
        parts.push(type_name.clone());
    }
    if column.primary_key {
        parts.push("PK".to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn sql_ddl_impact(sql: &str) -> Option<SqlDdlImpact> {
    let tokens = sql_identifier_tokens(sql);
    let mut impact = SqlDdlImpact::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].to_ascii_lowercase();
        match token.as_str() {
            "create" => {
                let Some(kind) = tokens.get(index + 1).map(|value| value.to_ascii_lowercase()) else {
                    impact.database_wide = true;
                    break;
                };
                match kind.as_str() {
                    "table" | "view" | "trigger" => {
                        if let Some(table) = ddl_object_name_after(&tokens, index + 2) {
                            impact.tables.insert(table);
                        } else {
                            impact.database_wide = true;
                        }
                    }
                    "index" | "unique" => {
                        if let Some(on_index) = tokens[index + 1..]
                            .iter()
                            .position(|value| value.eq_ignore_ascii_case("on"))
                            .map(|offset| index + 1 + offset)
                        {
                            if let Some(table) = ddl_object_name_after(&tokens, on_index + 1) {
                                impact.tables.insert(table);
                            } else {
                                impact.database_wide = true;
                            }
                        } else {
                            impact.database_wide = true;
                        }
                    }
                    "database" | "schema" => impact.database_wide = true,
                    _ => impact.database_wide = true,
                }
                index += 1;
            }
            "alter" | "drop" | "truncate" => {
                let next_index = index + 1;
                let next = tokens.get(next_index).map(|value| value.to_ascii_lowercase());
                if matches!(next.as_deref(), Some("database" | "schema")) {
                    impact.database_wide = true;
                } else {
                    let name_index = if matches!(next.as_deref(), Some("table" | "view" | "trigger")) {
                        next_index + 1
                    } else {
                        next_index
                    };
                    if let Some(table) = ddl_object_name_after(&tokens, name_index) {
                        impact.tables.insert(table);
                    } else {
                        impact.database_wide = true;
                    }
                }
                index += 1;
            }
            "rename" => {
                if tokens
                    .get(index + 1)
                    .is_some_and(|value| value.eq_ignore_ascii_case("table"))
                {
                    if let Some(old_table) = ddl_object_name_after(&tokens, index + 2) {
                        impact.tables.insert(old_table);
                    } else {
                        impact.database_wide = true;
                    }
                    if let Some(to_index) = tokens[index + 2..]
                        .iter()
                        .position(|value| value.eq_ignore_ascii_case("to"))
                        .map(|offset| index + 2 + offset)
                    {
                        if let Some(new_table) = ddl_object_name_after(&tokens, to_index + 1) {
                            impact.tables.insert(new_table);
                        }
                    }
                } else {
                    impact.database_wide = true;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    (impact.database_wide || !impact.tables.is_empty()).then_some(impact)
}

fn ddl_object_name_after(tokens: &[String], mut index: usize) -> Option<String> {
    while let Some(token) = tokens.get(index) {
        let lower = token.to_ascii_lowercase();
        if matches!(lower.as_str(), "if" | "not" | "exists" | "temporary") {
            index += 1;
            continue;
        }
        let (_, table) = split_qualified_table_name(token);
        return Some(table.to_ascii_lowercase());
    }
    None
}

fn matches_completion_prefix(value: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || value
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
}

fn matches_completion_fuzzy(value: &str, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }

    let value = value.to_ascii_lowercase();
    let mut value_chars = value.chars();
    filter
        .to_ascii_lowercase()
        .chars()
        .all(|filter_char| value_chars.any(|value_char| value_char == filter_char))
}

fn completion_match_rank(value: &str, filter: &str) -> (u8, usize, usize, usize) {
    let filter = filter.trim().to_ascii_lowercase();
    if filter.is_empty() {
        return (0, 0, value.len(), 0);
    }

    let value_lower = value.to_ascii_lowercase();
    if value_lower == filter {
        return (0, 0, value.len(), 0);
    }
    if value_lower.starts_with(&filter) {
        return (1, 0, value.len(), value.len().saturating_sub(filter.len()));
    }
    if let Some(index) = value_lower.find(&filter) {
        return (
            2,
            index,
            value.len(),
            value.len().saturating_sub(filter.len()),
        );
    }

    let mut positions = Vec::new();
    let mut search_from = 0;
    for filter_char in filter.chars() {
        let Some(relative_index) = value_lower[search_from..]
            .chars()
            .position(|value_char| value_char == filter_char)
        else {
            return (u8::MAX, usize::MAX, value.len(), usize::MAX);
        };
        search_from += relative_index;
        positions.push(search_from);
        search_from += filter_char.len_utf8();
    }

    let first = positions.first().copied().unwrap_or(usize::MAX);
    let span = positions
        .last()
        .zip(positions.first())
        .map(|(last, first)| last.saturating_sub(*first))
        .unwrap_or(usize::MAX);
    (3, first, span, value.len())
}

fn completion_kind_label(kind: QueryCompletionKind) -> &'static str {
    match kind {
        QueryCompletionKind::Keyword => "keyword",
        QueryCompletionKind::Snippet => "snippet",
        QueryCompletionKind::Schema => "schema",
        QueryCompletionKind::Table => "table",
        QueryCompletionKind::View => "view",
        QueryCompletionKind::Column => "column",
        QueryCompletionKind::Function => "function",
        QueryCompletionKind::Procedure => "procedure",
        QueryCompletionKind::Trigger => "trigger",
        QueryCompletionKind::RedisCommand => "command",
        QueryCompletionKind::RedisSubCommand => "subcommand",
        QueryCompletionKind::RedisArgument => "argument",
    }
}

fn dedupe_completion_items(items: Vec<QueryCompletionItem>) -> Vec<QueryCompletionItem> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for item in items {
        // 同名对象可能来自不同数据库/schema；保留 detail 不同的候选，交给 UI 在
        // 发生歧义时显示来源，完全相同的候选仍然去重。
        let key = format!(
            "{:?}:{}:{}",
            item.kind,
            item.label.to_ascii_lowercase(),
            item.detail.as_deref().unwrap_or_default().to_ascii_lowercase()
        );
        if seen.insert(key) {
            deduped.push(item);
        }
    }
    deduped
}

const SQL_COMPLETION_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS NULL",
    "IS NOT NULL",
    "LIKE",
    "BETWEEN",
    "EXISTS",
    "GROUP BY",
    "HAVING",
    "ORDER BY",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "CREATE",
    "ALTER",
    "DROP",
    "TRUNCATE",
    "TABLE",
    "VIEW",
    "INDEX",
    "DATABASE",
    "PRIMARY KEY",
    "FOREIGN KEY",
    "REFERENCES",
    "CALL",
    "SHOW",
    "DESCRIBE",
    "EXPLAIN",
    "DISTINCT",
    "UNION",
    "UNION ALL",
    "WITH",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "IF EXISTS",
    "IF NOT EXISTS",
];

const MYSQL_CREATE_TABLE_KEYWORDS: &[&str] = &[
    "CHAR",
    "VARCHAR",
    "TEXT",
    "TINYINT",
    "SMALLINT",
    "MEDIUMINT",
    "INT",
    "BIGINT",
    "DECIMAL",
    "FLOAT",
    "DOUBLE",
    "BIT",
    "BOOLEAN",
    "DATE",
    "TIME",
    "DATETIME",
    "TIMESTAMP",
    "JSON",
    "BLOB",
    "ENUM",
    "UNSIGNED",
    "ZEROFILL",
    "AUTO_INCREMENT",
    "COMMENT",
    "CHARACTER SET",
    "COLLATE",
    "ENGINE",
];

const SQLITE_COMPLETION_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "LEFT JOIN",
    "INNER JOIN",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS NULL",
    "IS NOT NULL",
    "LIKE",
    "BETWEEN",
    "EXISTS",
    "GROUP BY",
    "HAVING",
    "ORDER BY",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "CREATE",
    "ALTER",
    "DROP",
    "TABLE",
    "VIEW",
    "INDEX",
    "TRIGGER",
    "PRAGMA",
    "EXPLAIN",
    "DISTINCT",
    "UNION",
    "UNION ALL",
    "WITH",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "IF EXISTS",
    "IF NOT EXISTS",
];

const SQL_COMPLETION_FUNCTIONS: &[&str] = &[
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "IFNULL",
    "NULLIF",
    "CONCAT",
    "SUBSTRING",
    "LOWER",
    "UPPER",
    "TRIM",
    "LENGTH",
    "ROUND",
    "NOW",
    "DATE_FORMAT",
    "JSON_EXTRACT",
    "CAST",
];

const SQLITE_COMPLETION_FUNCTIONS: &[&str] = &[
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "TOTAL",
    "COALESCE",
    "IFNULL",
    "NULLIF",
    "LOWER",
    "UPPER",
    "TRIM",
    "LENGTH",
    "SUBSTR",
    "INSTR",
    "REPLACE",
    "ROUND",
    "ABS",
    "RANDOM",
    "DATE",
    "TIME",
    "DATETIME",
    "JULIANDAY",
    "STRFTIME",
    "JSON_EXTRACT",
    "HEX",
    "QUOTE",
    "TYPEOF",
    "CAST",
];
