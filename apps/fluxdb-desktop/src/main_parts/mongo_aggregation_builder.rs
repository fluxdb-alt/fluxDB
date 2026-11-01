// MongoDB 聚合管道的 stage 卡片视图：管道文本 ↔ 卡片，同一份内容。
//
// 管道文本（Extended JSON 数组）始终是唯一真源，卡片只是它的输入法：每次改动都
// 重新序列化写回文本，与条件构建器同一套约定，再由自动执行（防抖）跑一次。
//
// 只接受「每个 stage 是单键对象、键以 `$` 开头」的数组。其它写法原样留在文本里、
// 拒绝进入卡片——猜着改写会悄悄改变管道语义。
//
// 单阶段预览只跑「当前阶段及之前的管道」，与 Compass 的 stage preview 一致：
// 看的是这一步的输入/输出，而不是最终结果。

/// 运算符下拉的一项：运算符名 + 一句说明（语言包 key 已解析）。
///
/// 下拉行是**一行**「运算符名 + 说明」：两行排版每项要 30px 以上，31 个运算符
/// 滚起来很费劲；并排一行既保留了「选之前就知道它是干什么的」，又不比默认项高。
#[derive(Clone)]
struct MongoStageOperatorItem {
    name: SharedString,
    hint: SharedString,
}

impl SelectItem for MongoStageOperatorItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.name
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = ui_colors_from_theme(cx.theme().mode, cx);
        h_flex()
            .min_w(px(0.))
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .font_family(crate::PLATFORM_MONOSPACE_FONT)
                    .text_color(colors.success)
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(10.))
                    .text_color(colors.muted)
                    .child(self.hint.clone()),
            )
    }
}

/// 下拉候选：补全用的同一张 stage 名表，说明取自语言包。
fn mongo_stage_operator_items() -> Vec<MongoStageOperatorItem> {
    MONGO_STAGE_NAMES
        .iter()
        .map(|(stage, _, _)| MongoStageOperatorItem {
            name: SharedString::from(*stage),
            // 说明表按**运算符**查（`$limit`），不是按语言包 key 查。
            hint: mongo_stage_hint(stage).map(tr).unwrap_or_default(),
        })
        .collect()
}

/// 当前运算符在候选表里的位置（下拉打开时的高亮项）。
fn mongo_stage_operator_index(key: &str) -> Option<IndexPath> {
    MONGO_STAGE_NAMES
        .iter()
        .position(|(stage, _, _)| *stage == key)
        .map(IndexPath::new)
}

/// 一张 stage 卡片。参数编辑器是独立实体，改动直接回写管道文本。
struct MongoAggregationStageCell {
    key: String,
    body: Entity<EditorState>,
    /// 运算符下拉（可输入筛选，同 Compass 的 stage 选择器）。
    key_select: Entity<SelectState<SearchableVec<MongoStageOperatorItem>>>,
    /// 下拉订阅：随卡片一起释放。
    _key_subscription: Subscription,
    enabled: bool,
    /// 卡片折叠：只留卡头。阶段多的时候用来腾出纵向空间（同 Compass 的折叠箭头）。
    collapsed: bool,
    /// 编辑器订阅：随卡片一起释放。
    _subscription: Subscription,
    /// 单阶段预览结果；`None` 表示没跑过或已关闭。
    preview: Option<LoadState<fluxdb_core::MongoDocumentsPage>>,
    /// 本次预览对应的管道文本：回写时比对，过期结果直接丢弃。
    preview_pipeline: Option<String>,
    /// 在途的预览任务。放在卡片里而不是按标签页索引：卡片移动后下标会变，
    /// 按 `(标签页, 下标)` 记会让完成的任务去取消另一张卡片的请求。
    /// 任务随卡片一起 drop，也就随标签页关闭一起取消。
    _preview_task: Option<Task<()>>,
}

impl MongoAggregationStageCell {
    fn to_model(&self, cx: &App) -> MongoAggregationStage {
        MongoAggregationStage {
            key: self.key.clone(),
            body: self.body.read(cx).value().to_string(),
            enabled: self.enabled,
        }
    }
}

