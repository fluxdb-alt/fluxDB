// MongoDB 集合工作台：分节页签 + 查询面板 +
// 文档/表格/JSON 三种展示 + 右侧文档详情抽屉。
//
// 只读边界：视图（`read_only`）不提供任何写入入口；文档值只在本页渲染，
// 不进入 AI 上下文（AI 侧另行按需走有界只读工具）。

const MONGO_QUERY_ROW_HEIGHT: f32 = 34.;
/// 附加条件格子（投影 / 排序 / 排序规则 / 索引提示）的高度。
///
/// 32 是压过的：多行编辑器的上下内边距改不掉（`Input::from_state` 按
/// `Size::Medium` 写死 8px），能调的只有字号——11px × 1.5 = 16.5px 行高，
/// 一行占 8 + 16.5 + 8 = 32.5px，取 32 只裁掉半像素，字形中心仍在框中线上。
/// 这四格不是主入口，压扁它们才能把纵向空间留给下面的结果区。
const MONGO_QUERY_CONTROL_HEIGHT: f32 = 32.;

/// 多行编辑器恒定加在内容区上下的内边距：`Input::from_state` 的多行分支按
/// `Size::Medium` 写死 8px，外部改不掉。查询框的高度是倒推它的（见
/// `MONGO_QUERY_CONTROL_HEIGHT`），布局测试也按它算首行字形中心。
#[cfg(test)]
pub(crate) fn mongo_query_editor_padding() -> f32 {
    8.
}

/// 编辑器自然行高：Filter 字号 12px × 编辑器固定行高倍数 1.5。
///
/// 同样只用于倒推框高与校验字形居中，改字号时两边一起改。
/// 附加条件格子用 11px，行高 [`mongo_query_option_line_height`]。
const MONGO_QUERY_LINE_HEIGHT: f32 = 18.;

#[cfg(test)]
pub(crate) fn mongo_query_line_height() -> f32 {
    MONGO_QUERY_LINE_HEIGHT
}

/// 附加条件格子的编辑器行高：11px × 1.5。
#[cfg(test)]
pub(crate) fn mongo_query_option_line_height() -> f32 {
    16.5
}
/// Filter 的**一行**高度：交互稿 `.filter-input` 的 34px 量的是内容区，
/// 加上上下各 1px 描边才是外框（`8 + 18 + 8 + 2`）。
///
/// 取整到 36 而不是 34：多行编辑器的内容区必须容下 `8 + 18 + 8`，否则最后一行
/// 的底部会被裁掉 2px、编辑器还会以为自己可滚动而露出滚动条。首行字形中心仍落在
/// 外框中线上（`1 + 8 + 9 = 18 = 36 / 2`）。
const MONGO_QUERY_FILTER_HEIGHT: f32 = 36.;
/// 外框描边宽度：外框高度按 border-box 算，内容区要扣掉上下两条。
const MONGO_QUERY_BOX_BORDER: f32 = 1.;
/// Filter 最多长到几行；再多就固定高度、由编辑器自己滚动。
///
/// 4 行是 Compass 的档位：再高查询面板就吃掉结果区的半屏，而剩下的行靠滚动
/// 仍看得全——长条件本来就该在编辑器里滚，不该把结果区顶走。
const MONGO_QUERY_FILTER_MAX_ROWS: usize = 4;

/// Filter 外框高度：按内容行数长高，封顶 [`MONGO_QUERY_FILTER_MAX_ROWS`] 行。
///
/// 编辑器自己是 `EditorState`（要挂补全），标不出 `auto_grow`——那是
/// `TextareaState` 才有的模式，且在 `gpui-base` 里是 `pub(super)`。所以行数由
/// 视图数换行符推出来，框高再按「每多一行加一个行高」长。
fn mongo_query_filter_height(text: &str) -> f32 {
    MONGO_QUERY_FILTER_HEIGHT
        + (mongo_query_filter_rows(text).saturating_sub(1)) as f32 * MONGO_QUERY_LINE_HEIGHT
}

/// Filter 内容占几行，已夹在 `1..=MONGO_QUERY_FILTER_MAX_ROWS`。
///
/// 数换行符而不是 `str::lines`：光标停在末尾换行之后那行时，`lines()` 不把空尾行
/// 算进去，框会比内容矮一行。不折行（`soft_wrap(false)`），所以行数只由换行决定。
pub(crate) fn mongo_query_filter_rows(text: &str) -> usize {
    text.split('\n')
        .count()
        .clamp(1, MONGO_QUERY_FILTER_MAX_ROWS)
}

/// 查询行里操作按钮的高度，比 Filter 矮一档（交互稿 `.query-main .button` 的 32px，这里再压到 28）。
///
/// 不跟输入框同高：输入框的高度是被多行编辑器的内边距和行高撑出来的（见上），
/// 按钮没有这层约束，做成同高只会显得又粗又笨。组内三者等高，与输入框共用行中线。
const MONGO_QUERY_ACTION_HEIGHT: f32 = 28.;
/// 查询行内元素间距：与 Compass query bar 的 `spacing[200]` 一致。
const MONGO_QUERY_GAP: f32 = 8.;
/// Filter 内嵌标签列宽：容得下英文 `Filter`。
///
/// 附加条件的铭牌各自量体裁宽（见 `mongo_query_cell_label`），不受这一列约束
/// ——`Max Time MS` 这类长标签也不会把首行撑宽。
const MONGO_QUERY_LABEL_WIDTH: f32 = 84.;
/// 「返回条数」格宽（交互稿 110px）。铭牌只放名称，边界 `1–500` 走悬停提示。
const MONGO_QUERY_NUMBER_WIDTH: f32 = 110.;
/// 「起始位置」格宽：同上。
const MONGO_QUERY_POSITION_WIDTH: f32 = 110.;
/// 「执行上限」格宽（交互稿 `expert-options` 末列 150px）。铭牌只放名称，`ms` 走悬停提示。
const MONGO_QUERY_MAX_TIME_WIDTH: f32 = 110.;
const MONGO_SECTION_HEIGHT: f32 = 38.;
const MONGO_TOOLBAR_HEIGHT: f32 = 38.;
const MONGO_PAGER_HEIGHT: f32 = 34.;
const MONGO_INSPECTOR_WIDTH: f32 = 380.;

/// 分节页签顺序与文案 key（顺序即页签顺序）。
const MONGO_SECTIONS: [(MongoSection, &str); 4] = [
    (MongoSection::Documents, "app.mongo.section.documents"),
    (MongoSection::Aggregation, "app.mongo.section.aggregation"),
    (MongoSection::Indexes, "app.mongo.section.indexes"),
    (MongoSection::Info, "app.mongo.section.info"),
];

/// 结果展示方式页签（顺序即页签顺序）。
const MONGO_DISPLAY_MODES: [(MongoDisplayMode, &str); 3] = [
    (MongoDisplayMode::Documents, "app.mongo.display.documents"),
    (MongoDisplayMode::Table, "app.mongo.display.table"),
    (MongoDisplayMode::Json, "app.mongo.display.json"),
];

/// 错误块的重试动作：按出错区域决定重跑哪条命令。
#[derive(Clone, Copy)]
enum MongoRetry {
    Documents,
    Info,
    Indexes,
}

impl NavicatMain {
    pub(crate) fn mongo_collection_content(
        &mut self,
        tab_id: TabId,
        state: &MongoCollectionState,
        window: &mut Window,
        colors: UiColors,
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        let inputs = self.mongo_query_inputs_for(tab_id, window, cx);
        let section_bar = mongo_section_bar(tab_id, state, colors, cx);
        let body = match state.section {
            MongoSection::Documents => {
                self.mongo_documents_section(tab_id, state, inputs, colors, window, cx)
            }
            MongoSection::Aggregation => {
                self.mongo_aggregation_section(tab_id, colors, window, cx)
            }
            MongoSection::Indexes => {
                self.mongo_indexes_section(tab_id, state, colors, cx)
            }
            MongoSection::Info => mongo_info_section(tab_id, state, colors, cx),
        };

        div()
            .flex_1()
            .min_h(px(0.))
            .bg(colors.panel_bg)
            .flex()
            .flex_col()
            .debug_selector({
                let namespace = format!(
                    "{}/{}",
                    state.object.database.clone().unwrap_or_default(),
                    state.object.name
                );
                move || format!("mongo-collection-page:{namespace}")
            })
            .child(section_bar)
            .child(body)
    }

    fn mongo_documents_section(
        &mut self,
        tab_id: TabId,
        state: &MongoCollectionState,
        inputs: Entity<MongoQueryInputs>,
        colors: UiColors,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        self.ensure_mongo_document_cache(tab_id, state);
        let selected = state.selected_document.and_then(|index| {
            state
                .page
                .as_ref()
                .and_then(|page| page.documents.get(index))
                .map(|document| (index, document))
        });
        // 三个交互集合都按 `Rc` 取：没有改动时不复制，虚拟列表闭包也直接共享。
        let collapsed_documents = Rc::new(
            self.mongo_collapsed_documents
                .get(&tab_id)
                .cloned()
                .unwrap_or_default(),
        );
        let expanded_fields = Rc::new(
            self.mongo_expanded_fields
                .get(&tab_id)
                .cloned()
                .unwrap_or_default(),
        );
        let json_fold_toggles = Rc::new(
            self.mongo_json_fold_toggles
                .get(&tab_id)
                .cloned()
                .unwrap_or_default(),
        );
        let cache = &self.mongo_document_cache[&tab_id];
        let documents = MongoDocumentView {
            tab_id,
            collapsed_documents,
            expanded_fields,
            json_fold_toggles,
            page: cache.data.clone(),
            document_order_base: 0,
            colors,
        };
        let results = mongo_results(state, cache, &documents, colors, cx);
        // 抽屉只有两种形态：编辑中的文档给编辑器，其余给只读详情。
        let drawer = match (&self.mongo_document_editor, selected) {
            (Some(editor), _) if editor.tab_id == tab_id => {
                Some(mongo_document_editor_drawer(editor, colors, cx))
            }
            (_, Some((index, document))) => {
                Some(mongo_inspector(tab_id, index, document, colors, cx))
            }
            _ => None,
        };
        let body = match drawer {
            Some(drawer) => div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .child(results)
                .child(drawer),
            None => results,
        };

        // 两层折叠都默认收起；用户没点过时，已生效的查询填过对应层的条件就默认展开，
        // 免得条件生效了却藏在收起的面板里（低频参数展开时连同常用项一起展开）。
        let options_expanded = self
            .mongo_query_options_expanded
            .get(&tab_id)
            .copied()
            .unwrap_or_else(|| mongo_has_any_options(&state.applied_query));
        let advanced_expanded = self
            .mongo_query_advanced_expanded
            .get(&tab_id)
            .copied()
            .unwrap_or_else(|| mongo_query_has_advanced(&state.applied_query));
        let builder_expanded = self
            .mongo_query_builder_expanded
            .get(&tab_id)
            .copied()
            .unwrap_or(false);
        let builder = mongo_query_builder_view(&self.mongo_query_builders, tab_id);
        let limit_ceiling = mongo_query_limit(self.controller.state().settings.page_size);

        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(mongo_query_panel(
                tab_id,
                state,
                inputs,
                options_expanded,
                advanced_expanded,
                builder_expanded,
                builder,
                limit_ceiling,
                colors,
                window,
                cx,
            ))
            .child(mongo_results_toolbar(tab_id, state, colors, cx))
            .child(body)
            .child(mongo_pager(tab_id, state, colors, cx))
    }

