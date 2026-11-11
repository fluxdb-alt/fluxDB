// MongoDB 集合工作台的文档展示区：可折叠文档卡片、类型化字段列表、表格视图与整页 JSON。
//
// 字段列表对齐 Compass 的 `.field-list` 写法：
// 每行 `key: value` 带 BSON 类型徽标，对象/数组默认收起为 `{ N fields }` / `[ N items ]`。
// 卡片与嵌套字段的展开态都是本页交互状态，按标签页记忆，不写回标签页数据。

// 别名：与 `json_editor/parser.rs` 同处 crate 根作用域，同名导入会冲突。
use serde_json::Value as JsonValue;

/// 表格单元格最大字符数：超长值截断，完整内容看悬停提示或详情抽屉。
const MONGO_VALUE_MAX_CHARS: usize = 240;

/// 表格列宽（与 Compass 相同的定宽列，超出横向滚动）。
const MONGO_TABLE_CELL_WIDTH: f32 = 180.;

/// 表格行高与行号列宽。
const MONGO_TABLE_ROW_HEIGHT: f32 = 24.;
const MONGO_TABLE_ROW_NUM_WIDTH: f32 = 38.;

/// 表格右侧固定操作列的宽度：四个 24px 图标按钮 + 间距 + 内边距。
const MONGO_TABLE_ACTIONS_WIDTH: f32 = 116.;

/// 180px 宽、11px 等宽字体大约能显示 26 个字符；超过才挂悬停提示。
const MONGO_TABLE_CELL_VISIBLE_CHARS: usize = 26;

/// 卡片头图标按钮边长（交互稿 `.document-head .icon-button` 为 24px）。
const MONGO_CARD_ICON_BUTTON_SIZE: f32 = 24.;

/// JSON 视图的缩进步长与行高（交互稿 `.json-code` 为 11px/1.9）。
const MONGO_JSON_INDENT: usize = 2;
const MONGO_JSON_ROW_HEIGHT: f32 = 21.;

/// 行号列宽（交互稿 `.line-number`：31px 右对齐，右侧留 12px）。
const MONGO_JSON_LINE_NUM_WIDTH: f32 = 31.;

/// Extended JSON 的 BSON 标量包装：单键且键名以 `$` 开头，不是嵌套文档。
const MONGO_BSON_WRAPPERS: &[(&str, &str)] = &[
    ("$oid", "ObjectId"),
    ("$date", "Date"),
    ("$numberDecimal", "Decimal128"),
    ("$numberLong", "Int64"),
    ("$timestamp", "Timestamp"),
    ("$binary", "Binary"),
    ("$regularExpression", "Regex"),
    ("$uuid", "Binary"),
];

/// 字段值的 BSON 类型（与原型 `bsonType` / Compass 徽标一致）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MongoFieldKind {
    /// BSON 标量包装，携带展示用类型名（`ObjectId` / `Decimal128` 等）。
    Bson(&'static str),
    Object,
    Array,
    String,
    Boolean,
    Int32,
    Double,
    Null,
}

impl MongoFieldKind {
    fn label(self) -> &'static str {
        match self {
            Self::Bson(name) => name,
            Self::Object => "Object",
            Self::Array => "Array",
            Self::String => "String",
            Self::Boolean => "Boolean",
            Self::Int32 => "Int32",
            Self::Double => "Double",
            Self::Null => "Null",
        }
    }

    /// 对象/数组才可展开，其余都是单行标量。
    fn is_nested(self) -> bool {
        matches!(self, Self::Object | Self::Array)
    }
}

/// 判类型：只有「单键且键名是已知 BSON 包装」才当标量，避免把 `{ "$oid": … }`
/// 误渲染成一层的嵌套对象。
fn mongo_field_kind(value: &JsonValue) -> MongoFieldKind {
    match value {
        JsonValue::Null => MongoFieldKind::Null,
        JsonValue::Bool(_) => MongoFieldKind::Boolean,
        JsonValue::String(_) => MongoFieldKind::String,
        JsonValue::Array(_) => MongoFieldKind::Array,
        // serde_json 里整数字面量不会带小数点，够区分 Int32 与 Double。
        JsonValue::Number(number) => {
            if number.is_f64() {
                MongoFieldKind::Double
            } else {
                MongoFieldKind::Int32
            }
        }
        JsonValue::Object(map) => {
            let wrapper = (map.len() == 1)
                .then(|| map.keys().next())
                .flatten()
                .and_then(|key| {
                    MONGO_BSON_WRAPPERS
                        .iter()
                        .find(|(name, _)| name == key)
                        .map(|(_, label)| *label)
                });
            wrapper.map_or(MongoFieldKind::Object, MongoFieldKind::Bson)
        }
    }
}

/// 标量值文本：BSON 包装按 Compass 风格写 `ObjectId("…")`，其余用紧凑 JSON。
fn mongo_field_value_label(value: &JsonValue) -> String {
    for (key, name) in MONGO_BSON_WRAPPERS {
        let Some(inner) = value.get(*key) else {
            continue;
        };
        return match inner {
            JsonValue::String(text) => format!("{name}(\"{text}\")"),
            // `$date` 的规范形式是 `{ "$numberLong": "…" }`。
            JsonValue::Object(fields) => fields
                .get("$numberLong")
                .and_then(JsonValue::as_str)
                .map(|millis| format!("{name}(\"{millis}\")"))
                .unwrap_or_else(|| format!("{name}({})", JsonValue::Object(fields.clone()))),
            other => format!("{name}({other})"),
        };
    }
    value.to_string()
}

/// 嵌套项的收起摘要：`{ N fields }` / `[ N items ]`。
fn mongo_field_summary(kind: MongoFieldKind, value: &JsonValue) -> SharedString {
    let count = match value {
        JsonValue::Array(items) => items.len(),
        JsonValue::Object(map) => map.len(),
        _ => 0,
    };
    let key = if kind == MongoFieldKind::Array {
        "app.mongo.item_count"
    } else {
        "app.mongo.field_count"
    };
    tr(key).replace("%{count}", &count.to_string()).into()
}

/// 值文字色沿用 JSON 语法配色，避免另起一套色板。
fn mongo_field_value_color(
    kind: MongoFieldKind,
    theme: JsonEditorTheme,
    colors: UiColors,
) -> gpui::Rgba {
    match kind {
        MongoFieldKind::String => theme.string,
        MongoFieldKind::Boolean => theme.boolean,
        MongoFieldKind::Null => colors.muted,
        MongoFieldKind::Int32 | MongoFieldKind::Double => theme.number,
        MongoFieldKind::Bson("Decimal128")
        | MongoFieldKind::Bson("Int64")
        | MongoFieldKind::Bson("Timestamp") => theme.number,
        _ => colors.muted,
    }
}