/// 一个集合标签页的管道卡片状态（输入框实体随结构一起 drop）。
struct MongoAggregationBuilder {
    stages: Vec<MongoAggregationStageCell>,
    /// 首次解析当前管道的错误（存储里的管道不是合法的 stage 数组）；
    /// 一旦卡片重写出合法管道就清掉。
    error: Option<SharedString>,
    /// 卡片列表的滚动位置；滚动条是覆盖层，句柄必须跨帧保留。
    scroll: ScrollHandle,
}

impl MongoAggregationBuilder {
    fn new() -> Self {
        Self {
            stages: Vec::new(),
            error: None,
            scroll: ScrollHandle::default(),
        }
    }

    fn models(&self, cx: &App) -> Vec<MongoAggregationStage> {
        self.stages.iter().map(|cell| cell.to_model(cx)).collect()
    }
}

impl NavicatMain {
    /// 取（或惰性创建）指定标签页的管道卡片状态。
    ///
    /// 首次创建时从管道文本解析；解析不了就记下错误、给一张空卡片表——
    /// 用户新加/改一张卡就会把整条管道重写成合法文本。
    fn mongo_aggregation_builder(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> &mut MongoAggregationBuilder {
        if !self.mongo_aggregation_builders.contains_key(&tab_id) {
            let text = self
                .mongo_query_inputs
                .get(&tab_id)
                .map(|inputs| inputs.read(cx).aggregation.read(cx).value().to_string())
                .unwrap_or_default();
            let mut builder = MongoAggregationBuilder::new();
            match parse_mongo_aggregation_stages(&text) {
                Ok(stages) => {
                    builder.stages = stages
                        .into_iter()
                        .map(|stage| {
                            self.new_mongo_aggregation_stage_cell(tab_id, stage, window, cx)
                        })
                        .collect();
                }
                Err(error) => builder.error = Some(error),
            }
            self.mongo_aggregation_builders.insert(tab_id, builder);
        }
        self.mongo_aggregation_builders
            .get_mut(&tab_id)
            .expect("builder inserted above")
    }

    /// 新建一张卡片的参数编辑器。
    ///
    /// 参数是 JSON 片段，用代码编辑器并开 JSON 高亮；不挂补全——补全按「整条管道」
    /// 的语法位置判定，喂它一段 stage 参数只会给出错误的键位候选。
    ///
    /// 占位提示跟着运算符走（`mongo_stage_hint`）：用户选了 `$limit` 就得看见
    /// 「限制条数（整数）」，而不是一句泛泛的「阶段参数」。
    fn new_mongo_aggregation_stage_cell(
        &mut self,
        tab_id: TabId,
        stage: MongoAggregationStage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MongoAggregationStageCell {
        let body = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .placeholder(mongo_aggregation_body_placeholder(&stage.key))
                .default_value(stage.body.clone());
            editor.set_highlighter(crate::JSON_HIGHLIGHT_LANGUAGE, cx);
            editor.set_folding(false, window, cx);
            // 行号留着：卡片右栏是分步预览，用户常要对照「第几行的参数」说话（同 Compass）。
            editor.set_line_number(true, window, cx);
            editor
        });
        let subscription = cx.subscribe_in(
            &body,
            window,
            move |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sync_mongo_aggregation_pipeline(tab_id, window, cx);
                }
            },
        );
        let key_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(mongo_stage_operator_items()),
                mongo_stage_operator_index(&stage.key),
                window,
                cx,
            )
            .searchable(true)
        });
        // 下标按实体现找：卡片会增删换序，订阅创建时捕获的下标会指到别的卡片。
        let key_subscription = cx.subscribe_in(
            &key_select,
            window,
            move |this: &mut Self,
                  select: &Entity<SelectState<SearchableVec<MongoStageOperatorItem>>>,
                  event: &SelectEvent<SearchableVec<MongoStageOperatorItem>>,
                  window,
                  cx| {
                // 只有确认（点选 / 回车）才改运算符；关掉下拉不发事件。
                let SelectEvent::Confirm(Some(key)) = event else {
                    return;
                };
                let Some(index) = this
                    .mongo_aggregation_builders
                    .get(&tab_id)
                    .and_then(|builder| {
                        builder
                            .stages
                            .iter()
                            .position(|cell| cell.key_select == *select)
                    })
                else {
                    return;
                };
                this.set_mongo_aggregation_stage_key(tab_id, index, key.to_string(), window, cx);
            },
        );
        MongoAggregationStageCell {
            key: stage.key,
            body,
            key_select,
            _key_subscription: key_subscription,
            enabled: stage.enabled,
            collapsed: false,
            _subscription: subscription,
            preview: None,
            preview_pipeline: None,
            _preview_task: None,
        }
    }

    /// 卡片 → 管道文本，写回聚合编辑器。手改参数时每键都会走到这里，
    /// 因此只在错误消失或预览失效时通知视图。
    fn sync_mongo_aggregation_pipeline(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self
            .mongo_query_inputs
            .get(&tab_id)
            .map(|inputs| inputs.read(cx).aggregation.clone())
        else {
            return;
        };
        let Some(builder) = self.mongo_aggregation_builders.get(&tab_id) else {
            return;
        };
        let text = mongo_aggregation_pipeline_text(&builder.models(cx));
        let had_error = builder.error.is_some();
        editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
        let mut stale_preview = false;
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id) {
            builder.error = None;
            // 管道变了，所有预览都过期：留着旧结果会让人以为「改完没生效」。
            // 预览面板本身留着（还是展开态），只把内容标成待重跑。
            for cell in &mut builder.stages {
                if cell.preview.is_some() {
                    cell.preview = Some(LoadState::NotLoaded);
                    cell.preview_pipeline = None;
                    stale_preview = true;
                }
            }
        }
        // 管道文本每键都变，但视图只在错误消失或预览失效时重绘：
        // 编辑器自己会重绘，卡片头不需要跟着逐键重排。
        if had_error || stale_preview {
            cx.notify();
        }
    }

    /// 在 `index` 之后插入一张新卡片；`None` 表示追加到末尾。
    pub(crate) fn add_mongo_aggregation_stage(
        &mut self,
        tab_id: TabId,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = {
            let builder = self.mongo_aggregation_builder(tab_id, window, cx);
            if builder.stages.len() >= MONGO_AGGREGATION_MAX_STAGES {
                return;
            }
            index.map_or(builder.stages.len(), |index| {
                (index + 1).min(builder.stages.len())
            })
        };
        let cell = self.new_mongo_aggregation_stage_cell(
            tab_id,
            mongo_aggregation_new_stage(),
            window,
            cx,
        );
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id) {
            builder.stages.insert(at, cell);
        }
        self.sync_mongo_aggregation_pipeline(tab_id, window, cx);
        cx.notify();
    }

    pub(crate) fn remove_mongo_aggregation_stage(
        &mut self,
        tab_id: TabId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id)
            && index < builder.stages.len()
        {
            builder.stages.remove(index);
        }
        self.sync_mongo_aggregation_pipeline(tab_id, window, cx);
        cx.notify();
    }

    /// 上移/下移一张卡片。`delta` 只取 ±1，越界直接忽略。
    pub(crate) fn move_mongo_aggregation_stage(
        &mut self,
        tab_id: TabId,
        index: usize,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id) else {
            return;
        };
        let Some(target) = index.checked_add_signed(delta) else {
            return;
        };
        if index >= builder.stages.len() || target >= builder.stages.len() {
            return;
        }
        builder.stages.swap(index, target);
        self.sync_mongo_aggregation_pipeline(tab_id, window, cx);
        cx.notify();
    }

    /// 停用/启用一张卡片。停用的 stage 留在卡片里，但不写进管道。
    pub(crate) fn toggle_mongo_aggregation_stage(
        &mut self,
        tab_id: TabId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id)
            && let Some(cell) = builder.stages.get_mut(index)
        {
            cell.enabled = !cell.enabled;
        }
        self.sync_mongo_aggregation_pipeline(tab_id, window, cx);
        cx.notify();
    }

    /// 改 stage 运算符：卡片头下拉里的那一项。
    ///
    /// 参数框**空着**、或还是上一个运算符的默认参数（与 Compass 的
    /// `changeStageOperator` 同一条判据）时才换模板：内容还是没动过的默认值，
    /// 换运算符就该跟着换；手写过的参数一律保留——换个运算符就把用户刚敲的
    /// JSON 冲掉，是最难恢复的一类误操作。占位提示始终跟着新运算符走。
    pub(crate) fn set_mongo_aggregation_stage_key(
        &mut self,
        tab_id: TabId,
        index: usize,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((body, replace_body)) = self
            .mongo_aggregation_builders
            .get(&tab_id)
            .and_then(|builder| builder.stages.get(index))
            .map(|cell| {
                let current = cell.body.read(cx).value().trim().to_string();
                let is_default = current.is_empty()
                    || current == mongo_aggregation_default_body(&cell.key).trim();
                (cell.body.clone(), is_default)
            })
        else {
            return;
        };
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id)
            && let Some(cell) = builder.stages.get_mut(index)
        {
            cell.key = key.clone();
        }
        let body = body.clone();
        body.update(cx, |editor, cx| {
            editor.set_placeholder(mongo_aggregation_body_placeholder(&key), window, cx);
            if replace_body {
                editor.set_value(mongo_aggregation_default_body(&key), window, cx);
            }
        });
        self.sync_mongo_aggregation_pipeline(tab_id, window, cx);
        cx.notify();
    }

    /// 跑一次单阶段预览：预览区常驻可见，所以这里只管「跑」，不再管开关。
    pub(crate) fn refresh_mongo_aggregation_stage_preview(
        &mut self,
        tab_id: TabId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_mongo_aggregation_stage_preview(tab_id, index, window, cx);
    }

    /// 折叠/展开一张卡片：阶段多的时候只留卡头，把纵向空间让给别的阶段。
    pub(crate) fn toggle_mongo_aggregation_stage_collapsed(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id)
            && let Some(cell) = builder.stages.get_mut(index)
        {
            cell.collapsed = !cell.collapsed;
            cx.notify();
        }
    }

    /// 跑「前 `index` 个阶段（含）且已启用」的管道，结果只落到这张卡片上。
    fn run_mongo_aggregation_stage_preview(
        &mut self,
        tab_id: TabId,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pipeline = {
            let Some(builder) = self.mongo_aggregation_builders.get(&tab_id) else {
                return;
            };
            if index >= builder.stages.len() {
                return;
            }
            let mut stages = builder.models(cx)[..=index].to_vec();
            // `$out` / `$merge` 会真的写库：预览不能顺带写一次数据。
            if stages
                .iter()
                .any(|stage| stage.enabled && matches!(stage.key.as_str(), "$out" | "$merge"))
            {
                if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id)
                    && let Some(cell) = builder.stages.get_mut(index)
                {
                    cell.preview = Some(LoadState::Failed(fluxdb_core::UserFacingError {
                        title: tr("app.mongo.aggregation.preview_write_stage").to_string(),
                        message: String::new(),
                        detail: None,
                        retryable: false,
                    }));
                    cell.preview_pipeline = None;
                }
                cx.notify();
                return;
            }
            // 预览只是「看一眼这一步输出什么」，不是取全量：末尾补一条 `$limit`
            // 把结果压到预览量级。没有它，对一个千万级集合预览 `$group` 会真的
            // 全表扫一遍（与 Compass 的 `createPreviewAggregation` 同一取舍）。
            stages.push(MongoAggregationStage {
                key: "$limit".to_string(),
                body: MONGO_AGGREGATION_PREVIEW_LIMIT.to_string(),
                enabled: true,
            });
            mongo_aggregation_pipeline_text(&stages)
        };
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id) {
            let cell = &mut builder.stages[index];
            cell.preview = Some(LoadState::Loading);
            cell.preview_pipeline = Some(pipeline.clone());
        }
        let task = self.spawn_controller_command(
            AppCommand::RunMongoAggregationPreview {
                tab_id,
                pipeline: pipeline.clone(),
            },
            move |this, event, cx| {
                let AppEvent::MongoAggregationPreviewLoaded {
                    pipeline: done,
                    result,
                    ..
                } = event
                else {
                    return;
                };
                let Some(builder) = this.mongo_aggregation_builders.get_mut(&tab_id) else {
                    return;
                };
                // 只回写发起这次预览的那张卡片：期间用户可能又改了参数、关了预览，
                // 或者把卡片挪走了（挪走不改 `preview_pipeline`，所以还要比对管道文本）。
                let Some(cell) = builder
                    .stages
                    .get_mut(index)
                    .filter(|cell| cell.preview_pipeline.as_deref() == Some(done.as_str()))
                else {
                    return;
                };
                cell.preview = Some(match result {
                    Ok(page) => LoadState::Loaded(page),
                    Err(error) => LoadState::Failed(error),
                });
                cx.notify();
            },
            cx,
        );
        if let Some(builder) = self.mongo_aggregation_builders.get_mut(&tab_id)
            && let Some(cell) = builder.stages.get_mut(index)
        {
            // 换掉旧任务即取消上一轮预览，同一张卡片不会并发两条请求。
            cell._preview_task = Some(task);
        }
    }
}

