// MongoDB 查询面板的「条件构建」：递归条件组直接改写成 Filter JSON。
//
// 它只是 Filter 的另一种输入法，不持有查询：每改一次就把 JSON 写回 Filter 编辑器，
// 真正执行的仍是面板里那一份文本。构建器与手写 JSON 永远一致。
//
// 反向也走同一条路：展开时解析当前 Filter。能无损表达的嵌套逻辑进入条件组；
// 其他复杂表达式拒绝改写并提示继续手写 JSON。

/// 条件构建器支持的运算符，顺序即下拉菜单顺序（对齐交互稿）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MongoQueryOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    Exists,
}

impl MongoQueryOperator {
    const ALL: [Self; 8] = [
        Self::Eq,
        Self::Ne,
        Self::Gt,
        Self::Gte,
        Self::Lt,
        Self::Lte,
        Self::In,
        Self::Exists,
    ];

    /// 写进 JSON 的查询运算符；`eq` 平铺成 `{ "field": value }`，没有 `$eq`。
    fn json_operator(self) -> Option<&'static str> {
        match self {
            Self::Eq => None,
            Self::Ne => Some("$ne"),
            Self::Gt => Some("$gt"),
            Self::Gte => Some("$gte"),
            Self::Lt => Some("$lt"),
            Self::Lte => Some("$lte"),
            Self::In => Some("$in"),
            Self::Exists => Some("$exists"),
        }
    }

    /// 运算符格与下拉里显示的写法：`$eq` / `$ne` / `$gte`。
    ///
    /// 等值也显示 `$eq`，八项统一成一种记号；它写进 JSON 仍是平铺的
    /// `{ "field": value }`（见 [`Self::json_operator`]），显示与序列化是两回事。
    fn symbol(self) -> &'static str {
        match self {
            Self::Eq => "$eq",
            _ => self.json_operator().unwrap_or("="),
        }
    }

    /// 下拉与运算符格上显示的就是运算符本身。
    ///
    /// 不拼中文名：`$ne` 已经是它写进 JSON 的样子，比「不等于」更短，也就不会
    /// 把 140px 的运算符列撑成半句中文。
    fn label(self) -> SharedString {
        SharedString::from(self.symbol())
    }

    fn from_json_operator(operator: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.json_operator() == Some(operator))
    }
}

/// 构建器控件高度。
///
/// 交互稿是 35px，这里压到 28px：一行 35px 在常驻面板里太占地方，
/// 而整块构建器的存在感应当低于它下面的结果区。字段/值两格是代码编辑器，
/// 盒子按 [`MONGO_QUERY_BUILDER_CELL_EDITOR_HEIGHT`] 定高后由外框居中。
const MONGO_QUERY_BUILDER_CONTROL_HEIGHT: f32 = 28.;
/// 组的左右内边距与嵌套缩进：比交互稿（10px / 30px）再收一档，
/// 条件行因此多出十几像素给值格，长值不会被挤成省略号。
const MONGO_QUERY_BUILDER_GROUP_PADDING: f32 = 6.;
const MONGO_QUERY_BUILDER_GROUP_INDENT: f32 = 11.;
/// 条件树的固定高度上限（交互稿 `.tree{max-height:240px}`）。
///
/// 超过就在树内滚动：条件再多也不许把结果区挤没——构建器是编辑条件的手段，
/// 不是这一屏的主角。
const MONGO_QUERY_BUILDER_TREE_MAX_HEIGHT: f32 = 240.;
/// 运算符列的固定宽度，与字段/值两侧的自适应列分开。
const MONGO_QUERY_BUILDER_OPERATOR_WIDTH: f32 = 140.;
/// 字段格最小宽度：再窄连 `customer.tier` 这种路径都看不全。
const MONGO_QUERY_BUILDER_FIELD_MIN_WIDTH: f32 = 140.;
/// 值格最小宽度：等宽小字下约十来个字符，够看清 `"paid"` 或 `500`。
const MONGO_QUERY_BUILDER_VALUE_MIN_WIDTH: f32 = 120.;
/// 条件格编辑器的内容高度：上下各 8px 内边距 + 一行 11px 文字（行高 1.5）。
///
/// 编辑器不像单行 `Input` 会把自己垂直居中，它总是从盒子顶边往下排，
/// 盒子贴着 28px 的格子放就会低 3.5px。给它这个「内边距 + 一行」的高度、
/// 再由外框 `items_center` 居中，首行才落在格子中线上；多出的内边距被
/// 外框裁掉，裁不到字。
const MONGO_QUERY_BUILDER_CELL_EDITOR_HEIGHT: f32 = 8. + 11. * 1.5 + 8.;

type MongoBuilderPath = Vec<usize>;

/// 数组组头部的字段格：只有 `$elemMatch` / `$not+$elemMatch` 组才有。
///
/// 单行 `InputState`——这一格不需要补全（数组路径从上面继承下来），
/// 也就省掉多行编辑器那根滑块。
#[derive(Debug)]
struct MongoQueryBuilderFieldCell {
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// 一个集合标签页的构建器状态（输入框实体随结构一起 drop）。
struct MongoQueryBuilder {
    root: MongoBuilderGroup,
    error: Option<SharedString>,
    /// 条件树的滚动位置。滚动条由 `vertical_scrollbar` 覆盖层绘制，句柄必须跨帧保留，
    /// 否则每次重绘都回到顶部，拖不动。
    scroll: ScrollHandle,
}

impl MongoQueryBuilder {
    fn new() -> Self {
        Self {
            root: MongoBuilderGroup::new(MongoBuilderLogic::And),
            error: None,
            scroll: ScrollHandle::default(),
        }
    }

    fn error(&self) -> Option<SharedString> {
        self.error.clone()
    }

    fn group(&self, path: &[usize]) -> Option<&MongoBuilderGroup> {
        let mut group = &self.root;
        for index in path {
            match group.children.get(*index)? {
                MongoBuilderNode::Group(child) => group = child,
                MongoBuilderNode::Condition(_) => return None,
            }
        }
        Some(group)
    }

    fn group_mut(&mut self, path: &[usize]) -> Option<&mut MongoBuilderGroup> {
        let mut group = &mut self.root;
        for index in path {
            match group.children.get_mut(*index)? {
                MongoBuilderNode::Group(child) => group = child,
                MongoBuilderNode::Condition(_) => return None,
            }
        }
        Some(group)
    }

    /// 只给单测用：按路径取条件，避免测试自己再走一遍树。
    #[cfg(test)]
    fn condition(&self, path: &[usize]) -> Option<&MongoBuilderCondition> {
        let (index, parent) = path.split_last()?;
        match self.group(parent)?.children.get(*index)? {
            MongoBuilderNode::Condition(condition) => Some(condition),
            MongoBuilderNode::Group(_) => None,
        }
    }