    fn mongo_aggregation_section(
        &mut self,
        tab_id: TabId,
        colors: UiColors,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        // 卡片状态懒建：首次进入聚合页才解析管道，没进来过就不解析。
        self.mongo_aggregation_builder(tab_id, window, cx);
        let view = mongo_aggregation_view(
            &self.mongo_aggregation_builders,
            tab_id,
            std::rc::Rc::new(
                self.mongo_expanded_fields
                    .get(&tab_id)
                    .cloned()
                    .unwrap_or_default(),
            ),
        );
        // 没有工具条，也没有底部的整条管道结果区（对齐 Compass）：输出看每张卡右栏的
        // 单阶段预览。这里只在存储里的文本解析不了时给一行错误。
        let error_line = view.error.clone().map(|error| {
            div()
                .h(px(MONGO_QUERY_ROW_HEIGHT))
                .px_2()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(colors.border)
                .overflow_hidden()
                .text_size(px(11.))
                .text_color(colors.danger)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(error)
        });

        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .children(error_line)
            .child(mongo_aggregation_cards(tab_id, &view, colors, cx))
    }
}

/// 命名空间条：`库 / 集合`，视图额外标注只读。
/// 「库/集合」已在标签页标题里，页内不再重复一行命名空间条；只读标记移到本行右侧。
fn mongo_section_bar(
    tab_id: TabId,
    state: &MongoCollectionState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = MONGO_SECTIONS
        .iter()
        .position(|(section, _)| *section == state.section)
        .unwrap_or(0);
    let view = cx.entity().downgrade();
    let tab_bar = MONGO_SECTIONS
        .iter()
        .map(|(section, label)| {
            // 与 Compass 一致：索引页签直接带索引数量，不再另占页面右上角。
            match (*section == MongoSection::Indexes)
                .then(|| mongo_index_count(state))
                .flatten()
            {
                Some(count) => Tab::from(SharedString::from(format!("{} {count}", tr(label)))),
                None => Tab::from(tr(label)),
            }
        })
        .fold(
            TabBar::new(("mongo-section-tabs", tab_id.0))
                .segmented()
                .small()
                .selected_index(selected)
                .on_click(move |index, _, cx| {
                    if let Some((section, _)) = MONGO_SECTIONS.get(*index) {
                        let section = *section;
                        let _ = view.update(cx, |this, cx| {
                            this.select_mongo_section(tab_id, section, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        );

    div()
        .h(px(MONGO_SECTION_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(colors.border)
        .child(tab_bar)
        .child(div().flex_1())
        // 只读标记原来在命名空间条上；该条已移除，这里保留同样的可辨识提示。
        .when(state.read_only, |this| {
            this.child(
                div()
                    .px_1p5()
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border)
                    .text_size(px(11.))
                    .text_color(colors.warning)
                    .child(tr("app.mongo.read_only")),
            )
        })
}

fn mongo_index_count(state: &MongoCollectionState) -> Option<usize> {
    match &state.indexes {
        LoadState::Loaded(indexes) => Some(indexes.len()),
        _ => None,
    }
}

/// 查询面板：一张卡片，四段。
///
/// 1. 首行 `[Filter | 重置 | 查询 | 更多选项]`：唯一常驻的一行，结果区不被长期挤占。
/// 2. 「条件构建」：展开后是字段/运算符/值的条件行，改动直接写回 Filter JSON。
/// 3. 「更多选项」：一行 `[条件构建 | 投影 | 排序 | 返回条数 | 高级查询]`，全部挤在一行。
/// 4. 「高级查询」：展开后才是 `[排序规则 | 起始位置 | 索引提示 | 执行上限]`，虚线分隔。
///
/// 条件构建与更多选项是两个独立入口，展开态各记各的：构建器不是第三个筛选项，
/// 它是 Filter 的另一种输入法，收起时不该被算进「还有几个条件在生效」。
///
/// 格子用框内铭牌，不用「标签在上、输入在下」：标签行每个格子白吃一条 ~19px 的
/// 横向带，四格叠起来就把结果区挤走了。铭牌右侧顺带放边界或单位（`1–500`、`ms`），
/// 输入框附近不再有任何其他说明文字。
///
/// 只读视图（`read_only`）不展示写入相关的东西，这里不受影响——查询本身是只读的。
// 渲染函数的参数就是这一屏需要的状态，包成 struct 只是把参数表挪个位置。
#[allow(clippy::too_many_arguments)]
fn mongo_query_panel(
    tab_id: TabId,
    state: &MongoCollectionState,
    inputs: Entity<MongoQueryInputs>,
    expanded: bool,
    advanced_expanded: bool,
    builder_expanded: bool,
    builder: MongoQueryBuilderView,
    limit_ceiling: u64,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 输入框是独立实体：父视图不会因为输入而重绘，所以这里不按「是否改过」禁用 Reset，
    // 否则禁用态会停在上一帧，用户改完条件仍点不动。
    let (filter, projection, sort, collation, hint, skip, limit, max_time_ms) = {
        let inputs = inputs.read(cx);
        (
            inputs.filter.clone(),
            inputs.projection.clone(),
            inputs.sort.clone(),
            inputs.collation.clone(),
            inputs.hint.clone(),
            inputs.skip.clone(),
            inputs.limit.clone(),
            inputs.max_time_ms.clone(),
        )
    };
    let tone = mongo_query_tone(colors);
    // Filter 随内容增行：高度按编辑框里的换行数算，超过 4 行就固定、由编辑器滚动。
    let filter_height = mongo_query_filter_height(&filter.read(cx).value());

    // 两层各自收起时都看不见自己的格子，角标是「条件还在生效」的唯一提示：
    // 首层收起 → 汇总两层个数；首层展开而第二层收起 → 只标第二层。
    let (common_count, advanced_count) = mongo_query_option_counts(&state.applied_query);
    div()
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .child(
            // 卡片只有一层描边 + 一层极淡投影：查询区与结果区之间不再叠分隔线。
            div()
                .rounded(px(11.))
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .overflow_hidden()
                .shadow(vec![box_shadow(
                    px(0.),
                    px(3.),
                    px(12.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.28 } else { 0.03 }),
                )])
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_2()
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(MONGO_QUERY_GAP))
                        // Filter 是主入口：占满剩余宽度，操作按钮固定在右侧（与 Compass 一致）。
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .debug_selector(|| "mongo-query-filter".to_string())
                                .child(mongo_query_input(
                                    &filter,
                                    filter_height,
                                    tone,
                                    colors,
                                    window,
                                    cx,
                                )),
                        )
                        // 顺序对齐交互稿：重置（图标）| 查询（主色）| 更多选项。
                        .child(mongo_query_reset_button(tab_id, state.loading, colors, cx))
                        .child(mongo_query_find_button(tab_id, state.loading, cx))
                        // 收起时两层条件都看不见：角标给两层的和，用户才知道还有东西在生效。
                        .child(mongo_query_options_button(
                            tab_id,
                            expanded,
                            common_count + advanced_count,
                            tone,
                            colors,
                            cx,
                        )),
                )
                // 条件构建紧跟在 Filter 下面：它改的就是上面那一份 JSON，中间不隔别的段。
                // 入口在「更多选项」网格里，网格收起时构建器一并收起（状态也在那里同步清掉）。
                .when(expanded && builder_expanded, |this| {
                    this.child(mongo_query_builder_section(
                        tab_id,
                        &builder,
                        colors,
                        tone,
                        window,
                        cx,
                    ))
                })
                // 常用项一层：条件构建、投影/排序/返回条数、高级查询入口全部挤在一行，
                // 高级查询展开后另起一行（交互稿 `grid-column: 1 / -1`）。
                //
                // 标签塞进框内左侧（与 Filter 的 `Filter` 铭牌同一套写法）：标签独占一行时
                // 每个格子白吃掉一条 ~19px 的名称行，展开后整块逼近结果区的高度。
                .when(expanded, |this| {
                    this.child(
                        div()
                            .debug_selector(|| "mongo-query-options-grid".to_string())
                            .px(px(12.))
                            .py(px(7.))
                            .border_t_1()
                            .border_color(colors.border_soft)
                            .bg(tone.mint)
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    // 首列：条件构建入口。与「高级查询」同一档 ghost 文字按钮，
                                    // 首行已经有实心「查询」，这里再放描边按钮会把权重拉平。
                                    .child(div().flex_none().child(mongo_query_builder_toggle(
                                        tab_id,
                                        builder_expanded,
                                        builder.root.group_count(),
                                        tone,
                                        colors,
                                        cx,
                                    )))
                                    .child(mongo_query_option_cell(
                                        "projection",
                                        None,
                                        1.,
                                        colors,
                                        mongo_query_editor_box(
                                            "projection",
                                            &projection,
                                            tr("app.mongo.query.projection"),
                                            Some(tr("app.mongo.query.optional")),
                                            tone,
                                            colors,
                                            window,
                                            cx,
                                        ),
                                    ))
                                    .child(mongo_query_option_cell(
                                        "sort",
                                        None,
                                        1.,
                                        colors,
                                        mongo_query_editor_box(
                                            "sort",
                                            &sort,
                                            tr("app.mongo.query.sort"),
                                            Some(tr("app.mongo.query.optional")),
                                            tone,
                                            colors,
                                            window,
                                            cx,
                                        ),
                                    ))
                                    .child(mongo_query_option_cell(
                                        "limit",
                                        Some(MONGO_QUERY_NUMBER_WIDTH),
                                        0.,
                                        colors,
                                        mongo_query_number_box(
                                            "limit",
                                            &limit,
                                            tr("app.mongo.query.result_limit"),
                                            Some(mongo_query_limit_range(limit_ceiling)),
                                            tone,
                                            colors,
                                            window,
                                            cx,
                                        ),
                                    ))
                                    // 「高级查询」挂在同一行末尾：单独占一行只为放一个
                                    // 10px 的文字按钮，收起时白花一条 ~26px 的横向带。
                                    .child(div().flex_none().child(
                                        mongo_query_advanced_toggle(
                                            tab_id,
                                            advanced_expanded,
                                            advanced_count,
                                            tone,
                                            colors,
                                            cx,
                                        ),
                                    )),
                            )
                            .when(advanced_expanded, |this| {
                                this.child(
                                    div()
                                        .debug_selector(|| "mongo-query-expert-grid".to_string())
                                        .pt(px(6.))
                                        .border_t_1()
                                        .border_dashed()
                                        .border_color(colors.border)
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(mongo_query_option_cell(
                                            "collation",
                                            None,
                                            1.5,
                                            colors,
                                            mongo_query_editor_box(
                                                "collation",
                                                &collation,
                                                tr("app.mongo.query.collation"),
                                                Some(tr("app.mongo.query.hint_collation")),
                                                tone,
                                                colors,
                                                window,
                                                cx,
                                            ),
                                        ))
                                        .child(mongo_query_option_cell(
                                            "skip",
                                            Some(MONGO_QUERY_POSITION_WIDTH),
                                            0.,
                                            colors,
                                            mongo_query_number_box(
                                                "skip",
                                                &skip,
                                                tr("app.mongo.query.skip"),
                                                Some(tr("app.mongo.query.hint_skip")),
                                                tone,
                                                colors,
                                                window,
                                                cx,
                                            ),
                                        ))
                                        .child(mongo_query_option_cell(
                                            "hint",
                                            None,
                                            1.5,
                                            colors,
                                            mongo_query_editor_box(
                                                "hint",
                                                &hint,
                                                tr("app.mongo.query.hint"),
                                                Some(tr("app.mongo.query.hint_index")),
                                                tone,
                                                colors,
                                                window,
                                                cx,
                                            ),
                                        ))
                                        .child(mongo_query_option_cell(
                                            "max_time_ms",
                                            Some(MONGO_QUERY_MAX_TIME_WIDTH),
                                            0.,
                                            colors,
                                            mongo_query_number_box(
                                                "max_time_ms",
                                                &max_time_ms,
                                                tr("app.mongo.query.max_time"),
                                                Some(tr("app.mongo.query.unit_ms")),
                                                tone,
                                                colors,
                                                window,
                                                cx,
                                            ),
                                        )),
                                )
                            }),
                    )
                }),
        )
}

/// 查询面板的输入外框：圆角、自己画焦点态，高度由调用方给定。
///
/// Filter、附加条件格子与条件构建器共用这一层，描边与悬停规则只有一处来源。
/// 常态填输入底色，实心框才读得成「可填写的格子」。
///
/// 高度不是同一个来路：Filter 与附加格子是被多行编辑器的内边距倒推出来的
/// （[`MONGO_QUERY_FILTER_HEIGHT`] / [`MONGO_QUERY_CONTROL_HEIGHT`]），
/// 条件构建器都是单行控件，直接取 [`MONGO_QUERY_BUILDER_CONTROL_HEIGHT`]。
fn mongo_query_box_at(
    height: f32,
    focused: bool,
    colors: UiColors,
    control: impl IntoElement,
) -> Div {
    let hover_border = mongo_query_input_hover_border(colors);
    div()
        .h(px(height))
        .w_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(if focused {
            mongo_query_input_border(true, colors)
        } else {
            mongo_query_input_border(false, colors)
        })
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .overflow_hidden()
        .hover(move |style| {
            if focused {
                style
            } else {
                style.border_color(hover_border)
            }
        })
        // 内容直接挂在外框上，不再包一层 `flex_1`：那层包装会让铭牌与输入区
        // 变成两个等分的兄弟，铭牌只要不是整格宽，输入区就被推到这一行的中间
        // （Filter 的 `{}` 落在正中间就是这么来的）。
        .child(control)
}

/// Filter 行：代码编辑器，带补全。
///
/// 用 `Editor` 而不是单行 `Input`：只有 `EditorState` 能挂补全 provider。
/// 外观由外框接管（`appearance(false)` + `bordered(false)`），不折行——单个条件
/// 通常一行写完，整条结构才看得全。
fn mongo_query_input(
    input: &Entity<EditorState>,
    height: f32,
    tone: MongoQueryTone,
    colors: UiColors,
    window: &Window,
    cx: &App,
) -> Div {
    // 外层只管「在行里垂直居中」，调试选择器挂在**编辑器自己那块**尺寸上：
    // 挂在外层的 h_full 包裹 div 上，量出来的永远是行高，与文字落点无关。
    let selector = "mongo-query-editor:filter".to_string();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    mongo_query_box_at(
        height,
        focused,
        colors,
        div()
            .h_full()
            .w(px(MONGO_QUERY_LABEL_WIDTH))
            .flex_none()
            .pl(px(11.))
            .pr(px(11.))
            .border_r_1()
            .border_color(colors.border)
            .bg(tone.mint)
            .flex()
            .items_center()
            .text_size(px(10.))
            .font_family(crate::PLATFORM_MONOSPACE_FONT)
            .text_color(colors.muted)
            .child(tr("app.mongo.query.filter")),
    )
    .child(
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .items_center()
            .child(mongo_query_editor_body(
                input,
                Some(selector),
                12.,
                Some(height - 2. * MONGO_QUERY_BOX_BORDER),
                false,
            )),
    )
}

/// 编辑器本体：盒子在内容区里垂直居中，超出的高度由外层裁掉。
///
/// Filter 与条件构建的两格用这一层（都要挂补全）。附加条件仍是单行输入，
/// 走 [`mongo_query_input_body`]。
///
/// `selector` 只给布局测试用；附加条件里的编辑器不需要单独测量。
/// `text_size` 由调用方给：Filter 是主入口用 12px，附加格子用 11px 压扁高度。
/// `height` 给「内容区有多高就该落多高」的调用方：Filter 给整块框高，超出部分由
/// 编辑器自己滚动（`Editor` 自带滚动条与滚轮处理），而不是被外层裁掉——裁掉的话
/// 光标能跑到看不见的地方；条件构建的两格给 `2 × 上内边距 + 一行`，居中后首行
/// 正好落在 28px 格子的中线上。
/// `disabled` 只给条件构建的值格：`$exists` 的取值恒为布尔，不需要输入。
fn mongo_query_editor_body(
    input: &Entity<EditorState>,
    selector: Option<String>,
    text_size: f32,
    height: Option<f32>,
    disabled: bool,
) -> Div {
    // 编辑器盒子在内容区里**垂直居中**，多出的部分被 `overflow_hidden` 裁掉。
    //
    // 编辑器自己不会居中（它总是从盒子顶边往下排，顶部还压着 8px 内边距），
    // 所以盒子高度必须让「首行」正好落在容器中线上：调用方把 `height` 给成
    // `2 × 上内边距 + 一行`，居中后首行既不会顶到上边框，也不会比中线低。
    // 不给 `height` 时盒子仍按内容长高，与之前一样贴顶排版。
    div()
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .child(
            div()
                .w_full()
                .when_some(selector, |this, selector| {
                    this.debug_selector(move || selector.clone())
                })
                .child(
                    Editor::new(input)
                        .appearance(false)
                        .bordered(false)
                        .disabled(disabled)
                        .w_full()
                        .text_size(px(text_size))
                        // 查询语句是 JSON 片段，等宽才对得齐括号与缩进。
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .when_some(height, |this, height| this.h(px(height))),
                ),
        )
}

/// 单行输入本体：竖向居中，撑满格子剩余宽度。
///
/// 单行 `Input` 在 `request_layout` 里自建 `Style`，只认自己的行高，
/// 外面给的 `h_full` 进不去；不额外居中就会贴住格子顶边。
fn mongo_query_input_body(
    input: &Entity<InputState>,
    selector: Option<String>,
    text_size: f32,
    disabled: bool,
) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .items_center()
        .child(
            div()
                .w_full()
                .when_some(selector, |this, selector| {
                    this.debug_selector(move || selector.clone())
                })
                .child(
                    Input::new(input)
                        .small()
                        .appearance(false)
                        .focus_bordered(false)
                        .disabled(disabled)
                        .w_full()
                        .px_2()
                        .text_size(px(text_size))
                        .font_family(crate::PLATFORM_MONOSPACE_FONT),
                ),
        )
}

/// 附加条件里的输入格子（投影 / 排序 / 排序规则 / 索引提示）。
///
/// `field` 只用于测试选择器：格子外面还包着尺寸层（`mongo_query_option_cell`），
/// 断言输入框本身没变形要单独量这一层。
///
/// 标签与 Filter 的铭牌同一套写法（框内左侧、右侧分隔线、mint 底），但更淡：
/// 附加格子不是主入口，名称不该抢 Filter 的注意力。
#[allow(clippy::too_many_arguments)]
fn mongo_query_editor_box(
    field: &'static str,
    input: &Entity<InputState>,
    label: SharedString,
    hint: Option<SharedString>,
    tone: MongoQueryTone,
    colors: UiColors,
    window: &Window,
    cx: &App,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    mongo_query_box_at(
        MONGO_QUERY_CONTROL_HEIGHT,
        focused,
        colors,
        mongo_query_cell_label(label, hint, tone, colors),
    )
    .child(mongo_query_input_body(input, None, 11., false))
    .debug_selector(move || format!("mongo-query-editor-box:{field}"))
}

/// 附加条件里的数字格子（条数 / 起始位置 / 执行上限）。
///
/// 保持 `InputState` 而不是换成编辑器：这几格只收数字，补全没有意义，
/// 而 `InputState::validate` 能在按键阶段直接拒掉非法值。
///
/// `hint`（边界 `1–500`、单位 `ms`）走 hover 提示而不是铭牌：数字格本来就窄，
/// 铭牌里再塞一段边界文字会把输入区挤到看不见。
#[allow(clippy::too_many_arguments)]
fn mongo_query_number_box(
    field: &'static str,
    input: &Entity<InputState>,
    label: SharedString,
    hint: Option<SharedString>,
    tone: MongoQueryTone,
    colors: UiColors,
    window: &Window,
    cx: &App,
) -> Stateful<Div> {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    // 悬停提示要挂在有状态的元素上（`tooltip` 来自 `StatefulInteractiveElement`）。
    let id = SharedString::from(format!("mongo-query-number:{field}"));
    mongo_query_box_at(
        MONGO_QUERY_CONTROL_HEIGHT,
        focused,
        colors,
        mongo_query_cell_label(label, None, tone, colors),
    )
    .child(
        // 单行 `Input` 在 `request_layout` 里自建 `Style`，只认「行高」，
        // 外面给的 `h_full` 进不去。所以居中要在这一层做：否则 32px 的格子里
        // 16px 高的输入段落贴顶，字形看起来比铭牌高出一截。
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .items_center()
            .child(
                div()
                    .debug_selector(move || format!("mongo-query-number-body:{field}"))
                    .w_full()
                    .child(
                        Input::new(input)
                            .small()
                            .appearance(false)
                            .focus_bordered(false)
                            .w_full()
                            .px_2()
                            .text_size(px(11.))
                            .font_family(crate::PLATFORM_MONOSPACE_FONT),
                    ),
            ),
    )
    .id(id)
    .when_some(hint, |this, hint| {
        this.tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
    })
}

/// 附加格子的框内铭牌：名称在左，可选旁注（`可选`）在右，整块垫 mint 底并加一条右分隔线。
///
/// 用铭牌而不是框上方的标签行：标签行每个格子都白吃掉一条 ~19px 的横向带，
/// 展开「更多选项」时四格叠起来就是一整段空白，正是结果区被挤走的来源。
/// 数字格不传 `hint`——它们窄，边界与单位改走悬停提示。
fn mongo_query_cell_label(
    label: SharedString,
    hint: Option<SharedString>,
    tone: MongoQueryTone,
    colors: UiColors,
) -> Div {
    div()
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(8.))
        .border_r_1()
        .border_color(colors.border)
        .bg(tone.mint)
        .text_size(px(10.))
        .text_color(colors.muted)
        .whitespace_nowrap()
        .child(label)
        .when_some(hint, |this, hint| {
            this.child(
                div()
                    .text_size(px(9.))
                    .text_color(colors.muted)
                    .child(hint),
            )
        })
}

