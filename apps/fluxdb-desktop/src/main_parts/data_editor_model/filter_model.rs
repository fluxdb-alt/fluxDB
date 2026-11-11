#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataFilterPopoverKind {
    Field,
    Operator,
    Value,
    SortField,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DataFilterPopover {
    tab_id: TabId,
    kind: DataFilterPopoverKind,
    rule_index: Option<usize>,
    sort_index: Option<usize>,
    /// 触发点的窗口坐标：弹层贴着被点的字段/条件/值显示，避免浮在固定位置。
    anchor: Point<Pixels>,
}

#[derive(Clone, Debug, PartialEq)]
struct DataFilterRule {
    enabled: bool,
    field: Option<String>,
    operator: DataFilterOperator,
    /// 有序值：BETWEEN 的上下限不能被排序交换；IN 列表也保持用户输入顺序。
    values: Vec<String>,
    /// 输入模式中无法无损拆成字段/运算符/值的表达式文本。
    /// 该行只读，避免切换模式时静默改变语义。
    advanced_expression: Option<String>,
    /// 从输入表达式解析出的原始类型。表单编辑值后丢弃，避免把用户新输入的
    /// `"007"` 错误保留成整数类型。
    typed_values: Option<Vec<fluxdb_core::FilterValue>>,
    /// 本行与下一行之间的组合符；最后一行的值不参与表达式。
    connector: fluxdb_core::FilterCombinator,
    /// 0 为顶层；正值表示该规则所在的括号嵌套深度。
    group_depth: u8,
    /// 正值表示本行之后关闭到该深度。
    group_end: u8,
}

/// 值编辑器草稿：打开「值」弹层时从条件快照初始化；「确定」才写回条件。
/// 字段切换、异步建议值请求返回都会校验 `field` 是否仍是本草稿目标字段，
/// 避免旧字段的选择或请求结果覆盖新字段。
#[derive(Clone, Debug, Eq, PartialEq)]
struct DataFilterValueDraft {
    tab_id: TabId,
    rule_index: usize,
    /// 打开弹层时的目标字段快照，用于字段切换后的身份/竞态校验。
    field: String,
    /// 打开弹层时的运算符快照，用于确认时校验值形态兼容性。
    operator: DataFilterOperator,
    values: Vec<String>,
    /// 单值输入框文案（保留用户正在输入、尚未提交的文本）。
    input: String,
    /// 批量粘贴区是否展开。
    batch_open: bool,
    /// 批量粘贴区多行文本内容。
    batch_text: String,
    /// 批量粘贴分隔方式。
    batch_separator: BatchSeparator,
}

/// 批量粘贴的分隔方式。默认每行一个值；明确选择时才按逗号/制表符拆分，
/// 不假装支持完整 CSV 引号规则。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum BatchSeparator {
    #[default]
    Newline,
    Comma,
    Tab,
}

impl BatchSeparator {
    fn label(self) -> SharedString {
        match self {
            Self::Newline => tr("app.filter.separator_newline"),
            Self::Comma => tr("app.filter.separator_comma"),
            Self::Tab => tr("app.filter.separator_tab"),
        }
    }
}

/// 按选定的分隔方式切分批量粘贴文本。
///
/// 规则必须明确：默认每行一个值；明确选择时才按逗号/制表符拆分。
/// 这里不做完整 CSV 引号解析（不伪装支持带引号/转义的 CSV），
/// 只保留原始片段，交由调用方做 trim 去空与去重。
fn split_filter_batch_text(text: &str, separator: BatchSeparator) -> Vec<&str> {
    match separator {
        BatchSeparator::Newline => text.split('\n').collect(),
        BatchSeparator::Comma => text.split(',').collect(),
        BatchSeparator::Tab => text.split('\t').collect(),
    }
}

/// 运算符还能否再加入一个值。
///
/// 上限由值形态决定：无值 0、单值 1、区间 2（介于的上下限），
/// 列表（IN / NOT IN）不限数量。
fn data_filter_rule_can_add_more_value(operator: DataFilterOperator, current_len: usize) -> bool {
    operator.max_values().is_none_or(|max| current_len < max)
}

/// 从批量片段解析出的待添加值：trim、忽略空行，纯逻辑便于单测。
fn filter_batch_values_to_add(
    text: &str,
    separator: BatchSeparator,
    selected: &[String],
) -> (Vec<String>, usize, usize) {
    let mut added = Vec::new();
    let mut duplicated = 0usize;
    let mut ignored_empty = 0usize;
    for part in split_filter_batch_text(text, separator) {
        let trimmed = part.trim();
        // 忽略空行；保留字符串中有意义的内部空格/引号/逗号，不擅自改写。
        if trimmed.is_empty() {
            ignored_empty += 1;
            continue;
        }
        let value = trimmed.to_string();
        if selected.contains(&value) {
            duplicated += 1;
        } else {
            added.push(value);
        }
    }
    (added, duplicated, ignored_empty)
}

/// 切换字段后按新字段类型收敛旧值：文本/未知类型原样保留，数字/布尔列只保留
/// 能按该类型解析的值。直接保留 `abc` 到整数列会在 MySQL 静默按 0 比较、
/// 在 Postgres 直接报错，必须在这里拦住。
///
/// 区间（介于/不介于）任一上下限不兼容就整体清空，避免留下无法成立的单边区间；
/// 单值/列表逐个过滤。`typed_values` 一律丢弃，由查询期按新列类型重新定型，
/// 防止旧列的整数类型覆盖文本列的 `"007"` 语义。
///
/// 返回 (新规则, 保留值个数, 丢弃值个数)。
fn data_filter_rule_after_field_switch(
    field: String,
    mut rule: DataFilterRule,
    new_type_name: Option<&str>,
) -> (DataFilterRule, usize, usize) {
    rule.field = Some(field);
    let previous_len = rule.values.len();
    let operator = rule.operator;
    let compatible = |value: &String| {
        data_filter_value_matches_column(value, new_type_name, operator)
    };
    let values = match operator.value_arity() {
        // 区间缺一个边界就不能成立；宁可整体清空，也不生成半个 BETWEEN。
        DataFilterValueArity::Range if !rule.values.iter().all(compatible) => Vec::new(),
        _ => rule
            .values
            .iter()
            .filter(|value| compatible(value))
            .cloned()
            .collect(),
    };
    let values = data_filter_values_for_arity(&values, operator);
    let kept = values.len();
    rule.values = values;
    rule.typed_values = None;
    (rule, kept, previous_len - kept)
}

/// 值能否直接用于目标列类型：解析规则与查询期 `data_filter_value_for_column` 一致，
/// 避免“编辑器允许保留、执行时又转换失败”的不一致。
/// LIKE 系列把值当模式文本，任何列类型都不做数字/布尔校验。
fn data_filter_value_matches_column(
    value: &str,
    type_name: Option<&str>,
    operator: DataFilterOperator,
) -> bool {
    if matches!(
        operator,
        DataFilterOperator::Contains
            | DataFilterOperator::NotContains
            | DataFilterOperator::StartsWith
            | DataFilterOperator::NotStartsWith
            | DataFilterOperator::EndsWith
            | DataFilterOperator::NotEndsWith
    ) {
        return true;
    }
    let text = value.trim();
    match data_filter_column_scalar_type(type_name) {
        DataFilterColumnScalarType::Integer => text.parse::<i64>().is_ok(),
        DataFilterColumnScalarType::Float => text.parse::<f64>().is_ok(),
        DataFilterColumnScalarType::Boolean => matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "1" | "false" | "0"
        ),
        DataFilterColumnScalarType::Other => true,
    }
}