    fn condition_mut(&mut self, path: &[usize]) -> Option<&mut MongoBuilderCondition> {
        let (index, parent) = path.split_last()?;
        match self.group_mut(parent)?.children.get_mut(*index)? {
            MongoBuilderNode::Condition(condition) => Some(condition),
            MongoBuilderNode::Group(_) => None,
        }
    }

    fn can_add_group(&self) -> bool {
        self.root.group_count() < MONGO_QUERY_BUILDER_MAX_GROUPS
    }

    fn condition_count(&self) -> usize {
        self.root.condition_count()
    }
}

/// 把纯 JSON 树实例化为 GPUI 编辑器；每个叶节点只创建一次。
fn build_mongo_builder_group(
    view: &mut NavicatMain,
    tab_id: TabId,
    model: &MongoBuilderGroup,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> MongoBuilderGroup {
    let children = model
        .children
        .iter()
        .map(|node| match node {
            MongoBuilderNode::Condition(condition) => {
                let row = view.new_mongo_builder_row(
                    tab_id,
                    condition.field.clone(),
                    condition.value.clone(),
                    window,
                    cx,
                );
                MongoBuilderNode::Condition(MongoBuilderCondition {
                    field: String::new(),
                    operator: condition.operator,
                    value: String::new(),
                    row: Some(row),
                })
            }
            MongoBuilderNode::Group(group) => MongoBuilderNode::Group(build_mongo_builder_group(
                view, tab_id, group, window, cx,
            )),
        })
        .collect();
    MongoBuilderGroup {
        logic: model.logic,
        field: model.field.clone(),
        open: model.open,
        children,
        field_input: model
            .logic
            .is_array()
            .then(|| view.new_mongo_builder_field_cell(tab_id, model.field.clone(), window, cx)),
    }
}
/// 构建器里的一行条件。字段与值由输入框实体持有，运算符只存在树里的条件上。
///
/// 两格都是 `EditorState`：只有代码编辑器能挂 `Lsp.completion_provider`，
/// 而两格都要补全——字段补路径，值补同一行字段的取值。两者的滚动条问题
/// 用同一招压掉，见 `new_mongo_builder_row`。
///
/// 行里不存运算符副本：下拉改的是 [`MongoBuilderCondition::operator`]，
/// 渲染再读行里的副本就会永远显示旧值。
#[derive(Debug)]
struct MongoQueryBuilderRow {
    field: Entity<EditorState>,
    value: Entity<EditorState>,
    /// 两个输入框的改动都要回写 JSON；订阅随行一起释放。
    _field_subscription: Subscription,
    _value_subscription: Subscription,
}


impl NavicatMain {
    /// 展开/收起条件构建器。
    ///
    /// 展开时重新解析 Filter：解析不了（语法错、复杂表达式）只报错、不改 JSON，
    /// 用户继续手写即可。收起不动 JSON——构建器从来没持有过独立状态。
    pub(crate) fn toggle_mongo_query_builder(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .mongo_query_builder_expanded
            .get(&tab_id)
            .copied()
            .unwrap_or(false)
        {
            self.mongo_query_builder_expanded.insert(tab_id, false);
            cx.notify();
            return;
        }
        let Some(filter) = self
            .mongo_query_inputs
            .get(&tab_id)
            .map(|inputs| inputs.read(cx).filter.clone())
        else {
            return;
        };
        let text = filter.read(cx).value().to_string();
        match parse_mongo_builder_filter(&text) {
            Ok(model) => {
                let root = build_mongo_builder_group(self, tab_id, &model, window, cx);
                let builder = self
                    .mongo_query_builders
                    .entry(tab_id)
                    .or_insert_with(MongoQueryBuilder::new);
                builder.root = root;
                builder.error = None;
                self.mongo_query_builder_expanded.insert(tab_id, true);
            }
            Err(error) => {
                // 拒绝时清空编辑器草稿，Filter 一个字都不动。
                let builder = self
                    .mongo_query_builders
                    .entry(tab_id)
                    .or_insert_with(MongoQueryBuilder::new);
                builder.root = MongoBuilderGroup::new(MongoBuilderLogic::And);
                builder.error = Some(error);
                self.mongo_query_builder_expanded.insert(tab_id, true);
            }
        }
        cx.notify();
    }

    /// 新建一行条件；两个输入框的改动都立刻回写 Filter。
    ///
    /// 两格都是 `EditorState`：只有代码编辑器能挂 `Lsp.completion_provider`，
    /// 补全是这两格的主要输入方式。它是多行编辑器，所以要关掉那处会凭空长出一根
    /// 滑块的设置——`scroll_beyond_last_line(Some(0))`。默认值在末行下方留半屏空白，
    /// 滚动条把它当成可滚内容，28px 的格子里就多出一根拖不动的滑块。
    /// 行数保持默认两行（没有对外改行的入口），多出的高度由外壳 `overflow_hidden` 裁掉。
    /// `submit_on_enter` 则让回车只结束编辑——两格都是单值（路径 / 标量），
    /// 多行编辑器默认会往里塞换行。
    fn new_mongo_builder_row(
        &mut self,
        tab_id: TabId,
        field: String,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Rc<MongoQueryBuilderRow> {
        let view = cx.entity().downgrade();
        let field_input = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .placeholder(tr("app.mongo.query.builder_field_placeholder"))
                .default_value(field)
                .soft_wrap(false)
                .submit_on_enter(true)
                .scroll_beyond_last_line(Some(0));
            configure_mongo_builder_field_completion(&mut editor, view.clone(), tab_id, window, cx);
            editor
        });
        // 值候选要问同行字段格「你是什么字段」，所以先把字段格实体降级传进去。
        let field_weak = field_input.downgrade();
        let value_input = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .placeholder(tr("app.mongo.query.builder_value_placeholder"))
                .default_value(value)
                .soft_wrap(false)
                .submit_on_enter(true)
                .scroll_beyond_last_line(Some(0));
            configure_mongo_builder_value_completion(
                &mut editor,
                field_weak,
                view,
                tab_id,
                window,
                cx,
            );
            editor
        });
        let field_subscription = cx.subscribe_in(
            &field_input,
            window,
            move |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sync_mongo_query_builder(tab_id, window, cx);
                }
            },
        );
        let value_subscription = cx.subscribe_in(
            &value_input,
            window,
            move |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sync_mongo_query_builder(tab_id, window, cx);
                }
            },
        );
        Rc::new(MongoQueryBuilderRow {
            field: field_input,
            value: value_input,
            _field_subscription: field_subscription,
            _value_subscription: value_subscription,
        })
    }

    /// 数组组头部的字段格：`$elemMatch` 必须知道作用在哪个数组上。
    fn new_mongo_builder_field_cell(
        &mut self,
        tab_id: TabId,
        field: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Rc<MongoQueryBuilderFieldCell> {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr("app.mongo.query.builder_array_field_placeholder"))
                .default_value(field)
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sync_mongo_query_builder(tab_id, window, cx);
                }
            },
        );
        Rc::new(MongoQueryBuilderFieldCell {
            input,
            _subscription: subscription,
        })
    }

    /// 添加一行条件并聚焦到值输入框。
    pub(crate) fn add_mongo_builder_condition(
        &mut self,
        tab_id: TabId,
        path: MongoBuilderPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let can_add = self.mongo_query_builders.get(&tab_id).is_some_and(|builder| {
            builder.group(&path).is_some()
                && builder.condition_count() < MONGO_QUERY_BUILDER_MAX_CONDITIONS
        });
        if !can_add {
            return;
        }
        let row = self.new_mongo_builder_row(tab_id, String::new(), String::new(), window, cx);
        row.value
            .update(cx, |input, cx| input.focus_handle(cx).focus(window, cx));
        if let Some(group) = self
            .mongo_query_builders
            .get_mut(&tab_id)
            .and_then(|builder| builder.group_mut(&path))
        {
            group.children.push(MongoBuilderNode::Condition(MongoBuilderCondition {
                field: String::new(),
                operator: MongoQueryOperator::Eq,
                value: String::new(),
                row: Some(row),
            }));
        }
        cx.notify();
    }

    pub(crate) fn add_mongo_builder_group(
        &mut self,
        tab_id: TabId,
        path: MongoBuilderPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let can_add = self.mongo_query_builders.get(&tab_id).is_some_and(|builder| {
            builder.can_add_group()
                && builder.condition_count() < MONGO_QUERY_BUILDER_MAX_CONDITIONS
                && path.len() < MONGO_QUERY_BUILDER_MAX_DEPTH
                && builder.group(&path).is_some()
        });
        if !can_add {
            return;
        }
        let mut child = MongoBuilderGroup::new(MongoBuilderLogic::And);
        // 新建的子组直接展开：刚建出来就折成一行，用户会找不到自己刚加的组。
        child.open = true;
        child.children.push(MongoBuilderNode::Condition(MongoBuilderCondition {
            field: String::new(),
            operator: MongoQueryOperator::Eq,
            value: String::new(),
            row: None,
        }));
        let child = build_mongo_builder_group(self, tab_id, &child, window, cx);
        if let Some(group) = self
            .mongo_query_builders
            .get_mut(&tab_id)
            .and_then(|builder| builder.group_mut(&path))
        {
            group.children.push(MongoBuilderNode::Group(child));
        }
        self.sync_mongo_query_builder(tab_id, window, cx);
        cx.notify();
    }

    /// 删除条件或子组；根组只清空，不移除自己的编辑入口。
    pub(crate) fn remove_mongo_builder_node(
        &mut self,
        tab_id: TabId,
        path: MongoBuilderPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(builder) = self.mongo_query_builders.get_mut(&tab_id) {
            if path.is_empty() {
                builder.root.children.clear();
            } else if let Some((index, parent)) = path.split_last()
                && let Some(group) = builder.group_mut(parent)
                && *index < group.children.len()
            {
                group.children.remove(*index);
            }
        }
        self.sync_mongo_query_builder(tab_id, window, cx);
        cx.notify();
    }

    pub(crate) fn set_mongo_builder_operator(
        &mut self,
        tab_id: TabId,
        path: MongoBuilderPath,
        operator: MongoQueryOperator,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(condition) = self
            .mongo_query_builders
            .get_mut(&tab_id)
            .and_then(|builder| builder.condition_mut(&path))
        {
            condition.operator = operator;
        }
        self.sync_mongo_query_builder(tab_id, window, cx);
        // 运算符存在树上、不在输入框实体里，编辑器的 Change 事件不会替这里重绘：
        // 不通知的话格子与下拉里的勾都停在旧值，看起来就是「选了没反应」。
        cx.notify();
    }

    pub(crate) fn set_mongo_builder_logic(
        &mut self,
        tab_id: TabId,
        path: MongoBuilderPath,
        logic: MongoBuilderLogic,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let needs_field_input = logic.is_array();
        if let Some(group) = self
            .mongo_query_builders
            .get_mut(&tab_id)
            .and_then(|builder| builder.group_mut(&path))
        {
            // 离开数组组时字段路径跟着失效：留着会让下一次切回来显示出
            // 一个用户已经看不见的旧值。
            if !needs_field_input {
                group.field.clear();
                group.field_input = None;
            }
            group.logic = logic;
            group.open = true;
        }
        // 切进数组组时补一个字段输入框——它只在数组组里有意义。
        if needs_field_input && self.mongo_builder_group_field_input(tab_id, &path).is_none() {
            let cell = self.new_mongo_builder_field_cell(tab_id, String::new(), window, cx);
            if let Some(group) = self
                .mongo_query_builders
                .get_mut(&tab_id)
                .and_then(|builder| builder.group_mut(&path))
            {
                group.field_input = Some(cell);
            }
        }
        self.sync_mongo_query_builder(tab_id, window, cx);
        cx.notify();
    }

    fn mongo_builder_group_field_input(
        &self,
        tab_id: TabId,
        path: &[usize],
    ) -> Option<Rc<MongoQueryBuilderFieldCell>> {
        self.mongo_query_builders
            .get(&tab_id)?
            .group(path)?
            .field_input
            .clone()
    }

    /// 展开/收起子组。收起态只画组头，展开才铺开子项——默认收起，
    /// 否则十个条件一展开就吃掉半屏，结果区跟着被挤没。
    pub(crate) fn toggle_mongo_builder_group(
        &mut self,
        tab_id: TabId,
        path: MongoBuilderPath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = self
            .mongo_query_builders
            .get_mut(&tab_id)
            .and_then(|builder| builder.group_mut(&path))
        {
            group.open = !group.open;
        }
        cx.notify();
    }

    /// 把递归条件树拼成 Filter JSON。未完成条件被跳过，空组不产出过滤条件。
    fn sync_mongo_query_builder(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(filter) = self
            .mongo_query_inputs
            .get(&tab_id)
            .map(|inputs| inputs.read(cx).filter.clone())
        else {
            return;
        };
        let Some(builder) = self.mongo_query_builders.get(&tab_id) else {
            return;
        };
        let model = mongo_builder_snapshot(&builder.root, cx);
        filter.update(cx, |editor, cx| {
            editor.set_value(mongo_builder_group_json(&model).to_string(), window, cx)
        });
        let had_error = builder.error.is_some();
        if let Some(builder) = self.mongo_query_builders.get_mut(&tab_id) {
            builder.error = None;
        }
        // 每键不通知整页；编辑器与条件格各自重绘。
        if had_error {
            cx.notify();
        }
    }
}

