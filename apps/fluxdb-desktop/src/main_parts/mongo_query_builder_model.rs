// MongoDB 条件构建器的无损表达式树。
//
// 逻辑组与字段条件分开建模，避免把嵌套条件平铺成对象时覆盖同名字段。
// 只接受构建器能完整表达的 JSON；其他表达式保留在 Filter 中，不能猜测改写。

/// 条件组的连接方式，与 BSON 的组级运算符一一对应。
///
/// `ElemMatch` / `NotElemMatch` 与另外三种不同：它们不是「连接条件」，
/// 而是把条件的作用范围收窄到某个数组字段的单个元素上，因此组自带字段路径。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MongoBuilderLogic {
    And,
    Or,
    Nor,
    ElemMatch,
    NotElemMatch,
}

impl MongoBuilderLogic {
    const ALL: [Self; 5] = [
        Self::And,
        Self::Or,
        Self::Nor,
        Self::ElemMatch,
        Self::NotElemMatch,
    ];

    /// 组级 JSON 运算符；数组组不直接产出运算符，由 `field` 包一层。
    fn json_operator(self) -> Option<&'static str> {
        match self {
            Self::And => Some("$and"),
            Self::Or => Some("$or"),
            Self::Nor => Some("$nor"),
            Self::ElemMatch | Self::NotElemMatch => None,
        }
    }

    /// 是否需要数组字段路径（`{ 字段: { $elemMatch: … } }`）。
    fn is_array(self) -> bool {
        matches!(self, Self::ElemMatch | Self::NotElemMatch)
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::And => "app.mongo.query.builder_match_all",
            Self::Or => "app.mongo.query.builder_match_any",
            Self::Nor => "app.mongo.query.builder_match_none",
            Self::ElemMatch => "app.mongo.query.builder_match_element",
            Self::NotElemMatch => "app.mongo.query.builder_match_no_element",
        }
    }

    /// 分段控件上的短标签，全局唯一，也是无头用例的选择器片段。
    fn short_label(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
            Self::Nor => "NOR",
            Self::ElemMatch => "ELEM",
            Self::NotElemMatch => "NOT·ELEM",
        }
    }

    fn selector(self) -> &'static str {
        match self {
            Self::And => "and",
            Self::Or => "or",
            Self::Nor => "nor",
            Self::ElemMatch => "elem",
            Self::NotElemMatch => "not_elem",
        }
    }
}

#[derive(Clone, Debug)]
struct MongoBuilderCondition {
    /// 字段路径；空字符串表示「数组元素本身」，只在数组组里有意义。
    field: String,
    operator: MongoQueryOperator,
    value: String,
    /// 解析/序列化的纯模型为 None；UI 叶节点持有输入框与订阅。
    row: Option<Rc<MongoQueryBuilderRow>>,
}

impl PartialEq for MongoBuilderCondition {
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field && self.operator == other.operator && self.value == other.value
    }
}

#[derive(Clone, Debug, PartialEq)]
enum MongoBuilderNode {
    Condition(MongoBuilderCondition),
    Group(MongoBuilderGroup),
}

#[derive(Clone, Debug)]
struct MongoBuilderGroup {
    logic: MongoBuilderLogic,
    /// 数组组的字段路径；其余组恒为空。
    field: String,
    /// 子组是否展开。根组恒展开；子组默认收起成一行预览。
    open: bool,
    children: Vec<MongoBuilderNode>,
    /// 数组组的字段输入框；纯模型为 None。
    field_input: Option<Rc<MongoQueryBuilderFieldCell>>,
}

/// 只比结构与字段：输入框实体是指针，不能进相等判断。
impl PartialEq for MongoBuilderGroup {
    fn eq(&self, other: &Self) -> bool {
        self.logic == other.logic
            && self.field == other.field
            && self.open == other.open
            && self.children == other.children
    }
}

impl MongoBuilderGroup {
    fn new(logic: MongoBuilderLogic) -> Self {
        Self {
            logic,
            field: String::new(),
            open: true,
            children: Vec::new(),
            field_input: None,
        }
    }

