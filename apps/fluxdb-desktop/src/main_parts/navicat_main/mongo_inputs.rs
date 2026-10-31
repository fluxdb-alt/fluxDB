// MongoDB 集合工作台的查询面板输入框：按标签页懒建并缓存。
//
// 面板文本只在执行查询那一刻被读取（见 `mongo_loading`），平时不写回标签页状态：
// 这样输入框就是唯一草稿来源，不需要每个按键都走一次派发。
//
// 只有 Filter 和聚合管道用 `EditorState`：多行编辑器总会画滚动条
// （`shows_scrollbar() == is_multi_line()`，且代码编辑器恒定预留底部空白），
// 32px 高的附加格子里就会塞进一根拖不动的滑块。附加条件都是单行输入，
// 因此用单行 `InputState`——它靠自身偏移跟随光标，永远没有滚动条。
//
// 代价：单行输入框没有 `Lsp.completion_provider` 入口，投影/排序等格子不再给
// 字段补全；Filter 与聚合管道不受影响。

use std::cell::Cell;

/// 一个集合标签页的查询面板输入框集合（订阅随结构一起 drop，标签页关闭即解绑）。
pub(crate) struct MongoQueryInputs {
    pub(crate) filter: Entity<EditorState>,
    /// Filter 增行时把面板叫醒重算高度。行数只在跨档（1↔2↔3↔4）时变，
    /// 逐键都通知会把整个主视图多排一帧。
    _filter_rows_subscription: Subscription,
    /// 附加条件（投影 / 排序 / 排序规则 / 索引提示）都是单行输入。
    pub(crate) projection: Entity<InputState>,
    pub(crate) sort: Entity<InputState>,
    pub(crate) collation: Entity<InputState>,
    pub(crate) hint: Entity<InputState>,
    /// 纯数字格：起始位置、条数、执行上限。
    pub(crate) skip: Entity<InputState>,
    pub(crate) limit: Entity<InputState>,
    pub(crate) max_time_ms: Entity<InputState>,
    pub(crate) aggregation: Entity<EditorState>,
}