/// 文档折叠态的身份：优先 `_id`（翻页、刷新后同一文档仍是同一张卡），缺失时退回页内下标。
fn mongo_document_key(document: &fluxdb_core::MongoDocument, index: usize) -> String {
    if document.id.is_empty() {
        format!("#{index}")
    } else {
        document.id.clone()
    }
}

/// 一页文档的只读快照：Extended JSON 只解析一次，三种展示方式与虚拟列表共用。
///
/// 放进 `Rc` 是为了让虚拟列表的逐项渲染闭包持有同一份数据（`'static`），
/// 不必每帧克隆文档或重新解析。
struct MongoPageData {
    /// 与 `values` 下标一一对应的原始文档（操作按钮、复制、删除仍要原文）。
    documents: Vec<fluxdb_core::MongoDocument>,
    /// 每篇文档解析后的值；非法 JSON 为 `None`，界面退回纯文本。
    values: Vec<Option<JsonValue>>,
    table: MongoTableLayout,
}

impl MongoPageData {
    fn build(page: Option<&fluxdb_core::MongoDocumentsPage>) -> Self {
        Self::from_documents(page.map_or_else(Vec::new, |page| page.documents.clone()))
    }

    /// 从任意一批文档建快照；工作台的结果不走 `MongoDocumentsPage`，共用同一套解析与表格列。
    fn from_documents(documents: Vec<fluxdb_core::MongoDocument>) -> Self {
        let values = documents
            .iter()
            .map(|document| serde_json::from_str::<JsonValue>(&document.json).ok())
            .collect::<Vec<_>>();
        Self {
            table: MongoTableLayout::build(&values),
            documents,
            values,
        }
    }
}

/// 一页文档的渲染缓存：数据快照 + 虚拟列表状态。
///
/// 逐帧重建的只有元素本身；解析结果、表格列与行文本、滚动状态都留在这里，
/// 因此重绘不会重新 `from_str` 100 篇文档，也不会把滚动位置顶回顶部。
/// 加载令牌（`load_generation`）、文档条数变化，或在途期间建的缓存等到结果落地时整份重建。
struct MongoDocumentCache {
    generation: u64,
    /// 建缓存时查询是否仍在途。在途期间 `page` 还是上一页的旧快照（`begin_load` 已推进
    /// 令牌但结果未回），据此建的缓存必须等结果落地后重建，光比对令牌与条数会漏判。
    loading: bool,
    count: usize,
    data: Rc<MongoPageData>,
    /// 文档 / JSON 卡片的虚拟列表状态。卡片高度随展开态变化，由 `ListState` 自行测量；
    /// 折叠、展开、切换展示方式后由对应动作 `remeasure`。
    card_list: ListState,
    /// 表格纵向滚动句柄：数据区与右侧固定操作列两个列表共用，保证逐行对齐。
    table_scroll: UniformListScrollHandle,
}

impl MongoDocumentCache {
    fn build(state: &MongoCollectionState) -> Self {
        let data = Rc::new(MongoPageData::build(state.page.as_ref()));
        Self {
            generation: state.load_generation,
            loading: state.loading,
            count: data.documents.len(),
            card_list: ListState::new(data.documents.len(), ListAlignment::Top, px(400.)),
            table_scroll: UniformListScrollHandle::new(),
            data,
        }
    }
}

impl NavicatMain {
    /// 确保标签页的文档渲染缓存在；加载令牌或条数变化时整份重建。
    fn ensure_mongo_document_cache(&mut self, tab_id: TabId, state: &MongoCollectionState) {
        let count = state.page.as_ref().map_or(0, |page| page.documents.len());
        let stale = self.mongo_document_cache.get(&tab_id).is_none_or(|cache| {
            cache.generation != state.load_generation
                || cache.count != count
                // 缓存是在查询在途时建的吗？那时拿到的是上一页的快照，令牌与条数都可能
                // 与落地后的结果一模一样（同样是 14 条、同一个令牌），只有内容换了，
                // 光比前两项会把过期内容当成最新的。结果一落地就重建。
                || (cache.loading && !state.loading)
        });
        if stale {
            self.mongo_document_cache
                .insert(tab_id, MongoDocumentCache::build(state));
        }
    }

    /// 卡片正文高度变了（折叠 / 展开 / 换展示方式）：让虚拟列表重新测量。
    ///
    /// `ListState` 只按锚点项保持滚动位置，不重测的话展开后下方卡片会按旧高度排布。
    fn remeasure_mongo_cards(&self, tab_id: TabId) {
        if let Some(cache) = self.mongo_document_cache.get(&tab_id) {
            cache.card_list.remeasure();
        }
    }

    /// 折叠/展开一张文档卡片。
    pub(crate) fn toggle_mongo_document_collapsed(
        &mut self,
        tab_id: TabId,
        document_key: String,
        cx: &mut Context<Self>,
    ) {
        let collapsed = self.mongo_collapsed_documents.entry(tab_id).or_default();
        if !collapsed.remove(&document_key) {
            collapsed.insert(document_key);
        }
        self.remeasure_mongo_cards(tab_id);
        cx.notify();
    }

    /// 展开/收起 JSON 视图里的一个 object/array 节点；`key` 形如 `{文档身份}\u{1e}{折叠路径}`。
    ///
    /// 集合记录的是「已展开」的路径：默认全部收起，收起即回到默认态。
    pub(crate) fn toggle_mongo_json_fold(
        &mut self,
        tab_id: TabId,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let toggles = self.mongo_json_fold_toggles.entry(tab_id).or_default();
        if !toggles.remove(&key) {
            toggles.insert(key);
        }
        self.remeasure_mongo_cards(tab_id);
        cx.notify();
    }

    /// 展开/收起文档内的嵌套对象或数组；`key` 形如 `{文档身份}/{字段路径}`。
    pub(crate) fn toggle_mongo_field(
        &mut self,
        tab_id: TabId,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let expanded = self.mongo_expanded_fields.entry(tab_id).or_default();
        if !expanded.remove(&key) {
            expanded.insert(key);
        }
        self.remeasure_mongo_cards(tab_id);
        cx.notify();
    }
}

