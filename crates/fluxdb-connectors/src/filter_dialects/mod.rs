//! 数据页筛选的方言适配层。
//!
//! 表单、条件文本和真实查询共用 [`fluxdb_core::FilterExpression`]；本目录只放
//! 各方言差异，不允许 UI 或控制器按数据库类型重复拼 WHERE。

use fluxdb_core::{
    CompiledFilterExpression, CompiledFilterNode, DatabaseKind, FilterCondition, FilterDialect,
    FilterExpression, FilterExpressionError, FilterOperator, FilterValue, ParsedFilterExpression,
};
use sqlparser::ast::{BinaryOperator, DataType, Expr, Ident, UnaryOperator, Value};
use sqlparser::dialect::{
    Dialect as SqlParserDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
};
use sqlparser::parser::Parser;

pub mod dameng;
pub mod mysql;
pub mod oracle;
pub mod postgres;
pub mod sqlite;
pub mod sqlserver;

/// 按数据库类型选择筛选方言。MongoDB/Redis 没有 SQL WHERE 语义，明确返回 `None`。
pub fn filter_dialect(kind: DatabaseKind) -> Option<&'static dyn FilterDialect> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            Some(&mysql::MYSQL_SYNTAX)
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => Some(&postgres::POSTGRES_SYNTAX),
        DatabaseKind::Sqlite => Some(&sqlite::SQLITE_SYNTAX),
        DatabaseKind::SqlServer => Some(&sqlserver::SQLSERVER_SYNTAX),
        // 达梦当前连接器使用 Oracle 风格标识符和 LIMIT/OFFSET；以 PostgreSQL
        // 的表达式语法为基础，差异只在引用与函数层，后续可独立替换实现。
        DatabaseKind::Dameng => Some(&dameng::DAMENG_SYNTAX),
        DatabaseKind::Oracle => Some(&oracle::ORACLE_SYNTAX),
        DatabaseKind::MongoDb | DatabaseKind::Redis => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FilterSyntax {
    Postgres,
    MySql,
    SqlServer,
    Sqlite,
    Dameng,
    Oracle,
}

impl FilterDialect for FilterSyntax {
    fn parse(
        &self,
        text: &str,
    ) -> std::result::Result<ParsedFilterExpression, FilterExpressionError> {
        parse_with_syntax(*self, text)
    }

    fn render(
        &self,
        expression: &FilterExpression,
    ) -> std::result::Result<String, FilterExpressionError> {
        render_expression(expression, *self)
    }

    fn render_with_columns(
        &self,
        expression: &FilterExpression,
        columns: &[fluxdb_core::Column],
    ) -> std::result::Result<String, FilterExpressionError> {
        render_expression_with_columns(expression, *self, columns)
    }

    fn compile(
        &self,
        expression: &FilterExpression,
    ) -> std::result::Result<CompiledFilterExpression, FilterExpressionError> {
        compile_expression(expression, *self)
    }

    fn compile_with_columns(
        &self,
        expression: &FilterExpression,
        columns: &[fluxdb_core::Column],
    ) -> std::result::Result<CompiledFilterExpression, FilterExpressionError> {
        compile_expression_with_columns(expression, *self, columns)
    }
}

/// 把统一表达式编译成连接器可绑定的条件树。参数顺序与树遍历顺序一致。
pub fn compile_filter_tree(
    kind: DatabaseKind,
    expression: &FilterExpression,
    columns: &[fluxdb_core::Column],
) -> Result<Option<CompiledFilterNode>, FilterExpressionError> {
    let Some(syntax) = filter_syntax(kind) else {
        return Ok(None);
    };
    let tree = compile_tree(expression, syntax, columns)?;
    if let CompiledFilterNode::And(items) = &tree
        && items.is_empty()
    {
        return Ok(None);
    }
    Ok(Some(tree))
}

fn filter_syntax(kind: DatabaseKind) -> Option<FilterSyntax> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            Some(FilterSyntax::MySql)
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => Some(FilterSyntax::Postgres),
        DatabaseKind::Sqlite => Some(FilterSyntax::Sqlite),
        DatabaseKind::SqlServer => Some(FilterSyntax::SqlServer),
        DatabaseKind::Dameng => Some(FilterSyntax::Dameng),
        DatabaseKind::Oracle => Some(FilterSyntax::Oracle),
        DatabaseKind::MongoDb | DatabaseKind::Redis => None,
    }
}

fn parser_dialect(syntax: FilterSyntax) -> &'static dyn SqlParserDialect {
    static POSTGRES: PostgreSqlDialect = PostgreSqlDialect {};
    static MYSQL: MySqlDialect = MySqlDialect {};
    static SQLSERVER: MsSqlDialect = MsSqlDialect {};
    static SQLITE: SQLiteDialect = SQLiteDialect {};
    match syntax {
        FilterSyntax::Postgres => &POSTGRES,
        FilterSyntax::MySql => &MYSQL,
        FilterSyntax::SqlServer => &SQLSERVER,
        FilterSyntax::Sqlite => &SQLITE,
        // 达梦沿用 SQL 表达式语义，仅渲染目标类型与标识符规则单独处理。
        FilterSyntax::Dameng => &POSTGRES,
        // Oracle 条件表达式与 PostgreSQL 足够接近（`IS NULL`/`LIKE … ESCAPE`/`BETWEEN`），
        // 差异集中在引用、占位符与假值语义，由渲染层处理。
        FilterSyntax::Oracle => &POSTGRES,
    }
}

fn parse_expression(
    syntax: FilterSyntax,
    text: &str,
) -> Result<FilterExpression, FilterExpressionError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(FilterExpression::And(Vec::new()));
    }
    let trimmed = trimmed.trim_end_matches(';').trim();
    // 条件输入框只接受表达式。显式包一层 SELECT 可复用 sqlparser 的完整
    // WHERE 解析，而不开放任意查询语句。
    let statement = format!("SELECT 1 WHERE {trimmed}");
    let parsed = Parser::parse_sql(parser_dialect(syntax), &statement)
        .map_err(|error| FilterExpressionError::new(format!("筛选条件语法错误：{error}")))?;
    if parsed.len() != 1 {
        return Err(FilterExpressionError::new(
            "筛选条件只能包含一个 SQL 表达式",
        ));
    }
    let Some(sqlparser::ast::Statement::Query(query)) = parsed.into_iter().next() else {
        return Err(FilterExpressionError::new("筛选条件不是 SQL 表达式"));
    };
    let sqlparser::ast::SetExpr::Select(select) = query.body.as_ref() else {
        return Err(FilterExpressionError::new("筛选条件不是 SQL 表达式"));
    };
    let Some(selection) = select.selection.as_ref() else {
        return Err(FilterExpressionError::new("筛选条件为空"));
    };
    expression_from_sql(selection, trimmed.to_string(), syntax)
}