impl NavicatMain {
    /// 取（或惰性创建）指定集合标签页的查询输入框。
    ///
    /// 初值取「最后一次真正执行的查询」，因此刷新、切走再切回来都还是原来那份条件。
    pub(crate) fn mongo_query_inputs_for(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<MongoQueryInputs> {
        if let Some(inputs) = self.mongo_query_inputs.get(&tab_id) {
            return inputs.clone();
        }
        let (query, pipeline) = mongo_query_seed(self.controller.state(), tab_id);
        // 条数上限取设置里的默认查询限制：用户填得进去的值，执行时就必须原样生效，
        // 不能被连接器静默改小（否则「填 1000 只返回 500 条」看起来像丢了数据）。
        let limit_ceiling = mongo_query_limit(self.controller.state().settings.page_size);
        // 三框与聚合管道各自是独立实体：逐个在视图上下文里建，再装进 `MongoQueryInputs`。
        // 不嵌在 `cx.new(|cx| …)` 里建，是因为嵌套的上下文类型不同，闭包会把外层的
        // `cx` 与内层的搞混（编译器只能看到一个 `cx` 名）。
        let view = cx.entity().downgrade();
        let editor = |value: &str,
                      placeholder: &'static str,
                      mode: MongoCompletionMode,
                      view: gpui::WeakEntity<NavicatMain>,
                      window: &mut Window,
                      cx: &mut Context<NavicatMain>| {
            let value = value.to_string();
            cx.new(|cx| {
                let mut editor = EditorState::new(window, cx)
                    .placeholder(tr(placeholder))
                    .default_value(value.clone());
                if mode == MongoCompletionMode::Query {
                    // 单个条件通常一行写完：不折行才看得清整条结构。
                    editor = editor.soft_wrap(false);
                }
                configure_mongo_completion(&mut editor, mode, view, tab_id, window, cx);
                editor
            })
        };
        let filter = editor(
            &query.filter,
            "app.mongo.query.filter_placeholder",
            MongoCompletionMode::Query,
            view.clone(),
            window,
            cx,
        );
        // 附加条件：单行输入，不再挂补全（单行输入框没有 provider 入口）。
        let single = |value: &str,
                      placeholder: &'static str,
                      window: &mut Window,
                      cx: &mut Context<NavicatMain>| {
            let value = value.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(placeholder))
                    .default_value(value)
            })
        };
        let projection = single(
            &query.projection,
            "app.mongo.query.projection_placeholder",
            window,
            cx,
        );
        let sort = single(&query.sort, "app.mongo.query.sort_placeholder", window, cx);
        let collation = single(
            &query.collation,
            "app.mongo.query.collation_placeholder",
            window,
            cx,
        );
        let hint = single(&query.hint, "app.mongo.query.hint_placeholder", window, cx);
        let aggregation = editor(
            &pipeline,
            "app.mongo.query.aggregation_placeholder",
            MongoCompletionMode::Aggregation,
            view,
            window,
            cx,
        );
        // 不填就走默认值的数字格（起点 / 执行上限）：空串合法，其余必须是纯数字。
        let optional_number = |value: &str,
                               placeholder: &'static str,
                               window: &mut Window,
                               cx: &mut Context<NavicatMain>| {
            let value = value.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(placeholder))
                    .default_value(value)
                    .validate(|text, _| {
                        let text = text.trim();
                        text.is_empty() || text.chars().all(|ch| ch.is_ascii_digit())
                    })
            })
        };
        // 只接受 1..=ceiling 的纯数字：超出上限的输入在按键阶段就被拒绝，
        // 不会出现「先输进去、执行时再被悄悄改小」。
        let limit_value = query.limit.clone();
        let limit = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr("app.mongo.query.limit_placeholder"))
                .default_value(limit_value)
                .validate(move |text, _| {
                    let text = text.trim();
                    text.chars().all(|ch| ch.is_ascii_digit())
                        && text
                            .parse::<u64>()
                            .is_ok_and(|limit| (1..=limit_ceiling).contains(&limit))
                })
        });
        let skip = optional_number(
            &query.skip,
            "app.mongo.query.skip_placeholder",
            window,
            cx,
        );
        let max_time_ms = optional_number(
            &query.max_time_ms,
            "app.mongo.query.max_time_placeholder",
            window,
            cx,
        );
        // Filter 会随内容增行，但输入框是独立实体：打字只重绘编辑器自己，面板不会
        // 跟着重算高度。这里观察它的变化，只在**行数跨档**时通知面板；逐键都通知会把
        // 整个主视图（侧栏树、标签条、文档列表）多排一帧，正是「打开条件构建后掉帧」的来源。
        //
        // 观察而不是订阅 `InputEvent::Change`：条件构建器写回 JSON 走 `set_value`，
        // 它不发 Change 事件，但会 notify。
        let filter_rows = Rc::new(Cell::new(mongo_query_filter_rows(&filter.read(cx).value())));
        let observed_rows = filter_rows.clone();
        let filter_rows_subscription = cx.observe_in(&filter, window, move |_, filter, _, cx| {
            let rows = mongo_query_filter_rows(&filter.read(cx).value());
            if observed_rows.replace(rows) != rows {
                cx.notify();
            }
        });
        let entity = cx.new(|_| MongoQueryInputs {
            filter,
            _filter_rows_subscription: filter_rows_subscription,
            projection,
            sort,
            collation,
            hint,
            skip,
            limit,
            max_time_ms,
            aggregation,
        });
        self.mongo_query_inputs.insert(tab_id, entity.clone());
        entity
    }

    /// 切换分节；索引/集合信息按需懒加载（各自只加载一次，重试走错误块）。
    pub(crate) fn select_mongo_section(
        &mut self,
        tab_id: TabId,
        section: MongoSection,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(AppCommand::SelectMongoSection { tab_id, section }, cx);
        let (load_info, load_indexes) = {
            let Some(state) = mongo_collection_state(self.controller.state(), tab_id) else {
                return;
            };
            (
                section == MongoSection::Info && matches!(state.info, LoadState::NotLoaded),
                section == MongoSection::Indexes && matches!(state.indexes, LoadState::NotLoaded),
            )
        };
        if load_info {
            self.load_mongo_collection_info(tab_id, cx);
        }
        if load_indexes {
            self.load_mongo_indexes(tab_id, cx);
        }
    }

    pub(crate) fn select_mongo_display(
        &mut self,
        tab_id: TabId,
        mode: MongoDisplayMode,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(AppCommand::SelectMongoDisplayMode { tab_id, mode }, cx);
        // 文档正文在「字段列表 / 整篇 JSON」之间切换后卡片高度完全不同，
        // 虚拟列表沿用旧高度会把下方卡片排错，必须重新测量。
        self.remeasure_mongo_cards(tab_id);
    }

    /// 打开/关闭文档详情抽屉；`None` 关闭。
    pub(crate) fn select_mongo_document(
        &mut self,
        tab_id: TabId,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(AppCommand::SelectMongoDocument { tab_id, index }, cx);
    }

    /// 翻页：把新起点写进面板的「起始位置」再按当前条件重查。
    ///
    /// 起点只有面板这一份：分页器与输入框各存一份的话，用户手填过 skip 之后
    /// 再翻页就会互相覆盖。
    pub(crate) fn set_mongo_offset(
        &mut self,
        tab_id: TabId,
        offset: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(AppCommand::SetMongoOffset { tab_id, offset }, cx);
        if let Some(skip) = self
            .mongo_query_inputs
            .get(&tab_id)
            .map(|inputs| inputs.read(cx).skip.clone())
        {
            skip.update(cx, |input, cx| input.set_value(offset.to_string(), window, cx));
        }
        self.run_mongo_query(tab_id, cx);
    }

    /// 重置查询面板为默认查询（`{}`、无投影/排序、当前页大小）并立即重查。
    pub(crate) fn reset_mongo_query(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(inputs) = self.mongo_query_inputs.get(&tab_id).cloned() else {
            return;
        };
        let limit = mongo_collection_state(self.controller.state(), tab_id)
            .map(|state| state.pagination.limit)
            .unwrap_or_else(|| mongo_query_limit(self.controller.state().settings.page_size));
        let draft = MongoQueryDraft::new(limit);
        let (editors, numbers) = {
            let inputs = inputs.read(cx);
            (
                [(inputs.filter.clone(), draft.filter.clone())],
                [
                    (inputs.projection.clone(), draft.projection.clone()),
                    (inputs.sort.clone(), draft.sort.clone()),
                    (inputs.collation.clone(), draft.collation.clone()),
                    (inputs.hint.clone(), draft.hint.clone()),
                    (inputs.skip.clone(), draft.skip.clone()),
                    (inputs.limit.clone(), draft.limit.clone()),
                    (inputs.max_time_ms.clone(), draft.max_time_ms.clone()),
                ],
            )
        };
        // 多行编辑器与单行输入框是两种状态类型，分开写。
        for (input, value) in editors {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        for (input, value) in numbers {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.run_mongo_query(tab_id, cx);
    }

    /// 错误块重试：按出错区域只重跑对应命令。
    pub(crate) fn retry_mongo_load(
        &mut self,
        tab_id: TabId,
        retry: MongoRetry,
        cx: &mut Context<Self>,
    ) {
        match retry {
            MongoRetry::Documents => self.run_mongo_query(tab_id, cx),
            MongoRetry::Aggregation => self.run_mongo_aggregation(tab_id, cx),
            MongoRetry::Info => self.load_mongo_collection_info(tab_id, cx),
            MongoRetry::Indexes => self.load_mongo_indexes(tab_id, cx),
        }
    }

    /// 展开/收起首层「更多选项」（投影 / 排序 / 返回条数）。
    ///
    /// 首次点击前的默认值由「已生效查询是否填过任一附加项」推得，因此这里先落一次当前可见态，
    /// 再翻转：否则用户点一下会看到「没反应」（默认已展开 → 写入 true → 再展开一次）。
    pub(crate) fn toggle_mongo_query_options(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let current = self
            .mongo_query_options_expanded
            .get(&tab_id)
            .copied()
            .unwrap_or_else(|| {
                mongo_collection_state(self.controller.state(), tab_id)
                    .is_some_and(|state| mongo_has_any_options(&state.applied_query))
            });
        self.mongo_query_options_expanded.insert(tab_id, !current);
        // 收起点「更多选项」时把第二层一起收起：否则下次展开常用项，
        // 低频参数会跟着一并弹出来，用户以为面板记住了收起状态。
        if !self.mongo_query_options_expanded[&tab_id] {
            self.mongo_query_advanced_expanded.insert(tab_id, false);
            // 条件构建的入口也在这张网格里：收起网格，构建器跟着收起，
            // 否则入口不见了、面板还留着，找不回关闭按钮。
            self.mongo_query_builder_expanded.insert(tab_id, false);
        }
        cx.notify();
    }

    /// 展开/收起「高级查询」（第二层：排序规则 / 起始 / 索引提示 / 执行上限）。
    pub(crate) fn toggle_mongo_query_advanced(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let current = self
            .mongo_query_advanced_expanded
            .get(&tab_id)
            .copied()
            .unwrap_or_else(|| {
                mongo_collection_state(self.controller.state(), tab_id)
                    .is_some_and(|state| mongo_query_has_advanced(&state.applied_query))
            });
        self.mongo_query_advanced_expanded.insert(tab_id, !current);
        cx.notify();
    }

    /// 关闭标签页时释放输入框实体。
    pub(crate) fn close_mongo_tab_state(&mut self, tab_id: TabId) {
        self.mongo_query_inputs.remove(&tab_id);
        self.mongo_query_options_expanded.remove(&tab_id);
        self.mongo_query_advanced_expanded.remove(&tab_id);
        self.mongo_query_builder_expanded.remove(&tab_id);
        self.mongo_query_builders.remove(&tab_id);
        self.mongo_collapsed_documents.remove(&tab_id);
        self.mongo_expanded_fields.remove(&tab_id);
        self.mongo_json_fold_toggles.remove(&tab_id);
        self.mongo_document_cache.remove(&tab_id);
        self._mongo_load_tasks.remove(&tab_id.0);
        self._mongo_aggregation_tasks.remove(&tab_id.0);
        self._mongo_info_tasks.remove(&tab_id.0);
        self._mongo_index_tasks.remove(&tab_id.0);
        self._mongo_field_sample_tasks.remove(&tab_id.0);
        // 编辑抽屉是纯界面会话：关标签即丢弃草稿，不留悬挂的编辑器实体。
        if self
            .mongo_document_editor
            .as_ref()
            .is_some_and(|editor| editor.tab_id == tab_id)
        {
            self.mongo_document_editor = None;
            self._mongo_document_editor_task = None;
            self._mongo_document_insert_task = None;
        }
    }
}

/// 已生效的查询是否填过「更多选项」里的常用项（投影 / 排序）。
///
/// 不含条数：条数有默认值，用它判断会让面板永远默认展开。
fn mongo_query_has_options(query: &MongoQueryDraft) -> bool {
    !query.projection.trim().is_empty() || !query.sort.trim().is_empty()
}

/// 已生效的查询是否填过「高级查询」里的低频参数。
fn mongo_query_has_advanced(query: &MongoQueryDraft) -> bool {
    !query.collation.trim().is_empty()
        || !query.hint.trim().is_empty()
        || !query.max_time_ms.trim().is_empty()
        || query.skip_value() > 0
}

/// 任一层填过附加项：决定「更多选项」按钮的角标与首层展开。
fn mongo_has_any_options(query: &MongoQueryDraft) -> bool {
    mongo_query_has_options(query) || mongo_query_has_advanced(query)
}

/// 已生效查询里两层各填过几项，用作折叠态角标（首层常用项 / 第二层低频参数）。
///
/// 收起时用户看不到这些格子，角标是唯一提示——条件还在生效，不能悄无声息地藏起来。
pub(crate) fn mongo_query_option_counts(query: &MongoQueryDraft) -> (usize, usize) {
    let count = |filled: [bool; 2]| filled.iter().filter(|filled| **filled).count();
    let common = count([
        !query.projection.trim().is_empty(),
        !query.sort.trim().is_empty(),
    ]);
    let advanced = count([
        !query.collation.trim().is_empty(),
        !query.hint.trim().is_empty(),
    ]) + usize::from(!query.max_time_ms.trim().is_empty()) + usize::from(query.skip_value() > 0);
    (common, advanced)
}

/// 输入框初值：已执行查询 + 聚合管道；无标签页状态时给默认值。
fn mongo_query_seed(state: &AppState, tab_id: TabId) -> (MongoQueryDraft, String) {
    state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            TabKind::MongoCollection(state) => Some((
                state.applied_query.clone(),
                state.aggregation_pipeline.clone(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| {
            (
                MongoQueryDraft::new(state.settings.page_size),
                MongoCollectionState::default_aggregation_pipeline(),
            )
        })
}