/// 文档展示区的渲染参数：同一标签页内不变的交互状态。
///
/// 卡片与字段行是递归的，状态逐层传参会堆到 8 个参数，收进一个借用视图里更清楚。
/// 三个集合都是 `Rc`：虚拟列表逐项渲染的闭包要 `'static` 持有同一份状态，
/// 每帧克隆一份 `BTreeSet` 反而更贵。
#[derive(Clone)]
struct MongoDocumentView {
    tab_id: TabId,
    /// 已折叠的卡片（文档身份集合）。
    collapsed_documents: Rc<BTreeSet<String>>,
    /// 已展开的嵌套字段（`{文档身份}/{字段路径}` 集合）。
    expanded_fields: Rc<BTreeSet<String>>,
    /// 被显式展开的 JSON 节点（`{文档身份}\u{1e}{折叠路径}` 集合）；其余嵌套节点默认收起。
    json_fold_toggles: Rc<BTreeSet<String>>,
    /// 本页文档快照（原文 + 解析结果），来自渲染缓存。
    page: Rc<MongoPageData>,
    /// 排在当前列表之前的文档数：窗口级选择按文档序号 + 行号排阅读顺序，
    /// 同一窗口里有多个列表（工作台的多条记录）时必须把序号接上，否则行序重复会让选区跳到别的卡片。
    document_order_base: u64,
    colors: UiColors,
}

impl MongoDocumentView {
    /// 第 `index` 篇文档的解析结果；非法 JSON 为 `None`。
    fn value(&self, index: usize) -> Option<&JsonValue> {
        self.page.values.get(index).and_then(Option::as_ref)
    }
}

/// 卡片内的无边框图标按钮（折叠箭头、详情、复制、删除）。
///
/// 交互稿里 `.document-head .icon-button` 是 24px 无边框、hover 才出色块；
/// 通用工具栏按钮（30px + 描边）塞进 30px 的卡片头会顶满并显得笨重。
/// `enabled = false` 用于尚未实现的入口：仍然占位（保持按钮顺序稳定），
/// 但降透明、不响应点击、光标不变手型。
/// `spinning` 让图标自身旋转（刷新在途），不必换成另一种图形。
#[allow(clippy::too_many_arguments)]
fn mongo_card_icon_button(
    id: String,
    selector: String,
    icon: AppIcon,
    tooltip: SharedString,
    color: gpui::Rgba,
    enabled: bool,
    spinning: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .size(px(MONGO_CARD_ICON_BUTTON_SIZE))
        .flex_none()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .debug_selector(move || selector);
    if enabled {
        button
            .cursor_pointer()
            .hover(move |style| style.bg(colors.hover))
            .child(mongo_card_icon(icon, color, spinning))
    } else {
        // 卡片头整条可点折叠，占位按钮要把点击吞掉，否则点它会误折叠卡片。
        button
            .cursor_default()
            .opacity(0.4)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(mongo_card_icon(icon, color, spinning))
    }
}

/// 卡片头图标：`spinning` 时绕自身中心匀速旋转（刷新在途的就地反馈）。
fn mongo_card_icon(icon: AppIcon, color: gpui::Rgba, spinning: bool) -> AnyElement {
    let icon = svg()
        .size(px(12.))
        .path(app_icon_path(icon))
        .text_color(color);
    if !spinning {
        return icon.into_any_element();
    }
    icon.with_animation(
        "mongo-card-icon-spin",
        Animation::new(Duration::from_secs_f64(0.75))
            .repeat()
            .with_easing(ease_in_out),
        |this, delta| this.with_transformation(Transformation::rotate(percentage(delta))),
    )
    .into_any_element()
}

/// 一篇文档的四个操作按钮：编辑、复制文档（新建一条）、复制到剪贴板、删除。
///
/// 文档卡片、JSON 卡片与表格操作列共用这一份实现——三处的按钮顺序、可用状态
/// 与点击行为必须完全一致，否则用户换个展示方式就找不到同一个操作。
/// 只读对象（视图）只保留「编辑」（实际打开只读详情）与「复制」。
fn mongo_document_actions(
    view: &MongoDocumentView,
    index: usize,
    document: &fluxdb_core::MongoDocument,
    selected: bool,
    read_only: bool,
    cx: &mut Context<NavicatMain>,
) -> Vec<Stateful<Div>> {
    let tab_id = view.tab_id;
    let colors = view.colors;
    let json = document.json.clone();
    let delete_json = document.json.clone();
    let mut actions = Vec::with_capacity(4);
    // 编辑与详情共用一个入口：点开就是右侧抽屉，抽屉内直接可编辑。
    actions.push(
        mongo_card_icon_button(
            format!("mongo-doc-detail-{index}"),
            format!("mongo-document-detail:{index}"),
            AppIcon::Edit,
            tr("app.mongo.edit_document"),
            if selected { colors.info } else { colors.text },
            true,
            false,
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if selected {
                    this.close_mongo_document_drawer(tab_id, cx);
                } else {
                    this.open_mongo_document_drawer(tab_id, index, window, cx);
                }
                cx.stop_propagation();
            }),
        ),
    );
    // 复制文档：把这篇剥掉 `_id` 后插成新文档；只读对象不渲染插入入口。
    if !read_only {
        let duplicate_json = document.json.clone();
        actions.push(
            mongo_card_icon_button(
                format!("mongo-doc-duplicate-{index}"),
                format!("mongo-document-duplicate:{index}"),
                AppIcon::CopyPlus,
                tr("app.mongo.duplicate_document"),
                colors.text,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.open_mongo_document_duplicate(tab_id, &duplicate_json, window, cx);
                    cx.stop_propagation();
                }),
            ),
        );
    }
    actions.push(
        mongo_card_icon_button(
            format!("mongo-doc-copy-{index}"),
            format!("mongo-document-copy:{index}"),
            AppIcon::Copy,
            tr("app.mongo.copy_document"),
            colors.text,
            true,
            false,
            colors,
        )
        // 写剪贴板是无声操作：不给反馈，用户无法判断到底复制成功没有。
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(json.clone()));
                this.show_message(
                    tr("app.mongo.copy_done").as_ref(),
                    AppMessageKind::Success,
                    cx,
                );
                cx.stop_propagation();
            }),
        ),
    );
    if !read_only {
        actions.push(
            mongo_card_icon_button(
                format!("mongo-doc-delete-{index}"),
                format!("mongo-document-delete:{index}"),
                AppIcon::Trash,
                tr("app.mongo.delete_document"),
                colors.danger,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.request_delete_mongo_document(tab_id, delete_json.clone(), cx);
                    cx.stop_propagation();
                }),
            ),
        );
    }
    actions
}

/// 卡片渲染需要的那部分标签页状态：只读、展示方式、是否选中。
///
/// 虚拟列表的逐项渲染闭包必须 `'static`，借不到 `&MongoCollectionState`；
/// 这三个都是 `Copy` 的界面状态，逐项复制即可。
#[derive(Clone, Copy)]
struct MongoCardContext {
    display: MongoDisplayMode,
    read_only: bool,
    selected_document: Option<usize>,
}