    fn group_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(|node| match node {
                MongoBuilderNode::Condition(_) => 0,
                MongoBuilderNode::Group(group) => group.group_count(),
            })
            .sum::<usize>()
    }

    fn condition_count(&self) -> usize {
        self.children
            .iter()
            .map(|node| match node {
                MongoBuilderNode::Condition(_) => 1,
                MongoBuilderNode::Group(group) => group.condition_count(),
            })
            .sum()
    }
}

/// 构建器控件有界，避免极端 JSON 生成成千上万个编辑器或无限递归。
///
/// 只按**总数**限：组内子项不限，条件树自己封顶 240px 并滚动，
/// 再限制每组条数只会让「添加条件」提前置灰。
const MONGO_QUERY_BUILDER_MAX_DEPTH: usize = 4;
const MONGO_QUERY_BUILDER_MAX_CONDITIONS: usize = 24;
const MONGO_QUERY_BUILDER_MAX_GROUPS: usize = 8;

fn parse_mongo_builder_filter(text: &str) -> Result<MongoBuilderGroup, SharedString> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(MongoBuilderGroup::new(MongoBuilderLogic::And));
    }
    let value = parse_mongo_relaxed_json(trimmed)
        .ok_or_else(|| tr("app.mongo.query.builder_json_error"))?;
    let serde_json::Value::Object(document) = value else {
        return Err(tr("app.mongo.query.builder_json_error"));
    };
    let mut conditions = 0;
    let mut groups = 1;
    let mut group = parse_mongo_builder_document(&document, 0, false, &mut conditions, &mut groups)?;
    // 顶层只有一个组时把它提升成根本身：根 AND 再包一个组只是多一层括号，
    // 数组组还得把字段一起提上来，否则界面会显示「根 AND 里有个没字段的 ELEM 组」。
    if document.len() == 1
        && group.logic == MongoBuilderLogic::And
        && let [MongoBuilderNode::Group(only)] = group.children.as_slice()
    {
        group.logic = only.logic;
        group.field = only.field.clone();
        group.children = only.children.clone();
    }
    Ok(group)
}

