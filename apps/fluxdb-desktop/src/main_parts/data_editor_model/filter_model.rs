#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataFilterPopoverKind {
    Field,
    Operator,
    Value,
    SortField,
    SortMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DataFilterPopover {
    tab_id: TabId,
    kind: DataFilterPopoverKind,
    rule_index: Option<usize>,
    sort_index: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DataFilterRule {
    enabled: bool,
    field: Option<String>,
    operator: DataFilterOperator,
    values: BTreeSet<String>,
    grouped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DataSortRule {
    enabled: bool,
    field: String,
    ascending: bool,
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
            values: BTreeSet::new(),
            grouped: false,
        }
    }
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
    fn label(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Contains => "包含",
            Self::NotContains => "不包含",
            Self::StartsWith => "开头是",
            Self::NotStartsWith => "开头不是",
            Self::EndsWith => "结尾是",
            Self::NotEndsWith => "结尾不是",
            Self::IsNull => "是 null",
            Self::IsNotNull => "不是 null",
            Self::IsEmpty => "是空的",
            Self::IsNotEmpty => "是非空的",
            Self::Between => "介于",
            Self::NotBetween => "不介于",
            Self::InList => "在列表",
            Self::NotInList => "不在列表",
        }
    }

    fn requires_values(self) -> bool {
        !matches!(
            self,
            Self::IsNull | Self::IsNotNull | Self::IsEmpty | Self::IsNotEmpty
        )
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

fn data_search_matches(rows: &[Vec<SharedString>], query: &str) -> Vec<DataSearchMatch> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    rows.iter()
        .enumerate()
        .flat_map(|(row_ix, row)| {
            let query = query.clone();
            row.iter().enumerate().filter_map(move |(col_ix, value)| {
                value
                    .to_ascii_lowercase()
                    .contains(query.as_str())
                    .then_some(DataSearchMatch {
                        row_ix,
                        col_ix: col_ix + 1,
                    })
            })
        })
        .collect()
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
        Some(position) => format!("{position}/{} 匹配", matches.len()),
        None => "0/0 匹配".to_string(),
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