impl MongoCardContext {
    fn from_state(state: &MongoCollectionState) -> Self {
        Self {
            display: state.display,
            read_only: state.read_only,
            selected_document: state.selected_document,
        }
    }
}

/// 一条文档卡片：折叠箭头 + 序号 + `_id`、四个操作按钮；正文是字段列表。
fn mongo_document_card(
    view: &MongoDocumentView,
    index: usize,
    document: &fluxdb_core::MongoDocument,
    context: MongoCardContext,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tab_id = view.tab_id;
    let colors = view.colors;
    let selected = context.selected_document == Some(index);
    let document_key = mongo_document_key(document, index);
    let collapsed = view.collapsed_documents.contains(&document_key);
    let toggle_key = document_key.clone();
    let card_selector = format!("mongo-document-card:{index}");
    let head_selector = format!("mongo-document-head:{index}");
    let toggle_selector = format!("mongo-document-toggle:{index}");
    let actions = mongo_document_actions(view, index, document, selected, context.read_only, cx);

    // 解析结果来自渲染缓存：逐帧重绘不再 `from_str` 整篇文档。
    let value = view.value(index);
    // 原型同一张卡片在「文档 / JSON」两种展示下共用卡头，只换正文：
    // 文档视图给字段列表，JSON 视图给带行号着色的整篇 JSON。
    let body = if collapsed {
        div()
    } else if context.display == MongoDisplayMode::Json {
        mongo_json_block(view, index, document, value, &document_key, "mongo-document", cx)
    } else {
        mongo_field_list(view, index, document, value, &document_key, "mongo-document", cx)
    };

    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(if selected { colors.info } else { colors.border })
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .overflow_hidden()
        .debug_selector(move || card_selector)
        .child(
            div()
                .h(px(30.))
                .px_1p5()
                .flex()
                .items_center()
                .gap_1p5()
                .border_b_1()
                .border_color(colors.border_soft)
                // 整条卡头都是折叠区域：点箭头、点序号、点空白处都能收起/展开。
                // 操作按钮各自 `stop_propagation`，所以点它们不会连带折叠卡片。
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .debug_selector(move || head_selector)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_mongo_document_collapsed(tab_id, toggle_key.clone(), cx);
                        cx.stop_propagation();
                    }),
                )
                .child(mongo_card_icon_button(
                    format!("mongo-doc-toggle-{index}"),
                    toggle_selector,
                    if collapsed {
                        AppIcon::ChevronRight
                    } else {
                        AppIcon::ChevronDown
                    },
                    if collapsed {
                        tr("app.mongo.expand_document")
                    } else {
                        tr("app.mongo.collapse_document")
                    },
                    colors.muted,
                    true,
                    false,
                    colors,
                ))
                .child(
                    div()
                        .pr_2()
                        .border_r_1()
                        .border_color(colors.border_soft)
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(SharedString::from(format!("#{}", index + 1))),
                )
                .child(
                    // `_id` 是字段名不是文案，复用表格列头同一个 key，别另起字面量。
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_size(ui_text_size(11.))
                        .text_color(colors.text)
                        .child(SharedString::from(format!(
                            "{}: {}",
                            tr("app.mongo.table.id"),
                            document.id
                        ))),
                )
                .children(actions),
        )
        .child(body)
}

/// 卡片内 JSON 折叠键：`{文档身份}\u{1e}{折叠路径}`，与嵌套字段展开键同属标签页级交互状态。
fn mongo_json_fold_key(document_key: &str, path_key: &str) -> String {
    format!("{document_key}\u{1e}{path_key}")
}