/// 文档 → 条件组。
///
/// `inside_elem` 表示当前文档是 `$elemMatch` 的内层：这时以 `$` 开头的键
/// 表示「数组元素自身上的运算符」（`{ $gt: 2 }`），字段路径留空。
fn parse_mongo_builder_document(
    document: &serde_json::Map<String, serde_json::Value>,
    depth: usize,
    inside_elem: bool,
    conditions: &mut usize,
    groups: &mut usize,
) -> Result<MongoBuilderGroup, SharedString> {
    let complex = || tr("app.mongo.query.builder_complex_error");
    if depth >= MONGO_QUERY_BUILDER_MAX_DEPTH {
        return Err(complex());
    }
    let mut group = MongoBuilderGroup::new(MongoBuilderLogic::And);
    for (field, expression) in document {
        if let Some(logic) = match field.as_str() {
            "$and" => Some(MongoBuilderLogic::And),
            "$or" => Some(MongoBuilderLogic::Or),
            "$nor" => Some(MongoBuilderLogic::Nor),
            _ => None,
        } {
            let serde_json::Value::Array(items) = expression else {
                return Err(complex());
            };
            if items.is_empty() || (items.len() > 1 && items.iter().any(|item| item.as_object().is_some_and(|document| document.is_empty()))) {
                return Err(complex());
            }
            let mut nested = MongoBuilderGroup::new(logic);
            for item in items {
                let serde_json::Value::Object(document) = item else {
                    return Err(complex());
                };
                let child =
                    parse_mongo_builder_document(document, depth + 1, inside_elem, conditions, groups)?;
                // 同逻辑单条件可摊平；重复字段仍须保留子组，避免 AND 平铺后覆盖。
                let can_flatten = child.logic == logic
                    && child.children.len() == 1
                    && match &child.children[0] {
                        MongoBuilderNode::Condition(condition) => nested.children.iter().all(
                            |node| !matches!(node, MongoBuilderNode::Condition(existing) if existing.field == condition.field),
                        ),
                        // 子组可能已含冲突字段；内部保留层级比检查整棵树更直接。
                        MongoBuilderNode::Group(_) => false,
                    };
                if can_flatten {
                    nested.children.extend(child.children);
                } else {
                    *groups += 1;
                    if *groups > MONGO_QUERY_BUILDER_MAX_GROUPS {
                        return Err(complex());
                    }
                    nested.children.push(MongoBuilderNode::Group(child));
                }
            }
            if group.children.is_empty() {
                // 根组初始为 AND，但顶层 $or / $nor 必须切换根逻辑；
                // 否则同名字段会被 AND 平铺覆盖，或语义整个反过来。
                group.logic = logic;
                group.children = nested.children;
            } else {
                *groups += 1;
                if *groups > MONGO_QUERY_BUILDER_MAX_GROUPS {
                    return Err(complex());
                }
                group.children.push(MongoBuilderNode::Group(nested));
            }
            continue;
        }
        // 数组元素自身上的运算符，例如 `{ $gt: 2 }`。
        //
        // `$eq` 单独接：它没有对应的 JSON 运算符（等值在字段上是裸值），
        // 但在元素自身的位置上只能写成 `{ $eq: 2 }`。
        if inside_elem && field.starts_with('$') {
            let operator = if field == "$eq" {
                MongoQueryOperator::Eq
            } else {
                MongoQueryOperator::from_json_operator(field).ok_or_else(complex)?
            };
            let value = parse_mongo_builder_value(operator, expression).ok_or_else(complex)?;
            *conditions += 1;
            if *conditions > MONGO_QUERY_BUILDER_MAX_CONDITIONS {
                return Err(complex());
            }
            group.children.push(MongoBuilderNode::Condition(MongoBuilderCondition {
                field: String::new(),
                operator,
                value,
                row: None,
            }));
            continue;
        }
        if field.starts_with('$') {
            return Err(complex());
        }
        // 数组组：`{ 字段: { $elemMatch: {…} } }` / `{ 字段: { $not: { $elemMatch: … } } }`
        if let Some((logic, inner)) = mongo_builder_elem_match(expression) {
            let child = parse_mongo_builder_document(inner, depth + 1, true, conditions, groups)?;
            *groups += 1;
            if *groups > MONGO_QUERY_BUILDER_MAX_GROUPS {
                return Err(complex());
            }
            let mut nested = MongoBuilderGroup::new(logic);
            nested.field = field.clone();
            nested.open = false;
            nested.children = child.children;
            group.children.push(MongoBuilderNode::Group(nested));
            continue;
        }
        let (operator, value) = match expression {
            serde_json::Value::Object(wrapper) => {
                let mut entries = wrapper.iter();
                let (Some((key, value)), None) = (entries.next(), entries.next()) else {
                    return Err(complex());
                };
                let Some(operator) = MongoQueryOperator::from_json_operator(key) else {
                    return Err(complex());
                };
                (operator, value)
            }
            other => (MongoQueryOperator::Eq, other),
        };
        let value = parse_mongo_builder_value(operator, value).ok_or_else(complex)?;
        *conditions += 1;
        if *conditions > MONGO_QUERY_BUILDER_MAX_CONDITIONS {
            return Err(complex());
        }
        group.children.push(MongoBuilderNode::Condition(MongoBuilderCondition {
            field: field.clone(),
            operator,
            value,
            row: None,
        }));
    }
    Ok(group)
}

/// `{ $elemMatch: … }` 或 `{ $not: { $elemMatch: … } }` 的内层文档。
fn mongo_builder_elem_match(
    expression: &serde_json::Value,
) -> Option<(MongoBuilderLogic, &serde_json::Map<String, serde_json::Value>)> {
    let wrapper = expression.as_object()?;
    if let Some(inner) = wrapper.get("$elemMatch").and_then(|inner| inner.as_object())
        && wrapper.len() == 1
    {
        return Some((MongoBuilderLogic::ElemMatch, inner));
    }
    let negated = wrapper.get("$not")?.as_object()?;
    if wrapper.len() == 1 && negated.len() == 1 {
        return Some((
            MongoBuilderLogic::NotElemMatch,
            negated.get("$elemMatch")?.as_object()?,
        ));
    }
    None
}