fn expression_from_sql(
    expression: &Expr,
    raw: String,
    syntax: FilterSyntax,
) -> Result<FilterExpression, FilterExpressionError> {
    match expression {
        Expr::Nested(inner) => expression_from_sql(inner, raw, syntax),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => Ok(FilterExpression::And(vec![
                expression_from_sql(left, raw.clone(), syntax)?,
                expression_from_sql(right, raw, syntax)?,
            ])),
            BinaryOperator::Or => Ok(FilterExpression::Or(vec![
                expression_from_sql(left, raw.clone(), syntax)?,
                expression_from_sql(right, raw, syntax)?,
            ])),
            BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq => {
                let Some(field) = field_name(left) else {
                    return Ok(FilterExpression::Raw(raw));
                };
                let Some(value) = literal_value(right) else {
                    return Ok(FilterExpression::Raw(raw));
                };
                let operator = match op {
                    BinaryOperator::Eq => FilterOperator::Eq,
                    BinaryOperator::NotEq => FilterOperator::Ne,
                    BinaryOperator::Lt => FilterOperator::Lt,
                    BinaryOperator::LtEq => FilterOperator::Le,
                    BinaryOperator::Gt => FilterOperator::Gt,
                    BinaryOperator::GtEq => FilterOperator::Ge,
                    _ => unreachable!(),
                };
                // 表单里的「是空的/是非空的」渲染为等值空文本；回填时恢复专用运算符，
                // 避免表单 → 输入 → 表单后语义仍相同但显示成普通等值条件。
                let operator = match (operator, &value) {
                    (FilterOperator::Eq, FilterValue::Text(value)) if value.is_empty() => {
                        FilterOperator::IsEmpty
                    }
                    (FilterOperator::Ne, FilterValue::Text(value)) if value.is_empty() => {
                        FilterOperator::IsNotEmpty
                    }
                    _ => operator,
                };
                let values = if matches!(
                    operator,
                    FilterOperator::IsEmpty | FilterOperator::IsNotEmpty
                ) {
                    Vec::new()
                } else {
                    vec![value]
                };
                Ok(FilterExpression::Condition(FilterCondition {
                    field,
                    op: operator,
                    values,
                }))
            }
            _ => Ok(FilterExpression::Raw(raw)),
        },
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr,
        } => Ok(FilterExpression::Not(Box::new(expression_from_sql(
            expr, raw, syntax,
        )?))),
        Expr::IsNull(inner) => {
            let field = field_name(inner)
                .ok_or_else(|| FilterExpressionError::new("IS NULL 左侧必须是字段"))?;
            Ok(FilterExpression::Condition(FilterCondition {
                field,
                op: FilterOperator::IsNull,
                values: Vec::new(),
            }))
        }
        Expr::IsNotNull(inner) => {
            let field = field_name(inner)
                .ok_or_else(|| FilterExpressionError::new("IS NOT NULL 左侧必须是字段"))?;
            Ok(FilterExpression::Condition(FilterCondition {
                field,
                op: FilterOperator::IsNotNull,
                values: Vec::new(),
            }))
        }
        Expr::Like {
            negated,
            expr,
            pattern,
            escape_char,
            ..
        } => {
            let field = like_field_name(expr)
                .ok_or_else(|| FilterExpressionError::new("LIKE 左侧必须是字段"))?;
            let Some(FilterValue::Text(pattern)) = literal_value(pattern) else {
                return Ok(FilterExpression::Raw(raw));
            };
            let escape = escape_char.as_ref().and_then(single_char_literal);
            let Some((op, value)) = like_operator(*negated, pattern, escape) else {
                return Ok(FilterExpression::Raw(raw));
            };
            Ok(FilterExpression::Condition(FilterCondition {
                field,
                op,
                values: vec![FilterValue::Text(value)],
            }))
        }
        Expr::Between {
            expr,
            negated,
            low,
            high,
        } => {
            let field = field_name(expr)
                .ok_or_else(|| FilterExpressionError::new("BETWEEN 左侧必须是字段"))?;
            let Some(low) = literal_value(low) else {
                return Ok(FilterExpression::Raw(raw));
            };
            let Some(high) = literal_value(high) else {
                return Ok(FilterExpression::Raw(raw));
            };
            Ok(FilterExpression::Condition(FilterCondition {
                field,
                op: if *negated {
                    FilterOperator::NotBetween
                } else {
                    FilterOperator::Between
                },
                values: vec![low, high],
            }))
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let field =
                field_name(expr).ok_or_else(|| FilterExpressionError::new("IN 左侧必须是字段"))?;
            let Some(values) = list.iter().map(literal_value).collect::<Option<Vec<_>>>() else {
                return Ok(FilterExpression::Raw(raw));
            };
            Ok(FilterExpression::Condition(FilterCondition {
                field,
                op: if *negated {
                    FilterOperator::NotInList
                } else {
                    FilterOperator::InList
                },
                values,
            }))
        }
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) => {
            let _ = syntax;
            Ok(FilterExpression::Raw(raw))
        }
        _ => Ok(FilterExpression::Raw(raw)),
    }
}

fn field_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Identifier(Ident { value, .. }) => Some(value.clone()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|ident| ident.value.clone()),
        Expr::Nested(inner) => field_name(inner),
        _ => None,
    }
}

fn like_field_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Cast {
            expr, data_type, ..
        } if is_text_cast_type(data_type) => field_name(expr),
        _ => field_name(expression),
    }
}

fn is_text_cast_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Text
            | DataType::TinyText
            | DataType::MediumText
            | DataType::LongText
            | DataType::String(_)
            | DataType::Char(_)
            | DataType::Character(_)
            | DataType::Varchar(_)
            | DataType::Nvarchar(_)
            | DataType::CharacterVarying(_)
            | DataType::CharVarying(_)
    )
}

fn single_char_literal(value: &sqlparser::ast::ValueWithSpan) -> Option<char> {
    let Value::SingleQuotedString(value) = &value.value else {
        return None;
    };
    let mut chars = value.chars();
    let escape = chars.next()?;
    chars.next().is_none().then_some(escape)
}

fn literal_value(expression: &Expr) -> Option<FilterValue> {
    match expression {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(value)
            | Value::DoubleQuotedString(value)
            | Value::NationalStringLiteral(value) => Some(FilterValue::Text(value.clone())),
            Value::DollarQuotedString(value) => Some(FilterValue::Text(value.value.clone())),
            Value::Number(value, _) => Some(if let Ok(value) = value.parse::<i64>() {
                FilterValue::Integer(value)
            } else {
                FilterValue::Float(value.parse().ok()?)
            }),
            Value::Boolean(value) => Some(FilterValue::Boolean(*value)),
            Value::Null => Some(FilterValue::Null),
            _ => None,
        },
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => match literal_value(expr)? {
            FilterValue::Integer(value) => Some(FilterValue::Integer(-value)),
            FilterValue::Float(value) => Some(FilterValue::Float(-value)),
            _ => None,
        },
        Expr::Nested(inner) => literal_value(inner),
        _ => None,
    }
}