/// 管道视图的渲染快照：克隆轻量实体句柄，不在渲染期间读视图状态。
struct MongoAggregationView {
    error: Option<SharedString>,
    stages: Vec<MongoAggregationStageView>,
    scroll: ScrollHandle,
    /// 已展开的嵌套字段（`{文档身份}/{路径}`）。预览复用文档页那套字段行，
    /// 展开态必须跟着同一份集合走，否则点开箭头下一帧又收回去。
    expanded_fields: Rc<BTreeSet<String>>,
}

struct MongoAggregationStageView {
    key: String,
    body: Entity<EditorState>,
    key_select: Entity<SelectState<SearchableVec<MongoStageOperatorItem>>>,
    enabled: bool,
    collapsed: bool,
    preview: Option<LoadState<fluxdb_core::MongoDocumentsPage>>,
}

fn mongo_aggregation_view(
    builders: &BTreeMap<TabId, MongoAggregationBuilder>,
    tab_id: TabId,
    expanded_fields: Rc<BTreeSet<String>>,
) -> MongoAggregationView {
    builders.get(&tab_id).map_or_else(
        || MongoAggregationView {
            error: None,
            stages: Vec::new(),
            scroll: ScrollHandle::default(),
            expanded_fields: expanded_fields.clone(),
        },
        |builder| MongoAggregationView {
            error: builder.error.clone(),
            stages: builder
                .stages
                .iter()
                .map(|cell| MongoAggregationStageView {
                    key: cell.key.clone(),
                    body: cell.body.clone(),
                    key_select: cell.key_select.clone(),
                    enabled: cell.enabled,
                    collapsed: cell.collapsed,
                    preview: cell.preview.clone(),
                })
                .collect(),
            scroll: builder.scroll.clone(),
            expanded_fields: expanded_fields.clone(),
        },
    )
}