/// 运算符 + JSON 值 → 输入框文本；返回 `None` 表示这个值构建器表达不了。
fn parse_mongo_builder_value(
    operator: MongoQueryOperator,
    value: &serde_json::Value,
) -> Option<String> {
    match (operator, value) {
        (MongoQueryOperator::Exists, value) if !value.is_boolean() => None,
        (MongoQueryOperator::In, serde_json::Value::Array(items)) => {
            Some(items.iter().map(format_mongo_builder_scalar).collect::<Vec<_>>().join(", "))
        }
        (_, value) if value.is_array() || value.is_object() => None,
        (_, value) => Some(format_mongo_builder_scalar(value)),
    }
}

/// 表达式树写回 Filter。无冲突的 AND 合并为普通对象；有同名字段时用 `$and`
/// 数组保留每一项，OR / NOR 始终用各自的运算符，不能靠后写覆盖先写的条件。
fn mongo_builder_group_json(group: &MongoBuilderGroup) -> serde_json::Value {
    if group.logic.is_array() {
        return mongo_builder_elem_group_json(group);
    }
    let children: Vec<serde_json::Value> = group
        .children
        .iter()
        .filter_map(|node| match node {
            MongoBuilderNode::Condition(condition) => {
                mongo_builder_condition_json(condition, false)
            }
            // 空组不产出条件。数组组缺字段时同样返回 `{}`，必须在这里剔掉——
            // 留着它进 `$or` 会变成「匹配全部」，把整条过滤条件的意思反过来。
            MongoBuilderNode::Group(group) => {
                let child = mongo_builder_group_json(group);
                (!child.as_object().is_some_and(|child| child.is_empty())).then_some(child)
            }
        })
        .collect();
    if children.is_empty() {
        return serde_json::Value::Object(serde_json::Map::new());
    }
    match group.logic {
        MongoBuilderLogic::And => mongo_builder_merge_json(children),
        logic => {
            let mut document = serde_json::Map::new();
            document.insert(
                logic.json_operator().unwrap_or("$and").to_string(),
                serde_json::Value::Array(children),
            );
            serde_json::Value::Object(document)
        }
    }
}

/// AND 组的合并：键不冲突就摊平成普通对象，冲突的项溢出进 `$and`。
fn mongo_builder_merge_json(children: Vec<serde_json::Value>) -> serde_json::Value {
    let mut document = serde_json::Map::new();
    let can_merge = children.iter().all(|child| {
        let can_insert = child.as_object().is_some_and(|child| {
            child.len() == 1
                && !child.contains_key("$and")
                && child.keys().all(|key| !document.contains_key(key))
        });
        if can_insert && let serde_json::Value::Object(child) = child {
            document.extend(child.clone());
        }
        can_insert
    });
    if can_merge {
        for child in children {
            if let serde_json::Value::Object(child) = child {
                document.extend(child);
            }
        }
        return serde_json::Value::Object(document);
    }
    if children.len() == 1 {
        return children.into_iter().next().unwrap_or_else(|| {
            serde_json::Value::Object(serde_json::Map::new())
        });
    }
    let mut document = serde_json::Map::new();
    document.insert("$and".to_string(), serde_json::Value::Array(children));
    serde_json::Value::Object(document)
}