/// 读取所有叶编辑器，生成不包含 GPUI 类型的序列化快照。
///
/// 字段留空的条件只在数组组里有意义（指「数组元素自身」）；普通组里它就是
/// 一条没写完的条件，跳过不写进 JSON——不编造字段名，免得静默查错字段。
fn mongo_builder_snapshot(group: &MongoBuilderGroup, cx: &App) -> MongoBuilderGroup {
    let inside_elem = group.logic.is_array();
    let children = group
        .children
        .iter()
        .filter_map(|node| match node {
            MongoBuilderNode::Condition(condition) => {
                let row = condition.row.as_ref()?;
                let field = row.field.read(cx).value().trim().to_string();
                if field.is_empty() && !inside_elem {
                    return None;
                }
                Some(MongoBuilderNode::Condition(MongoBuilderCondition {
                    field,
                    operator: condition.operator,
                    value: row.value.read(cx).value().to_string(),
                    row: Some(row.clone()),
                }))
            }
            MongoBuilderNode::Group(group) => {
                Some(MongoBuilderNode::Group(mongo_builder_snapshot(group, cx)))
            }
        })
        .collect();
    MongoBuilderGroup {
        logic: group.logic,
        // 数组组的字段路径同样来自输入框，快照里换成纯字符串。
        field: group
            .field_input
            .as_ref()
            .map_or_else(|| group.field.clone(), |cell| cell.input.read(cx).value().trim().to_string()),
        open: group.open,
        children,
        field_input: group.field_input.clone(),
    }
}