/// 按目标运算符的值形态收敛已有值：列表保留全部，单值/区间只保留前 1/2 个，
/// 无值运算符清空——旧值会以错误的 OR 组合或多余的 BETWEEN 参数参与查询。
/// 单值之间互切（`=` 改 `!=`）保留同一个值，避免改运算符就丢输入。
fn data_filter_values_for_arity(
    values: &[String],
    operator: DataFilterOperator,
) -> Vec<String> {
    match operator.max_values() {
        Some(max) => values.iter().take(max).cloned().collect(),
        None => values.to_vec(),
    }
}

fn data_filter_rule_after_operator_switch(
    operator: DataFilterOperator,
    mut rule: DataFilterRule,
) -> DataFilterRule {
    rule.values = data_filter_values_for_arity(&rule.values, operator);
    rule.typed_values = rule.typed_values.as_ref().map(|values| {
        match operator.max_values() {
            Some(max) => values.iter().take(max).cloned().collect(),
            None => values.clone(),
        }
    });
    rule.operator = operator;
    rule
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DataSortRule {
    enabled: bool,
    field: String,
    ascending: bool,
}

/// 排序拖动载荷：只带被拖动的源位置；放置目标由每个标签自己的 index 决定。
#[derive(Clone)]
struct DataSortDrag {
    tab_id: TabId,
    from: usize,
    field: String,
    colors: UiColors,
}

impl Render for DataSortDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        data_sort_drag_preview(self.field.clone(), self.colors)
    }
}