/// 附加条件格子的尺寸层：标签已经塞进框里，这里只剩宽度分配。
///
/// `width` 为空时格子吃掉整行剩余宽度；给定宽度用于数字格（固定窄框，不随窗口拉长）。
/// `grow` 是交互稿里的列配比：投影/排序各 1 份，专家行里给对象格 1.5 份。
fn mongo_query_option_cell(
    field: &'static str,
    width: Option<f32>,
    grow: f32,
    _colors: UiColors,
    control: impl IntoElement,
) -> Div {
    let control = div().min_w(px(0.)).child(control);
    let (control, cell) = match width {
        // 定宽格子连外层一起钉住：数字格不该随窗口拉长，右侧留白会像漏掉的空列。
        Some(width) => (
            control.w(px(width)).flex_none(),
            div().w(px(width)).flex_none(),
        ),
        // `flex_1` 先把基数钉成 0，再按 `grow` 分配：只写 `flex_grow` 时基数是内容宽度，
        // 格子里的 `w_full` 会以整行为基准，把对象格压到比数字格还窄。
        None => (
            control.w_full(),
            div().flex_1().flex_grow(grow).min_w(px(0.)),
        ),
    };
    cell.flex()
        .flex_col()
        .min_w(px(0.))
        .child(control)
        .debug_selector(move || format!("mongo-query-field:{field}"))
}