fn like_operator(
    negated: bool,
    pattern: String,
    escape: Option<char>,
) -> Option<(FilterOperator, String)> {
    let starts = pattern.starts_with('%');
    let ends = pattern.len() > 1 && ends_with_unescaped_percent(&pattern, escape);
    let start = usize::from(starts);
    let end = pattern.len().saturating_sub(usize::from(ends));
    if start > end {
        return None;
    }
    let body = &pattern[start..end];
    if has_unescaped_like_wildcard(body, escape) {
        return None;
    }
    let value = unescape_like_literal(body, escape)?;
    Some(match (negated, starts, ends) {
        (false, false, false) => (FilterOperator::Eq, value),
        (false, true, false) => (FilterOperator::EndsWith, value),
        (false, false, true) => (FilterOperator::StartsWith, value),
        (false, true, true) => (FilterOperator::Contains, value),
        (true, false, false) => (FilterOperator::Ne, value),
        (true, true, false) => (FilterOperator::NotEndsWith, value),
        (true, false, true) => (FilterOperator::NotStartsWith, value),
        (true, true, true) => (FilterOperator::NotContains, value),
    })
}

fn ends_with_unescaped_percent(value: &str, escape: Option<char>) -> bool {
    if !value.ends_with('%') {
        return false;
    }
    let Some(escape) = escape else {
        return true;
    };
    let preceding_escapes = value[..value.len() - 1]
        .chars()
        .rev()
        .take_while(|ch| *ch == escape)
        .count();
    preceding_escapes % 2 == 0
}

fn has_unescaped_like_wildcard(value: &str, escape: Option<char>) -> bool {
    let mut escaped = false;
    for ch in value.chars() {
        if escape == Some(ch) && !escaped {
            escaped = true;
            continue;
        }
        if !escaped && matches!(ch, '%' | '_') {
            return true;
        }
        escaped = false;
    }
    escaped
}

fn unescape_like_literal(value: &str, escape: Option<char>) -> Option<String> {
    let Some(escape) = escape else {
        return Some(value.to_string());
    };
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == escape {
            result.push(chars.next()?);
        } else {
            result.push(ch);
        }
    }
    Some(result)
}

fn parse_with_syntax(
    syntax: FilterSyntax,
    text: &str,
) -> Result<ParsedFilterExpression, FilterExpressionError> {
    let expression = parse_expression(syntax, text)?;
    let text = render_expression(&expression, syntax)?;
    Ok(ParsedFilterExpression { expression, text })
}

pub(crate) fn parse_for_syntax(
    syntax: FilterSyntax,
    text: &str,
) -> Result<ParsedFilterExpression, FilterExpressionError> {
    parse_with_syntax(syntax, text)
}

pub(crate) fn render_for_syntax(
    syntax: FilterSyntax,
    expression: &FilterExpression,
) -> Result<String, FilterExpressionError> {
    render_expression(expression, syntax)
}

pub(crate) fn render_for_syntax_with_columns(
    syntax: FilterSyntax,
    expression: &FilterExpression,
    columns: &[fluxdb_core::Column],
) -> Result<String, FilterExpressionError> {
    render_expression_with_columns(expression, syntax, columns)
}

pub(crate) fn compile_for_syntax(
    syntax: FilterSyntax,
    expression: &FilterExpression,
) -> Result<CompiledFilterExpression, FilterExpressionError> {
    compile_expression(expression, syntax)
}

pub(crate) fn compile_for_syntax_with_columns(
    syntax: FilterSyntax,
    expression: &FilterExpression,
    columns: &[fluxdb_core::Column],
) -> Result<CompiledFilterExpression, FilterExpressionError> {
    compile_expression_with_columns(expression, syntax, columns)
}

fn render_expression(
    expression: &FilterExpression,
    syntax: FilterSyntax,
) -> Result<String, FilterExpressionError> {
    render_expression_with_columns(expression, syntax, &[])
}

fn render_expression_with_columns(
    expression: &FilterExpression,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
) -> Result<String, FilterExpressionError> {
    match expression {
        FilterExpression::Condition(condition) => render_condition(condition, syntax, columns),
        FilterExpression::And(items) => Ok(render_group(items, syntax, columns, "AND")),
        FilterExpression::Or(items) => Ok(render_group(items, syntax, columns, "OR")),
        FilterExpression::Not(inner) => Ok(format!(
            "NOT ({})",
            render_expression_with_columns(inner, syntax, columns)?
        )),
        FilterExpression::Raw(raw) => Ok(raw.clone()),
    }
}

fn render_group(
    items: &[FilterExpression],
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
    operator: &str,
) -> String {
    let rendered = items
        .iter()
        .map(|item| render_expression_with_columns(item, syntax, columns).unwrap_or_default())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if rendered.len() <= 1 {
        rendered.into_iter().next().unwrap_or_default()
    } else {
        format!("({})", rendered.join(&format!(" {operator} ")))
    }
}