/// JSON 视图的文档正文：带行号、语法配色与折叠箭头的 pretty JSON。
///
/// 折叠能力来自 `json_editor`（`build_pretty` 产出节点 + `json_fold_view` 渲染），
/// 这里只负责按文档身份筛出该卡片自己的折叠集合，不自己实现折叠。
fn mongo_json_block(
    view: &MongoDocumentView,
    index: usize,
    document: &fluxdb_core::MongoDocument,
    value: Option<&JsonValue>,
    document_key: &str,
    // 视图作用域：同一页可能有多个宿主（集合页、工作台多条记录）同时渲染第 0 篇文档，
    // 元素 id 必须带上宿主前缀，否则不同卡片的折叠行会共用全局状态。
    id_scope: &str,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selector = format!("{id_scope}-json:{index}");
    let colors = view.colors;
    // 用户点过的节点（去掉文档身份前缀）。默认态是「根展开、嵌套全收起」，
    // 与默认相反的节点才会出现在这里，因此空集合直接等于完整默认态。
    let prefix = format!("{document_key}\u{1e}");
    let toggled = view
        .json_fold_toggles
        .iter()
        .filter_map(|key| key.strip_prefix(&prefix))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    // 先完整解析拿到所有可折叠节点，再按「深度 > 0 且未点开」算出需要折叠的路径。
    // 展开态结果同时用来取折叠行的真实内容（复制用），避免再解析一遍。
    let unfolded = value.map(|value| build_pretty_value(value, MONGO_JSON_INDENT, &BTreeSet::new()));
    let folds = unfolded
        .as_ref()
        .map(|unfolded| {
            unfolded
                .nodes
                .iter()
                .filter(|node| {
                    !node.path.is_empty() && !toggled.contains(&json_fold_path_key(&node.path))
                })
                .map(|node| json_fold_path_key(&node.path))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    let body = match value {
        Some(value) => {
            let pretty = build_pretty_value(value, MONGO_JSON_INDENT, &folds);
            let expanded_copy = unfolded
                .as_ref()
                .map(|unfolded| mongo_expanded_copy(unfolded, &pretty))
                .unwrap_or_default();
            let entity = cx.entity().downgrade();
            let tab_id = view.tab_id;
            let document_key = document_key.to_string();
            json_fold_view(
                &JsonFoldView {
                    display: &pretty.text,
                    nodes: &pretty.nodes,
                    expanded_copy: Some(&expanded_copy),
                    row_height: MONGO_JSON_ROW_HEIGHT,
                    line_num_width: Some(MONGO_JSON_LINE_NUM_WIDTH),
                    line_num_text_size: 9.,
                    text_size: 11.,
                    folding: true,
                    highlight: true,
                    indent_size: MONGO_JSON_INDENT,
                    error_line: None,
                    // 与其他卡片的行序错开：文档序号进高位，行号留低位。
                    order_base: (view.document_order_base + index as u64) << 32,
                    selector_prefix: Some(format!("{id_scope}-json-fold:{index}")),
                    id_salt: format!("{id_scope}-json-fold:{index}").into(),
                },
                colors,
                move |path, _window, cx| {
                    let key = mongo_json_fold_key(&document_key, &json_fold_path_key(&path));
                    let _ = entity.update(cx, |this, cx| {
                        this.toggle_mongo_json_fold(tab_id, key, cx);
                    });
                },
            )
        }
        // 非法 JSON（理论上不会出现，服务端已序列化）：退回纯文本，至少可读。
        None => div().px_2().py_2().child(
            div()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(SharedString::from(document.json.clone())),
        ),
    };

    div()
        .px_2()
        .py_2()
        .flex()
        .flex_col()
        .bg(colors.panel_bg)
        .debug_selector(move || selector)
        .child(body)
}

/// 折叠行的复制文本：按路径匹配展开态的同一节点，用它的完整行区间替换屏上的 `{ … }`。
///
/// 键是展示态行号（行号随折叠变化，所以每帧重算），值是展开后的多行文本（含 key 与缩进）。
fn mongo_expanded_copy(
    unfolded: &PrettyJson,
    folded: &PrettyJson,
) -> std::collections::HashMap<usize, String> {
    let lines = unfolded.text.lines().collect::<Vec<_>>();
    let mut copy = std::collections::HashMap::new();
    for node in folded.nodes.iter().filter(|node| node.folded) {
        let key = json_fold_path_key(&node.path);
        let Some(unfolded_node) = unfolded
            .nodes
            .iter()
            .find(|candidate| json_fold_path_key(&candidate.path) == key)
        else {
            continue;
        };
        let Some(block) = lines.get(unfolded_node.start_line..=unfolded_node.end_line) else {
            continue;
        };
        copy.insert(node.start_line, block.join("\n"));
    }
    copy
}

/// 文档正文：顶层字段逐行渲染。
///
/// `id_scope` 区分同一窗口里可能同时存在的多个宿主（集合页、工作台、聚合预览），
/// 字段文本元素 id 带上它，避免不同卡片共用选择状态。
fn mongo_field_list(
    view: &MongoDocumentView,
    index: usize,
    document: &fluxdb_core::MongoDocument,
    value: Option<&JsonValue>,
    document_key: &str,
    id_scope: &str,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let colors = view.colors;
    let Some(JsonValue::Object(map)) = value else {
        // 驱动只会返回 JSON；真解析不了就展示原文，别留一片空白。
        return div()
            .p_2()
            .font_family(crate::PLATFORM_MONOSPACE_FONT)
            .text_size(ui_text_size(11.))
            .text_color(colors.text)
            .child(SharedString::from(document.json.clone()));
    };
    let selector = format!("mongo-document-fields:{index}");
    let mut list = div()
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .debug_selector(move || selector);
    // 一张卡片的共享渲染上下文：配色只算一次，行序号按渲染顺序（先父行、后子行）递增，
    // 它同时决定拖选/复制时的阅读顺序。
    // 高位放文档序号、低位放行号：同一窗口的所有卡片行序必须全局唯一，
    // 否则选区端点比较会误判方向，拖选时跳到别的文档。
    let mut context = MongoFieldContext {
        id_scope,
        document_key,
        theme: JsonEditorTheme::from_colors(colors),
        order: (view.document_order_base + index as u64) << 32,
    };
    for (key, value) in map {
        list = list.child(mongo_field_row(view, &mut context, key, value, "", cx));
    }
    list
}

/// 文档字段行的共享渲染上下文；`order` 随渲染递增，标记选择文档里的行序。
struct MongoFieldContext<'a> {
    id_scope: &'a str,
    document_key: &'a str,
    theme: JsonEditorTheme,
    order: u64,
}

/// 字段文本元素 id：同一卡片内按字段路径唯一，跨帧稳定（选择句柄按它保留）。
fn mongo_field_text_id(id_scope: &str, document_key: &str, path: &str) -> gpui::ElementId {
    gpui::ElementId::Name(format!("{id_scope}:field-text:{document_key}:{path}").into())
}

/// 字段行只给文字上色：选择层要求一行是一个参与者，否则复制会在每段间插换行。
fn mongo_field_highlight(color: gpui::Rgba) -> gpui::HighlightStyle {
    gpui::HighlightStyle {
        color: Some(color.into()),
        ..Default::default()
    }
}

/// 一个字段行：标量一行到底；对象/数组渲染成可点开的摘要 + 缩进子列表。
///
/// 文字部分整行合成一个可选中元素（双击选词、拖选、复制）；类型徽标不参与选择，
/// 避免复制结果里混入 `String` / `Object` 这类装饰信息。
fn mongo_field_row(
    view: &MongoDocumentView,
    context: &mut MongoFieldContext<'_>,
    key: &str,
    value: &JsonValue,
    parent_path: &str,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tab_id = view.tab_id;
    let colors = view.colors;
    let path = if parent_path.is_empty() {
        key.to_string()
    } else {
        format!("{parent_path}/{key}")
    };
    let kind = mongo_field_kind(value);
    let row_order = context.order;
    context.order += 1;
    let (id_scope, document_key, theme) =
        (context.id_scope, context.document_key, context.theme);

    let row = div()
        .min_h(px(22.))
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_size(ui_text_size(11.));

    // `key: 值` 一行一个元素，按字节区间给 key / 冒号 / 值分别上色。
    // `copy_text` 用于嵌套行：屏上是 `{ … }`，复制时给完整内容。
    let field_text =
        |value_text: &str, value_color: gpui::Rgba, copy_text: Option<gpui::SharedString>| {
        let mut text = String::with_capacity(key.len() + value_text.len() + 2);
        text.push_str(key);
        text.push(':');
        let colon = key.len()..text.len();
        text.push(' ');
        let value_range = text.len()..text.len() + value_text.len();
        text.push_str(value_text);
        let highlights = vec![
            (0..key.len(), mongo_field_highlight(theme.key)),
            (colon, mongo_field_highlight(theme.punctuation)),
            (value_range, mongo_field_highlight(value_color)),
        ];
        let text_selector = format!("{id_scope}:field-text:{document_key}:{path}");
        let mut element =
            SelectableStyledText::new(mongo_field_text_id(id_scope, document_key, &path), text)
                .highlights(highlights)
                .document_order(row_order);
        if let Some(copy_text) = copy_text {
            element = element.copy_text(copy_text);
        }
        div()
            .min_w(px(0.))
            .truncate()
            // 调试选择器供无头用例定位可选中的字段文本。
            .debug_selector(move || text_selector.clone())
            .child(element)
    };

    if !kind.is_nested() {
        return row
            .flex()
            .items_center()
            .overflow_hidden()
            .child(field_text(
                &mongo_field_value_label(value),
                mongo_field_value_color(kind, theme, colors),
                None,
            ))
            .child(
                div()
                    .flex_none()
                    .pl_3()
                    .text_size(ui_text_size(9.))
                    .text_color(colors.muted)
                    .child(SharedString::from(kind.label())),
            );
    }

    // 嵌套项：先给摘要行，展开后再按同一规则递归渲染子项；数组下标与原型一致用 `[i]`。
    let expanded_key = format!("{document_key}/{path}");
    let is_open = view.expanded_fields.contains(&expanded_key);
    let toggle_key = expanded_key.clone();
    let selector = format!("mongo-field-toggle:{document_key}/{path}");
    let summary = mongo_field_summary(kind, value);
    // 嵌套行屏上是 `{ … }` 摘要；复制时给真实内容，保留 key 前缀。
    let expanded = serde_json::to_string_pretty(value)
        .map(|text| gpui::SharedString::from(format!("{key}: {text}")))
        .ok();
    let header = div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            // 折叠命中区只保留箭头：整行可点会和双击选词争抢同一次按下。
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .rounded(colors.radius)
                .hover(move |style| style.bg(colors.hover))
                .debug_selector(move || selector)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        // 双击留给文本选择，不连带折叠。
                        if event.click_count > 1 {
                            return;
                        }
                        this.toggle_mongo_field(tab_id, toggle_key.clone(), cx);
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    if is_open {
                        AppIcon::ChevronDown
                    } else {
                        AppIcon::ChevronRight
                    },
                    10.,
                    colors.muted,
                )),
        )
        .child(field_text(&summary, colors.muted, expanded))
        .child(
            div()
                .flex_none()
                .pl_3()
                .text_size(ui_text_size(9.))
                .text_color(colors.muted)
                .child(SharedString::from(kind.label())),
        );
    // 子项只在展开时遍历：收起状态下克隆整棵子树纯属浪费。
    let mut nested = div().flex().flex_col().child(header);
    if is_open {
        let children_selector = format!("mongo-field-children:{document_key}/{path}");
        let mut children_column = div()
            .ml_1()
            .pl_3()
            .border_l_1()
            .border_color(colors.border_soft)
            .flex()
            .flex_col()
            .debug_selector(move || children_selector);
        let children = match value {
            JsonValue::Array(items) => items
                .iter()
                .enumerate()
                .map(|(item_index, item)| (format!("[{item_index}]"), item))
                .collect::<Vec<_>>(),
            JsonValue::Object(fields) => fields
                .iter()
                .map(|(child_key, child)| (child_key.clone(), child))
                .collect(),
            _ => Vec::new(),
        };
        for (child_key, child) in children {
            children_column =
                children_column.child(mongo_field_row(view, context, &child_key, child, &path, cx));
        }
        nested = nested.child(children_column);
    }
    row.child(nested)
}