/// 数组组：`{ 字段: { $elemMatch: 内层文档 } }`，取反时外面再包一层 `$not`。
///
/// 字段路径是必需的——没有它就无法确定作用范围。缺字段的组不产出条件，
/// 也不编造一个假键，交由界面标红提示。
fn mongo_builder_elem_group_json(group: &MongoBuilderGroup) -> serde_json::Value {
    let path = group.field.trim();
    let inner: Vec<serde_json::Value> = group
        .children
        .iter()
        .filter_map(|node| match node {
            MongoBuilderNode::Condition(condition) => {
                mongo_builder_condition_json(condition, true)
            }
            MongoBuilderNode::Group(group) => {
                let child = mongo_builder_group_json(group);
                (!child.as_object().is_some_and(|child| child.is_empty())).then_some(child)
            }
        })
        .collect();
    if path.is_empty() || inner.is_empty() {
        return serde_json::Value::Object(serde_json::Map::new());
    }
    let spec = serde_json::json!({ "$elemMatch": mongo_builder_merge_json(inner) });
    let mut document = serde_json::Map::new();
    document.insert(
        path.to_string(),
        if group.logic == MongoBuilderLogic::NotElemMatch {
            let mut negated = serde_json::Map::new();
            negated.insert("$not".to_string(), spec);
            serde_json::Value::Object(negated)
        } else {
            spec
        },
    );
    serde_json::Value::Object(document)
}

/// 一条条件 → 单键文档。
///
/// `inside_elem` 为真且字段为空时，条件说的是「数组元素自身」：表达式直接摊进
/// 内层文档，等值要显式写成 `{ $eq: v }`，否则标量进不了文档。
fn mongo_builder_condition_json(
    condition: &MongoBuilderCondition,
    inside_elem: bool,
) -> Option<serde_json::Value> {
    let value = mongo_builder_condition_value(condition.operator, &condition.value)?;
    let expression = mongo_builder_expression(condition.operator, value);
    let mut document = serde_json::Map::new();
    if inside_elem && condition.field.is_empty() {
        // 等值在这里只能写成 `{ $eq: v }`：内层文档没有「键 = 值」的位置。
        if condition.operator == MongoQueryOperator::Eq {
            document.insert("$eq".to_string(), expression);
        } else {
            match expression {
                serde_json::Value::Object(object) => document.extend(object),
                other => {
                    document.insert("$eq".to_string(), other);
                }
            }
        }
    } else {
        document.insert(condition.field.clone(), expression);
    }
    Some(serde_json::Value::Object(document))
}

#[cfg(test)]
mod mongo_query_builder_model_tests {
    use super::*;

    #[test]
    fn nested_logic_and_duplicate_fields_round_trip() {
        let filter = r#"{"convert_state":1,"$or":[{"id":"foo"},{"id":"bar","active":true}]}"#;
        let group = parse_mongo_builder_filter(filter).expect("nested filter");
        assert_eq!(group.logic, MongoBuilderLogic::And);
        assert_eq!(group.group_count(), 4);
        assert_eq!(mongo_builder_group_json(&group).to_string(), filter);

        let duplicate = r#"{"$and":[{"status":"paid"},{"status":"pending"}]}"#;
        let group = parse_mongo_builder_filter(duplicate).expect("duplicate fields");
        assert_eq!(mongo_builder_group_json(&group).to_string(), duplicate);

        let root_or = r#"{"$or":[{"status":"paid"},{"status":"pending"}]}"#;
        let group = parse_mongo_builder_filter(root_or).expect("root OR");
        assert_eq!(group.logic, MongoBuilderLogic::Or);
        assert_eq!(mongo_builder_group_json(&group).to_string(), root_or);

        let precedence = r#"{"$or":[{"a":1,"b":2},{"c":3}]}"#;
        let group = parse_mongo_builder_filter(precedence).expect("nested precedence");
        assert_eq!(mongo_builder_group_json(&group).to_string(), precedence);
    }

    /// 五种组类型都要原样往返：`$nor` 是同级运算符，数组组换成字段作用域。
    #[test]
    fn nor_and_elem_match_round_trip() {
        for filter in [
            r#"{"$nor":[{"status":"deleted"},{"archived":true}]}"#,
            r#"{"convert_state":1,"$nor":[{"a":1},{"b":2}]}"#,
            r#"{"items":{"$elemMatch":{"sku":"A-1","$gt":2}}}"#,
            r#"{"tags":{"$not":{"$elemMatch":{"$eq":"internal"}}}}"#,
            r#"{"$or":[{"items":{"$elemMatch":{"sku":"A-1"}}},{"a":1}]}"#,
        ] {
            let group = parse_mongo_builder_filter(filter).expect(filter);
            assert_eq!(mongo_builder_group_json(&group).to_string(), filter);
        }
    }