/// 第二层折叠的入口：「高级查询」+ 箭头，展开时箭头朝上。
///
/// 用无框 `ghost` 按钮而不是描边控件：这已经是卡片里的第三层控件，再加一个带框按钮
/// 会和首行的「更多选项」抢同一档视觉权重。复用 `Button` 而不是自绘 `div`，
/// 悬停、焦点环与键盘触发才有统一来源。
fn mongo_query_advanced_toggle(
    tab_id: TabId,
    expanded: bool,
    advanced_count: usize,
    tone: MongoQueryTone,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    Button::new(("mongo-query-advanced", tab_id.0))
        .debug_selector(|| "mongo-query-advanced".to_string())
        .small()
        .h(px(20.))
        .ghost()
        .px(px(0.))
        .text_size(px(10.))
        .label(tr("app.mongo.query.advanced"))
        // 首层开着、第二层收起时，低频条件同样不可见：这里也要给计数。
        .when(advanced_count > 0 && !expanded, |this| {
            this.child(mongo_query_count_badge(
                "mongo-query-advanced-count",
                advanced_count,
                tone,
                colors,
            ))
        })
        .child(app_icon(
            if expanded {
                AppIcon::ChevronUp
            } else {
                AppIcon::ChevronDown
            },
            11.,
            colors.muted,
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_mongo_query_advanced(tab_id, cx);
            cx.stop_propagation();
        }))
}

/// 条数格的边界悬停提示：取当前生效上限，写成 `1–500`。
fn mongo_query_limit_range(ceiling: u64) -> SharedString {
    SharedString::from(format!("1–{ceiling}"))
}

/// 输入框外框描边：聚焦时用主题的强调线，其余用常规分隔线。
fn mongo_query_input_border(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        if colors.is_dark {
            rgb(0x8ab4ff)
        } else {
            rgb(0x111111)
        }
    } else {
        colors.border
    }
}

/// 未聚焦时的悬停描边：比常规分隔线深一档，提示这一块可点可输入。
fn mongo_query_input_hover_border(colors: UiColors) -> gpui::Rgba {
    if colors.is_dark {
        rgb(0x5b6675)
    } else {
        rgb(0xb8c0cc)
    }
}

/// 查询：面板里唯一的实心按钮，也是主操作（交互稿 `.button.primary`）。
fn mongo_query_find_button(tab_id: TabId, loading: bool, cx: &mut Context<NavicatMain>) -> Button {
    Button::new(("mongo-query-find", tab_id.0))
        .debug_selector(|| "mongo-query-find".to_string())
        .small()
        .h(px(MONGO_QUERY_ACTION_HEIGHT))
        // 按钮不参与拉伸：Filter 是唯一该长的那一格，否则窗口一宽按钮就变成大色块。
        .flex_none()
        // 交互稿给主按钮 90px 下限：两个字加图标也不至于缩成小方块。
        .min_w(px(90.))
        .primary()
        // 在途时按钮自带转圈并吞掉点击，避免连点重复发查询。
        .when(loading, |this| this.loading(true))
        .label(tr("app.mongo.query.find"))
        .disabled(loading)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.run_mongo_query(tab_id, cx);
            cx.stop_propagation();
        }))
}

/// 重置：只留图标（交互稿 `.reset-button span { display: none }`），
/// 文案改挂 tooltip / 无障碍标签——首行已经有主按钮，第二个带字按钮会把它们拉成同等分量。
fn mongo_query_reset_button(
    tab_id: TabId,
    loading: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    let label = tr("app.mongo.query.reset");
    Button::new(("mongo-query-reset", tab_id.0))
        .debug_selector(|| "mongo-query-reset".to_string())
        .small()
        .h(px(MONGO_QUERY_ACTION_HEIGHT))
        .px(px(9.))
        .flex_none()
        .outline()
        .tooltip(label.clone())
        .accessibility_label(label)
        .child(app_icon(AppIcon::Refresh, 13., colors.muted))
        .disabled(loading)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.reset_mongo_query(tab_id, window, cx);
            cx.stop_propagation();
        }))
}

/// 折叠态计数角标（交互稿 `.option-count`）：绿色浅底 + 绿字。
fn mongo_query_count_badge(
    selector: &'static str,
    count: usize,
    tone: MongoQueryTone,
    colors: UiColors,
) -> Div {
    div()
        .debug_selector(move || selector.to_string())
        .flex_none()
        .h(px(15.))
        .min_w(px(15.))
        .px(px(3.))
        .rounded(px(4.))
        .bg(tone.green_soft)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(9.))
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_color(colors.success)
        .child(SharedString::from(count.to_string()))
}

/// 更多选项：文字 + 箭头，无描边无底色（与 Compass 的 `OptionsToggle` 一致）。
///
/// 这一行已有实心的查询和描边的重置，再加第三种带框控件会把三个按钮拉成
/// 同等分量。展开时换成绿底绿字（交互稿 `[aria-expanded="true"]`）：
/// 网点一展开，这条按钮与下方内容之间就需要一个「是我打开的」的回执。
fn mongo_query_options_button(
    tab_id: TabId,
    expanded: bool,
    option_count: usize,
    tone: MongoQueryTone,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    Button::new(("mongo-query-options", tab_id.0))
        .debug_selector(|| "mongo-query-options".to_string())
        .small()
        .h(px(MONGO_QUERY_ACTION_HEIGHT))
        .flex_none()
        .ghost()
        .when(expanded, |this| {
            this.bg(tone.green_soft)
                .text_color(colors.success)
                .border_1()
                .border_color(tone.green_line)
        })
        .label(tr("app.mongo.query.options"))
        // 角标只在收起且确有生效条件时出现：收起时格子不可见，这是「条件还在」的唯一提示。
        .when(option_count > 0 && !expanded, |this| {
            this.child(mongo_query_count_badge(
                "mongo-query-option-count",
                option_count,
                tone,
                colors,
            ))
        })
        .child(app_icon(
            if expanded {
                AppIcon::ChevronUp
            } else {
                AppIcon::ChevronDown
            },
            11.,
            if expanded { colors.success } else { colors.muted },
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_mongo_query_options(tab_id, cx);
            cx.stop_propagation();
        }))
}