/// 卡片列表：逐张 stage 卡片 + 末尾「添加阶段」。
fn mongo_aggregation_cards(
    tab_id: TabId,
    view: &MongoAggregationView,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let total = view.stages.len();
    let mut rows = div().flex().flex_col().gap(px(6.)).p(px(8.));
    if total == 0 {
        // 空态给一句能照着做的话，不是干巴巴的「还没有阶段」。
        rows = rows.child(
            div()
                .p_2()
                .rounded(colors.radius)
                .border_1()
                .border_dashed()
                .border_color(colors.border)
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.mongo.aggregation.no_stages")),
        );
    }
    for (index, stage) in view.stages.iter().enumerate() {
        rows = rows.child(mongo_aggregation_stage_card(
            tab_id,
            index,
            total,
            stage,
            &view.expanded_fields,
            colors,
            cx,
        ));
    }
    let can_add = total < MONGO_AGGREGATION_MAX_STAGES;
    rows = rows.child(
        div().pt(px(2.)).child(
            Button::new(("mongo-aggregation-add", tab_id.0))
                .debug_selector(|| "mongo-aggregation-add".to_string())
                .small()
                .outline()
                .h(px(26.))
                .text_size(px(11.))
                .text_color(colors.success)
                .disabled(!can_add)
                .label(tr("app.mongo.aggregation.add_stage"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.add_mongo_aggregation_stage(tab_id, None, window, cx);
                    cx.stop_propagation();
                })),
        ),
    );
    // 滚动条覆盖层挂在滚动容器之外，理由见条件构建器的同一处注释。
    // 卡片区封顶：stage 再多也不许把结果区挤没，超出的在卡片区内滚动。
    let area = div()
        .id(("mongo-aggregation-cards", tab_id.0))
        .w_full()
        .max_h(px(MONGO_AGGREGATION_CARDS_MAX_HEIGHT))
        .flex_none()
        .min_h(px(0.))
        .track_scroll(&view.scroll)
        .overflow_y_scroll()
        .child(rows);
    div()
        .debug_selector(|| "mongo-aggregation-cards".to_string())
        .relative()
        .flex_none()
        .w_full()
        .child(area)
        .vertical_scrollbar(&view.scroll)
        .into_any_element()
}