/// 表格一行：`_id` 与其类型 + 顶层字段（保持文档内的字段顺序）。
struct MongoTableRow {
    id: String,
    id_kind: MongoFieldKind,
    /// `(字段名, 展示文本, BSON 类型)`
    fields: Vec<(String, String, MongoFieldKind)>,
}

/// 整页表格的列与行文本：随渲染缓存一起构建，切展示方式或滚动时不再重建。
struct MongoTableLayout {
    rows: Vec<MongoTableRow>,
    /// 列序取字段首次出现顺序，不按字母排。
    columns: Vec<String>,
    /// 每个列在整页里首次出现的 BSON 类型，用于列头徽标。
    column_kinds: Vec<Option<MongoFieldKind>>,
}

impl MongoTableLayout {
    fn build(values: &[Option<JsonValue>]) -> Self {
        let rows = values
            .iter()
            .map(|value| {
                value
                    .as_ref()
                    .map_or_else(MongoTableRow::empty, mongo_table_row)
            })
            .collect::<Vec<_>>();
        let mut columns: Vec<String> = Vec::new();
        for row in &rows {
            for (key, _, _) in &row.fields {
                if !columns.iter().any(|column| column == key) {
                    columns.push(key.clone());
                }
            }
        }
        let column_kinds = columns
            .iter()
            .map(|column| {
                rows.iter().find_map(|row| {
                    row.fields
                        .iter()
                        .find(|(key, _, _)| key == column)
                        .map(|(_, _, kind)| *kind)
                })
            })
            .collect();
        Self {
            rows,
            columns,
            column_kinds,
        }
    }
}

impl MongoTableRow {
    fn empty() -> Self {
        Self {
            id: String::new(),
            id_kind: MongoFieldKind::Null,
            fields: Vec::new(),
        }
    }

    /// 按列取值：缺列给 `None`，由调用方渲染占位符。
    fn cell(&self, column: &str) -> Option<(&str, MongoFieldKind)> {
        self.fields
            .iter()
            .find(|(key, _, _)| key == column)
            .map(|(_, text, kind)| (text.as_str(), *kind))
    }
}

/// 表格的一行数据：行号 + `_id` + 各列（缺列给占位符）。
fn mongo_table_row_view(
    index: usize,
    row: &MongoTableRow,
    table: &MongoTableLayout,
    theme: JsonEditorTheme,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .flex_none()
        .h(px(MONGO_TABLE_ROW_HEIGHT))
        .border_b_1()
        .border_color(colors.border_soft)
        .child(mongo_table_row_number(
            (index + 1).to_string(),
            false,
            colors,
        ))
        .child(mongo_table_cell(
            format!("mongo-table-cell:{index}:id"),
            row.id.clone(),
            row.id_kind,
            theme,
            colors,
        ))
        .children(table.columns.iter().map(|column| {
            let selector = format!("mongo-table-cell:{index}:{column}");
            match row.cell(column) {
                Some((text, kind)) => {
                    mongo_table_cell(selector, text.to_string(), kind, theme, colors)
                }
                // 该文档缺这个字段：留占位符，不伪造空字符串。
                None => mongo_table_cell(
                    selector,
                    tr("app.mongo.table.missing").to_string(),
                    MongoFieldKind::Null,
                    theme,
                    colors,
                ),
            }
        }))
}