/// 一行条件对应的值；返回 `None` 表示这行还没填完，跳过。
fn mongo_builder_condition_value(operator: MongoQueryOperator, text: &str) -> Option<serde_json::Value> {
    match operator {
        // `$exists` 只关心字段在不在：值恒为布尔，与手写 `{ "a": { "$exists": false } }` 一致。
        MongoQueryOperator::Exists => Some(serde_json::Value::Bool(text.trim() != "false")),
        _ if text.trim().is_empty() => None,
        MongoQueryOperator::In => Some(serde_json::Value::Array(
            text.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(parse_mongo_builder_scalar)
                .collect(),
        )),
        _ => Some(parse_mongo_builder_scalar(text.trim())),
    }
}

/// 把一行条件包成它对应的 JSON 表达式：等值平铺，其余包一层 `$运算符`。
fn mongo_builder_expression(
    operator: MongoQueryOperator,
    value: serde_json::Value,
) -> serde_json::Value {
    match operator.json_operator() {
        Some(operator) => {
            let mut wrapper = serde_json::Map::new();
            wrapper.insert(operator.to_string(), value);
            serde_json::Value::Object(wrapper)
        }
        None => value,
    }
}

/// 输入框文本转 JSON 标量：`true`/`false`/`null`/数字原样，带引号的去掉引号，
/// 其余按字符串。用户不必为了写字符串再套一层引号。
fn parse_mongo_builder_scalar(text: &str) -> serde_json::Value {
    match text {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        "null" => serde_json::Value::Null,
        "" => serde_json::Value::String(String::new()),
        _ => {
            if let Ok(number) = text.parse::<i64>() {
                return serde_json::Value::from(number);
            }
            if let Ok(number) = text.parse::<f64>() {
                return serde_json::Value::from(number);
            }
            let unquoted = text
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .or_else(|| {
                    text.strip_prefix('\'')
                        .and_then(|rest| rest.strip_suffix('\''))
                });
            serde_json::Value::String(unquoted.unwrap_or(text).to_string())
        }
    }
}

/// 按「先严格 JSON、再 JSON5」解析，与连接器 `mongo_parse_json` 同规则。
///
/// 先试严格 JSON 不只是省事：JSON5 把所有数字读成 f64，`{"age": 41}` 会落成
/// Double 而匹配不到存成 Int32 的字段；写回新 JSON 时那一位就悄悄改了类型。
///
/// `NaN` / `Infinity` 直接拒：serde_json 会把它读成 `null`，等于把条件换成
/// 「字段等于 null」——那种改写比报错糟得多。
fn parse_mongo_relaxed_json(text: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        return Some(value);
    }
    if text.contains("NaN") || text.contains("Infinity") {
        return None;
    }
    json5::from_str::<serde_json::Value>(text).ok()
}

/// JSON 标量回填到输入框：字符串保留引号。
///
/// 去掉引号再写回会把 `"500"` 读成数字 500——一次往返就悄悄改了字段类型。
fn format_mongo_builder_scalar(value: &serde_json::Value) -> String {
    value.to_string()
}

/// 构建器渲染快照：克隆轻量实体句柄，不在渲染期间读视图状态。
#[derive(Clone)]
struct MongoQueryBuilderView {
    root: MongoBuilderGroup,
    error: Option<SharedString>,
    condition_count: usize,
    can_add_group: bool,
    scroll: ScrollHandle,
}

fn mongo_query_builder_view(
    builders: &BTreeMap<TabId, MongoQueryBuilder>,
    tab_id: TabId,
) -> MongoQueryBuilderView {
    builders.get(&tab_id).map_or_else(
        || MongoQueryBuilderView {
            root: MongoBuilderGroup::new(MongoBuilderLogic::And),
            error: None,
            condition_count: 0,
            can_add_group: true,
            scroll: ScrollHandle::default(),
        },
        |builder| MongoQueryBuilderView {
            root: builder.root.clone(),
            error: builder.error(),
            condition_count: builder.condition_count(),
            can_add_group: builder.can_add_group(),
            scroll: builder.scroll.clone(),
        },
    )
}

/// 交互稿的薄荷底与绿色系：面板里两层展开区共用这一组色，明暗各一份。
#[derive(Clone, Copy)]
struct MongoQueryTone {
    /// 展开区底色（交互稿 `--mint`）：比卡片底色浅一档，分层不靠描边。
    mint: gpui::Rgba,
    /// 绿色浅底（交互稿 `--green-soft`）：角标、展开态按钮、AND 徽标。
    green_soft: gpui::Rgba,
    /// 绿色描边（交互稿 `--green-line`）。
    green_line: gpui::Rgba,
    /// 数组组（`$elemMatch`）的青灰字色：和普通逻辑组区分开，
    /// 因为它们换的是作用范围，不只是连接词。
    array_ink: gpui::Rgba,
}

fn mongo_query_tone(colors: UiColors) -> MongoQueryTone {
    if colors.is_dark {
        MongoQueryTone {
            mint: rgb(0x222c26),
            green_soft: rgb(0x263e2f),
            green_line: rgb(0x3b5b45),
            array_ink: rgb(0x7fa8c9),
        }
    } else {
        MongoQueryTone {
            mint: rgb(0xf6faf7),
            green_soft: rgb(0xeaf4ed),
            green_line: rgb(0xc6ddce),
            array_ink: rgb(0x2f6f9e),
        }
    }
}