/// 条件构建入口：与投影/排序/返回条数同处一行。
///
/// 形态跟「高级查询」一致——ghost 文字按钮 + 图标，展开时绿字。这一行已经全是
/// 输入框，再放一个描边大方块按钮会把整行的视觉重心拖到左边，也白占一格宽度。
fn mongo_query_builder_toggle(
    tab_id: TabId,
    expanded: bool,
    group_count: usize,
    tone: MongoQueryTone,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    Button::new(("mongo-query-builder-toggle", tab_id.0))
        .debug_selector(|| "mongo-query-builder-toggle".to_string())
        .small()
        .h(px(MONGO_QUERY_CONTROL_HEIGHT))
        .ghost()
        .px(px(8.))
        .text_size(px(10.))
        .when(expanded, |this| {
            this.bg(tone.green_soft).text_color(colors.success)
        })
        .label(if group_count > 1 {
            SharedString::from(format!(
                "{} · {} {}",
                tr("app.mongo.query.builder"),
                group_count,
                tr("app.mongo.query.builder_groups")
            ))
        } else {
            tr("app.mongo.query.builder")
        })
        .child(app_icon(
            AppIcon::Filter,
            12.,
            if expanded { colors.success } else { colors.muted },
        ))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.toggle_mongo_query_builder(tab_id, window, cx);
            cx.stop_propagation();
        }))
}

fn mongo_results_toolbar(
    tab_id: TabId,
    state: &MongoCollectionState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = MONGO_DISPLAY_MODES
        .iter()
        .position(|(mode, _)| *mode == state.display)
        .unwrap_or(0);
    let view = cx.entity().downgrade();
    let bar = MONGO_DISPLAY_MODES
        .iter()
        .map(|(mode, label)| {
            // 展示方式页签需要稳定选择器，无头用例才能点击 JSON 视图。
            let mode = *mode;
            Tab::from(tr(label)).debug_selector(move || format!("mongo-display-tab:{mode:?}"))
        })
        .fold(
            TabBar::new(("mongo-display-tabs", tab_id.0))
                .segmented()
                .small()
                .selected_index(selected)
                .on_click(move |index, _, cx| {
                    if let Some((mode, _)) = MONGO_DISPLAY_MODES.get(*index) {
                        let mode = *mode;
                        let _ = view.update(cx, |this, cx| {
                            this.select_mongo_display(tab_id, mode, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        );
    let summary: SharedString = state
        .page
        .as_ref()
        .map(|page| match page.total {
            Some(total) => SharedString::from(
                tr("app.mongo.result.total").replace("%{count}", &total.to_string()),
            ),
            None => SharedString::from(
                tr("app.mongo.result.returned").replace("%{count}", &page.returned.to_string()),
            ),
        })
        .unwrap_or_else(|| tr("app.mongo.result.none"));

    div()
        .h(px(MONGO_TOOLBAR_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(colors.border)
        .child(bar)
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(summary),
        )
        .child(div().flex_1())
        .when(!state.read_only, |bar| {
            bar.child(mongo_results_insert_button(tab_id, colors, cx))
                .child(mongo_results_import_button(tab_id, colors, cx))
        })
        .child(mongo_results_refresh_button(
            tab_id,
            state.loading,
            colors,
            cx,
        ))
}

/// 结果条右侧的「新增文档」按钮：打开与编辑 / 复制同一个抽屉，初值只有 `_id`。
///
/// 只读对象（视图）不渲染：视图不能写入，摆一个点了就报错的按钮只会误导用户。
fn mongo_results_insert_button(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    mongo_card_icon_button(
        format!("mongo-results-insert-{}", tab_id.0),
        "mongo-results-insert".to_string(),
        AppIcon::Plus,
        tr("app.mongo.insert_document"),
        colors.text,
        true,
        false,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, window, cx| {
            this.open_mongo_document_insert(tab_id, window, cx);
            cx.stop_propagation();
        }),
    )
}

/// 结果条右侧的「导入文档」按钮：从 JSON / CSV 文件批量写入当前集合。
///
/// 与新增按钮同样只读对象不渲染；导入过程中弹窗自带中止入口。
fn mongo_results_import_button(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    mongo_card_icon_button(
        format!("mongo-results-import-{}", tab_id.0),
        "mongo-results-import".to_string(),
        AppIcon::FolderInput,
        tr("app.mongo.import.action"),
        colors.text,
        true,
        false,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.open_mongo_import_dialog(tab_id, cx);
            cx.stop_propagation();
        }),
    )
}

/// 结果条右侧的刷新按钮。
///
/// 结果条只有 38px 高、旁边是 11px 统计文字：通用 30px 描边工具栏按钮在这行里显得过大，
/// 改用卡片头 / 抽屉头同款的 24px 无边框图标按钮。查询在途让刷新图标自身旋转：
/// 反馈就落在用户刚点的那颗按钮上，不额外引入第二种图形。
fn mongo_results_refresh_button(
    tab_id: TabId,
    loading: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    mongo_card_icon_button(
        format!("mongo-results-refresh-{}", tab_id.0),
        "mongo-results-refresh".to_string(),
        AppIcon::Refresh,
        tr("app.common.refresh"),
        colors.text,
        !loading,
        // 刷新在途：图标自转，而不是换成另一个图形。
        loading,
        colors,
    )
    // 在途时按钮降为不可用样式，但点击仍要吞掉，避免连点重复发查询。
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            if !loading {
                this.run_mongo_query(tab_id, cx);
            }
            cx.stop_propagation();
        }),
    )
}

fn mongo_results(
    state: &MongoCollectionState,
    cache: &MongoDocumentCache,
    documents: &MongoDocumentView,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tab_id = documents.tab_id;
    let container = div().flex_1().min_w(px(0.)).min_h(px(0.));
    if let Some(error) = &state.error {
        return container.child(mongo_error_block(
            tab_id,
            MongoRetry::Documents,
            error,
            colors,
            cx,
        ));
    }
    let Some(page) = &state.page else {
        // 结果区容器默认不是 flex，直接塞居中提示会只按文本高度布局而贴顶；
        // 空态分支显式开 flex，让提示占满结果区并垂直居中。
        let container = container.flex();
        return container.child(if state.loading {
            center_loading_message(tr("app.mongo.loading"), colors)
        } else {
            center_message(tr("app.mongo.empty"), colors)
        });
    };
    if page.documents.is_empty() {
        return container
            .flex()
            .child(center_message(tr("app.mongo.empty"), colors));
    }
    let context = MongoCardContext::from_state(state);
    // 文档与 JSON 共用卡片列表（交互稿同一套卡头），只有正文不同；表格单独成表。
    match state.display {
        // 卡片高度随折叠态变化：虚拟列表逐项测量，只渲染视口附近的行。
        MongoDisplayMode::Documents | MongoDisplayMode::Json => {
            let view = documents.clone();
            let cards = list(
                cache.card_list.clone(),
                cx.processor(move |_this, index: usize, _window, cx| {
                    let document = &view.page.documents[index];
                    // 卡片间距靠外层内边距（`list` 不提供 gap）：上边距即两卡之间与顶部的留白。
                    div()
                        .px_2()
                        .pt_2()
                        .child(mongo_document_card(&view, index, document, context, cx))
                        .into_any_element()
                }),
            )
            .size_full();
            // 虚拟列表要有确定的可视区高度才能算出渲染范围：结果区改成撑满的 flex 列，
            // 视口拿到剩余高度（与侧边栏虚拟树同一套写法）。
            let viewport = div()
                .id(("mongo-card-list", tab_id.0))
                .debug_selector(|| "mongo-card-list".to_string())
                .flex_1()
                .min_h(px(0.))
                .relative()
                .overflow_hidden()
                .child(cards)
                .vertical_scrollbar(&cache.card_list);
            container.flex().flex_col().child(viewport)
        }
        // 表格的数据区横向滚动（操作列固定在右侧不动），纵向滚动由两个虚拟列表共享。
        MongoDisplayMode::Table => {
            container.child(mongo_document_table(documents, cache, context, colors, cx))
        }
    }
}

fn mongo_error_block(
    tab_id: TabId,
    retry: MongoRetry,
    error: &fluxdb_core::UserFacingError,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.danger)
                .child(SharedString::from(error.title.clone())),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(SharedString::from(error.message.clone())),
        )
        .when_some(error.detail.clone(), |this, detail| {
            this.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(SharedString::from(detail)),
            )
        })
        .child(backup_text_button(
            tr("app.common.retry"),
            colors,
            cx.listener(move |this, _, _, cx| {
                this.retry_mongo_load(tab_id, retry, cx);
                cx.stop_propagation();
            }),
        ))
}