    /// 数组组的字段作用域与子项位置：`$elemMatch` 内层允许字段留空。
    #[test]
    fn elem_match_group_shape() {
        let group = parse_mongo_builder_filter(
            r#"{"items":{"$elemMatch":{"sku":"A-1","qty":{"$gt":2}}}}"#,
        )
        .expect("elem match");
        assert_eq!(group.logic, MongoBuilderLogic::ElemMatch);
        assert_eq!(group.field, "items");
        let MongoBuilderNode::Condition(first) = &group.children[0] else {
            panic!("field condition");
        };
        assert_eq!(first.field, "sku");
        let MongoBuilderNode::Condition(second) = &group.children[1] else {
            panic!("element condition");
        };
        // `{ "qty": { "$gt": 2 } }` 是字段上的运算，不是元素自身。
        assert_eq!(second.field, "qty");
        assert_eq!(second.operator, MongoQueryOperator::Gt);

        let element = parse_mongo_builder_filter(r#"{"items":{"$elemMatch":{"$gt":2}}}"#)
            .expect("bare element operator");
        let MongoBuilderNode::Condition(condition) = &element.children[0] else {
            panic!("element condition");
        };
        assert!(condition.field.is_empty(), "bare operator targets the element");
        assert_eq!(
            mongo_builder_group_json(&element).to_string(),
            r#"{"items":{"$elemMatch":{"$gt":2}}}"#
        );
    }

    /// 缺数组字段的组不产出条件，且不能变成 `{}` 混进 `$or`：
    /// `{"$or":[{}, …]}` 等于「匹配全部」，那会把整条过滤条件的意思反过来。
    #[test]
    fn elem_group_without_field_drops_out_of_or() {
        let mut group = MongoBuilderGroup::new(MongoBuilderLogic::Or);
        let mut elem = MongoBuilderGroup::new(MongoBuilderLogic::ElemMatch);
        elem.children.push(MongoBuilderNode::Condition(MongoBuilderCondition {
            field: "sku".into(),
            operator: MongoQueryOperator::Eq,
            value: r#""A-1""#.into(),
            row: None,
        }));
        group.children.push(MongoBuilderNode::Group(elem));
        group.children.push(MongoBuilderNode::Condition(MongoBuilderCondition {
            field: "a".into(),
            operator: MongoQueryOperator::Eq,
            value: "1".into(),
            row: None,
        }));
        assert_eq!(mongo_builder_group_json(&group).to_string(), r#"{"$or":[{"a":1}]}"#);
    }

    #[test]
    fn unsupported_or_excessive_filters_are_not_rewritten() {
        for filter in [
            r#"{"a":{"$regex":"^x"}}"#,
            r#"{"a":{"b":1}}"#,
            r#"{"$nor":[]}"#,
            r#"{"$or":[]}"#,
            r#"{"$or":[{}, {"a":1}]}"#,
            r#"{"$or":[{"$or":[{"$or":[{"$or":[{"a":1}]}]}]}]}"#,
            r#"{"a":{"$elemMatch":{"$gt":1},"$gt":1}}"#,
        ] {
            assert!(parse_mongo_builder_filter(filter).is_err(), "{filter}");
        }
    }

    /// 单个组里超过六条条件必须照常展开：条件树自己封顶滚动，
    /// 组内再卡条数只会让「添加条件」在第 7 条时莫名置灰。
    #[test]
    fn a_wide_group_is_not_capped_per_group() {
        let filter = r#"{"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8}"#;
        let group = parse_mongo_builder_filter(filter).expect("eight siblings");
        assert_eq!(group.children.len(), 8);
        assert_eq!(mongo_builder_group_json(&group).to_string(), filter);
    }
}