/// 条件构建抽屉。
///
/// 结构对齐交互稿：顶部一条 28px 工具条（根组逻辑 + 计数 + 新建入口），
/// 下面是条件树。树**封顶 240px**并在内部滚动——条件再多也不许把结果区挤没，
/// 构建器是编辑条件的手段，不是这一屏的主角。抽屉收起时完全不占高度。
fn mongo_query_builder_section(
    tab_id: TabId,
    builder: &MongoQueryBuilderView,
    colors: UiColors,
    tone: MongoQueryTone,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let path = MongoBuilderPath::new();
    let selector_path = mongo_builder_path_label(&path);
    let root = &builder.root;
    // 根组的数组字段格也挂在这条工具条上：根组若选了 `$elemMatch`，
    // 字段和逻辑一样是这一层的属性，不该另起一行。
    let root_field_cell = root
        .field_input
        .as_ref()
        .map(|cell| mongo_query_builder_array_field_cell(&path, cell, colors, window, cx));
    let mut strip = div()
        .debug_selector(|| "mongo-query-builder-toolbar".to_string())
        .h(px(28.))
        .flex_none()
        .px(px(10.))
        .flex()
        .items_center()
        // 窄窗口下工具条内容会顶出卡片，右侧按钮被推到窗外——点击落到窗口外会被
        // gpui 静默丢弃，看起来像「按钮没反应」。计数文字先让位（截断），按钮不让。
        .overflow_hidden()
        .gap_2()
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(tone.mint)
        .child(mongo_query_builder_logic_toggle(
            tab_id,
            &path,
            root.logic,
            &selector_path,
            tone,
            colors,
            cx,
        ));
    if let Some(cell) = root_field_cell {
        strip = strip.child(cell);
    }
    strip = strip
        // 说明与计数同处一个可收缩块：窄窗口下这里先被截断，右侧按钮保持完整宽度。
        // 按钮本身不让位——它们被挤出窗口就点不到了。
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_2()
                // 说明随逻辑走：分段控件上只写 AND/OR/NOR/ELEM，这里说清它对这一组意味着什么。
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .child(tr(root.logic.label_key())),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(format!(
                            "{} · {}",
                            tr("app.mongo.query.builder_condition_count")
                                .replace("%{count}", &builder.condition_count.to_string()),
                            tr("app.mongo.query.builder_group_count").replace(
                                "%{count}",
                                &root.group_count().saturating_sub(1).to_string()
                            ),
                        ))),
                ),
        )
        .child(mongo_query_builder_add_button(
            tab_id,
            path.clone(),
            builder.condition_count < MONGO_QUERY_BUILDER_MAX_CONDITIONS,
            colors,
            cx,
        ))
        .child(mongo_query_builder_add_group_button(
            tab_id,
            path,
            builder.can_add_group && builder.condition_count < MONGO_QUERY_BUILDER_MAX_CONDITIONS,
            colors,
            cx,
        ))
        .child(mongo_query_builder_collapse_button(tab_id, colors, cx));
    // 内边距放在**内层**：滚动容器自己带内边距时，gpui 会把这段内边距同时算进
    // 内容尺寸与视口尺寸，`max_offset` 于是恒等于两倍内边距——单个条件也会判成
    // 「内容溢出」，右侧凭空冒出拖不动的滚动条。滚动容器零内边距，内容多高视口
    // 就多高，只有真正超出 240px 才有可滚区间。
    let mut rows = div().w_full().flex().flex_col().gap(px(4.));
    // 根组自己的子项直接排在树里，不再套一层卡片：根没有「括号」可言，
    // 套一张卡只是白吃一圈 6px 内边距和一条描边。
    for (index, node) in root.children.iter().enumerate() {
        rows = match node {
            MongoBuilderNode::Condition(condition) => {
                let Some(row) = &condition.row else { continue };
                rows.child(mongo_query_builder_row(
                    tab_id,
                    vec![index],
                    row,
                    condition.operator,
                    colors,
                    window,
                    cx,
                ))
            }
            MongoBuilderNode::Group(group) => rows.child(mongo_query_builder_group(
                tab_id,
                group,
                vec![index],
                builder.condition_count,
                builder.can_add_group,
                colors,
                tone,
                window,
                cx,
            )),
        };
    }
    if root.children.is_empty() {
        rows = rows.child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(tr("app.mongo.query.builder_empty")),
        );
    }
    // 滚动条覆盖层必须挂在**滚动容器之外**。
    //
    // `vertical_scrollbar` 加的是 `absolute().inset_0()` 的覆盖层。它和
    // `overflow_y_scroll()` 挂在同一个 div 上时，覆盖层就成了滚动内容的子节点：
    // 内容滚多远它跟着滚多远，看上去永远是「滚动条不动」。所以照 `Scrollable`
    // 的结构分两层——外层只有裁剪边界与覆盖层，内层才承接滚动。
    let tree_area = div()
        .debug_selector(|| "mongo-query-builder-tree".to_string())
        .id("mongo-query-builder-tree")
        // 高度按内容走、被 `max_h` 截住：树在 `flex_col` 里若允许收缩，它会被下面的
        // 结果区一路挤扁（条件行被裁掉、点都点不到），若保持默认的 `min-height: auto`
        // 又会被内容撑过 240px。`flex_none` + `max_h` 两者兼得：内容多高就多高，
        // 到 240px 停住，超出的在树内滚动。
        .w_full()
        .max_h(px(MONGO_QUERY_BUILDER_TREE_MAX_HEIGHT))
        .flex_none()
        .min_h(px(0.))
        .track_scroll(&builder.scroll)
        // 用原生 `overflow_y_scroll()`，不用 `overflow_y_scrollbar()`：后者的根节点
        // 固定 `size_full()`，在高度按内容的外层里会把自己压成 0 高。
        .overflow_y_scroll()
        .child(rows.px(px(10.)).py(px(5.)));
    // 外层与外层同高（内层被 `max_h` 截住后即视口高度），覆盖层才有正确的视口。
    let tree = div()
        .relative()
        .flex_none()
        .w_full()
        .child(tree_area)
        .vertical_scrollbar(&builder.scroll);
    let mut section = div()
        .debug_selector(|| "mongo-query-builder".to_string())
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .child(strip)
        .child(tree);
    if let Some(error) = &builder.error {
        section = section.child(
            div()
                .px(px(10.))
                .pb(px(5.))
                .text_size(ui_text_size(10.))
                .text_color(colors.danger)
                .child(error.clone()),
        );
    }
    section
}