/// 表格视图：行号 + `_id` 一列 + 各顶层字段一列，右侧是**固定**的操作列。
///
/// 对齐 Compass：列头带 BSON 类型徽标，值按类型着色、超长省略并悬停显示全文，
/// 列宽固定、数据区横向滚动（字段多的集合在窄窗口里也能看全）。
/// 列序取字段首次出现顺序，不按字母排——与文档里的书写顺序一致更易读。
///
/// 操作列放在横向滚动区**之外**（`flex_none` 贴着右边缘），字段一多需要左右拖动时
/// 编辑/删除入口仍然固定在原地。纵向滚动由两个 `uniform_list` 共享同一滚动句柄：
/// 行高统一，因此始终逐行对齐，且只渲染视口内的行（一页 500 条也不会逐帧建满）。
fn mongo_document_table(
    view: &MongoDocumentView,
    cache: &MongoDocumentCache,
    context: MongoCardContext,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let theme = JsonEditorTheme::from_colors(colors);
    let table = &cache.data.table;
    let item_count = table.rows.len();
    let header = div()
        .flex()
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(mongo_table_row_number(String::new(), true, colors))
        .child(mongo_table_header_cell(
            tr("app.mongo.table.id").to_string(),
            table.rows.first().map(|row| row.id_kind),
            colors,
        ))
        .children(
            table
                .columns
                .iter()
                .zip(&table.column_kinds)
                .map(|(column, kind)| mongo_table_header_cell(column.clone(), *kind, colors)),
        );

    let rows = cache.data.clone();
    let body = uniform_list(
        ("mongo-table-body", view.tab_id.0),
        item_count,
        cx.processor(move |_this, range: std::ops::Range<usize>, _, _| {
            range
                .map(|index| {
                    mongo_table_row_view(index, &rows.table.rows[index], &rows.table, theme, colors)
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        }),
    )
    .flex_1()
    .min_h(px(0.))
    .track_scroll(&cache.table_scroll);

    // 操作列的每一行与数据行同高同序，横向滚动时不动；纵向滚动与数据区共用一个句柄。
    let action_rows = cache.data.clone();
    let scroll = cache.table_scroll.clone();
    let actions_list = uniform_list(
        ("mongo-table-actions", view.tab_id.0),
        item_count,
        cx.processor({
            let view = view.clone();
            move |_this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let document = &action_rows.documents[index];
                        let selected = context.selected_document == Some(index);
                        div()
                            .flex_none()
                            .h(px(MONGO_TABLE_ROW_HEIGHT))
                            .px_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .border_b_1()
                            .border_color(colors.border_soft)
                            .children(mongo_document_actions(
                                &view,
                                index,
                                document,
                                selected,
                                context.read_only,
                                cx,
                            ))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }
        }),
    )
    .flex_1()
    .min_h(px(0.))
    .track_scroll(&scroll);

    // 横向滚动只作用于数据区（表头随数据行一起左右移动）；纵向滚动留在两个列表里。
    // 横向位置由 `overflow_x_scrollbar` 的元素状态持有，跨帧不掉。
    div()
        .size_full()
        .debug_selector(|| "mongo-document-table".to_string())
        .flex()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .relative()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id(("mongo-table-hscroll", view.tab_id.0))
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_x_scrollbar()
                        .child(
                            div()
                                .h_full()
                                .min_w_full()
                                .flex()
                                .flex_col()
                                .child(header)
                                .child(body),
                        ),
                )
                .vertical_scrollbar(&cache.table_scroll),
        )
        .child(
            div()
                .w(px(MONGO_TABLE_ACTIONS_WIDTH))
                .flex_none()
                .flex()
                .flex_col()
                .border_l_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .child(
                    div()
                        .flex_none()
                        .h(px(MONGO_TABLE_ROW_HEIGHT))
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(colors.panel_alt)
                        .text_size(ui_text_size(11.))
                        .text_color(colors.text)
                        .child(tr("app.mongo.table.actions")),
                )
                .child(actions_list),
        )
}

/// 行号列（表头处留空）。
fn mongo_table_row_number(text: String, header: bool, colors: UiColors) -> Div {
    div()
        .w(px(MONGO_TABLE_ROW_NUM_WIDTH))
        .flex_none()
        .h(px(MONGO_TABLE_ROW_HEIGHT))
        .pr_2()
        .flex()
        .items_center()
        .justify_end()
        .text_size(ui_text_size(10.))
        .text_color(colors.muted)
        .child(SharedString::from(text))
        .when(header, |this| this.bg(colors.panel_alt))
}