fn render_condition(
    condition: &FilterCondition,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
) -> Result<String, FilterExpressionError> {
    let field = quote_identifier(&condition.field, syntax);
    let value = |index: usize| {
        condition
            .values
            .get(index)
            .map(|value| render_literal(value, syntax))
            .ok_or_else(|| FilterExpressionError::new("筛选条件缺少必要的比较值"))
    };
    if !condition.op.requires_values() {
        return Ok(match condition.op {
            FilterOperator::IsNull => format!("{field} IS NULL"),
            FilterOperator::IsNotNull => format!("{field} IS NOT NULL"),
            FilterOperator::IsEmpty => empty_condition_sql(&field, syntax, false),
            FilterOperator::IsNotEmpty => empty_condition_sql(&field, syntax, true),
            _ => unreachable!(),
        });
    }
    if condition.values.is_empty() {
        return Err(FilterExpressionError::new("筛选条件缺少必要的比较值"));
    }
    if matches!(
        condition.op,
        FilterOperator::Between | FilterOperator::NotBetween
    ) && condition.values.len() < 2
    {
        return Err(FilterExpressionError::new("介于条件缺少区间终点"));
    }
    Ok(match condition.op {
        FilterOperator::Eq => format!("{field} = {}", value(0)?),
        FilterOperator::Ne => format!("{field} != {}", value(0)?),
        FilterOperator::Lt => format!("{field} < {}", value(0)?),
        FilterOperator::Le => format!("{field} <= {}", value(0)?),
        FilterOperator::Gt => format!("{field} > {}", value(0)?),
        FilterOperator::Ge => format!("{field} >= {}", value(0)?),
        FilterOperator::Contains
        | FilterOperator::NotContains
        | FilterOperator::StartsWith
        | FilterOperator::NotStartsWith
        | FilterOperator::EndsWith
        | FilterOperator::NotEndsWith => {
            let FilterValue::Text(value) = &condition.values[0] else {
                return Err(FilterExpressionError::new("LIKE 需要文本值"));
            };
            let pattern = like_pattern(condition.op, value)
                .ok_or_else(|| FilterExpressionError::new("未知的 LIKE 运算符"))?;
            let escape = like_value_needs_escape(value);
            format!(
                "{}{} LIKE {}{}",
                like_field_sql(&field, &condition.field, syntax, columns),
                if matches!(
                    condition.op,
                    FilterOperator::NotContains
                        | FilterOperator::NotStartsWith
                        | FilterOperator::NotEndsWith
                ) {
                    " NOT"
                } else {
                    ""
                },
                render_literal(&FilterValue::Text(pattern), syntax),
                if escape { LIKE_ESCAPE_CLAUSE } else { "" }
            )
        }
        FilterOperator::IsNull => format!("{field} IS NULL"),
        FilterOperator::IsNotNull => format!("{field} IS NOT NULL"),
        FilterOperator::IsEmpty => empty_condition_sql(&field, syntax, false),
        FilterOperator::IsNotEmpty => empty_condition_sql(&field, syntax, true),
        FilterOperator::Between => format!("{field} BETWEEN {} AND {}", value(0)?, value(1)?),
        FilterOperator::NotBetween => {
            format!("{field} NOT BETWEEN {} AND {}", value(0)?, value(1)?)
        }
        FilterOperator::InList => format!(
            "{field} IN ({})",
            condition
                .values
                .iter()
                .map(|value| render_literal(value, syntax))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        FilterOperator::NotInList => format!(
            "{field} NOT IN ({})",
            condition
                .values
                .iter()
                .map(|value| render_literal(value, syntax))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

fn render_literal(value: &FilterValue, syntax: FilterSyntax) -> String {
    match value {
        FilterValue::Text(value) => format!("'{}'", value.replace('\'', "''")),
        FilterValue::Integer(value) => value.to_string(),
        FilterValue::Float(value) => value.to_string(),
        FilterValue::Boolean(value) => {
            // SQL Server 与 Oracle 都没有可在 WHERE 中直接使用的布尔字面量：
            // Oracle 的 BOOLEAN 只在 PL/SQL 中可用，SQL 层统一按 1/0 表达。
            if matches!(syntax, FilterSyntax::SqlServer | FilterSyntax::Oracle) {
                if *value {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            } else {
                value.to_string()
            }
        }
        FilterValue::Null => "NULL".to_string(),
    }
}

pub(crate) const LIKE_ESCAPE_CLAUSE: &str = " ESCAPE '!'";

/// 构造可安全作为 LIKE 字面量匹配的模式；`!` 由 SQL 的 ESCAPE 子句声明为转义符。
pub(crate) fn like_pattern(operator: FilterOperator, value: &str) -> Option<String> {
    let escaped = escape_like_literal(value);
    Some(match operator {
        FilterOperator::Contains | FilterOperator::NotContains => format!("%{escaped}%"),
        FilterOperator::StartsWith | FilterOperator::NotStartsWith => format!("{escaped}%"),
        FilterOperator::EndsWith | FilterOperator::NotEndsWith => format!("%{escaped}"),
        _ => return None,
    })
}

fn escape_like_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '!' | '%' | '_') {
            escaped.push('!');
        }
        escaped.push(ch);
    }
    escaped
}

pub(crate) fn like_value_needs_escape(value: &str) -> bool {
    value.chars().any(|ch| matches!(ch, '!' | '%' | '_'))
}

fn like_field_sql(
    field: &str,
    field_name: &str,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
) -> String {
    if matches!(syntax, FilterSyntax::MySql | FilterSyntax::Sqlite) {
        return field.to_string();
    }
    let column = columns.iter().find(|column| column.name == field_name);
    if column.is_none_or(filter_column_is_text) {
        return field.to_string();
    }
    let target = match syntax {
        FilterSyntax::Postgres | FilterSyntax::Sqlite => "TEXT",
        FilterSyntax::MySql => "CHAR",
        FilterSyntax::SqlServer => "NVARCHAR(MAX)",
        FilterSyntax::Dameng => "VARCHAR(4000)",
        FilterSyntax::Oracle => "VARCHAR2(4000)",
    };
    format!("CAST({field} AS {target})")
}

pub(crate) fn filter_column_is_text(column: &fluxdb_core::Column) -> bool {
    let Some(type_name) = column.type_name.as_deref() else {
        return false;
    };
    let base = type_name
        .trim()
        .to_ascii_lowercase()
        .split('(')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    matches!(
        base.as_str(),
        "text"
            | "tinytext"
            | "mediumtext"
            | "longtext"
            | "varchar"
            | "varchar2"
            | "nvarchar"
            | "nvarchar2"
            | "char"
            | "nchar"
            | "character"
            | "character varying"
            | "char varying"
            | "bpchar"
            | "name"
            | "citext"
            | "clob"
            | "string"
            | "enum"
            | "set"
    )
}

fn quote_identifier(value: &str, syntax: FilterSyntax) -> String {
    match syntax {
        FilterSyntax::MySql => format!("`{}`", value.replace('`', "``")),
        FilterSyntax::SqlServer => format!("[{}]", value.replace(']', "]]")),
        _ => format!("\"{}\"", value.replace('"', "\"\"")),
    }
}

/// 空字符串字面量。
///
/// Oracle 把长度 0 的字符串当作 NULL，`col = ''` 永远不成立，因此 Oracle 的
/// 「是空的 / 非空的」不走本函数，而由 [`empty_condition_sql`] 渲染成
/// `IS NULL` / `IS NOT NULL`。
fn empty_string_literal(syntax: FilterSyntax) -> &'static str {
    if matches!(syntax, FilterSyntax::SqlServer) {
        "N''"
    } else {
        "''"
    }
}

/// 「是空的 / 是非空的」条件。
///
/// Oracle 无法区分空字符串与 NULL（`''` 即 NULL），只能表达 `IS [NOT] NULL`；
/// 其它方言沿用 `= ''` / `!= ''`。
fn empty_condition_sql(field: &str, syntax: FilterSyntax, negated: bool) -> String {
    match syntax {
        FilterSyntax::Oracle => format!("{field} IS {}NULL", if negated { "NOT " } else { "" }),
        _ => format!(
            "{field} {} {}",
            if negated { "!=" } else { "=" },
            empty_string_literal(syntax)
        ),
    }
}

fn compile_expression(
    expression: &FilterExpression,
    syntax: FilterSyntax,
) -> Result<CompiledFilterExpression, FilterExpressionError> {
    compile_expression_with_columns(expression, syntax, &[])
}

fn compile_expression_with_columns(
    expression: &FilterExpression,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
) -> Result<CompiledFilterExpression, FilterExpressionError> {
    let mut parameters = Vec::new();
    let sql = compile_sql(expression, syntax, columns, &mut parameters)?;
    Ok(CompiledFilterExpression { sql, parameters })
}

fn compile_sql(
    expression: &FilterExpression,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
    parameters: &mut Vec<FilterValue>,
) -> Result<String, FilterExpressionError> {
    match expression {
        FilterExpression::Condition(condition) => {
            compile_condition(condition, syntax, columns, parameters)
        }
        FilterExpression::And(items) => {
            let sql = items
                .iter()
                .map(|item| compile_sql(item, syntax, columns, parameters))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>();
            Ok(if sql.len() <= 1 {
                sql.into_iter().next().unwrap_or_default()
            } else {
                format!("({})", sql.join(" AND "))
            })
        }
        FilterExpression::Or(items) => {
            let sql = items
                .iter()
                .map(|item| compile_sql(item, syntax, columns, parameters))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>();
            Ok(if sql.len() <= 1 {
                sql.into_iter().next().unwrap_or_default()
            } else {
                format!("({})", sql.join(" OR "))
            })
        }
        FilterExpression::Not(inner) => Ok(format!(
            "NOT ({})",
            compile_sql(inner, syntax, columns, parameters)?
        )),
        FilterExpression::Raw(raw) => Ok(raw.clone()),
    }
}

fn compile_tree(
    expression: &FilterExpression,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
) -> Result<CompiledFilterNode, FilterExpressionError> {
    match expression {
        FilterExpression::Condition(condition) => {
            let mut parameters = Vec::new();
            let sql = compile_condition(condition, syntax, columns, &mut parameters)?;
            Ok(CompiledFilterNode::Condition { sql, parameters })
        }
        FilterExpression::And(items) => Ok(CompiledFilterNode::And(
            items
                .iter()
                .map(|item| compile_tree(item, syntax, columns))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        FilterExpression::Or(items) => Ok(CompiledFilterNode::Or(
            items
                .iter()
                .map(|item| compile_tree(item, syntax, columns))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        FilterExpression::Not(inner) => Ok(CompiledFilterNode::Not(Box::new(compile_tree(
            inner, syntax, columns,
        )?))),
        FilterExpression::Raw(raw) => Ok(CompiledFilterNode::Condition {
            sql: raw.clone(),
            parameters: Vec::new(),
        }),
    }
}

fn compile_condition(
    condition: &FilterCondition,
    syntax: FilterSyntax,
    columns: &[fluxdb_core::Column],
    parameters: &mut Vec<FilterValue>,
) -> Result<String, FilterExpressionError> {
    let field = quote_identifier(&condition.field, syntax);
    let value = |index: usize| {
        condition
            .values
            .get(index)
            .cloned()
            .ok_or_else(|| FilterExpressionError::new("筛选条件缺少必要的比较值"))
    };
    let bind = |parameters: &mut Vec<FilterValue>, value: FilterValue| {
        parameters.push(value);
        match syntax {
            FilterSyntax::Postgres => format!("${}", parameters.len()),
            FilterSyntax::SqlServer => format!("@P{}", parameters.len()),
            // Oracle 只接受 `:N` 位置绑定，且同一个 `:N` 出现在多处时驱动会要求
            // 提供同等数量的绑定值；因此这里按出现顺序生成唯一的 `:1, :2, …`。
            FilterSyntax::Oracle => format!(":{}", parameters.len()),
            _ => "?".to_string(),
        }
    };
    if !condition.op.requires_values() {
        return Ok(match condition.op {
            FilterOperator::IsNull => format!("{field} IS NULL"),
            FilterOperator::IsNotNull => format!("{field} IS NOT NULL"),
            FilterOperator::IsEmpty => empty_condition_sql(&field, syntax, false),
            FilterOperator::IsNotEmpty => empty_condition_sql(&field, syntax, true),
            _ => unreachable!(),
        });
    }
    if condition.values.is_empty() {
        return Err(FilterExpressionError::new("筛选条件缺少必要的比较值"));
    }
    if matches!(
        condition.op,
        FilterOperator::Between | FilterOperator::NotBetween
    ) && condition.values.len() < 2
    {
        return Err(FilterExpressionError::new("介于条件缺少区间终点"));
    }
    Ok(match condition.op {
        FilterOperator::Eq => format!("{field} = {}", bind(parameters, value(0)?)),
        FilterOperator::Ne => format!("{field} != {}", bind(parameters, value(0)?)),
        FilterOperator::Lt => format!("{field} < {}", bind(parameters, value(0)?)),
        FilterOperator::Le => format!("{field} <= {}", bind(parameters, value(0)?)),
        FilterOperator::Gt => format!("{field} > {}", bind(parameters, value(0)?)),
        FilterOperator::Ge => format!("{field} >= {}", bind(parameters, value(0)?)),
        FilterOperator::Contains
        | FilterOperator::NotContains
        | FilterOperator::StartsWith
        | FilterOperator::NotStartsWith
        | FilterOperator::EndsWith
        | FilterOperator::NotEndsWith => {
            let negated = matches!(
                condition.op,
                FilterOperator::NotContains
                    | FilterOperator::NotStartsWith
                    | FilterOperator::NotEndsWith
            );
            let FilterValue::Text(value) = value(0)? else {
                return Err(FilterExpressionError::new("LIKE 需要文本值"));
            };
            let pattern = like_pattern(condition.op, &value)
                .ok_or_else(|| FilterExpressionError::new("未知的 LIKE 运算符"))?;
            let escape = like_value_needs_escape(&value);
            let placeholder = bind(parameters, FilterValue::Text(pattern));
            format!(
                "{}{} LIKE {placeholder}{}",
                like_field_sql(&field, &condition.field, syntax, columns),
                if negated { " NOT" } else { "" },
                if escape { LIKE_ESCAPE_CLAUSE } else { "" }
            )
        }
        FilterOperator::IsNull => format!("{field} IS NULL"),
        FilterOperator::IsNotNull => format!("{field} IS NOT NULL"),
        FilterOperator::IsEmpty => empty_condition_sql(&field, syntax, false),
        FilterOperator::IsNotEmpty => empty_condition_sql(&field, syntax, true),
        FilterOperator::Between | FilterOperator::NotBetween => {
            let first = bind(parameters, value(0)?);
            let second = bind(parameters, value(1)?);
            format!(
                "{field}{} BETWEEN {first} AND {second}",
                if condition.op == FilterOperator::NotBetween {
                    " NOT"
                } else {
                    ""
                }
            )
        }
        FilterOperator::InList | FilterOperator::NotInList => {
            let values = condition
                .values
                .iter()
                .map(|value| bind(parameters, value.clone()))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{field}{} IN ({values})",
                if condition.op == FilterOperator::NotInList {
                    " NOT"
                } else {
                    ""
                }
            )
        }
    })
}

#[cfg(test)]
mod filter_dialect_tests {
    use super::*;

    #[test]
    fn postgres_parses_and_or_precedence_into_a_shared_tree() {
        let parsed = postgres::POSTGRES_SYNTAX
            .parse("status = 'failed' OR amount > 10 AND name LIKE 'A%'")
            .expect("valid filter");
        let FilterExpression::Or(items) = parsed.expression else {
            panic!("OR must be the root");
        };
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], FilterExpression::Condition(_)));
        assert!(matches!(&items[1], FilterExpression::And(nodes) if nodes.len() == 2));
    }

    #[test]
    fn each_dialect_quotes_fields_and_compiles_bound_parameters() {
        let expression =
            FilterExpression::And(vec![FilterExpression::Condition(FilterCondition {
                field: "status".to_string(),
                op: FilterOperator::Eq,
                values: vec![FilterValue::Text("failed".to_string())],
            })]);
        let mysql = mysql::MYSQL_SYNTAX.render(&expression).unwrap();
        let postgres = postgres::POSTGRES_SYNTAX.render(&expression).unwrap();
        let sqlserver = sqlserver::SQLSERVER_SYNTAX.render(&expression).unwrap();
        assert_eq!(mysql, "`status` = 'failed'");
        assert_eq!(postgres, "\"status\" = 'failed'");
        assert_eq!(sqlserver, "[status] = 'failed'");

        let compiled = postgres::POSTGRES_SYNTAX.compile(&expression).unwrap();
        assert_eq!(compiled.sql, "\"status\" = $1");
        assert_eq!(
            compiled.parameters,
            vec![FilterValue::Text("failed".into())]
        );
    }

    #[test]
    fn unsupported_expression_is_preserved_as_raw_dialect_text() {
        let parsed = postgres::POSTGRES_SYNTAX
            .parse("lower(name) = 'abc'")
            .expect("parse expression");
        assert!(matches!(parsed.expression, FilterExpression::Raw(_)));
        let rendered = postgres::POSTGRES_SYNTAX
            .render(&parsed.expression)
            .expect("render raw");
        assert!(rendered.to_ascii_lowercase().contains("lower(name)"));
    }

    #[test]
    fn parse_rejects_multiple_statements() {
        let error = postgres::POSTGRES_SYNTAX
            .parse("status = 'ok'; DELETE FROM users")
            .expect_err("multiple statements must be rejected");
        assert!(error.to_string().contains("只能包含一个"));
    }

    #[test]
    fn semicolon_inside_literal_is_not_a_statement_separator() {
        let parsed = postgres::POSTGRES_SYNTAX
            .parse("status = 'a;b';")
            .expect("literal semicolon");
        assert_eq!(
            parsed.expression,
            FilterExpression::Condition(FilterCondition {
                field: "status".into(),
                op: FilterOperator::Eq,
                values: vec![FilterValue::Text("a;b".into())],
            })
        );
    }

    #[test]
    fn projection_keeps_unflattenable_subtrees_as_advanced_rules() {
        let parsed = postgres::POSTGRES_SYNTAX
            .parse("status = 'failed' AND lower(name) = 'abc'")
            .expect("valid expression");
        let projection =
            fluxdb_core::filter_form_projection_from_expression(&parsed.expression, |expression| {
                postgres::POSTGRES_SYNTAX
                    .render(expression)
                    .unwrap_or_default()
            });
        assert_eq!(projection.rules.len(), 2);
        assert_eq!(projection.rules[0].field.as_deref(), Some("status"));
        assert!(projection.rules[1].field.is_none());
        assert!(
            fluxdb_core::filter_value_text(&projection.rules[1].values[0])
                .to_ascii_lowercase()
                .contains("lower(name)")
        );
    }

    #[test]
    fn between_values_keep_source_order() {
        let parsed = postgres::POSTGRES_SYNTAX
            .parse("score BETWEEN 100 AND 10")
            .expect("valid expression");
        let FilterExpression::Condition(condition) = parsed.expression else {
            panic!("single condition");
        };
        assert_eq!(
            condition.values,
            vec![FilterValue::Integer(100), FilterValue::Integer(10)]
        );
        assert_eq!(
            postgres::POSTGRES_SYNTAX
                .render(&FilterExpression::Condition(condition))
                .unwrap(),
            "\"score\" BETWEEN 100 AND 10"
        );
    }

    #[test]
    fn like_patterns_escape_literal_wildcards_and_escape_character() {
        for (operator, expected) in [
            (FilterOperator::Contains, "%a!%b!_c!!d%"),
            (FilterOperator::NotContains, "%a!%b!_c!!d%"),
            (FilterOperator::StartsWith, "a!%b!_c!!d%"),
            (FilterOperator::NotStartsWith, "a!%b!_c!!d%"),
            (FilterOperator::EndsWith, "%a!%b!_c!!d"),
            (FilterOperator::NotEndsWith, "%a!%b!_c!!d"),
        ] {
            assert_eq!(
                like_pattern(operator, "a%b_c!d").as_deref(),
                Some(expected),
                "{operator:?}"
            );
        }
    }

    #[test]
    fn each_dialect_keeps_text_like_plain_and_casts_numeric_fields() {
        let expression = FilterExpression::Condition(FilterCondition {
            field: "name".into(),
            op: FilterOperator::Contains,
            values: vec![FilterValue::Text("a%b_c!d".into())],
        });
        let name_column = fluxdb_core::Column {
            name: "name".into(),
            type_name: Some("varchar(128)".into()),
            nullable: true,
            primary_key: false,
            comment: None,
        };
        let numeric_expression = FilterExpression::Condition(FilterCondition {
            field: "name".into(),
            op: FilterOperator::Contains,
            values: vec![FilterValue::Text("AD".into())],
        });
        let numeric_column = fluxdb_core::Column {
            type_name: Some("integer".into()),
            ..name_column.clone()
        };
        for (syntax, plain_field, cast_field) in [
            (FilterSyntax::Postgres, "\"name\"", "CAST(\"name\" AS TEXT)"),
            (FilterSyntax::MySql, "`name`", "`name`"),
            (
                FilterSyntax::SqlServer,
                "[name]",
                "CAST([name] AS NVARCHAR(MAX))",
            ),
            (FilterSyntax::Sqlite, "\"name\"", "\"name\""),
            (
                FilterSyntax::Dameng,
                "\"name\"",
                "CAST(\"name\" AS VARCHAR(4000))",
            ),
        ] {
            let rendered = render_for_syntax_with_columns(
                syntax,
                &expression,
                std::slice::from_ref(&name_column),
            )
            .expect("rendered text LIKE SQL");
            assert_eq!(
                rendered,
                format!("{plain_field} LIKE '%a!%b!_c!!d%' ESCAPE '!'"),
                "{syntax:?}"
            );

            let parsed = parse_for_syntax(syntax, &rendered).expect("LIKE round trip");
            assert_eq!(
                parsed.expression,
                FilterExpression::Condition(FilterCondition {
                    field: "name".into(),
                    op: FilterOperator::Contains,
                    values: vec![FilterValue::Text("a%b_c!d".into())],
                }),
                "{syntax:?}"
            );

            let plain = render_for_syntax_with_columns(
                syntax,
                &numeric_expression,
                std::slice::from_ref(&name_column),
            )
            .expect("rendered plain LIKE SQL");
            assert_eq!(plain, format!("{plain_field} LIKE '%AD%'"), "{syntax:?}");

            let casted = render_for_syntax_with_columns(
                syntax,
                &numeric_expression,
                std::slice::from_ref(&numeric_column),
            )
            .expect("rendered numeric LIKE SQL");
            assert_eq!(casted, format!("{cast_field} LIKE '%AD%'"), "{syntax:?}");

            let compiled = compile_for_syntax_with_columns(
                syntax,
                &expression,
                std::slice::from_ref(&name_column),
            )
            .expect("compiled LIKE SQL");
            assert!(compiled.sql.contains(plain_field), "{syntax:?}");
            assert!(
                compiled.sql.contains(" LIKE ") && compiled.sql.contains(" ESCAPE '!'"),
                "{syntax:?}: {}",
                compiled.sql
            );
            assert_eq!(
                compiled.parameters,
                vec![FilterValue::Text("%a!%b!_c!!d%".into())],
                "{syntax:?}"
            );
        }
    }

    #[test]
    fn dialect_specific_integer_like_and_empty_string_sql_stay_canonical() {
        let columns = [
            fluxdb_core::Column {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            fluxdb_core::Column {
                name: "name".into(),
                type_name: Some("varchar(128)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let integer_like = FilterExpression::Condition(FilterCondition {
            field: "id".into(),
            op: FilterOperator::Contains,
            values: vec![FilterValue::Text("1".into())],
        });
        for (syntax, expected, compiled_sql) in [
            (FilterSyntax::MySql, "`id` LIKE '%1%'", "`id` LIKE ?"),
            (FilterSyntax::Sqlite, "\"id\" LIKE '%1%'", "\"id\" LIKE ?"),
            (
                FilterSyntax::Postgres,
                "CAST(\"id\" AS TEXT) LIKE '%1%'",
                "CAST(\"id\" AS TEXT) LIKE $1",
            ),
            (
                FilterSyntax::SqlServer,
                "CAST([id] AS NVARCHAR(MAX)) LIKE '%1%'",
                "CAST([id] AS NVARCHAR(MAX)) LIKE @P1",
            ),
            (
                FilterSyntax::Dameng,
                "CAST(\"id\" AS VARCHAR(4000)) LIKE '%1%'",
                "CAST(\"id\" AS VARCHAR(4000)) LIKE ?",
            ),
        ] {
            assert_eq!(
                render_for_syntax_with_columns(syntax, &integer_like, &columns).unwrap(),
                expected,
                "{syntax:?}"
            );
            assert_eq!(
                compile_for_syntax_with_columns(syntax, &integer_like, &columns)
                    .unwrap()
                    .sql,
                compiled_sql,
                "{syntax:?}"
            );
        }

        let empty = FilterExpression::Condition(FilterCondition {
            field: "name".into(),
            op: FilterOperator::IsEmpty,
            values: Vec::new(),
        });
        assert_eq!(
            render_for_syntax_with_columns(FilterSyntax::Postgres, &empty, &columns).unwrap(),
            "\"name\" = ''"
        );
        assert_eq!(
            render_for_syntax_with_columns(FilterSyntax::SqlServer, &empty, &columns).unwrap(),
            "[name] = N''"
        );
    }

    #[test]
    fn comparison_in_list_and_boolean_follow_each_dialect() {
        let columns = [
            fluxdb_core::Column {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            fluxdb_core::Column {
                name: "active".into(),
                type_name: Some("boolean".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
            fluxdb_core::Column {
                name: "name".into(),
                type_name: Some("varchar(128)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let expression = FilterExpression::And(vec![
            FilterExpression::Condition(FilterCondition {
                field: "id".into(),
                op: FilterOperator::Eq,
                values: vec![FilterValue::Integer(1)],
            }),
            FilterExpression::Condition(FilterCondition {
                field: "active".into(),
                op: FilterOperator::Eq,
                values: vec![FilterValue::Boolean(true)],
            }),
            FilterExpression::Condition(FilterCondition {
                field: "name".into(),
                op: FilterOperator::InList,
                values: vec![FilterValue::Text("O'Reilly".into())],
            }),
        ]);
        for (syntax, expected) in [
            (
                FilterSyntax::MySql,
                "(`id` = 1 AND `active` = true AND `name` IN ('O''Reilly'))",
            ),
            (
                FilterSyntax::Postgres,
                "(\"id\" = 1 AND \"active\" = true AND \"name\" IN ('O''Reilly'))",
            ),
            (
                FilterSyntax::Sqlite,
                "(\"id\" = 1 AND \"active\" = true AND \"name\" IN ('O''Reilly'))",
            ),
            (
                FilterSyntax::SqlServer,
                "([id] = 1 AND [active] = 1 AND [name] IN ('O''Reilly'))",
            ),
            (
                FilterSyntax::Dameng,
                "(\"id\" = 1 AND \"active\" = true AND \"name\" IN ('O''Reilly'))",
            ),
        ] {
            assert_eq!(
                render_for_syntax_with_columns(syntax, &expression, &columns).unwrap(),
                expected,
                "{syntax:?}"
            );
        }
    }

    #[test]
    fn mixed_and_or_conditions_render_and_compile_across_dialects() {
        let columns = [
            fluxdb_core::Column {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            fluxdb_core::Column {
                name: "name".into(),
                type_name: Some("varchar(128)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let expression = FilterExpression::Or(vec![
            FilterExpression::And(vec![
                FilterExpression::Condition(FilterCondition {
                    field: "id".into(),
                    op: FilterOperator::Gt,
                    values: vec![FilterValue::Integer(1)],
                }),
                FilterExpression::Condition(FilterCondition {
                    field: "name".into(),
                    op: FilterOperator::Contains,
                    values: vec![FilterValue::Text("a".into())],
                }),
            ]),
            FilterExpression::Condition(FilterCondition {
                field: "id".into(),
                op: FilterOperator::Lt,
                values: vec![FilterValue::Integer(10)],
            }),
        ]);
        let cases = [
            (
                FilterSyntax::MySql,
                "((`id` > 1 AND `name` LIKE '%a%') OR `id` < 10)",
            ),
            (
                FilterSyntax::Postgres,
                "((\"id\" > 1 AND \"name\" LIKE '%a%') OR \"id\" < 10)",
            ),
            (
                FilterSyntax::Sqlite,
                "((\"id\" > 1 AND \"name\" LIKE '%a%') OR \"id\" < 10)",
            ),
            (
                FilterSyntax::SqlServer,
                "(([id] > 1 AND [name] LIKE '%a%') OR [id] < 10)",
            ),
            (
                FilterSyntax::Dameng,
                "((\"id\" > 1 AND \"name\" LIKE '%a%') OR \"id\" < 10)",
            ),
        ];
        for (syntax, expected) in cases {
            let rendered =
                render_for_syntax_with_columns(syntax, &expression, &columns).expect("rendered");
            assert_eq!(rendered, expected, "{syntax:?}");
            let parsed = parse_for_syntax(syntax, &rendered).expect("parsed combined SQL");
            assert_eq!(
                render_for_syntax_with_columns(syntax, &parsed.expression, &columns)
                    .expect("canonical rendered"),
                rendered,
                "{syntax:?}"
            );
            let compiled = compile_for_syntax_with_columns(syntax, &expression, &columns)
                .expect("compiled combined SQL");
            assert_eq!(
                compiled.parameters,
                vec![
                    FilterValue::Integer(1),
                    FilterValue::Text("%a%".into()),
                    FilterValue::Integer(10)
                ],
                "{syntax:?}"
            );
            assert!(compiled.sql.contains(" AND "), "{syntax:?}");
            assert!(compiled.sql.contains(" OR "), "{syntax:?}");
        }
    }

    #[test]
    fn like_round_trip_preserves_all_boundary_operators() {
        for (operator, value) in [
            (FilterOperator::Contains, "100%"),
            (FilterOperator::NotContains, "100%"),
            (FilterOperator::StartsWith, "100%"),
            (FilterOperator::NotStartsWith, "100%"),
            (FilterOperator::EndsWith, "100%"),
            (FilterOperator::NotEndsWith, "100%"),
        ] {
            let expression = FilterExpression::Condition(FilterCondition {
                field: "id".into(),
                op: operator,
                values: vec![FilterValue::Text(value.into())],
            });
            let rendered =
                render_for_syntax(FilterSyntax::Postgres, &expression).expect("rendered LIKE SQL");
            let parsed =
                parse_for_syntax(FilterSyntax::Postgres, &rendered).expect("parsed LIKE SQL");
            assert_eq!(parsed.expression, expression, "{operator:?}");
        }
    }

    #[test]
    fn unsupported_like_wildcards_remain_raw_instead_of_changing_semantics() {
        let parsed = postgres::POSTGRES_SYNTAX
            .parse("name LIKE 'a%b'")
            .expect("valid SQL");
        assert!(matches!(parsed.expression, FilterExpression::Raw(_)));
    }

    #[test]
    fn every_form_operator_round_trips_for_each_sql_database() {
        use fluxdb_core::{FilterCombinator, FilterFormRule};

        let columns = vec![
            fluxdb_core::Column {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            fluxdb_core::Column {
                name: "name".into(),
                type_name: Some("varchar(128)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let cases = [
            ("id", FilterOperator::Eq, vec![FilterValue::Integer(2)]),
            ("id", FilterOperator::Ne, vec![FilterValue::Integer(2)]),
            ("id", FilterOperator::Lt, vec![FilterValue::Integer(2)]),
            ("id", FilterOperator::Le, vec![FilterValue::Integer(2)]),
            ("id", FilterOperator::Gt, vec![FilterValue::Integer(2)]),
            ("id", FilterOperator::Ge, vec![FilterValue::Integer(2)]),
            (
                "name",
                FilterOperator::Contains,
                vec![FilterValue::Text("AD".into())],
            ),
            (
                "name",
                FilterOperator::NotContains,
                vec![FilterValue::Text("AD".into())],
            ),
            (
                "name",
                FilterOperator::StartsWith,
                vec![FilterValue::Text("An".into())],
            ),
            (
                "name",
                FilterOperator::NotStartsWith,
                vec![FilterValue::Text("An".into())],
            ),
            (
                "name",
                FilterOperator::EndsWith,
                vec![FilterValue::Text("ra".into())],
            ),
            (
                "name",
                FilterOperator::NotEndsWith,
                vec![FilterValue::Text("ra".into())],
            ),
            ("name", FilterOperator::IsNull, Vec::new()),
            ("name", FilterOperator::IsNotNull, Vec::new()),
            ("name", FilterOperator::IsEmpty, Vec::new()),
            ("name", FilterOperator::IsNotEmpty, Vec::new()),
            (
                "id",
                FilterOperator::Between,
                vec![FilterValue::Integer(1), FilterValue::Integer(3)],
            ),
            (
                "id",
                FilterOperator::NotBetween,
                vec![FilterValue::Integer(1), FilterValue::Integer(3)],
            ),
            (
                "name",
                FilterOperator::InList,
                vec![
                    FilterValue::Text("Andorra".into()),
                    FilterValue::Text("Angola".into()),
                ],
            ),
            (
                "name",
                FilterOperator::NotInList,
                vec![
                    FilterValue::Text("Andorra".into()),
                    FilterValue::Text("Angola".into()),
                ],
            ),
        ];
        let database_kinds = [
            DatabaseKind::MySql,
            DatabaseKind::TiDb,
            DatabaseKind::Postgres,
            DatabaseKind::Sqlite,
            DatabaseKind::SqlServer,
            DatabaseKind::Dameng,
        ];

        for kind in database_kinds {
            let syntax = filter_syntax(kind).expect("SQL filter dialect");
            for (field, operator, values) in cases.clone() {
                let rule = FilterFormRule {
                    enabled: true,
                    field: Some(field.into()),
                    operator,
                    values,
                    grouped: false,
                };
                let expression = fluxdb_core::filter_expression_from_form(
                    std::slice::from_ref(&rule),
                    FilterCombinator::And,
                )
                .expect("form expression");
                let text = render_for_syntax_with_columns(syntax, &expression, &columns)
                    .expect("form SQL");
                let parsed = parse_for_syntax(syntax, &text).expect("parsed form SQL");
                let projection = fluxdb_core::filter_form_projection_from_expression(
                    &parsed.expression,
                    |expression| {
                        render_for_syntax_with_columns(syntax, expression, &columns)
                            .unwrap_or_default()
                    },
                );
                assert_eq!(
                    projection.rules,
                    vec![rule],
                    "{kind:?} {operator:?}: {text}"
                );
            }
        }
    }
}