#[allow(clippy::too_many_arguments)]
fn mongo_query_builder_group(
    tab_id: TabId,
    group: &MongoBuilderGroup,
    path: MongoBuilderPath,
    condition_count: usize,
    can_add_group: bool,
    colors: UiColors,
    tone: MongoQueryTone,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selector_path = mongo_builder_path_label(&path);
    let logic = group.logic;
    let is_array = logic.is_array();
    // 组默认折成一行：组头本身就是「这组是什么」的完整摘要，
    // 展开才铺开子项。这样十个条件也只占十行，不会一展开就吃掉半屏。
    let mut head = div()
        .debug_selector({
            let path = selector_path.clone();
            move || format!("mongo-query-builder-group-head:{path}")
        })
        .h(px(26.))
        .flex()
        .items_center()
        .gap_2()
        .pl(px(7.))
        .pr(px(4.))
        .rounded(px(5.))
        .border_1()
        .border_dashed()
        .border_color(tone.green_line)
        .bg(tone.mint)
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let path = path.clone();
                move |this, _: &MouseDownEvent, window, cx| {
                    this.toggle_mongo_builder_group(tab_id, path.clone(), window, cx);
                    cx.stop_propagation();
                }
            }),
        )
        .child(
            div()
                .flex_none()
                .w(px(10.))
                .text_size(ui_text_size(8.))
                .text_color(colors.muted)
                .child(if group.open { "▾" } else { "▸" }),
        )
        .child(mongo_query_builder_logic_toggle(
            tab_id,
            &path,
            logic,
            &selector_path,
            tone,
            colors,
            cx,
        ));
    if let Some(cell) = group
        .field_input
        .as_ref()
        .map(|cell| mongo_query_builder_array_field_cell(&path, cell, colors, window, cx))
    {
        head = head.child(cell);
    }
    // 收起时组头自己就是一句摘要（`sku = "A-1" …`），展开时摘要让位给内容。
    if !group.open {
        head = head.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_size(ui_text_size(10.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_color(colors.muted)
                .whitespace_nowrap()
                .child(mongo_builder_group_preview(group)),
        );
    } else {
        head = head.child(div().flex_1());
    }
    head = head
        .child(
            div()
                .flex_none()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .whitespace_nowrap()
                .child(SharedString::from(format!(
                    "{} {}",
                    group.condition_count(),
                    tr("app.mongo.query.builder_condition_unit")
                ))),
        )
        .child(mongo_query_builder_add_button(
            tab_id,
            path.clone(),
            condition_count < MONGO_QUERY_BUILDER_MAX_CONDITIONS,
            colors,
            cx,
        ))
        .child(mongo_query_builder_add_group_button(
            tab_id,
            path.clone(),
            can_add_group && condition_count < MONGO_QUERY_BUILDER_MAX_CONDITIONS,
            colors,
            cx,
        ))
        .child(mongo_query_builder_remove_group_button(
            tab_id,
            path.clone(),
            &selector_path,
            colors,
            cx,
        ));
    let container = div()
        .debug_selector({
            let path = selector_path.clone();
            move || format!("mongo-query-builder-group:{path}")
        })
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(head);
    if !group.open {
        return container;
    }
    // 展开后子项缩进 11px 并挂一条竖轨，用位置表达括号；
    // 组头已经画了边界，再套一张卡会多出两圈描边。
    let mut body = div()
        .ml(px(MONGO_QUERY_BUILDER_GROUP_INDENT))
        .pl(px(MONGO_QUERY_BUILDER_GROUP_PADDING))
        .border_l_2()
        .border_color(if is_array { tone.green_line } else { colors.border })
        .flex()
        .flex_col()
        .gap(px(4.));
    for (index, node) in group.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        body = match node {
            MongoBuilderNode::Condition(condition) => {
                let Some(row) = &condition.row else { continue };
                body.child(mongo_query_builder_row(
                    tab_id,
                    child_path,
                    row,
                    condition.operator,
                    colors,
                    window,
                    cx,
                ))
            }
            MongoBuilderNode::Group(group) => body.child(mongo_query_builder_group(
                tab_id,
                group,
                child_path,
                condition_count,
                can_add_group,
                colors,
                tone,
                window,
                cx,
            )),
        };
    }
    if group.children.is_empty() {
        body = body.child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(tr("app.mongo.query.builder_empty")),
        );
    }
    container.child(body)
}

/// 组头摘要：第一条条件 + 条数，收起时唯一能看见的信息。
///
/// 只读模型里的纯文本，不碰输入框实体——收起态不渲染子项，没有实体可读。
fn mongo_builder_group_preview(group: &MongoBuilderGroup) -> SharedString {
    let Some(first) = group.children.first() else {
        return tr("app.mongo.query.builder_empty_group");
    };
    let text = match first {
        MongoBuilderNode::Condition(condition) => format!(
            "{} {} {}",
            if condition.field.is_empty() {
                tr("app.mongo.query.builder_element")
            } else {
                condition.field.clone().into()
            },
            condition.operator.symbol(),
            condition.value
        ),
        MongoBuilderNode::Group(group) => group.logic.short_label().to_string(),
    };
    if group.children.len() > 1 {
        SharedString::from(format!("{text} …"))
    } else {
        SharedString::from(text)
    }
}

fn mongo_builder_path_label(path: &[usize]) -> String {
    if path.is_empty() {
        "root".to_string()
    } else {
        path.iter().map(usize::to_string).collect::<Vec<_>>().join("-")
    }
}

/// 组的逻辑分段控件（交互稿 `.seg`）：五个组类型全铺开。
///
/// 直接铺五段而不是收进下拉：组类型是**结构**差异（NOR 换连接词，ELEM 换作用范围），
/// 不是样式偏好；藏在二级菜单里每次切换都要多两次点击，还看不出有哪几种可选。
/// 两段数组组用青灰区分——它们选中的是「作用在某数组上」，不只是换个连接词。
fn mongo_query_builder_logic_toggle(
    tab_id: TabId,
    path: &MongoBuilderPath,
    current: MongoBuilderLogic,
    selector_path: &str,
    tone: MongoQueryTone,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let segments = MongoBuilderLogic::ALL
        .into_iter()
        .map(|logic| {
            let selected = logic == current;
            let label = logic.short_label();
            let name = logic.selector();
            let path = path.clone();
            Button::new(SharedString::from(format!(
                "mongo-builder-logic:{name}:{}:{selector_path}",
                tab_id.0
            )))
            .debug_selector({
                let selector_path = selector_path.to_string();
                move || format!("mongo-query-builder-logic:{selector_path}:{name}")
            })
            .ghost()
            .small()
            .h(px(18.))
            .px(px(6.))
            .rounded(px(3.))
            .text_size(ui_text_size(9.))
            .font_family(crate::PLATFORM_MONOSPACE_FONT)
            .when(selected, |this| {
                this.bg(colors.panel_bg).text_color(colors.success).shadow(vec![
                    box_shadow(
                        px(0.),
                        px(1.),
                        px(3.),
                        px(0.),
                        hsla(0., 0., 0., if colors.is_dark { 0.35 } else { 0.12 }),
                    ),
                ])
            })
            .when(!selected, |this| {
                this.text_color(if logic.is_array() {
                    tone.array_ink
                } else {
                    colors.muted
                })
            })
            .label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.set_mongo_builder_logic(tab_id, path.clone(), logic, window, cx);
                cx.stop_propagation();
            }))
        })
        .collect::<Vec<_>>();
    div()
        .debug_selector({
            let selector_path = selector_path.to_string();
            move || format!("mongo-query-builder-logic:{selector_path}")
        })
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(4.))
        .bg(tone.green_soft)
        .children(segments)
}