/// 列头：字段名 + BSON 类型（`_id ObjectId`，与 Compass 一致）。
fn mongo_table_header_cell(name: String, kind: Option<MongoFieldKind>, colors: UiColors) -> Div {
    div()
        .w(px(MONGO_TABLE_CELL_WIDTH))
        .flex_none()
        .h(px(MONGO_TABLE_ROW_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .gap_1p5()
        .overflow_hidden()
        .text_size(ui_text_size(11.))
        .text_color(colors.text)
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .child(SharedString::from(name)),
        )
        .when_some(kind, |this, kind| {
            this.child(
                div()
                    .flex_none()
                    .text_size(ui_text_size(9.))
                    .text_color(colors.muted)
                    .child(SharedString::from(kind.label())),
            )
        })
}

/// 单元格：定宽、单行、超长省略；截断的完整值用悬停提示补上。
fn mongo_table_cell(
    id: String,
    text: String,
    kind: MongoFieldKind,
    theme: JsonEditorTheme,
    colors: UiColors,
) -> impl IntoElement {
    let full: String = text.chars().take(MONGO_VALUE_MAX_CHARS).collect();
    // 约等于 180px 宽、11px 等宽字体的可见字符数，超过才挂提示，避免为每格建浮层。
    let truncated = full.chars().count() > MONGO_TABLE_CELL_VISIBLE_CHARS;
    let color = mongo_field_value_color(kind, theme, colors);
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector)
        .w(px(MONGO_TABLE_CELL_WIDTH))
        .flex_none()
        .h(px(MONGO_TABLE_ROW_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .overflow_hidden()
        .text_size(ui_text_size(11.))
        .text_color(color)
        .when(truncated, |this| {
            let tooltip: SharedString = full.clone().into();
            this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .child(SharedString::from(full)),
        )
}

/// 文档解析结果 → 表格一行（`_id` + 顶层字段，值取紧凑文本）。
///
/// 入参是缓存里已解析的值：表格整页只解析一次，逐帧渲染与滚动不再碰 JSON 文本。
fn mongo_table_row(value: &JsonValue) -> MongoTableRow {
    let JsonValue::Object(map) = value else {
        return MongoTableRow::empty();
    };
    let id_value = map.get("_id");
    let id = id_value
        .map(|value| match value {
            JsonValue::Object(object) => object
                .get("$oid")
                .and_then(|oid| oid.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default()),
            other => other
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| other.to_string()),
        })
        .unwrap_or_default();
    let id_kind = id_value.map_or(MongoFieldKind::Null, mongo_field_kind);
    let fields = map
        .iter()
        .filter(|(key, _)| key.as_str() != "_id")
        .map(|(key, value)| {
            let kind = mongo_field_kind(value);
            let text = if kind.is_nested() {
                mongo_field_summary(kind, value).to_string()
            } else {
                mongo_field_value_label(value)
            };
            (key.clone(), text, kind)
        })
        .collect();
    MongoTableRow {
        id,
        id_kind,
        fields,
    }
}

#[cfg(test)]
mod mongo_documents_tests {
    use super::*;

    fn kinds(json: &str) -> Vec<MongoFieldKind> {
        let JsonValue::Object(map) = serde_json::from_str::<JsonValue>(json).expect("json") else {
            panic!("对象");
        };
        map.values().map(mongo_field_kind).collect()
    }

    /// BSON 包装必须判成标量，否则 `_id` 会被渲染成一层可展开的空对象。
    #[test]
    fn extended_json_wrappers_are_scalars() {
        let kinds = kinds(
            r#"{"oid":{"$oid":"651700000000000000000001"},"date":{"$date":"2026-10-22T08:30:00Z"},
                "date_ms":{"$date":{"$numberLong":"1700000000000"}},"dec":{"$numberDecimal":"1.5"},
                "nested":{"name":"x"},"list":[1,2],"text":"a","num":3,"ratio":1.5,"ok":true,"none":null}"#,
        );
        assert_eq!(
            kinds,
            vec![
                MongoFieldKind::Bson("ObjectId"),
                MongoFieldKind::Bson("Date"),
                MongoFieldKind::Bson("Date"),
                MongoFieldKind::Bson("Decimal128"),
                MongoFieldKind::Object,
                MongoFieldKind::Array,
                MongoFieldKind::String,
                MongoFieldKind::Int32,
                MongoFieldKind::Double,
                MongoFieldKind::Boolean,
                MongoFieldKind::Null,
            ]
        );
    }

    #[test]
    fn scalar_labels_follow_compass_style() {
        let oid = serde_json::json!({ "$oid": "651700000000000000000001" });
        assert_eq!(
            mongo_field_value_label(&oid),
            "ObjectId(\"651700000000000000000001\")"
        );
        let date = serde_json::json!({ "$date": { "$numberLong": "1700000000000" } });
        assert_eq!(mongo_field_value_label(&date), "Date(\"1700000000000\")");
        assert_eq!(
            mongo_field_value_label(&serde_json::json!("paid")),
            "\"paid\""
        );
        assert_eq!(mongo_field_value_label(&serde_json::json!(12)), "12");
    }

    /// 表格行保留文档内字段顺序（不按字母排），并带出列头要用的 BSON 类型。
    #[test]
    fn table_row_keeps_document_order_and_types() {
        let value: JsonValue = serde_json::from_str(
            r#"{"_id":{"$oid":"66b000000000000000000001"},"amount":{"$numberDecimal":"100"},
                "channel":"online","items":[{},{}],"meta":{"a":1}}"#,
        )
        .expect("json");
        let row = mongo_table_row(&value);
        assert_eq!(row.id, "66b000000000000000000001");
        assert_eq!(row.id_kind, MongoFieldKind::Bson("ObjectId"));
        let order = row
            .fields
            .iter()
            .map(|(key, _, kind)| (key.as_str(), *kind))
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![
                ("amount", MongoFieldKind::Bson("Decimal128")),
                ("channel", MongoFieldKind::String),
                ("items", MongoFieldKind::Array),
                ("meta", MongoFieldKind::Object),
            ]
        );
        // 摘要文案走语言包，按同一模板渲染后再比，避免依赖 i18n 是否已初始化。
        assert_eq!(row.fields[0].1, "Decimal128(\"100\")");
        assert_eq!(row.fields[1].1, "\"online\"");
        assert_eq!(
            row.fields[2].1,
            tr("app.mongo.item_count").replace("%{count}", "2")
        );
        assert_eq!(
            row.fields[3].1,
            tr("app.mongo.field_count").replace("%{count}", "1")
        );
    }

    /// 整页表格列：取所有文档字段的并集，列序按首次出现顺序；列头类型取该列首个有值的文档。
    #[test]
    fn table_layout_unions_columns_in_first_seen_order() {
        let values = [
            Some(serde_json::json!({"a": 1, "b": "x"})),
            Some(serde_json::json!({"b": 2, "c": true})),
            None,
        ];
        let layout = MongoTableLayout::build(&values);
        assert_eq!(layout.columns, vec!["a", "b", "c"]);
        assert_eq!(
            layout.column_kinds,
            vec![
                Some(MongoFieldKind::Int32),
                Some(MongoFieldKind::String),
                Some(MongoFieldKind::Boolean),
            ]
        );
        assert_eq!(layout.rows.len(), 3);
        assert!(layout.rows[2].id.is_empty(), "非法 JSON 退化为空行");
        assert_eq!(layout.rows[1].cell("c").map(|(text, _)| text), Some("true"));
        assert!(
            layout.rows[0].cell("c").is_none(),
            "缺失列必须交给界面渲染占位符"
        );
    }

    /// 折叠键按文档身份隔离：不同文档的同名路径不能互相折叠。
    #[test]
    fn json_fold_keys_are_scoped_per_document() {
        let a = mongo_json_fold_key("doc-a", &json_fold_path_key(&[JsonKey::Object("x".into())]));
        let b = mongo_json_fold_key("doc-b", &json_fold_path_key(&[JsonKey::Object("x".into())]));
        assert_ne!(a, b);
        assert!(a.starts_with("doc-a\u{1e}"));
        // 同一文档同一路径必须稳定，否则折叠状态刷新后会丢。
        assert_eq!(
            a,
            mongo_json_fold_key("doc-a", &json_fold_path_key(&[JsonKey::Object("x".into())]))
        );
    }

    /// 文档身份优先 `_id`：翻页后同一文档保持折叠，缺失 `_id` 才退回下标。
    #[test]
    fn document_key_prefers_id() {
        let document = fluxdb_core::MongoDocument {
            id: "651700000000000000000001".into(),
            json: "{}".into(),
        };
        assert_eq!(mongo_document_key(&document, 3), "651700000000000000000001");
        let anonymous = fluxdb_core::MongoDocument {
            id: String::new(),
            json: "{}".into(),
        };
        assert_eq!(mongo_document_key(&anonymous, 3), "#3");
    }
}