/// 右侧详情抽屉：整篇文档的 Extended JSON（只读）。
fn mongo_inspector(
    tab_id: TabId,
    index: usize,
    document: &fluxdb_core::MongoDocument,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut component = JsonComponent::new(JsonEditorConfig::default());
    component.load(&document.json);
    div()
        .w(px(MONGO_INSPECTOR_WIDTH))
        .flex_none()
        .h_full()
        .border_l_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(MONGO_TOOLBAR_HEIGHT))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(colors.border)
                .child(app_icon(AppIcon::FileSearch, 14., colors.muted))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(colors.text)
                        .child(tr("app.mongo.document_detail")),
                )
                .child(
                    // 与编辑 / 复制抽屉同一档：38px 头部里放 30px 描边按钮会顶满整条，显得笨重。
                    mongo_card_icon_button(
                        format!("mongo-inspector-close-{}", tab_id.0),
                        "mongo-inspector-close".to_string(),
                        AppIcon::Close,
                        tr("app.common.close"),
                        colors.text,
                        true,
                        false,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.close_mongo_document_drawer(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.mongo.extended_json"))
                .child(div().flex_1())
                .child(tr("app.mongo.bson_preserved")),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .p_2()
                .child(component.render_preview(colors)),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .border_t_1()
                .border_color(colors.border)
                .text_size(px(10.))
                .text_color(colors.muted)
                .child(tr("app.mongo.not_ai_context")),
        )
        .child(
            div()
                .text_size(px(0.))
                .child(SharedString::from(format!("mongo-inspector-{}", index))),
        )
}

fn mongo_pager(
    tab_id: TabId,
    state: &MongoCollectionState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let offset = state.pagination.offset;
    let limit = state.pagination.limit.max(1);
    let page_no = offset / limit + 1;
    let has_more = state.page.as_ref().is_some_and(|page| page.has_more);
    let range = state
        .page
        .as_ref()
        .map(|page| {
            if page.documents.is_empty() {
                "0".to_string()
            } else {
                format!("{}–{}", offset + 1, offset + page.returned)
            }
        })
        .unwrap_or_else(|| "0".to_string());

    div()
        .h(px(MONGO_PAGER_HEIGHT))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_color(colors.border)
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(SharedString::from(format!(
            "{} {range}",
            tr("app.mongo.pager.range")
        )))
        .child(div().flex_1())
        .child(
            query_toolbar_icon_button(
                tr("app.mongo.pager.previous"),
                AppIcon::ChevronLeft,
                offset > 0 && !state.loading,
                false,
                colors.text,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.set_mongo_offset(tab_id, offset.saturating_sub(limit), window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(SharedString::from(format!(
            "{} / {}",
            page_no,
            if has_more {
                "…".to_string()
            } else {
                page_no.to_string()
            }
        )))
        .child(
            query_toolbar_icon_button(
                tr("app.mongo.pager.next"),
                AppIcon::ChevronRight,
                has_more && !state.loading,
                false,
                colors.text,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.set_mongo_offset(tab_id, offset + limit, window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

/// 索引表行高：与 Compass 索引表一致的单行高度，展开定义时另起一行。
const MONGO_INDEX_ROW_HEIGHT: f32 = 40.;
/// 操作列宽度：只放一个删除图标按钮。
const MONGO_INDEX_ACTION_WIDTH: f32 = 44.;

/// 索引表可排序的列，顺序与表头一致。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MongoIndexSort {
    Name,
    Type,
    Size,
    Usage,
    Rate,
    Properties,
    Status,
}

impl MongoIndexSort {
    /// 稳定的列标识：调试选择器与测试都按它定位表头。
    fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Type => "type",
            Self::Size => "size",
            Self::Usage => "usage",
            Self::Rate => "rate",
            Self::Properties => "properties",
            Self::Status => "status",
        }
    }
}

/// 索引表的一列：表头和数据行共用同一份定义，宽度或伸缩权重只在这里改。
struct MongoIndexColumn {
    /// 列对应的排序字段。
    sort: MongoIndexSort,
    /// 固定宽度；`None` 表示按 `grow` 平分剩余宽度。
    width: Option<f32>,
    /// 平分剩余宽度时的权重，定宽列填 0。
    grow: f32,
    /// 表头文案的国际化 key。
    label_key: &'static str,
}

/// 表头与数据行的列顺序 / 列宽，两边都从这里取，避免表头和数据各算各的。
const MONGO_INDEX_COLUMNS: [MongoIndexColumn; 7] = [
    MongoIndexColumn {
        sort: MongoIndexSort::Name,
        width: None,
        grow: 2.,
        label_key: "app.mongo.indexes.column.name",
    },
    MongoIndexColumn {
        sort: MongoIndexSort::Type,
        width: Some(80.),
        grow: 0.,
        label_key: "app.mongo.indexes.column.type",
    },
    MongoIndexColumn {
        sort: MongoIndexSort::Size,
        width: Some(72.),
        grow: 0.,
        label_key: "app.mongo.indexes.column.size",
    },
    MongoIndexColumn {
        sort: MongoIndexSort::Usage,
        width: None,
        grow: 3.,
        label_key: "app.mongo.indexes.column.usage",
    },
    MongoIndexColumn {
        sort: MongoIndexSort::Rate,
        width: None,
        grow: 3.,
        label_key: "app.mongo.indexes.column.usage_rate",
    },
    MongoIndexColumn {
        sort: MongoIndexSort::Properties,
        width: Some(106.),
        grow: 0.,
        label_key: "app.mongo.indexes.column.properties",
    },
    MongoIndexColumn {
        sort: MongoIndexSort::Status,
        width: Some(68.),
        grow: 0.,
        label_key: "app.mongo.indexes.column.status",
    },
];

/// 统计缺失的索引排在最后：把「没有数据」当成 0 会挤到最前，误导用户以为从没用过。
fn mongo_index_option_cmp<T: Ord>(left: Option<T>, right: Option<T>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// 按当前排序列比较两条索引；属性列按展示顺序里的第一个属性比较（同 Compass）。
fn mongo_index_sort_cmp(
    column: MongoIndexSort,
    left: &fluxdb_core::MongoIndexInfo,
    right: &fluxdb_core::MongoIndexInfo,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match column {
        MongoIndexSort::Name => left.name.cmp(&right.name),
        MongoIndexSort::Type => left.index_type.cmp(&right.index_type),
        MongoIndexSort::Size => mongo_index_option_cmp(left.size_bytes, right.size_bytes),
        MongoIndexSort::Usage => mongo_index_option_cmp(left.usage_count, right.usage_count),
        MongoIndexSort::Rate => {
            let left = mongo_index_rate_per_minute(left);
            let right = mongo_index_rate_per_minute(right);
            match (left, right) {
                (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
        MongoIndexSort::Properties => {
            let left = mongo_index_properties(left);
            let right = mongo_index_properties(right);
            left.first().map(|value| value.as_ref()).cmp(&right.first().map(|value| value.as_ref()))
        }
        MongoIndexSort::Status => left.ready.cmp(&right.ready),
    }
}

/// 当前排序下的索引行；未设排序时保持服务端返回的名称顺序。
fn mongo_index_sorted_rows(
    indexes: &[fluxdb_core::MongoIndexInfo],
    sort: Option<(MongoIndexSort, bool)>,
) -> Vec<&fluxdb_core::MongoIndexInfo> {
    let mut rows = indexes.iter().collect::<Vec<_>>();
    if let Some((column, descending)) = sort {
        rows.sort_by(|left, right| {
            let ordering = mongo_index_sort_cmp(column, left, right);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    rows
}

impl NavicatMain {
    /// 索引页：工具条 + 统计口径提示 + 表头 + 索引行（按 Compass 的列布局）。
    ///
    /// 只读：本页不提供创建 / 删除索引，工具条只留刷新。
    fn mongo_indexes_section(
        &self,
        tab_id: TabId,
        state: &MongoCollectionState,
        colors: UiColors,
        cx: &mut Context<Self>,
    ) -> Div {
        let container = div().flex_1().min_h(px(0.)).flex().flex_col();
        let loading = matches!(state.indexes, LoadState::Loading | LoadState::NotLoaded);
        match &state.indexes {
            LoadState::Loading | LoadState::NotLoaded => {
                // 提示靠 flex_1 撑满：容器不开 flex 时只按文本高度布局而贴顶。
                container.child(center_loading_message(tr("app.mongo.loading"), colors))
            }
            LoadState::Failed(error) => container.child(mongo_error_block(
                tab_id,
                MongoRetry::Indexes,
                error,
                colors,
                cx,
            )),
            LoadState::Loaded(indexes) if indexes.is_empty() => container
                .child(mongo_indexes_toolbar(tab_id, loading, cx))
                .child(div().flex_1().flex().child(center_message(
                    tr("app.mongo.indexes.empty"),
                    colors,
                ))),
            LoadState::Loaded(indexes) => {
                let expanded = self.mongo_expanded_indexes.get(&tab_id);
                let sort = self.mongo_index_sort.get(&tab_id).copied();
                container
                    .child(mongo_indexes_toolbar(tab_id, loading, cx))
                    .child(mongo_indexes_stats_banner(colors))
                    .child(mongo_indexes_header(tab_id, sort, colors, cx))
                    .child(
                        div()
                            .id(("mongo-indexes-rows", tab_id.0))
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .children(mongo_index_sorted_rows(indexes, sort).into_iter().map(
                                |index| {
                                    let is_expanded =
                                        expanded.is_some_and(|set| set.contains(&index.name));
                                    mongo_index_row(tab_id, index, is_expanded, colors, cx)
                                },
                            )),
                    )
            }
        }
    }

    /// 展开 / 收起一条索引的键定义。
    pub(crate) fn toggle_mongo_index_expanded(
        &mut self,
        tab_id: TabId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let expanded = self.mongo_expanded_indexes.entry(tab_id).or_default();
        if !expanded.remove(&name) {
            expanded.insert(name);
        }
        cx.notify();
    }

    /// 点表头切换排序：同一列再点一次翻转方向，换列则从升序开始。
    pub(crate) fn toggle_mongo_index_sort(
        &mut self,
        tab_id: TabId,
        column: MongoIndexSort,
        cx: &mut Context<Self>,
    ) {
        match self.mongo_index_sort.get_mut(&tab_id) {
            Some(entry) if entry.0 == column => entry.1 = !entry.1,
            Some(entry) => *entry = (column, false),
            None => {
                self.mongo_index_sort.insert(tab_id, (column, false));
            }
        }
        cx.notify();
    }

    /// 删除索引前必弹确认框：误删会让相关查询退回全表扫描，代价要用户自己确认。
    pub(crate) fn request_drop_mongo_index(
        &mut self,
        tab_id: TabId,
        index_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        let title = tr("app.mongo.indexes.drop.title").to_string();
        let description = tr("app.mongo.indexes.drop.description")
            .replace("%{name}", &index_name)
            .to_string();
        let confirm = tr("app.mongo.indexes.drop.confirm").to_string();
        let cancel = tr("app.common.cancel").to_string();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let index_name = index_name.clone();
            dialog
                .title(title.clone())
                .close_button(true)
                .overlay_closable(true)
                .keyboard(true)
                .child(div().text_size(px(13.)).child(description.clone()))
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("mongo-index-drop-cancel")
                                .debug_selector(|| "mongo-index-drop-cancel".into())
                                .cursor_pointer()
                                .child(cancel.clone())
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("mongo-index-drop-confirm")
                                .debug_selector(|| "mongo-index-drop-confirm".into())
                                .danger()
                                .cursor_pointer()
                                .child(confirm.clone())
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(gpui_component::dialog::Confirm {
                                            secondary: false,
                                        }),
                                        cx,
                                    )
                                }),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.start_drop_mongo_index(tab_id, index_name.clone(), cx)
                    });
                    true
                })
        });
    }

    /// 后台删除索引；成功后重取整表，因为大小与使用统计都挂在索引上。
    fn start_drop_mongo_index(
        &mut self,
        tab_id: TabId,
        index_name: String,
        cx: &mut Context<Self>,
    ) {
        if self._mongo_drop_index_tasks.contains_key(&tab_id.0) {
            return;
        }
        let Some(state) = mongo_collection_state(self.controller.state(), tab_id) else {
            return;
        };
        let object = state.object.clone();
        let Some(database) = object.database.clone() else {
            return;
        };
        let task = self.spawn_controller_command(
            AppCommand::DropMongoIndex {
                connection_id: object.connection_id,
                database,
                collection: object.name.clone(),
                index: index_name.clone(),
            },
            move |this, event, cx| {
                this._mongo_drop_index_tasks.remove(&tab_id.0);
                match event {
                    AppEvent::MongoIndexDropped { index, .. } => {
                        if let Some(expanded) = this.mongo_expanded_indexes.get_mut(&tab_id) {
                            expanded.remove(&index);
                        }
                        this.show_message(
                            tr("app.mongo.indexes.drop.done").replace("%{name}", &index),
                            AppMessageKind::Success,
                            cx,
                        );
                        this.retry_mongo_load(tab_id, MongoRetry::Indexes, cx);
                    }
                    AppEvent::Failed(error) => {
                        this.show_message(
                            tr("app.mongo.indexes.drop.failed")
                                .replace("%{error}", &error.message),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    _ => {}
                }
            },
            cx,
        );
        self._mongo_drop_index_tasks.insert(tab_id.0, task);
    }
}

/// 索引页工具条：只读视图只有刷新；与 Compass 一致放在左上角。
fn mongo_indexes_toolbar(
    tab_id: TabId,
    loading: bool,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .child(
            Button::new(("mongo-indexes-refresh", tab_id.0))
                .debug_selector(|| "mongo-indexes-refresh".to_string())
                .small()
                .outline()
                .loading(loading)
                .disabled(loading)
                .icon(gpui_component::Icon::default().path(app_icon_path(AppIcon::Refresh)))
                .label(tr("app.common.refresh"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.retry_mongo_load(tab_id, MongoRetry::Indexes, cx);
                    cx.stop_propagation();
                })),
        )
}

/// 统计口径提示：`$indexStats` 只反映当前连接节点的计数（同 Compass）。
fn mongo_indexes_stats_banner(colors: UiColors) -> Div {
    div()
        .px_3()
        .pb_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.))
        .text_color(colors.warning)
        .child(app_icon(AppIcon::TriangleAlert, 12., colors.warning))
        .child(tr("app.mongo.indexes.stats_node_note"))
}

/// 表头固定不参与滚动，列宽与数据行共用 [`mongo_index_cell`]；可排序列点击切换。
fn mongo_indexes_header(
    tab_id: TabId,
    sort: Option<(MongoIndexSort, bool)>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(34.))
        .px_3()
        .flex()
        .items_center()
        .border_t_1()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .children(
            MONGO_INDEX_COLUMNS
                .iter()
                .map(|column| mongo_index_header_cell(tab_id, column, sort, colors, cx)),
        )
        // 操作列没有标题，只为与数据行的删除按钮对齐。
        .child(mongo_index_cell(
            Some(MONGO_INDEX_ACTION_WIDTH),
            0.,
            div(),
        ))
}

/// 可排序表头单元格：箭头只在当前排序列上点亮，其余列给一个弱化的上下箭头示意。
fn mongo_index_header_cell(
    tab_id: TabId,
    column: &MongoIndexColumn,
    sort: Option<(MongoIndexSort, bool)>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sort_column = column.sort;
    let key = sort_column.key();
    let selector = SharedString::from(format!("mongo-index-sort:{key}"));
    let active = sort.filter(|(sorted, _)| *sorted == sort_column);
    let icon_color = if active.is_some() {
        colors.text
    } else {
        colors.muted.opacity(0.55)
    };
    let icon = match active {
        Some((_, true)) => AppIcon::ArrowDown,
        Some((_, false)) => AppIcon::ArrowUp,
        None => AppIcon::ArrowUpDown,
    };
    let label = tr(column.label_key);
    mongo_index_cell(
        column.width,
        column.grow,
        div()
            .id(SharedString::from(format!(
                "mongo-index-sort:{key}:{}",
                tab_id.0
            )))
            .w_full()
            .flex()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_mongo_index_sort(tab_id, sort_column, cx);
                    cx.stop_propagation();
                }),
            )
            .child(label)
            .child(app_icon(icon, 11., icon_color)),
    )
    .debug_selector(move || selector.to_string())
}