/// 数组组的字段格：`$elemMatch` 必须知道作用在哪个数组上。
///
/// 空字段不产出条件（见模型层的 `mongo_builder_elem_group_json`），
/// 所以这里把空格标成 danger 描边——它是必填，不是可选项。
fn mongo_query_builder_array_field_cell(
    path: &MongoBuilderPath,
    cell: &Rc<MongoQueryBuilderFieldCell>,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selector_path = mongo_builder_path_label(path);
    let body_selector = format!("mongo-query-builder-array-field-body:{selector_path}");
    let focused = cell.input.read(cx).focus_handle(cx).is_focused(window);
    let empty = cell.input.read(cx).value().trim().is_empty();
    div()
        .debug_selector(move || format!("mongo-query-builder-array-field:{selector_path}"))
        .flex_none()
        .w(px(150.))
        .h(px(22.))
        .rounded(px(4.))
        .border_1()
        .border_color(if empty {
            colors.danger
        } else {
            mongo_query_input_border(focused, colors)
        })
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .child(mongo_query_input_body(
            &cell.input,
            Some(body_selector),
            10.5,
            false,
        ))
}

/// 收起按钮：交互稿里的「收起 ⌃」。
fn mongo_query_builder_collapse_button(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    Button::new(("mongo-query-builder-collapse", tab_id.0))
        .debug_selector(|| "mongo-query-builder-collapse".to_string())
        .ghost()
        .small()
        .h(px(22.))
        .px(px(7.))
        .text_size(ui_text_size(10.5))
        .text_color(colors.success)
        .label(tr("app.mongo.query.builder_collapse"))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.toggle_mongo_query_builder(tab_id, window, cx);
            cx.stop_propagation();
        }))
}

/// 一行条件：字段 | 运算符 | 值 | 移除。
///
/// 字段留空的含义由所属组决定（数组组里是「元素自身」），判断在快照层做，
/// 这里只负责画。
#[allow(clippy::too_many_arguments)]
fn mongo_query_builder_row(
    tab_id: TabId,
    path: MongoBuilderPath,
    row: &Rc<MongoQueryBuilderRow>,
    operator: MongoQueryOperator,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selector_path = mongo_builder_path_label(&path);
    div()
        .debug_selector(move || format!("mongo-query-builder-row:{selector_path}"))
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                // 字段与值按交互稿的 1 : 1.45 分配剩余宽度，两边都从 0 起算
                // （`flex_1` 的 basis 是 0）。按内容算基数时，字段格子里的 `w_full`
                // 会以整行宽度作为基准，把值格挤成一条缝。
                .child(
                    div()
                        .flex_1()
                        .min_w(px(MONGO_QUERY_BUILDER_FIELD_MIN_WIDTH))
                        .child(mongo_query_builder_field_cell(
                            path.clone(),
                            &row.field,
                            colors,
                            window,
                            cx,
                        )),
                )
                .child(
                    div()
                        .debug_selector({
                            let path = mongo_builder_path_label(&path);
                            move || format!("mongo-query-builder-operator:{path}")
                        })
                        .w(px(MONGO_QUERY_BUILDER_OPERATOR_WIDTH))
                        .flex_none()
                        .child(mongo_query_builder_operator_cell(
                            tab_id, path.clone(), operator, colors, cx,
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .flex_grow(1.45)
                        .min_w(px(MONGO_QUERY_BUILDER_VALUE_MIN_WIDTH))
                        .child(mongo_query_builder_value_cell(
                            path.clone(), operator, &row.value, colors, window, cx,
                        ))
                )
                .child(mongo_query_builder_remove_button(
                    tab_id, path, colors, cx,
                )),
        )
}

/// 字段格：可输入任意文档路径，打字时按前缀补全字段。
///
/// 外面是统一的外框（`mongo_query_box_at`），里面是代码编辑器——只有
/// `EditorState` 能挂补全 provider。编辑器外壳负责把行高撑出的那点多余高度裁掉。
fn mongo_query_builder_field_cell(
    path: MongoBuilderPath,
    field: &Entity<EditorState>,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = field.read(cx).focus_handle(cx).is_focused(window);
    let selector_path = mongo_builder_path_label(&path);
    mongo_query_box_at(
        MONGO_QUERY_BUILDER_CONTROL_HEIGHT,
        focused,
        colors,
        div()
            .h_full()
            .w_full()
            .flex()
            .items_center()
            .child(mongo_query_editor_body(
                field,
                Some(format!("mongo-query-builder-field-body:{selector_path}")),
                11.,
                Some(MONGO_QUERY_BUILDER_CELL_EDITOR_HEIGHT),
                false,
            )),
    )
    .debug_selector(move || format!("mongo-query-builder-field:{selector_path}"))
}

/// 运算符格：固定八项，当前项打勾。
fn mongo_query_builder_operator_cell(
    tab_id: TabId,
    path: MongoBuilderPath,
    current: MongoQueryOperator,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let view = cx.entity().downgrade();
    let selector_path = mongo_builder_path_label(&path);
    (Button::new(SharedString::from(format!(
        "mongo-builder-operator:{}:{selector_path}",
        tab_id.0
    )))
    .outline()
    .h(px(MONGO_QUERY_BUILDER_CONTROL_HEIGHT))
    .w_full()
    .text_size(ui_text_size(11.))
    .font_family(crate::PLATFORM_MONOSPACE_FONT)
    .text_color(colors.success)
    .label(current.label())
    .dropdown_caret(true)
    // 同字段格：`TopLeft` 向下弹。运算符下拉在条件行里，向上弹会盖住字段格与值格。
    .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
        let mut menu = menu.min_w(px(200.));
        for operator in MongoQueryOperator::ALL {
            let view = view.clone();
            let condition_path = path.clone();
            menu = menu.item(
                PopupMenuItem::new(operator.label())
                    .checked(operator == current)
                    .on_click(move |_, window, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.set_mongo_builder_operator(tab_id, condition_path.clone(), operator, window, cx);
                        });
                    }),
            );
        }
        menu
    }))
    .into_any_element()
}

/// 值格：打字按同行字段的已有取值补全；`$exists` 时禁用——它的值恒为布尔。
fn mongo_query_builder_value_cell(
    path: MongoBuilderPath,
    operator: MongoQueryOperator,
    value: &Entity<EditorState>,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = value.read(cx).focus_handle(cx).is_focused(window);
    let selector_path = mongo_builder_path_label(&path);
    mongo_query_box_at(
        MONGO_QUERY_BUILDER_CONTROL_HEIGHT,
        focused,
        colors,
        div()
            .h_full()
            .w_full()
            .flex()
            .items_center()
            .child(mongo_query_editor_body(
                value,
                Some(format!("mongo-query-builder-value-body:{selector_path}")),
                11.,
                Some(MONGO_QUERY_BUILDER_CELL_EDITOR_HEIGHT),
                operator == MongoQueryOperator::Exists,
            )),
    )
    .debug_selector(move || format!("mongo-query-builder-value:{selector_path}"))
}