/// 一张 stage 卡片：卡头（折叠箭头 / 序号 / 运算符 / 停用 / 移动 / 删除）
/// + 左参数编辑器 + 右输出预览，两栏并排（对齐 Compass 的 stage 卡片）。
fn mongo_aggregation_stage_card(
    tab_id: TabId,
    index: usize,
    total: usize,
    stage: &MongoAggregationStageView,
    expanded_fields: &Rc<BTreeSet<String>>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body_value = stage.body.read(cx).value();
    let invalid = mongo_aggregation_body_value(&body_value).is_none();
    let hint = mongo_stage_hint(&stage.key).map(tr).unwrap_or_default();
    // 卡头一行：折叠箭头 + 序号 + 运算符 + 停用开关，右侧是移动/删除。
    // 说明文字留在中间：它解释「这个运算符要写什么」，光看名字猜不出来。
    let head = div()
        .h(px(34.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .child(
            mongo_card_icon_button(
                format!("mongo-aggregation-collapse-{index}"),
                format!("mongo-aggregation-collapse:{index}"),
                if stage.collapsed {
                    AppIcon::ChevronRight
                } else {
                    AppIcon::ChevronDown
                },
                tr("app.mongo.aggregation.collapse_stage"),
                colors.muted,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_mongo_aggregation_stage_collapsed(tab_id, index, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(SharedString::from(format!(
                    "{} {}",
                    tr("app.mongo.aggregation.stage"),
                    index + 1
                ))),
        )
        .child(mongo_aggregation_stage_key_cell(index, stage, colors))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(10.))
                .text_color(if invalid { colors.danger } else { colors.muted })
                .child(if invalid {
                    tr("app.mongo.aggregation.invalid_body")
                } else {
                    hint
                }),
        )
        .child(mongo_aggregation_stage_actions(
            tab_id, index, total, stage, colors, cx,
        ));

    let card = div()
        .debug_selector(move || format!("mongo-aggregation-stage:{index}"))
        .flex()
        .flex_col()
        .rounded(colors.radius)
        .border_1()
        .border_color(if invalid {
            colors.danger
        } else if stage.enabled {
            colors.border
        } else {
            colors.border_soft
        })
        .bg(colors.panel_bg)
        .when(!stage.enabled, |this| this.opacity(0.55))
        .child(head);
    if stage.collapsed {
        return card;
    }
    // 左右两栏等高：预览区跟着参数区的高度走，卡片不会因为预览短而忽高忽低。
    // 中间是可拖动的分隔线（同 AI 面板那条 `h_resizable`）：参数与预览谁更宽，
    // 取决于当前 stage 写的是长参数还是长结果，固定 320px 只方便其中一种。
    let row_height = mongo_aggregation_body_height(&body_value).max(150.);
    let split_id = SharedString::from(format!(
        "mongo-aggregation-split:{}:{index}",
        tab_id.0
    ));
    let split = gpui_component::h_resizable(split_id)
        .child(
            gpui_component::resizable_panel()
                .size_range(px(180.)..px(2400.))
                .child(
                    div().size_full().min_w(px(0.)).p_1p5().child(
                        mongo_query_editor_body(
                            &stage.body,
                            Some(format!("mongo-aggregation-body:{index}")),
                            11.,
                            Some(row_height - 12.),
                            !stage.enabled,
                        ),
                    ),
                ),
        )
        .child(
            gpui_component::resizable_panel()
                .size(px(320.))
                .flex_none()
                .size_range(px(160.)..px(1600.))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(mongo_aggregation_stage_preview(
                            tab_id,
                            index,
                            stage,
                            expanded_fields,
                            colors,
                            cx,
                        )),
                ),
        );
    card.child(
        div()
            .h(px(row_height))
            .border_t_1()
            .border_color(colors.border_soft)
            .child(split),
    )
}


/// 参数框占位提示：跟着运算符走，空参数时也能看出这里要写什么。
fn mongo_aggregation_body_placeholder(key: &str) -> SharedString {
    match mongo_stage_hint(key) {
        Some(hint) => tr(hint),
        None => tr("app.mongo.aggregation.body_placeholder"),
    }
}

/// 运算符格：可输入筛选的下拉（同 Compass 的 stage 选择器）。
fn mongo_aggregation_stage_key_cell(
    index: usize,
    stage: &MongoAggregationStageView,
    colors: UiColors,
) -> Div {
    div()
        .debug_selector(move || format!("mongo-aggregation-key:{index}"))
        .flex_none()
        .w(px(150.))
        .h(px(22.))
        .child(
            Select::new(&stage.key_select)
                .small()
                .w_full()
                .h_full()
                .text_size(px(11.))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_color(colors.success)
                .search_placeholder(tr("app.mongo.aggregation.search_stage"))
                .menu_width(px(300.))
                .menu_max_h(px(340.)),
        )
}

/// 卡片头的图标按钮组：预览 / 停用 / 上移 / 下移 / 删除。
fn mongo_aggregation_stage_actions(
    tab_id: TabId,
    index: usize,
    total: usize,
    stage: &MongoAggregationStageView,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let enabled = stage.enabled;
    let has_previous = index > 0;
    let has_next = index + 1 < total;
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(
            mongo_card_icon_button(
                format!("mongo-aggregation-preview-{index}"),
                format!("mongo-aggregation-preview:{index}"),
                AppIcon::Play,
                tr("app.mongo.aggregation.preview"),
                colors.info,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.refresh_mongo_aggregation_stage_preview(tab_id, index, window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            mongo_card_icon_button(
                format!("mongo-aggregation-toggle-{index}"),
                format!("mongo-aggregation-toggle:{index}"),
                if enabled { AppIcon::Eye } else { AppIcon::EyeOff },
                tr("app.mongo.aggregation.toggle_stage"),
                if enabled { colors.success } else { colors.muted },
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.toggle_mongo_aggregation_stage(tab_id, index, window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            mongo_card_icon_button(
                format!("mongo-aggregation-up-{index}"),
                format!("mongo-aggregation-up:{index}"),
                AppIcon::ArrowUp,
                tr("app.mongo.aggregation.move_up"),
                colors.text,
                has_previous,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.move_mongo_aggregation_stage(tab_id, index, -1, window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            mongo_card_icon_button(
                format!("mongo-aggregation-down-{index}"),
                format!("mongo-aggregation-down:{index}"),
                AppIcon::ArrowDown,
                tr("app.mongo.aggregation.move_down"),
                colors.text,
                has_next,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.move_mongo_aggregation_stage(tab_id, index, 1, window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            mongo_card_icon_button(
                format!("mongo-aggregation-remove-{index}"),
                format!("mongo-aggregation-remove:{index}"),
                AppIcon::Trash,
                tr("app.mongo.aggregation.remove_stage"),
                colors.danger,
                true,
                false,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.remove_mongo_aggregation_stage(tab_id, index, window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

/// 单阶段预览区（卡片右栏）：加载中 / 错误 / 文档。
///
/// 预览区常驻（而不是点按钮才展开）：卡片布局对齐 Compass，左边写参数、
/// 右边直接看结果；运行按钮在卡头，只负责「跑一次」，不兼任开关。
fn mongo_aggregation_stage_preview(
    tab_id: TabId,
    index: usize,
    stage: &MongoAggregationStageView,
    expanded_fields: &Rc<BTreeSet<String>>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = match stage.preview.as_ref() {
        // 还没跑过：右栏留空。空态说明只是噪音——运行按钮就在卡头上。
        None => div(),
        // 管道改过：结果作废，右栏留空——卡头的播放按钮就是重跑入口，
        // 再写一句「点播放重跑」只是噪音。
        Some(LoadState::NotLoaded) => div(),
        Some(LoadState::Loading) => center_loading_message(tr("app.mongo.loading"), colors),
        Some(LoadState::Failed(error)) => div()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.danger)
                    .child(SharedString::from(error.title.clone())),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(SharedString::from(error.message.clone())),
            ),
        Some(LoadState::Loaded(page)) if page.documents.is_empty() => {
            center_message(tr("app.mongo.aggregation.preview_empty"), colors)
        }
        // 对齐 Compass：每条文档是一张字段列表卡片（与文档页同一套渲染），
        // 不是整段 JSON 文本；多张卡片横向排开、横向滚动。
        Some(LoadState::Loaded(page)) => {
            let data = Rc::new(MongoPageData::build(Some(page)));
            let document_view = MongoDocumentView {
                tab_id,
                collapsed_documents: Rc::new(BTreeSet::new()),
                expanded_fields: expanded_fields.clone(),
                json_fold_toggles: Rc::new(BTreeSet::new()),
                page: data.clone(),
                colors,
            };
            let header = tr("app.mongo.aggregation.preview_results")
                .replace("%{count}", &page.documents.len().to_string());
            let cards = page.documents.iter().enumerate().map(
                |(document_index, document)| {
                    // 展开键加前缀：预览与文档页的同 `_id` 文档各管各的展开态。
                    let document_key =
                        format!("agg:{index}:{}", mongo_document_key(document, document_index));
                    div()
                        .debug_selector(move || {
                            format!("mongo-aggregation-preview-card:{document_index}")
                        })
                        .flex_none()
                        .w(px(MONGO_AGGREGATION_PREVIEW_CARD_WIDTH))
                        .h_full()
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(colors.panel_alt)
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(
                            div()
                                .h(px(26.))
                                .px_2()
                                .flex()
                                .items_center()
                                .border_b_1()
                                .border_color(colors.border_soft)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                                .text_size(px(11.))
                                .text_color(colors.text)
                                .child(SharedString::from(format!(
                                    "{}: {}",
                                    tr("app.mongo.table.id"),
                                    document.id
                                ))),
                        )
                        // 每张卡片自己竖着滚（对齐 Compass）：预览区本身不滚动，
                        // 卡片高度由预览区高度定死，长文档在卡内滚，卡片之间横着滚。
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "mongo-aggregation-preview-card-scroll:{index}:{document_index}"
                                )))
                                .flex_1()
                                .min_h(px(0.))
                                .overflow_y_scroll()
                                .child(mongo_field_list(
                                    &document_view,
                                    document_index,
                                    document,
                                    document_view.value(document_index),
                                    &document_key,
                                    cx,
                                )),
                        )
                },
            );
            div()
                .p(px(8.))
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(header),
                )
                .child(
                    div()
                        .id(("mongo-aggregation-preview-cards", index))
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .items_stretch()
                        .gap(px(8.))
                        .overflow_x_scrollbar()
                        .children(cards),
                )
        }
    };
    // 预览区本身不滚动：滚动交给每张预览卡片内部（同 Compass）。
    div()
        .debug_selector(move || format!("mongo-aggregation-preview-body:{index}"))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(body)
}