/// 单元格尺寸层：定宽列不参与拉伸，其余列按 `grow` 平分剩余宽度。
fn mongo_index_cell(
    width: Option<f32>,
    grow: f32,
    child: impl IntoElement,
) -> Div {
    let cell = div().min_w(px(0.)).px_2().overflow_hidden().child(child);
    match width {
        Some(width) => cell.w(px(width)).flex_none(),
        // 名称列吃掉剩余宽度，但不允许缩到只剩一个省略号。
        None => cell.flex_1().flex_grow(grow).min_w(px(110.)),
    }
}

/// 一个数据单元格的内容；列宽由 [`MONGO_INDEX_COLUMNS`] 统一给出，这里只管值。
fn mongo_index_row_cell(
    tab_id: TabId,
    index: &fluxdb_core::MongoIndexInfo,
    column: MongoIndexSort,
    expanded: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    match column {
        MongoIndexSort::Name => {
            let name = index.name.clone();
            let toggle_selector =
                SharedString::from(format!("mongo-index-toggle:{}", index.name));
            let toggle_id = SharedString::from(format!(
                "mongo-index-toggle:{}:{}",
                tab_id.0, index.name
            ));
            div()
                .id(toggle_id)
                .debug_selector(move || toggle_selector.to_string())
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_mongo_index_expanded(tab_id, name.clone(), cx);
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    if expanded {
                        AppIcon::ChevronDown
                    } else {
                        AppIcon::ChevronRight
                    },
                    13.,
                    colors.muted,
                ))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .child(SharedString::from(index.name.clone())),
                )
                .into_any_element()
        }
        MongoIndexSort::Type => mongo_index_badge(
            mongo_index_type_label(&index.index_type),
            colors.muted,
            colors,
        )
        .into_any_element(),
        MongoIndexSort::Size => {
            mongo_index_muted_text(mongo_index_size_text(index.size_bytes), colors)
                .into_any_element()
        }
        MongoIndexSort::Usage => {
            mongo_index_muted_text(mongo_index_usage_text(index), colors).into_any_element()
        }
        MongoIndexSort::Rate => {
            mongo_index_muted_text(mongo_index_rate_text(index), colors).into_any_element()
        }
        MongoIndexSort::Properties => div()
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .children(
                mongo_index_properties(index)
                    .into_iter()
                    .map(|badge| mongo_index_badge(badge, colors.muted, colors)),
            )
            .into_any_element(),
        MongoIndexSort::Status => mongo_index_status_pill(index.ready, colors).into_any_element(),
    }
}

/// 一条索引：主行 + 展开后的键定义。
fn mongo_index_row(
    tab_id: TabId,
    index: &fluxdb_core::MongoIndexInfo,
    expanded: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
 ) -> Div {
    let row_selector = SharedString::from(format!("mongo-index-row:{}", index.name));
    div()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(colors.border_soft)
        .debug_selector(move || row_selector.to_string())
        .when(expanded, |this| this.bg(colors.panel_alt))
        .child(
            div()
                .h(px(MONGO_INDEX_ROW_HEIGHT))
                .px_3()
                .flex()
                .items_center()
                .hover(|style| style.bg(colors.hover))
                .children(MONGO_INDEX_COLUMNS.iter().map(|column| {
                    let content =
                        mongo_index_row_cell(tab_id, index, column.sort, expanded, colors, cx);
                    let cell_name = index.name.clone();
                    let key = column.sort.key();
                    mongo_index_cell(column.width, column.grow, content)
                        .debug_selector(move || format!("mongo-index-cell:{cell_name}:{key}"))
                }))
                .child(mongo_index_cell(
                    Some(MONGO_INDEX_ACTION_WIDTH),
                    0.,
                    mongo_index_drop_action(tab_id, index, colors, cx),
                )),
        )
        .when(expanded, |this| {
            this.child(mongo_index_expanded_definition(index, colors))
        })
}