fn mongo_query_builder_remove_button(
    tab_id: TabId,
    path: MongoBuilderPath,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selector_path = mongo_builder_path_label(&path);
    div().w(px(28.)).flex_none().flex().items_center().justify_center().child(
        Button::new(SharedString::from(format!(
            "mongo-builder-remove:{}:{selector_path}",
            tab_id.0
        )))
        .debug_selector(move || format!("mongo-query-builder-remove:{selector_path}"))
        .ghost()
        .small()
        .h(px(28.))
        .w(px(28.))
        .tooltip(tr("app.mongo.query.builder_remove"))
        .child(app_icon(AppIcon::Close, 12., colors.muted))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.remove_mongo_builder_node(tab_id, path.clone(), window, cx);
            cx.stop_propagation();
        })),
    )
}

fn mongo_query_builder_remove_group_button(
    tab_id: TabId,
    path: MongoBuilderPath,
    selector_path: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    let selector = selector_path.to_string();
    Button::new(SharedString::from(format!(
        "mongo-builder-remove-group:{}:{selector_path}",
        tab_id.0
    )))
    .debug_selector(move || format!("mongo-query-builder-remove-group:{selector}"))
    .ghost()
    .small()
    .h(px(20.))
    .w(px(20.))
    .tooltip(tr("app.mongo.query.builder_remove_group"))
    .child(app_icon(AppIcon::Close, 11., colors.muted))
    .on_click(cx.listener(move |this, _, window, cx| {
        this.remove_mongo_builder_node(tab_id, path.clone(), window, cx);
        cx.stop_propagation();
    }))
}

fn mongo_query_builder_add_button(
    tab_id: TabId,
    path: MongoBuilderPath,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    let selector_path = mongo_builder_path_label(&path);
    Button::new(SharedString::from(format!(
        "mongo-query-builder-add:{}:{selector_path}",
        tab_id.0
    )))
        .debug_selector(move || format!("mongo-query-builder-add:{selector_path}"))
        .ghost()
        .small()
        .h(px(22.))
        .text_size(ui_text_size(10.))
        .text_color(colors.success)
        .label(tr("app.mongo.query.builder_add"))
        .child(app_icon(AppIcon::Plus, 12., colors.success))
        .disabled(!enabled)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.add_mongo_builder_condition(tab_id, path.clone(), window, cx);
            cx.stop_propagation();
        }))
}

fn mongo_query_builder_add_group_button(
    tab_id: TabId,
    path: MongoBuilderPath,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    let selector_path = mongo_builder_path_label(&path);
    Button::new(SharedString::from(format!(
        "mongo-query-builder-add-group:{}:{selector_path}",
        tab_id.0
    )))
    .debug_selector(move || format!("mongo-query-builder-add-group:{selector_path}"))
    .ghost()
    .small()
    .h(px(22.))
    .text_size(ui_text_size(10.))
    .text_color(colors.success)
    .label(tr("app.mongo.query.builder_add_group"))
    .child(app_icon(AppIcon::Plus, 12., colors.success))
    .disabled(!enabled)
    .on_click(cx.listener(move |this, _, window, cx| {
        this.add_mongo_builder_group(tab_id, path.clone(), window, cx);
        cx.stop_propagation();
    }))
}



#[cfg(test)]
mod mongo_query_builder_tests {
    use super::*;

    /// 条件行 → JSON：等值平铺、其余包 `$op`，`$in` 按逗号拆开，空值跳过。
    #[test]
    fn conditions_build_filter_json() {
        let mut document = serde_json::Map::new();
        for (field, operator, text) in [
            ("status", MongoQueryOperator::Eq, "paid"),
            ("amount", MongoQueryOperator::Gte, "500"),
        ] {
            let value = mongo_builder_condition_value(operator, text).expect("value");
            document.insert(field.into(), mongo_builder_expression(operator, value));
        }
        assert_eq!(
            serde_json::Value::Object(document).to_string(),
            r#"{"status":"paid","amount":{"$gte":500}}"#
        );
        // 空值不产出条件：加了一行还没填不该把结果查空。
        assert!(mongo_builder_condition_value(MongoQueryOperator::Eq, "  ").is_none());
        // `$exists` 没有空值概念，值恒为布尔。
        assert_eq!(
            mongo_builder_condition_value(MongoQueryOperator::Exists, ""),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(
            mongo_builder_condition_value(MongoQueryOperator::In, "a, 2"),
            Some(serde_json::Value::Array(vec![
                serde_json::Value::String("a".into()),
                serde_json::Value::from(2)
            ]))
        );
    }

    /// JSON → 条件行 → JSON 必须原样往返，尤其别把字符串 `"500"` 变成数字。
    #[test]
    fn filter_round_trips_through_conditions() {
        let filter = r#"{"code":"500","amount":500,"ok":true,"tags":{"$in":["a","b"]},"note":{"$exists":false}}"#;
        let conditions = parse_mongo_builder_filter(filter).expect("supported filter");
        assert_eq!(mongo_builder_group_json(&conditions).to_string(), filter);
    }

    /// 不支持的字段表达式拒绝；`$or` 的往返由表达式树测试覆盖。
    #[test]
    fn complex_filters_are_rejected() {
        for filter in [
            r#"{"a":{"$regex":"^x"}}"#,
            r#"{"a":{"b":1}}"#,
            r#"{"a":{"$gt":1,"$lt":9}}"#,
            r#"{"a":[1,2]}"#,
            r#"{"a":{"$exists":"yes"}}"#,
            r#"not json"#,
            r#"[1,2]"#,
        ] {
            assert!(
                parse_mongo_builder_filter(filter).is_err(),
                "{filter} must not be rewritten by the builder"
            );
        }
        assert!(parse_mongo_builder_filter("  ").unwrap().children.is_empty());
    }

    /// Compass 查询栏常见的宽松写法要能展开：连接器执行时本来就收（先严格 JSON、
    /// 再 JSON5），构建器若只认严格 JSON，就会出现「能查、构建器却报语法错」。
    #[test]
    fn relaxed_filter_syntax_parses() {
        // 无引号键（截图里那条）。
        let group = parse_mongo_builder_filter(r#"{event_type: "page_view"}"#)
            .expect("unquoted key is a Compass-style filter");
        let MongoBuilderNode::Condition(condition) = &group.children[0] else {
            panic!("condition");
        };
        assert_eq!(condition.field, "event_type");
        assert_eq!(condition.operator, MongoQueryOperator::Eq);
        assert_eq!(condition.value, r#""page_view""#);

        // 单引号、尾逗号、注释也一并收。
        let conditions = parse_mongo_builder_filter(
            "{\n  // 只看已支付\n  status: 'paid',\n  amount: 500,\n}",
        )
        .expect("single quotes, trailing comma and comments are accepted");
        assert_eq!(conditions.children.len(), 2);

        // 但 `NaN` 会被读成 null，等于把条件换成「等于 null」——必须拒。
        assert!(parse_mongo_builder_filter("{amount: NaN}").is_err());
        assert!(parse_mongo_builder_filter("{amount: Infinity}").is_err());
    }
}