fn data_sort_drag_preview(label: String, colors: UiColors) -> impl IntoElement {
    div()
        .h(px(26.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(rgb(0x1677ff))
        .bg(if colors.is_dark {
            rgb(0x18314d)
        } else {
            rgb(0xf2f8ff)
        })
        .shadow(vec![box_shadow(
            px(0.),
            px(6.),
            px(18.),
            px(0.),
            hsla(212. / 360., 0.6, 0.35, if colors.is_dark { 0.28 } else { 0.18 }),
        )])
        .opacity(0.96)
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon(AppIcon::GripVertical, 12., colors.muted))
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataFilterMode {
    Builder,
    Text,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedDataEditorSqlText {
    filter_text: String,
    sort_text: String,
    limit: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct SqlTextSelection {
    text: String,
    anchor: usize,
    cursor: usize,
    selecting: bool,
    bounds: Option<Bounds<Pixels>>,
}

impl SqlTextSelection {
    fn selected_range(&self) -> Option<(usize, usize)> {
        let start = self.anchor.min(self.cursor);
        let end = self.anchor.max(self.cursor);
        (start < end).then_some((start, end))
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        self.text.get(start..end).map(str::to_string)
    }
}

impl Default for DataFilterRule {
    fn default() -> Self {
        Self {
            enabled: true,
            field: None,
            operator: DataFilterOperator::Contains,
            values: Vec::new(),
            advanced_expression: None,
            typed_values: None,
            connector: fluxdb_core::FilterCombinator::And,
            group_depth: 0,
            group_end: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataFilterValueArity {
    /// 不需要值（是 null / 是空的…），不显示值槽。
    None,
    /// 恰好一个值（=、!=、<、包含…）：值槽就地输入，不用多值弹层。
    Single,
    /// 两个值构成区间（介于 / 不介于），用值弹层但最多两个值。
    Range,
    /// 任意多个值（在列表 / 不在列表），用多选值弹层。
    Many,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataFilterOperator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    NotContains,
    StartsWith,
    NotStartsWith,
    EndsWith,
    NotEndsWith,
    IsNull,
    IsNotNull,
    IsEmpty,
    IsNotEmpty,
    Between,
    NotBetween,
    InList,
    NotInList,
}

impl DataFilterOperator {
    fn label(self) -> SharedString {
        match self {
            Self::Eq => tr("app.filter.op_eq"),
            Self::Ne => tr("app.filter.op_ne"),
            Self::Lt => tr("app.filter.op_lt"),
            Self::Le => tr("app.filter.op_le"),
            Self::Gt => tr("app.filter.op_gt"),
            Self::Ge => tr("app.filter.op_ge"),
            Self::Contains => tr("app.filter.op_contains"),
            Self::NotContains => tr("app.filter.op_not_contains"),
            Self::StartsWith => tr("app.filter.op_starts_with"),
            Self::NotStartsWith => tr("app.filter.op_not_starts_with"),
            Self::EndsWith => tr("app.filter.op_ends_with"),
            Self::NotEndsWith => tr("app.filter.op_not_ends_with"),
            Self::IsNull => tr("app.filter.op_is_null"),
            Self::IsNotNull => tr("app.filter.op_is_not_null"),
            Self::IsEmpty => tr("app.filter.op_is_empty"),
            Self::IsNotEmpty => tr("app.filter.op_is_not_empty"),
            Self::Between => tr("app.filter.op_between"),
            Self::NotBetween => tr("app.filter.op_not_between"),
            Self::InList => tr("app.filter.op_in_list"),
            Self::NotInList => tr("app.filter.op_not_in_list"),
        }
    }

    /// 运算符要求的值形态，决定值槽用哪种编辑器。
    fn value_arity(self) -> DataFilterValueArity {
        match self {
            Self::IsNull | Self::IsNotNull | Self::IsEmpty | Self::IsNotEmpty => {
                DataFilterValueArity::None
            }
            Self::Between | Self::NotBetween => DataFilterValueArity::Range,
            Self::InList | Self::NotInList => DataFilterValueArity::Many,
            Self::Eq
            | Self::Ne
            | Self::Lt
            | Self::Le
            | Self::Gt
            | Self::Ge
            | Self::Contains
            | Self::NotContains
            | Self::StartsWith
            | Self::NotStartsWith
            | Self::EndsWith
            | Self::NotEndsWith => DataFilterValueArity::Single,
        }
    }

    fn requires_values(self) -> bool {
        self.value_arity() != DataFilterValueArity::None
    }

    /// 最多允许的值个数；`None` 表示不限（列表）。
    fn max_values(self) -> Option<usize> {
        match self.value_arity() {
            DataFilterValueArity::None => Some(0),
            DataFilterValueArity::Single => Some(1),
            DataFilterValueArity::Range => Some(2),
            DataFilterValueArity::Many => None,
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::Eq,
            Self::Ne,
            Self::Lt,
            Self::Le,
            Self::Gt,
            Self::Ge,
            Self::Contains,
            Self::NotContains,
            Self::StartsWith,
            Self::NotStartsWith,
            Self::EndsWith,
            Self::NotEndsWith,
            Self::IsNull,
            Self::IsNotNull,
            Self::IsEmpty,
            Self::IsNotEmpty,
            Self::Between,
            Self::NotBetween,
            Self::InList,
            Self::NotInList,
        ]
    }
}

fn data_filter_specs_from_rules(rules: &[DataFilterRule]) -> Vec<FilterSpec> {
    rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter_map(data_filter_spec_from_rule)
        .collect()
}

impl From<&DataFilterRule> for fluxdb_core::FilterFormRule {
    fn from(rule: &DataFilterRule) -> Self {
        Self {
            enabled: rule.enabled,
            field: rule.field.clone(),
            operator: match rule.operator {
                DataFilterOperator::Eq => fluxdb_core::FilterOperator::Eq,
                DataFilterOperator::Ne => fluxdb_core::FilterOperator::Ne,
                DataFilterOperator::Lt => fluxdb_core::FilterOperator::Lt,
                DataFilterOperator::Le => fluxdb_core::FilterOperator::Le,
                DataFilterOperator::Gt => fluxdb_core::FilterOperator::Gt,
                DataFilterOperator::Ge => fluxdb_core::FilterOperator::Ge,
                DataFilterOperator::Contains => fluxdb_core::FilterOperator::Contains,
                DataFilterOperator::NotContains => fluxdb_core::FilterOperator::NotContains,
                DataFilterOperator::StartsWith => fluxdb_core::FilterOperator::StartsWith,
                DataFilterOperator::NotStartsWith => fluxdb_core::FilterOperator::NotStartsWith,
                DataFilterOperator::EndsWith => fluxdb_core::FilterOperator::EndsWith,
                DataFilterOperator::NotEndsWith => fluxdb_core::FilterOperator::NotEndsWith,
                DataFilterOperator::IsNull => fluxdb_core::FilterOperator::IsNull,
                DataFilterOperator::IsNotNull => fluxdb_core::FilterOperator::IsNotNull,
                DataFilterOperator::IsEmpty => fluxdb_core::FilterOperator::IsEmpty,
                DataFilterOperator::IsNotEmpty => fluxdb_core::FilterOperator::IsNotEmpty,
                DataFilterOperator::Between => fluxdb_core::FilterOperator::Between,
                DataFilterOperator::NotBetween => fluxdb_core::FilterOperator::NotBetween,
                DataFilterOperator::InList => fluxdb_core::FilterOperator::InList,
                DataFilterOperator::NotInList => fluxdb_core::FilterOperator::NotInList,
            },
            values: rule
                .typed_values
                .as_ref()
                .filter(|values| values.len() == rule.values.len())
                .cloned()
                .unwrap_or_else(|| {
                    rule.values
                        .iter()
                        // 用户新输入的值必须保持字符串语义：`007` 不能变成整数 7。
                        .map(|value| fluxdb_core::FilterValue::Text(value.clone()))
                        .collect()
                }),
            grouped: rule.group_depth > 0,
        }
    }
}

fn data_filter_operator_from_core(operator: fluxdb_core::FilterOperator) -> DataFilterOperator {
    match operator {
        fluxdb_core::FilterOperator::Eq => DataFilterOperator::Eq,
        fluxdb_core::FilterOperator::Ne => DataFilterOperator::Ne,
        fluxdb_core::FilterOperator::Lt => DataFilterOperator::Lt,
        fluxdb_core::FilterOperator::Le => DataFilterOperator::Le,
        fluxdb_core::FilterOperator::Gt => DataFilterOperator::Gt,
        fluxdb_core::FilterOperator::Ge => DataFilterOperator::Ge,
        fluxdb_core::FilterOperator::Contains => DataFilterOperator::Contains,
        fluxdb_core::FilterOperator::NotContains => DataFilterOperator::NotContains,
        fluxdb_core::FilterOperator::StartsWith => DataFilterOperator::StartsWith,
        fluxdb_core::FilterOperator::NotStartsWith => DataFilterOperator::NotStartsWith,
        fluxdb_core::FilterOperator::EndsWith => DataFilterOperator::EndsWith,
        fluxdb_core::FilterOperator::NotEndsWith => DataFilterOperator::NotEndsWith,
        fluxdb_core::FilterOperator::IsNull => DataFilterOperator::IsNull,
        fluxdb_core::FilterOperator::IsNotNull => DataFilterOperator::IsNotNull,
        fluxdb_core::FilterOperator::IsEmpty => DataFilterOperator::IsEmpty,
        fluxdb_core::FilterOperator::IsNotEmpty => DataFilterOperator::IsNotEmpty,
        fluxdb_core::FilterOperator::Between => DataFilterOperator::Between,
        fluxdb_core::FilterOperator::NotBetween => DataFilterOperator::NotBetween,
        fluxdb_core::FilterOperator::InList => DataFilterOperator::InList,
        fluxdb_core::FilterOperator::NotInList => DataFilterOperator::NotInList,
    }
}

fn data_filter_rules_from_expression(
    expression: &fluxdb_core::FilterExpression,
    render_raw: impl Fn(&fluxdb_core::FilterExpression) -> String,
) -> (Vec<DataFilterRule>, fluxdb_core::FilterCombinator) {
    let projection = fluxdb_core::filter_form_projection_from_expression(expression, render_raw);
    let projection_combinator = projection.combinator;
    let projection_connectors = projection.connectors;
    let projection_group_depths = projection.group_depths;
    let projection_group_ends = projection.group_ends;
    let rules = projection
        .rules
        .into_iter()
        .enumerate()
        .map(|(index, rule)| {
            let advanced_expression = rule
                .field
                .is_none()
                .then(|| rule.values.first().cloned())
                .flatten()
                .map(|value| fluxdb_core::filter_value_text(&value));
            DataFilterRule {
                enabled: rule.enabled,
                field: rule.field,
                operator: data_filter_operator_from_core(rule.operator),
                typed_values: Some(rule.values.clone()),
                values: rule
                    .values
                    .iter()
                    .map(fluxdb_core::filter_value_text)
                    .collect(),
                advanced_expression,
                connector: projection_connectors
                    .get(index)
                    .copied()
                    .unwrap_or(projection_combinator),
                group_depth: projection_group_depths
                    .get(index)
                    .copied()
                    .unwrap_or_default(),
                group_end: projection_group_ends
                    .get(index)
                    .copied()
                    .unwrap_or_default(),
            }
        })
        .collect();
    (rules, projection_combinator)
}

fn data_filter_expression_from_rules(
    rules: &[DataFilterRule],
    columns: &[GdbColumn],
) -> fluxdb_core::Result<fluxdb_core::FilterExpression> {
    let mut index = 0;
    data_filter_expression_at_depth(rules, &mut index, 0, columns)
}

fn data_filter_expression_at_depth(
    rules: &[DataFilterRule],
    index: &mut usize,
    depth: u8,
    columns: &[GdbColumn],
) -> fluxdb_core::Result<fluxdb_core::FilterExpression> {
    let mut terms = Vec::new();
    while *index < rules.len() {
        let rule_depth = rules[*index].group_depth;
        if rule_depth < depth {
            break;
        }
        if rule_depth == depth
            && rules[*index].group_end > 0
            && rules[*index].group_end < depth
        {
            break;
        }
        if rule_depth > depth {
            let child_depth = rule_depth;
            let expression =
                data_filter_expression_at_depth(rules, index, child_depth, columns)?;
            if !matches!(&expression, fluxdb_core::FilterExpression::And(items) if items.is_empty())
            {
                terms.push((expression, rules[*index - 1].connector));
            }
            continue;
        }
        let rule_index = *index;
        let expression = data_filter_single_rule_expression(&rules[rule_index], columns)?;
        let connector = rules[rule_index].connector;
        *index += 1;
        if let Some(expression) = expression {
            terms.push((expression, connector));
        }
        if rules
            .get(*index)
            .is_some_and(|next| next.group_depth > depth)
        {
            let child_depth = rules[*index].group_depth;
            let expression = data_filter_expression_at_depth(rules, index, child_depth, columns)?;
            let child_grouped = !matches!(
                &expression,
                fluxdb_core::FilterExpression::Condition(_)
                    | fluxdb_core::FilterExpression::Raw(_)
                    | fluxdb_core::FilterExpression::Not(_)
            );
            if !matches!(&expression, fluxdb_core::FilterExpression::And(items) if items.is_empty())
            {
                if child_grouped
                    && let Some(last) = terms.last_mut()
                    && let fluxdb_core::FilterExpression::Condition(_)
                    | fluxdb_core::FilterExpression::Raw(_)
                    | fluxdb_core::FilterExpression::Not(_) = &last.0
                {
                    let previous_connector = last.1;
                    last.1 = rules[rule_index].connector;
                    terms.push((expression, previous_connector));
                } else {
                    terms.push((expression, rules[*index - 1].connector));
                }
            }
        }
        if rules[rule_index].group_end == depth && depth > 0 {
            break;
        }
    }
    Ok(data_filter_join_terms(terms))
}

fn data_filter_single_rule_expression(
    rule: &DataFilterRule,
    columns: &[GdbColumn],
) -> fluxdb_core::Result<Option<fluxdb_core::FilterExpression>> {
    if !rule.enabled {
        return Ok(None);
    }
    if let Some(expression) = rule
        .advanced_expression
        .as_deref()
        .filter(|expression| !expression.trim().is_empty())
    {
        return Ok(Some(fluxdb_core::FilterExpression::Raw(
            expression.to_string(),
        )));
    }
    let mut form_rule = fluxdb_core::FilterFormRule::from(rule);
    form_rule.grouped = false;
    let type_name = form_rule
        .field
        .as_deref()
        .and_then(|field| columns.iter().find(|column| column.name == field))
        .and_then(|column| column.type_name.as_deref());
    form_rule.values = form_rule
        .values
        .iter()
        .map(|value| data_filter_value_for_column(value, type_name, form_rule.operator))
        .collect();
    let expression = fluxdb_core::filter_expression_from_form(
        std::slice::from_ref(&form_rule),
        fluxdb_core::FilterCombinator::And,
    )
    .map_err(|error| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Query, error.to_string()))?;
    let fluxdb_core::FilterExpression::And(mut items) = expression else {
        return Ok(Some(expression));
    };
    Ok((items.len() == 1).then(|| items.remove(0)))
}

fn data_filter_value_for_column(
    value: &fluxdb_core::FilterValue,
    type_name: Option<&str>,
    operator: fluxdb_core::FilterOperator,
) -> fluxdb_core::FilterValue {
    if matches!(
        operator,
        fluxdb_core::FilterOperator::Contains
            | fluxdb_core::FilterOperator::NotContains
            | fluxdb_core::FilterOperator::StartsWith
            | fluxdb_core::FilterOperator::NotStartsWith
            | fluxdb_core::FilterOperator::EndsWith
            | fluxdb_core::FilterOperator::NotEndsWith
    ) {
        // LIKE 的操作数是模式文本；即使列是数字，也不能把 "001" 改成整数。
        return value.clone();
    }
    let fluxdb_core::FilterValue::Text(text) = value else {
        return value.clone();
    };
    match data_filter_column_scalar_type(type_name) {
        DataFilterColumnScalarType::Integer => text
            .trim()
            .parse::<i64>()
            .map(fluxdb_core::FilterValue::Integer)
            .unwrap_or_else(|_| value.clone()),
        DataFilterColumnScalarType::Float => text
            .trim()
            .parse::<f64>()
            .map(fluxdb_core::FilterValue::Float)
            .unwrap_or_else(|_| value.clone()),
        DataFilterColumnScalarType::Boolean => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => fluxdb_core::FilterValue::Boolean(true),
            "false" | "0" => fluxdb_core::FilterValue::Boolean(false),
            _ => value.clone(),
        },
        DataFilterColumnScalarType::Other => value.clone(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataFilterColumnScalarType {
    Integer,
    Float,
    Boolean,
    Other,
}

fn data_filter_column_scalar_type(type_name: Option<&str>) -> DataFilterColumnScalarType {
    let Some(type_name) = type_name else {
        return DataFilterColumnScalarType::Other;
    };
    let type_name = type_name.trim();
    if type_name.contains('[') || type_name.starts_with('_') {
        return DataFilterColumnScalarType::Other;
    }
    let base = type_name
        .to_ascii_lowercase()
        .split('(')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    match base.as_str() {
        "int" | "integer" | "tinyint" | "smallint" | "mediumint" | "bigint" | "int2"
        | "int4" | "int8" | "serial" | "smallserial" | "bigserial" | "year" => {
            DataFilterColumnScalarType::Integer
        }
        "numeric" | "decimal" | "dec" | "real" | "double" | "double precision" | "float"
        | "float4" | "float8" | "number" => DataFilterColumnScalarType::Float,
        "bool" | "boolean" | "bit" => DataFilterColumnScalarType::Boolean,
        _ => DataFilterColumnScalarType::Other,
    }
}

/// 行间组合符使用 SQL 默认优先级：AND 先结合，再由 OR 连接各段。
fn data_filter_join_terms(
    terms: Vec<(fluxdb_core::FilterExpression, fluxdb_core::FilterCombinator)>,
) -> fluxdb_core::FilterExpression {
    if terms.is_empty() {
        return fluxdb_core::FilterExpression::And(Vec::new());
    }
    let mut or_terms = Vec::new();
    let mut and_terms = Vec::new();
    for (index, (expression, connector)) in terms.iter().enumerate() {
        and_terms.push(expression.clone());
        if index + 1 == terms.len() || *connector == fluxdb_core::FilterCombinator::Or {
            let part = if and_terms.len() == 1 {
                and_terms.remove(0)
            } else {
                fluxdb_core::FilterExpression::And(std::mem::take(&mut and_terms))
            };
            or_terms.push(part);
        }
    }
    if or_terms.len() == 1 {
        or_terms.remove(0)
    } else {
        fluxdb_core::FilterExpression::Or(or_terms)
    }
}

fn normalized_data_filter_value(value: &str) -> String {
    value
        .trim()
        .trim_matches(['\'', '"', '‘', '’', '“', '”', '「', '」', '『', '』', '＂', '＇'])
        .to_string()
}

fn data_filter_spec_from_rule(rule: &DataFilterRule) -> Option<FilterSpec> {
    let field = rule.field.clone()?;
    let values = rule
        .values
        .iter()
        .map(|value| CellValue::Text(normalized_data_filter_value(value)))
        .collect::<Vec<_>>();
    let op = match rule.operator {
        DataFilterOperator::Eq => FilterOp::Eq,
        DataFilterOperator::Ne => FilterOp::NotEq,
        DataFilterOperator::Lt => FilterOp::LessThan,
        DataFilterOperator::Le => FilterOp::LessThanOrEqual,
        DataFilterOperator::Gt => FilterOp::GreaterThan,
        DataFilterOperator::Ge => FilterOp::GreaterThanOrEqual,
        DataFilterOperator::Contains => FilterOp::Contains,
        DataFilterOperator::NotContains => FilterOp::NotContains,
        DataFilterOperator::StartsWith => FilterOp::StartsWith,
        DataFilterOperator::NotStartsWith => FilterOp::NotStartsWith,
        DataFilterOperator::EndsWith => FilterOp::EndsWith,
        DataFilterOperator::NotEndsWith => FilterOp::NotEndsWith,
        DataFilterOperator::IsNull => FilterOp::IsNull,
        DataFilterOperator::IsNotNull => FilterOp::IsNotNull,
        DataFilterOperator::IsEmpty => FilterOp::IsEmpty,
        DataFilterOperator::IsNotEmpty => FilterOp::IsNotEmpty,
        DataFilterOperator::Between => FilterOp::Between,
        DataFilterOperator::NotBetween => FilterOp::NotBetween,
        DataFilterOperator::InList => FilterOp::InList,
        DataFilterOperator::NotInList => FilterOp::NotInList,
    };

    Some(FilterSpec {
        field,
        op,
        values,
        enabled: rule.enabled,
    })
}

fn local_filter_manager_condition_entries(
    filters: &BTreeMap<String, BTreeSet<String>>,
    editing_field: Option<&str>,
) -> Vec<(String, BTreeSet<String>)> {
    let mut entries = filters
        .iter()
        .filter(|(field, values)| {
            !values.is_empty() && editing_field.is_none_or(|editing| field.as_str() != editing)
        })
        .map(|(field, values)| (field.clone(), values.clone()))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn data_search_matches(
    page: &DataPage,
    visible_column_indexes: &[usize],
    query: &str,
) -> Vec<DataSearchMatch> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for (row_ix, row) in page.rows.iter().enumerate() {
        for (visible_col, source_col) in visible_column_indexes.iter().enumerate() {
            let Some(value) = row.values.get(*source_col) else {
                continue;
            };
            if data_cell_display_text(value)
                .to_ascii_lowercase()
                .contains(query.as_str())
            {
                matches.push(DataSearchMatch {
                    row_ix,
                    col_ix: visible_col + 1,
                });
            }
        }
    }
    matches
}

fn next_data_search_match(
    matches: &[DataSearchMatch],
    current: Option<DataSearchMatch>,
) -> Option<DataSearchMatch> {
    if matches.is_empty() {
        return None;
    }
    let Some(current) = current else {
        return matches.first().copied();
    };
    matches
        .iter()
        .position(|item| *item == current)
        .map(|index| matches[(index + 1) % matches.len()])
        .or_else(|| matches.first().copied())
}

fn data_search_match_position(
    matches: &[DataSearchMatch],
    current: Option<DataSearchMatch>,
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }

    current
        .and_then(|current| {
            matches
                .iter()
                .position(|item| *item == current)
                .map(|index| index + 1)
        })
        .or(Some(1))
}

fn data_search_match_label(
    matches: &[DataSearchMatch],
    current: Option<DataSearchMatch>,
) -> String {
    match data_search_match_position(matches, current) {
        Some(position) => tr("app.filter.matches")
            .replace("%{position}", &position.to_string())
            .replace("%{count}", &matches.len().to_string()),
        None => tr("app.filter.no_matches").to_string(),
    }
}

fn local_table_filters_after_value_toggle(
    mut filters: BTreeMap<String, BTreeSet<String>>,
    field: &str,
    value: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    let values = filters.entry(field.to_string()).or_default();
    if !values.remove(value) {
        values.insert(value.to_string());
    }
    if values.is_empty() {
        filters.remove(field);
    }
    filters
}

#[cfg(test)]
mod data_filter_expression_tests {
    use super::*;

    fn rule(field: &str, value: &str, connector: fluxdb_core::FilterCombinator) -> DataFilterRule {
        DataFilterRule {
            enabled: true,
            field: Some(field.into()),
            operator: DataFilterOperator::Eq,
            values: vec![value.into()],
            advanced_expression: None,
            typed_values: None,
            connector,
            group_depth: 0,
                        group_end: 0,
        }
    }

    #[test]
    fn row_connectors_follow_sql_and_precedence() {
        let expression = data_filter_expression_from_rules(
            &[
                rule("a", "1", fluxdb_core::FilterCombinator::Or),
                rule("b", "2", fluxdb_core::FilterCombinator::And),
                rule("c", "3", fluxdb_core::FilterCombinator::And),
            ],
            &[],
        )
        .expect("expression");
        let fluxdb_core::FilterExpression::Or(items) = expression else {
            panic!("OR root");
        };
        assert!(matches!(&items[0], fluxdb_core::FilterExpression::Condition(_)));
        assert!(matches!(&items[1], fluxdb_core::FilterExpression::And(nodes) if nodes.len() == 2));
    }

    #[test]
    fn grouped_rules_keep_parentheses_inside_and_or_combinations() {
        let grouped = data_filter_expression_from_rules(
            &[
                rule("id", "1", fluxdb_core::FilterCombinator::And),
                DataFilterRule {
                    connector: fluxdb_core::FilterCombinator::Or,
                    group_depth: 1,
                        group_end: 0,
                    ..rule("status", "failed", fluxdb_core::FilterCombinator::Or)
                },
                DataFilterRule {
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 1,
                        group_end: 0,
                    ..rule("status", "ok", fluxdb_core::FilterCombinator::And)
                },
            ],
            &[],
        )
        .expect("grouped expression");
        let fluxdb_core::FilterExpression::And(items) = grouped else {
            panic!("outer AND");
        };
        assert!(matches!(&items[0], fluxdb_core::FilterExpression::Condition(_)));
        assert!(matches!(&items[1], fluxdb_core::FilterExpression::Or(nodes) if nodes.len() == 2));

        let or_with_and_branch = data_filter_expression_from_rules(
            &[
                rule("id", "1", fluxdb_core::FilterCombinator::Or),
                rule("status", "failed", fluxdb_core::FilterCombinator::And),
                rule("name", "alpha", fluxdb_core::FilterCombinator::And),
            ],
            &[],
        )
        .expect("precedence expression");
        let fluxdb_core::FilterExpression::Or(items) = or_with_and_branch else {
            panic!("outer OR");
        };
        assert!(matches!(&items[0], fluxdb_core::FilterExpression::Condition(_)));
        assert!(matches!(&items[1], fluxdb_core::FilterExpression::And(nodes) if nodes.len() == 2));

        let deeply_nested = data_filter_expression_from_rules(
            &[
                rule("id", "1", fluxdb_core::FilterCombinator::Or),
                DataFilterRule {
                    group_depth: 1,
                        group_end: 0,
                    connector: fluxdb_core::FilterCombinator::And,
                    ..rule("amount", "2", fluxdb_core::FilterCombinator::And)
                },
                DataFilterRule {
                    group_depth: 2,
                        group_end: 0,
                    connector: fluxdb_core::FilterCombinator::Or,
                    ..rule("name", "alpha", fluxdb_core::FilterCombinator::Or)
                },
                DataFilterRule {
                    group_depth: 2,
                        group_end: 0,
                    connector: fluxdb_core::FilterCombinator::And,
                    ..rule("name", "beta", fluxdb_core::FilterCombinator::And)
                },
            ],
            &[],
        )
        .expect("deeply nested expression");
        let fluxdb_core::FilterExpression::Or(items) = deeply_nested else {
            panic!("nested outer OR");
        };
        let fluxdb_core::FilterExpression::And(and_items) = &items[1] else {
            panic!("nested AND group");
        };
        assert!(matches!(&and_items[0], fluxdb_core::FilterExpression::Condition(_)));
        assert!(
            matches!(&and_items[1], fluxdb_core::FilterExpression::Or(nodes) if nodes.len() == 2)
        );
    }

    #[test]
    fn numeric_column_text_values_use_native_filter_types() {
        let columns = vec![
            GdbColumn {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            GdbColumn {
                name: "amount".into(),
                type_name: Some("numeric(12,4)".into()),
                nullable: false,
                primary_key: false,
                comment: None,
            },
        ];
        let expression = data_filter_expression_from_rules(
            &[
                rule("id", "1", fluxdb_core::FilterCombinator::And),
                rule("amount", "2.5", fluxdb_core::FilterCombinator::And),
            ],
            &columns,
        )
        .expect("expression");
        let fluxdb_core::FilterExpression::And(items) = expression else {
            panic!("AND root");
        };
        let fluxdb_core::FilterExpression::Condition(id) = &items[0] else {
            panic!("id condition");
        };
        let fluxdb_core::FilterExpression::Condition(amount) = &items[1] else {
            panic!("amount condition");
        };
        assert_eq!(id.values, vec![fluxdb_core::FilterValue::Integer(1)]);
        assert_eq!(amount.values, vec![fluxdb_core::FilterValue::Float(2.5)]);
    }

    #[test]
    fn like_operators_keep_numeric_column_values_as_text() {
        let columns = vec![GdbColumn {
            name: "id".into(),
            type_name: Some("integer".into()),
            nullable: false,
            primary_key: true,
            comment: None,
        }];
        for (operator, expected) in [
            (DataFilterOperator::Contains, fluxdb_core::FilterOperator::Contains),
            (
                DataFilterOperator::NotContains,
                fluxdb_core::FilterOperator::NotContains,
            ),
            (
                DataFilterOperator::StartsWith,
                fluxdb_core::FilterOperator::StartsWith,
            ),
            (
                DataFilterOperator::NotStartsWith,
                fluxdb_core::FilterOperator::NotStartsWith,
            ),
            (
                DataFilterOperator::EndsWith,
                fluxdb_core::FilterOperator::EndsWith,
            ),
            (
                DataFilterOperator::NotEndsWith,
                fluxdb_core::FilterOperator::NotEndsWith,
            ),
        ] {
            let expression = data_filter_expression_from_rules(
                &[DataFilterRule {
                    enabled: true,
                    field: Some("id".into()),
                    operator,
                    values: vec!["001".into()],
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                        group_end: 0,
                }],
                &columns,
            )
            .expect("expression");
            let fluxdb_core::FilterExpression::Condition(condition) = &expression else {
                panic!("{operator:?} must produce a condition");
            };
            assert_eq!(condition.op, expected);
            assert_eq!(
                condition.values,
                vec![fluxdb_core::FilterValue::Text("001".into())],
                "{operator:?}"
            );
        }
    }

    #[test]
    fn array_columns_do_not_coerce_text_to_scalar_numbers() {
        let columns = vec![GdbColumn {
            name: "amounts".into(),
            type_name: Some("numeric(12,4)[]".into()),
            nullable: false,
            primary_key: false,
            comment: None,
        }];
        let expression = data_filter_expression_from_rules(
            &[rule("amounts", "1.5", fluxdb_core::FilterCombinator::And)],
            &columns,
        )
        .expect("expression");
        let fluxdb_core::FilterExpression::Condition(condition) = &expression else {
            panic!("array condition");
        };
        assert_eq!(
            condition.values,
            vec![fluxdb_core::FilterValue::Text("1.5".into())]
        );
    }

    #[test]
    fn every_operator_round_trips_between_form_and_input_for_each_database() {
        let _ = fluxdb_app::AppController::new();
        let columns = vec![
            GdbColumn {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            GdbColumn {
                name: "name".into(),
                type_name: Some("varchar(128)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let kinds = [
            fluxdb_core::DatabaseKind::MySql,
            fluxdb_core::DatabaseKind::TiDb,
            fluxdb_core::DatabaseKind::Postgres,
            fluxdb_core::DatabaseKind::Sqlite,
            fluxdb_core::DatabaseKind::SqlServer,
            fluxdb_core::DatabaseKind::Dameng,
        ];

        for kind in kinds {
            let dialect = fluxdb_core::filter_dialect_for(kind).expect("SQL filter dialect");
            for operator in DataFilterOperator::all() {
                let (field, values) = match operator {
                    DataFilterOperator::Contains
                    | DataFilterOperator::NotContains
                    | DataFilterOperator::StartsWith
                    | DataFilterOperator::NotStartsWith
                    | DataFilterOperator::EndsWith
                    | DataFilterOperator::NotEndsWith
                    | DataFilterOperator::InList
                    | DataFilterOperator::NotInList => ("name", vec!["AD", "Andorra"]),
                    DataFilterOperator::Between | DataFilterOperator::NotBetween => {
                        ("id", vec!["1", "3"])
                    }
                    DataFilterOperator::IsNull
                    | DataFilterOperator::IsNotNull
                    | DataFilterOperator::IsEmpty
                    | DataFilterOperator::IsNotEmpty => ("name", Vec::new()),
                    DataFilterOperator::Eq
                    | DataFilterOperator::Ne
                    | DataFilterOperator::Lt
                    | DataFilterOperator::Le
                    | DataFilterOperator::Gt
                    | DataFilterOperator::Ge => ("id", vec!["2"]),
                };
                let values = if matches!(
                    operator,
                    DataFilterOperator::Contains
                        | DataFilterOperator::NotContains
                        | DataFilterOperator::StartsWith
                        | DataFilterOperator::NotStartsWith
                        | DataFilterOperator::EndsWith
                        | DataFilterOperator::NotEndsWith
                ) {
                    vec![values[0].to_string()]
                } else {
                    values.into_iter().map(str::to_string).collect()
                };
                let rule = DataFilterRule {
                    enabled: true,
                    field: Some(field.into()),
                    operator: *operator,
                    values,
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                        group_end: 0,
                };
                let expression =
                    data_filter_expression_from_rules(std::slice::from_ref(&rule), &columns)
                        .expect("form expression");
                let text = dialect
                    .render_with_columns(&expression, &columns)
                    .expect("form SQL");
                let parsed = dialect.parse(&text).expect("input SQL");
                let (roundtrip, connector) = data_filter_rules_from_expression(
                    &parsed.expression,
                    |expression| {
                        dialect
                            .render_with_columns(expression, &columns)
                            .unwrap_or_default()
                    },
                );
                assert_eq!(
                    connector,
                    fluxdb_core::FilterCombinator::And,
                    "{kind:?} {operator:?}"
                );
                assert_eq!(
                    roundtrip.len(),
                    1,
                    "{kind:?} {operator:?}: {text}"
                );
                assert_eq!(
                    roundtrip[0].field.as_deref(),
                    rule.field.as_deref(),
                    "{kind:?} {operator:?}: {text}"
                );
                assert_eq!(
                    roundtrip[0].operator, rule.operator,
                    "{kind:?} {operator:?}: {text}"
                );
                assert_eq!(
                    roundtrip[0].values, rule.values,
                    "{kind:?} {operator:?}: {text}"
                );
                let roundtrip_expression =
                    data_filter_expression_from_rules(&roundtrip, &columns)
                        .expect("round-trip expression");
                assert_eq!(
                    roundtrip_expression, expression,
                    "{kind:?} {operator:?}: {text}"
                );
            }
        }
    }

    #[test]
    fn grouped_multi_condition_form_input_round_trip_preserves_each_dialect_sql() {
        let _ = fluxdb_app::AppController::new();
        let columns = vec![
            GdbColumn {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            GdbColumn {
                name: "amount".into(),
                type_name: Some("numeric(12,4)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
            GdbColumn {
                name: "name".into(),
                type_name: Some("varchar(128)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let rules = vec![
            DataFilterRule {
                enabled: true,
                field: Some("id".into()),
                operator: DataFilterOperator::Gt,
                values: vec!["1".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 0,
                        group_end: 0,
            },
            DataFilterRule {
                enabled: true,
                field: Some("amount".into()),
                operator: DataFilterOperator::Eq,
                values: vec!["2".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::Or,
                group_depth: 1,
                        group_end: 0,
            },
            DataFilterRule {
                enabled: true,
                field: Some("name".into()),
                operator: DataFilterOperator::Contains,
                values: vec!["a".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 1,
                        group_end: 0,
            },
        ];
        for kind in [
            fluxdb_core::DatabaseKind::MySql,
            fluxdb_core::DatabaseKind::TiDb,
            fluxdb_core::DatabaseKind::Postgres,
            fluxdb_core::DatabaseKind::Sqlite,
            fluxdb_core::DatabaseKind::SqlServer,
            fluxdb_core::DatabaseKind::Dameng,
        ] {
            let dialect = fluxdb_core::filter_dialect_for(kind).expect("SQL filter dialect");
            let expression =
                data_filter_expression_from_rules(&rules, &columns).expect("form expression");
            let rendered = dialect
                .render_with_columns(&expression, &columns)
                .expect("form SQL");
            let parsed = dialect.parse(&rendered).expect("input SQL");
            let (roundtrip, connector) = data_filter_rules_from_expression(
                &parsed.expression,
                |expression| {
                    dialect
                        .render_with_columns(expression, &columns)
                        .unwrap_or_default()
                },
            );
            assert_eq!(connector, fluxdb_core::FilterCombinator::And, "{kind:?}");
            assert_eq!(roundtrip.len(), 3, "{kind:?}");
            assert!(
                roundtrip
                    .iter()
                    .all(|rule| rule.advanced_expression.is_none()),
                "{kind:?}"
            );
            assert_eq!(
                roundtrip
                    .iter()
                    .map(|rule| rule.group_depth)
                    .collect::<Vec<_>>(),
                vec![0, 1, 1],
                "{kind:?}"
            );
            assert_eq!(
                roundtrip
                    .iter()
                    .map(|rule| rule.connector)
                    .collect::<Vec<_>>(),
                vec![
                    fluxdb_core::FilterCombinator::And,
                    fluxdb_core::FilterCombinator::Or,
                    fluxdb_core::FilterCombinator::And,
                ],
                "{kind:?}"
            );
            let expression_again = data_filter_expression_from_rules(&roundtrip, &columns)
                .expect("round-trip expression");
            assert_eq!(
                dialect
                    .render_with_columns(&expression_again, &columns)
                    .expect("round-trip SQL"),
                rendered,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn multi_condition_projection_keeps_supported_groups_and_falls_back_for_deep_trees() {
        let _ = fluxdb_app::AppController::new();
        let dialect = fluxdb_core::filter_dialect_for(fluxdb_core::DatabaseKind::MySql)
            .expect("MySQL filter dialect");
        let project = |text: &str| {
            let parsed = dialect.parse(text).expect("parse expression");
            data_filter_rules_from_expression(&parsed.expression, |expression| {
                dialect.render(expression).unwrap_or_default()
            })
            .0
        };

        let supported = project("id = 1 OR (amount = 2 AND name = 'alpha')");
        assert_eq!(supported.len(), 3);
        assert_eq!(
            supported
                .iter()
                .map(|rule| rule.field.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("id"), Some("amount"), Some("name")]
        );
        assert_eq!(
            supported
                .iter()
                .map(|rule| rule.group_depth)
                .collect::<Vec<_>>(),
            vec![0, 1, 1]
        );
        assert_eq!(
            supported
                .iter()
                .map(|rule| rule.connector)
                .collect::<Vec<_>>(),
            vec![
                fluxdb_core::FilterCombinator::Or,
                fluxdb_core::FilterCombinator::And,
                fluxdb_core::FilterCombinator::Or,
            ]
        );

        let adjacent_groups = project("(id = 1 OR amount = 2) AND (name = 'a' OR name = 'b')");
        assert_eq!(adjacent_groups.len(), 4);
        assert!(
            adjacent_groups
                .iter()
                .all(|rule| rule.advanced_expression.is_none())
        );
        assert_eq!(
            adjacent_groups
                .iter()
                .map(|rule| (rule.group_depth, rule.group_end))
                .collect::<Vec<_>>(),
            vec![(1, 0), (1, 1), (1, 0), (1, 1)]
        );

        let nested = project("id = 1 OR (amount = 2 AND (name = 'a' OR name = 'b'))");
        assert_eq!(nested.len(), 4);
        assert!(nested.iter().all(|rule| rule.advanced_expression.is_none()));
        assert_eq!(
            nested
                .iter()
                .map(|rule| rule.group_depth)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 2]
        );
        assert_eq!(
            nested
                .iter()
                .map(|rule| rule.connector)
                .collect::<Vec<_>>(),
            vec![
                fluxdb_core::FilterCombinator::Or,
                fluxdb_core::FilterCombinator::And,
                fluxdb_core::FilterCombinator::Or,
                fluxdb_core::FilterCombinator::Or,
            ]
        );

        let issue_sql = "(`id` = 1 OR ((`signup_date` < '2026-01-05' AND `signup_date` >= '2026-04-19') OR (`name` LIKE '%等等%' AND `id` = 1)))";
        let issue_rules = project(issue_sql);
        assert_eq!(issue_rules.len(), 5);
        assert!(
            issue_rules
                .iter()
                .all(|rule| rule.advanced_expression.is_none())
        );
        assert_eq!(
            issue_rules
                .iter()
                .map(|rule| (rule.group_depth, rule.group_end))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 0), (1, 1), (1, 0), (1, 1)]
        );
        let mut columns = vec![GdbColumn {
            name: "id".into(),
            type_name: Some("integer".into()),
            nullable: false,
            primary_key: true,
            comment: None,
        }];
        for name in ["signup_date", "name"] {
            columns.push(GdbColumn {
                name: name.into(),
                type_name: Some(if name == "name" {
                    "varchar(128)".into()
                } else {
                    "date".into()
                }),
                nullable: true,
                primary_key: false,
                comment: None,
            });
        }
        let regenerated_expression = data_filter_expression_from_rules(&issue_rules, &columns)
            .expect("regenerated issue expression");
        let dialect = fluxdb_core::filter_dialect_for(fluxdb_core::DatabaseKind::MySql)
            .expect("MySQL filter dialect");
        let regenerated = dialect
            .render_with_columns(&regenerated_expression, &columns)
            .expect("rendered issue expression");
        assert!(regenerated.contains("`signup_date` <"));
        assert!(regenerated.contains("`name` LIKE '%等等%'"));
        assert!(regenerated.starts_with("(`id` = 1 OR"));
    }

    #[test]
    fn two_groups_separated_by_condition_round_trip_for_each_database() {
        let _ = fluxdb_app::AppController::new();
        let columns = vec![
            GdbColumn {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            GdbColumn {
                name: "amount".into(),
                type_name: Some("numeric(12,4)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let rules = vec![
            DataFilterRule {
                enabled: true,
                field: Some("id".into()),
                operator: DataFilterOperator::Eq,
                values: vec!["1".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::Or,
                group_depth: 1,
                        group_end: 0,
            },
            DataFilterRule {
                enabled: true,
                field: Some("amount".into()),
                operator: DataFilterOperator::Eq,
                values: vec!["2".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 1,
                        group_end: 0,
            },
            DataFilterRule {
                enabled: true,
                field: Some("amount".into()),
                operator: DataFilterOperator::Ge,
                values: vec!["2".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 0,
                        group_end: 0,
            },
            DataFilterRule {
                enabled: true,
                field: Some("id".into()),
                operator: DataFilterOperator::Eq,
                values: vec!["4".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::Or,
                group_depth: 1,
                        group_end: 0,
            },
            DataFilterRule {
                enabled: true,
                field: Some("id".into()),
                operator: DataFilterOperator::Eq,
                values: vec!["1".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 1,
                        group_end: 0,
            },
        ];
        for kind in [
            fluxdb_core::DatabaseKind::MySql,
            fluxdb_core::DatabaseKind::TiDb,
            fluxdb_core::DatabaseKind::Postgres,
            fluxdb_core::DatabaseKind::Sqlite,
            fluxdb_core::DatabaseKind::SqlServer,
            fluxdb_core::DatabaseKind::Dameng,
        ] {
            let dialect = fluxdb_core::filter_dialect_for(kind).expect("SQL filter dialect");
            let expression =
                data_filter_expression_from_rules(&rules, &columns).expect("form expression");
            let rendered = dialect
                .render_with_columns(&expression, &columns)
                .expect("form SQL");
            let parsed = dialect.parse(&rendered).expect("input SQL");
            let (roundtrip, connector) = data_filter_rules_from_expression(
                &parsed.expression,
                |expression| {
                    dialect
                        .render_with_columns(expression, &columns)
                        .unwrap_or_default()
                },
            );
            assert_eq!(connector, fluxdb_core::FilterCombinator::And, "{kind:?}");
            assert_eq!(roundtrip.len(), 5, "{kind:?}");
            assert!(
                roundtrip
                    .iter()
                    .all(|rule| rule.advanced_expression.is_none()),
                "{kind:?}"
            );
            assert_eq!(
                roundtrip
                    .iter()
                    .map(|rule| rule.group_depth)
                    .collect::<Vec<_>>(),
                vec![1, 1, 0, 1, 1],
                "{kind:?}"
            );
            assert_eq!(
                roundtrip
                    .iter()
                    .map(|rule| rule.connector)
                    .collect::<Vec<_>>(),
                vec![
                    fluxdb_core::FilterCombinator::Or,
                    fluxdb_core::FilterCombinator::And,
                    fluxdb_core::FilterCombinator::And,
                    fluxdb_core::FilterCombinator::Or,
                    fluxdb_core::FilterCombinator::And,
                ],
                "{kind:?}"
            );
            let expression_again = data_filter_expression_from_rules(&roundtrip, &columns)
                .expect("round-trip expression");
            assert_eq!(
                dialect
                    .render_with_columns(&expression_again, &columns)
                    .expect("round-trip SQL"),
                rendered,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn advanced_expression_combined_with_group_round_trips() {
        let _ = fluxdb_app::AppController::new();
        let columns = vec![
            GdbColumn {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            GdbColumn {
                name: "amount".into(),
                type_name: Some("numeric(12,4)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        for kind in [
            fluxdb_core::DatabaseKind::MySql,
            fluxdb_core::DatabaseKind::Postgres,
            fluxdb_core::DatabaseKind::Sqlite,
            fluxdb_core::DatabaseKind::SqlServer,
            fluxdb_core::DatabaseKind::Dameng,
        ] {
            let dialect = fluxdb_core::filter_dialect_for(kind).expect("SQL filter dialect");
            let parsed = dialect
                .parse("lower(name) = 'alpha' AND (id = 1 OR amount = 2)")
                .expect("advanced expression");
            let (roundtrip, connector) = data_filter_rules_from_expression(
                &parsed.expression,
                |expression| {
                    dialect
                        .render_with_columns(expression, &columns)
                        .unwrap_or_default()
                },
            );
            assert_eq!(connector, fluxdb_core::FilterCombinator::And, "{kind:?}");
            assert_eq!(roundtrip.len(), 3, "{kind:?}");
            assert!(roundtrip[0].advanced_expression.is_some(), "{kind:?}");
            assert_eq!(
                roundtrip
                    .iter()
                    .map(|rule| rule.group_depth)
                    .collect::<Vec<_>>(),
                vec![0, 1, 1],
                "{kind:?}"
            );
            let expression = data_filter_expression_from_rules(&roundtrip, &columns)
                .expect("round-trip expression");
            let rendered = dialect
                .render_with_columns(&expression, &columns)
                .expect("round-trip SQL");
            assert_eq!(rendered, parsed.text, "{kind:?}");
        }
    }

    fn filter_rule_with(operator: DataFilterOperator, values: &[&str]) -> DataFilterRule {
        DataFilterRule {
            operator,
            values: values.iter().map(|value| value.to_string()).collect(),
            ..rule("old_field", "", fluxdb_core::FilterCombinator::And)
        }
    }

    #[test]
    fn field_switch_keeps_values_compatible_with_new_column_type() {
        // 先填值后选字段：新字段是文本列，值原样保留（前导零不能丢）。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "biz_id".into(),
            DataFilterRule {
                field: None,
                ..filter_rule_with(DataFilterOperator::Eq, &["007"])
            },
            Some("varchar(64)"),
        );
        assert_eq!(switched.field.as_deref(), Some("biz_id"));
        assert_eq!(switched.values, vec!["007"]);
        assert_eq!((kept, dropped), (1, 0));

        // 数字列接受可解析值；旧列解析出的 typed_values 必须丢弃，按新列重新定型。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "id".into(),
            DataFilterRule {
                typed_values: Some(vec![fluxdb_core::FilterValue::Text("1".into())]),
                ..filter_rule_with(DataFilterOperator::Eq, &["1"])
            },
            Some("bigint"),
        );
        assert_eq!(switched.values, vec!["1"]);
        assert!(switched.typed_values.is_none());
        assert_eq!((kept, dropped), (1, 0));

        // 类型未知（页面未加载/自定义类型）按文本处理，不丢用户输入。
        let (switched, _, dropped) = data_filter_rule_after_field_switch(
            "custom".into(),
            filter_rule_with(DataFilterOperator::Eq, &["abc"]),
            None,
        );
        assert_eq!(switched.values, vec!["abc"]);
        assert_eq!(dropped, 0);

        // LIKE 系列把值当模式文本，数字列也不做数字校验。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "id".into(),
            filter_rule_with(DataFilterOperator::Contains, &["001"]),
            Some("integer"),
        );
        assert_eq!(switched.values, vec!["001"]);
        assert_eq!((kept, dropped), (1, 0));
    }

    #[test]
    fn field_switch_drops_values_incompatible_with_new_column_type() {
        // 文本 -> 整数：只保留能解析的值。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "id".into(),
            filter_rule_with(DataFilterOperator::InList, &["123", "abc", "7.5"]),
            Some("bigint"),
        );
        assert_eq!(switched.values, vec!["123"]);
        assert_eq!((kept, dropped), (1, 2));

        // 文本 -> 布尔：只认 true/1/false/0（忽略大小写），空值同样丢弃。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "enabled".into(),
            filter_rule_with(DataFilterOperator::InList, &["TRUE", "0", "yes", ""]),
            Some("boolean"),
        );
        assert_eq!(switched.values, vec!["TRUE", "0"]);
        assert_eq!((kept, dropped), (2, 2));

        // 单值全部不兼容：清空并提示（由调用方按 dropped 决定文案）。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "amount".into(),
            filter_rule_with(DataFilterOperator::Eq, &["abc"]),
            Some("numeric(12,4)"),
        );
        assert!(switched.values.is_empty());
        assert_eq!((kept, dropped), (0, 1));
    }

    #[test]
    fn field_switch_clears_between_when_any_bound_is_incompatible() {
        // 区间缺一个可用的边界就不成立：整体清空，而不是留下单边时刻。
        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "amount".into(),
            filter_rule_with(DataFilterOperator::Between, &["1", "abc"]),
            Some("numeric(12,4)"),
        );
        assert!(switched.values.is_empty());
        assert_eq!((kept, dropped), (0, 2));

        let (switched, kept, dropped) = data_filter_rule_after_field_switch(
            "amount".into(),
            filter_rule_with(DataFilterOperator::Between, &["1", "9.5"]),
            Some("numeric(12,4)"),
        );
        assert_eq!(switched.values, vec!["1", "9.5"]);
        assert_eq!((kept, dropped), (2, 0));
    }
}