/// 操作列：只有可删除的索引才画删除按钮；`_id_` 是集合固有的索引，删不掉。
fn mongo_index_drop_action(
    tab_id: TabId,
    index: &fluxdb_core::MongoIndexInfo,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    if index.name == "_id_" {
        return div().into_any_element();
    }
    let name = index.name.clone();
    let selector = SharedString::from(format!("mongo-index-drop:{}", index.name));
    div()
        .id(SharedString::from(format!(
            "mongo-index-drop:{}:{}",
            tab_id.0, index.name
        )))
        .debug_selector(move || selector.to_string())
        .size(px(24.))
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(colors.hover))
        .child(app_icon(AppIcon::Trash, 13., colors.muted))
        .tooltip({
            let label = tr("app.mongo.indexes.drop.action");
            move |window, cx| Tooltip::new(label.clone()).build(window, cx)
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.request_drop_mongo_index(tab_id, name.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
        .into_any_element()
}

/// 键方向图标：1 / -1 用上下箭头，其余（text、hashed、2dsphere…）原样加括号。
fn mongo_index_key_direction(value: &str, colors: UiColors) -> AnyElement {
    match value {
        "1" => app_icon(AppIcon::ArrowUp, 11., colors.muted).into_any_element(),
        "-1" => app_icon(AppIcon::ArrowDown, 11., colors.muted).into_any_element(),
        other => div()
            .text_size(px(10.))
            .text_color(colors.muted)
            .child(SharedString::from(format!("({other})")))
            .into_any_element(),
    }
}

/// 展开行：键定义按 Compass 的样张排成圆角胶囊（字段名 + 方向箭头），部分索引另附过滤条件。
fn mongo_index_expanded_definition(
    index: &fluxdb_core::MongoIndexInfo,
    colors: UiColors,
) -> Div {
    let selector = SharedString::from(format!("mongo-index-keys:{}", index.name));
    div()
        .debug_selector(move || selector.to_string())
        .px(px(28.))
        .pt_1()
        .pb_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_wrap()
                .children(index.fields.iter().map(|field| {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_0p5()
                        .rounded(px(999.))
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(colors.panel_alt)
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_size(px(11.))
                        .text_color(colors.text)
                        .child(SharedString::from(field.field.clone()))
                        .child(mongo_index_key_direction(&field.value, colors))
                })),
        )
        .when_some(index.partial_filter.clone(), |this, filter| {
            this.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(SharedString::from(format!(
                        "{}: {filter}",
                        tr("app.mongo.indexes.partial_filter")
                    ))),
            )
        })
}

/// 类型徽标文案：未知类型直接原样展示，不吞掉服务端返回值。
fn mongo_index_type_label(index_type: &str) -> SharedString {
    match index_type {
        "regular" => tr("app.mongo.indexes.type.regular"),
        "geospatial" => tr("app.mongo.indexes.type.geospatial"),
        "hashed" => tr("app.mongo.indexes.type.hashed"),
        "text" => tr("app.mongo.indexes.type.text"),
        "wildcard" => tr("app.mongo.indexes.type.wildcard"),
        "columnstore" => tr("app.mongo.indexes.type.columnstore"),
        "clustered" => tr("app.mongo.indexes.type.clustered"),
        other => SharedString::from(other.to_string()),
    }
}

/// 属性徽标：与 Compass 同序（unique / sparse / partial / ttl / collation / hidden / compound）。
fn mongo_index_properties(index: &fluxdb_core::MongoIndexInfo) -> Vec<SharedString> {
    let mut properties = Vec::new();
    if index.unique || index.name == "_id_" {
        properties.push(tr("app.mongo.indexes.unique"));
    }
    if index.sparse {
        properties.push(tr("app.mongo.indexes.sparse"));
    }
    if index.partial_filter.is_some() {
        properties.push(tr("app.mongo.indexes.partial"));
    }
    if index.expire_after_seconds.is_some() {
        properties.push(tr("app.mongo.indexes.ttl"));
    }
    if index.has_collation {
        properties.push(tr("app.mongo.indexes.collation"));
    }
    if index.hidden {
        properties.push(tr("app.mongo.indexes.hidden"));
    }
    if index.fields.len() > 1 {
        properties.push(tr("app.mongo.indexes.compound"));
    }
    properties
}

/// 索引大小：与集合信息页同一套字节格式；拿不到时不显示数字。
fn mongo_index_size_text(size: Option<u64>) -> SharedString {
    match size {
        Some(bytes) => SharedString::from(mongo_human_bytes(bytes)),
        None => tr("app.mongo.indexes.size_unknown"),
    }
}

/// 使用次数：`$indexStats` 的 `accesses.ops` 与统计起点。
fn mongo_index_usage_text(index: &fluxdb_core::MongoIndexInfo) -> SharedString {
    let Some(count) = index.usage_count else {
        return tr("app.mongo.indexes.usage_unavailable");
    };
    match index.usage_since.and_then(mongo_index_since_text) {
        Some(since) => SharedString::from(
            tr("app.mongo.indexes.usage_since")
                .replace("%{count}", &count.to_string())
                .replace("%{since}", &since),
        ),
        None => SharedString::from(
            tr("app.mongo.indexes.usage_count_only")
                .replace("%{count}", &count.to_string()),
        ),
    }
}

/// 平均使用频率：`ops / 统计分钟数`。统计起点缺失或时间倒挂时不编造频率。
fn mongo_index_rate_text(index: &fluxdb_core::MongoIndexInfo) -> SharedString {
    let Some(rate) = mongo_index_rate_per_minute(index) else {
        return tr("app.mongo.indexes.rate_unavailable");
    };
    let formatted = if rate > 0.0 && rate < 1.0 {
        tr("app.mongo.indexes.rate_below_one")
    } else {
        SharedString::from(
            tr("app.mongo.indexes.rate_per_minute")
                .replace("%{count}", &format!("{rate:.0}")),
        )
    };
    match index.usage_since.and_then(mongo_index_since_text) {
        Some(since) => SharedString::from(
            tr("app.mongo.indexes.rate_with_since")
                .replace("%{rate}", formatted.as_ref())
                .replace("%{since}", &since),
        ),
        None => formatted,
    }
}

fn mongo_index_rate_per_minute(index: &fluxdb_core::MongoIndexInfo) -> Option<f64> {
    let usage = index.usage_count? as f64;
    let period_ms = chrono::Utc::now().timestamp_millis() - index.usage_since?;
    if period_ms <= 0 {
        return None;
    }
    Some(usage / period_ms as f64 * 60_000.)
}

/// 统计起点按 UTC 展示（与服务端 `$indexStats` 口径一致，避免本地时区跨日）。
fn mongo_index_since_text(millis: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|since| since.format("%a %b %d %Y").to_string())
}

/// 状态药丸：就绪（绿）/ 构建中（黄）。
fn mongo_index_status_pill(ready: bool, colors: UiColors) -> Div {
    let (label, color) = if ready {
        (tr("app.mongo.indexes.ready"), colors.success)
    } else {
        (tr("app.mongo.indexes.building"), colors.warning)
    };
    div()
        .px_1p5()
        .py_0p5()
        .rounded(px(999.))
        .bg(color.opacity(0.14))
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(color)
        .child(label)
}

fn mongo_index_muted_text(text: SharedString, colors: UiColors) -> Div {
    div()
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(text)
}

/// 小徽标：类型、属性共用。
fn mongo_index_badge(label: SharedString, color: gpui::Rgba, colors: UiColors) -> Div {
    div()
        .flex_none()
        .px_1()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .text_size(px(10.))
        .text_color(color)
        .whitespace_nowrap()
        .child(label)
}

fn mongo_info_section(
    tab_id: TabId,
    state: &MongoCollectionState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let container = div().flex_1().min_h(px(0.));
    match &state.info {
        LoadState::Loading | LoadState::NotLoaded => {
            // 同索引页：提示需要 flex 容器才能垂直居中。
            container
                .flex()
                .child(center_loading_message(tr("app.mongo.loading"), colors))
        }
        LoadState::Failed(error) => container.child(mongo_error_block(
            tab_id,
            MongoRetry::Info,
            error,
            colors,
            cx,
        )),
        LoadState::Loaded(info) => {
            let rows = [
                (tr("app.mongo.info.name"), info.name.clone()),
                (
                    tr("app.mongo.info.type"),
                    match info.kind {
                        fluxdb_core::MongoCollectionKind::Collection => {
                            tr("app.mongo.info.kind.collection").to_string()
                        }
                        fluxdb_core::MongoCollectionKind::View => {
                            tr("app.mongo.info.kind.view").to_string()
                        }
                        fluxdb_core::MongoCollectionKind::Timeseries => {
                            tr("app.mongo.info.kind.timeseries").to_string()
                        }
                    },
                ),
                (
                    tr("app.mongo.info.database"),
                    state.object.database.clone().unwrap_or_default(),
                ),
                (
                    tr("app.mongo.info.documents"),
                    mongo_optional_number(info.document_count),
                ),
                (
                    tr("app.mongo.info.avg_size"),
                    mongo_optional_bytes(info.avg_document_size),
                ),
                (
                    tr("app.mongo.info.data_size"),
                    mongo_optional_bytes(info.data_size),
                ),
                (
                    tr("app.mongo.info.storage_size"),
                    mongo_optional_bytes(info.storage_size),
                ),
                (
                    tr("app.mongo.info.indexes"),
                    mongo_optional_number(info.index_count),
                ),
            ];
            container.child(
                div()
                    .overflow_y_scrollbar()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows.into_iter().map(|(label, value)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_0p5()
                            .text_size(px(12.))
                            .child(div().w(px(140.)).text_color(colors.muted).child(label))
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(colors.text)
                                    .child(SharedString::from(value)),
                            )
                    }))
                    .when_some(info.view_on.clone(), |this, view_on| {
                        this.child(
                            div()
                                .mt_2()
                                .text_size(px(12.))
                                .text_color(colors.muted)
                                .child(SharedString::from(format!(
                                    "{}: {view_on}",
                                    tr("app.mongo.info.view_on")
                                ))),
                        )
                    })
                    .when_some(info.view_pipeline.clone(), |this, pipeline| {
                        let mut component = JsonComponent::new(JsonEditorConfig::default());
                        component.load(&pipeline);
                        this.child(div().mt_2().child(component.render_preview(colors)))
                    }),
            )
        }
    }
}

fn mongo_optional_number(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| tr("app.mongo.info.unknown").to_string())
}

fn mongo_optional_bytes(value: Option<u64>) -> String {
    match value {
        Some(bytes) => mongo_human_bytes(bytes),
        None => tr("app.mongo.info.unknown").to_string(),
    }
}

fn mongo_human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < UNITS.len() {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
