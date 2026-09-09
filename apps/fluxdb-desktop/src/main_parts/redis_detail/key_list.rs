
/// Folder 模式每批加载的 Key 数上限。单批先拉 10000 个 key 建前缀树，
/// 后续 Scan more 允许继续累加，不会被后端分页上限卡死。分隔符「口子」仍走
/// `RedisKeyDelimiter`（默认 `:`），UI 不硬编码。
const REDIS_KEY_FOLDER_BATCH: u64 = 10_000;
/// 平铺模式「滚动触底懒加载」每批增量（也是平铺切换时的初始分页上限）。
const REDIS_KEY_FLAT_LAZY_STEP: u64 = 200;

/// 计算当前模式下再累加一批后的新分页上限：平铺每批 +200，Folder 每批 +10000。
/// 纯函数便于单测；`current` 为 0（首次触发）时回落基础增量，保证至少前进一个批次。
fn redis_key_list_next_batch(mode: RedisKeyListMode, current: u64) -> u64 {
    let step = if mode.is_folder() {
        REDIS_KEY_FOLDER_BATCH
    } else {
        REDIS_KEY_FLAT_LAZY_STEP
    };
    current.saturating_add(step).max(step)
}

/// 收集「当前渲染可见行」里缺元信息且未在加载中的键名。
/// 平铺模式可见行 = 全部页行；Folder 模式可见行 = 展开集合拍平后的叶子行（与渲染一致）。
/// 已加载（`loaded`）与请求中（`pending`）的键都跳过，确保只对首次可见的行补元信息、不重复请求。
fn redis_visible_missing_metadata_keys(
    page: &DataPage,
    mode: RedisKeyListMode,
    expanded: BTreeSet<String>,
    pending: BTreeSet<String>,
) -> Vec<String> {
    let loaded: BTreeSet<String> = page
        .rows
        .iter()
        .filter(|row| {
            redis_row_text(page, row, "类型").is_some_and(|kind| !kind.is_empty())
        })
        .filter_map(|row| redis_row_text(page, row, "键"))
        .collect();
    let mut visible: Vec<String> = Vec::new();
    if mode.is_folder() {
        let entries: Vec<(String, usize)> = page
            .rows
            .iter()
            .enumerate()
            .filter_map(|(source_row, row)| {
                redis_row_text(page, row, "键").map(|key| (key, source_row))
            })
            .collect();
        let tree = redis_key_build_tree(&entries, RedisKeyDelimiter::default());
        let mut flat = Vec::new();
        redis_key_flatten(&tree, &expanded, 0, &mut flat);
        for row in flat {
            if row.kind == RedisKeyRowKind::Leaf {
                visible.push(row.key);
            }
        }
    } else {
        for row in &page.rows {
            if let Some(key) = redis_row_text(page, row, "键") {
                visible.push(key);
            }
        }
    }
    visible
        .into_iter()
        .filter(|key| !loaded.contains(key) && !pending.contains(key))
        .collect()
}
impl NavicatMain {
    /// 当前 tab 的 Redis Key 列表展示模式；未设置时默认平铺，保持既有行为。
    fn redis_key_list_mode(&self, tab_id: TabId) -> RedisKeyListMode {
        self.redis_key_list_modes
            .get(&tab_id)
            .copied()
            .unwrap_or(RedisKeyListMode::Flat)
    }

    /// 切换展示模式时先清空当前页并重置本 tab 的树态，再按模式重新拉首批。
    /// RedisInsight 也是切换视图就重置列表状态，而不是保留上一种视图的展开结果。
    fn set_redis_key_list_mode(
        &mut self,
        tab_id: TabId,
        mode: RedisKeyListMode,
        cx: &mut Context<Self>,
    ) {
        self.redis_key_list_modes.insert(tab_id, mode);
        self.reload_redis_key_list_for_mode(tab_id, cx);
    }

    fn redis_key_list_uniform_scroll_handle(&mut self, tab_id: TabId) -> UniformListScrollHandle {
        self.redis_key_list_uniform_scroll
            .entry(tab_id)
            .or_insert_with(UniformListScrollHandle::new)
            .clone()
    }

    fn redis_key_folder_visible_rows(
        &mut self,
        tab_id: TabId,
        page: &DataPage,
    ) -> Rc<Vec<RedisKeyVisibleRow>> {
        if let Some(visible) = self.redis_key_folder_visible_cache.get(&tab_id) {
            return visible.clone();
        }

        let entries: Vec<(String, usize)> = page
            .rows
            .iter()
            .enumerate()
            .filter_map(|(source_row, row)| {
                redis_row_text(page, row, "键").map(|key| (key, source_row))
            })
            .collect();
        let tree = redis_key_build_tree(&entries, RedisKeyDelimiter::default());
        let expanded = self.redis_key_list_expanded(tab_id);
        let mut visible = Vec::new();
        redis_key_flatten(&tree, &expanded, 0, &mut visible);
        let visible = Rc::new(visible);
        self.redis_key_folder_visible_cache
            .insert(tab_id, visible.clone());
        visible
    }

    /// 当前 tab 状态栏展示的「已扫描」键数（即该 tab 当前已请求加载的上限）。
    fn redis_key_list_loaded(&self, tab_id: TabId) -> u64 {
        self.redis_key_list_loaded
            .get(&tab_id)
            .copied()
            .unwrap_or(0)
    }

    fn set_redis_key_list_loaded(&mut self, tab_id: TabId, n: u64) {
        self.redis_key_list_loaded.insert(tab_id, n);
    }

    /// 切换 Redis Key 列表视图前，把上一个视图的展开、选中、hover、滚动和待补元信息清掉。
    fn reset_redis_key_list_view_state(&mut self, tab_id: TabId) {
        self.redis_key_list_expanded.remove(&tab_id);
        self.redis_key_list_selected_leaf.remove(&tab_id);
        self.redis_key_list_folder_hovered.remove(&tab_id);
        self.redis_key_metadata_pending.remove(&tab_id);
        self.redis_key_folder_visible_cache.remove(&tab_id);
        if self.redis_key_list_hovered_tab == Some(tab_id) {
            self.redis_key_list_hovered_tab = None;
        }
        self.redis_key_list_uniform_scroll.remove(&tab_id);
    }

    /// 该 tab 元信息补全「请求中」的键名集合，用于并发去重。
    fn redis_key_metadata_pending(&self, tab_id: TabId) -> BTreeSet<String> {
        self.redis_key_metadata_pending
            .get(&tab_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 惰性补全驱动：对当前渲染可见行里「缺元信息且不在请求中」的键批量拉取元信息。
    /// 由页面加载 / 元信息合并 / folder 展开收起后触发；补齐到位、无可补项时自然终止，
    /// 行内已有元信息的键跳过，进行中的键通过 `redis_key_metadata_pending` 去重。
    fn ensure_redis_key_metadata(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        // 只服务 Redis DB 键列表页（不是单键详情页）。
        let is_redis_db_list = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .is_some_and(|tab| {
                matches!(&tab.kind, TabKind::DataEditor(editor) if editor.object.kind == ObjectKind::RedisDb)
            });
        if !is_redis_db_list {
            return;
        }
        let Some(page) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.page.clone(),
                _ => None,
            })
        else {
            return;
        };
        let missing = redis_visible_missing_metadata_keys(
            &page,
            self.redis_key_list_mode(tab_id),
            self.redis_key_list_expanded(tab_id),
            self.redis_key_metadata_pending(tab_id),
        );
        if missing.is_empty() {
            return;
        }
        self.redis_key_metadata_pending
            .entry(tab_id)
            .or_default()
            .extend(missing.iter().cloned());

        let mut controller = self.controller.clone();
        let keys = missing;
        let requested_keys = keys.clone();
        cx.spawn(async move |view, cx| {
            let (requested_keys, result) = cx
                .background_spawn(async move {
                    let result = match controller.dispatch(AppCommand::LoadRedisKeyMetadata {
                        tab_id,
                        keys,
                    }) {
                        AppEvent::RedisKeyMetadataLoaded { .. } => Ok(controller),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "元信息加载失败".to_string(),
                            message: "Redis Key 元信息加载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    };
                    (requested_keys, result)
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if let Some(pending) = this.redis_key_metadata_pending.get_mut(&tab_id) {
                        for key in &requested_keys {
                            pending.remove(key);
                        }
                        if pending.is_empty() {
                            this.redis_key_metadata_pending.remove(&tab_id);
                        }
                    }
                    match result {
                        Ok(loaded) => {
                            // 后台请求运行在控制器克隆上，必须把结果按键合并回 live controller。
                            this.controller.merge_redis_key_metadata_from(
                                &loaded,
                                tab_id,
                                &requested_keys,
                            );
                            // 重建表格委托把 type/value/size/TTL 上屏，只刷新当前页。
                            this.refresh_active_data_table(tab_id, cx);
                            this.ensure_redis_key_metadata(tab_id, cx);
                        }
                        Err(error) => {
                            // 失败后保留为空，下次刷新或重新进入时重试，避免异步自旋请求。
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                    }
                });
            });
        })
        .detach();
    }

    /// 滚动触底 / Scan more 触发加载下一批：按当前模式在现有基础上累加一批，再以
    /// `offset = 0` + 更大的 `limit` 重新加载。连接器 SCAN 续扫语义会返回前 N 个键，
    /// 列表随之增长（平铺）或重建成更大的树（Folder）；不改连接器返回结构。
    ///
    /// 分隔符「口子」：本处只决定拉取的键数上限，不做任何 `:` 字符串处理，键树拆分全部
    /// 交由 `RedisKeyDelimiter`（默认 `:`）完成，UI 层不硬编码分隔符。
    fn redis_key_list_load_more(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let next = redis_key_list_next_batch(
            self.redis_key_list_mode(tab_id),
            self.redis_key_list_loaded(tab_id),
        );
        self.set_redis_key_list_loaded(tab_id, next);
        self.request_data_editor_pagination(tab_id, 0, next, cx);
        cx.notify();
    }

    /// 切换模式后按模式重新请求加载：Folder 直接拉满一整个 10000 批次建树，平铺回落默认分页。
    fn reload_redis_key_list_for_mode(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.controller.reset_data_page_for_reload(tab_id);
        self.reset_redis_key_list_view_state(tab_id);
        self.refresh_active_data_table(tab_id, cx);
        let mode = self.redis_key_list_mode(tab_id);
        let limit = if mode.is_folder() {
            REDIS_KEY_FOLDER_BATCH
        } else {
            REDIS_KEY_FLAT_LAZY_STEP
        };
        self.set_redis_key_list_loaded(tab_id, limit);
        self.request_data_editor_pagination(tab_id, 0, limit, cx);
    }

    fn redis_key_list_expanded(&self, tab_id: TabId) -> BTreeSet<String> {
        self.redis_key_list_expanded
            .get(&tab_id)
            .cloned()
            .unwrap_or_default()
    }

    /// folder 节点点击回调：只展开/收起，不打开详情。
    /// 展开/收起会改变可见叶子集合，因此顺便触发这些新暴露叶子的元信息惰性补全。
    fn toggle_redis_key_folder(&mut self, tab_id: TabId, prefix: String, cx: &mut Context<Self>) {
        let expanded = self.redis_key_list_expanded.entry(tab_id).or_default();
        if !expanded.remove(&prefix) {
            expanded.insert(prefix);
        }
        self.redis_key_folder_visible_cache.remove(&tab_id);
        self.ensure_redis_key_metadata(tab_id, cx);
        cx.notify();
    }

    fn redis_key_list_selected_leaf(&self, tab_id: TabId) -> Option<String> {
        self.redis_key_list_selected_leaf.get(&tab_id).cloned()
    }

    /// 点击叶子行：选中该叶子键（详情跟随叶子键；刷新/筛选/切换模式均按 key 反查，不跟随可见行下标）。
    fn set_redis_key_list_selected_leaf(
        &mut self,
        tab_id: TabId,
        key: String,
        cx: &mut Context<Self>,
    ) {
        self.redis_key_list_selected_leaf.insert(tab_id, key);
        cx.notify();
    }

    fn redis_key_list_folder_hovered(&self, tab_id: TabId) -> Option<usize> {
        self.redis_key_list_folder_hovered.get(&tab_id).copied()
    }

    fn set_redis_key_list_folder_hovered(&mut self, tab_id: TabId, source_row: Option<usize>) {
        match source_row {
            Some(row) => {
                self.redis_key_list_folder_hovered.insert(tab_id, row);
            }
            None => {
                self.redis_key_list_folder_hovered.remove(&tab_id);
            }
        }
    }
}

/// 左侧 Key 列表。平铺使用 GPUI 虚拟列表，只布局视口附近的固定高度行；Folder 保留树形列表。
fn redis_key_list(
    tab_id: TabId,
    page: &DataPage,
    table_state: &Entity<TableState<DataPageTableDelegate>>,
    refreshed_at: Option<Instant>,
    refreshing: bool,
    colors: UiColors,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let folder_mode = this.redis_key_list_mode(tab_id).is_folder();
    let mut list = div()
        .id(("redis-key-list", tab_id.0))
        .flex_1()
        .min_h(px(0.))
        .relative()
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.redis_key_list_hovered_tab = (*hovered).then_some(tab_id);
            cx.notify();
        }))
        .flex()
        .flex_col()
        .bg(colors.panel_bg);

    if page.rows.is_empty() {
        return list.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("暂无 Redis Key"),
        )
        .when(refreshing, |this| this.child(redis_key_list_loading_overlay(colors)));
    }

    if folder_mode {
        let visible = this.redis_key_folder_visible_rows(tab_id, page);
        let scroll = this.redis_key_list_uniform_scroll_handle(tab_id);
        let base_scroll = scroll.0.borrow().base_handle.clone();
        let page = page.clone();
        let item_count = visible.len();
        let folder_rows = uniform_list(
            ("redis-key-list-folder", tab_id.0),
            item_count,
            cx.processor(move |this, visible_range: std::ops::Range<usize>, _, cx| {
                visible_range
                    .map(|visible_ix| match visible[visible_ix].kind {
                        RedisKeyRowKind::Folder => {
                            redis_key_list_folder_row(tab_id, &visible[visible_ix], colors, this, cx)
                                .into_any_element()
                        }
                        RedisKeyRowKind::Leaf => redis_key_list_folder_leaf_row(
                            tab_id,
                            &page,
                            &visible[visible_ix],
                            visible_ix,
                            refreshed_at,
                            colors,
                            this,
                            cx,
                        )
                        .into_any_element(),
                    })
                    .collect()
            }),
        )
        .track_scroll(&scroll)
        .size_full();
        list = list
            .overflow_hidden()
            .child(folder_rows)
            .vertical_scrollbar(&base_scroll);
    } else {
        let scroll = this.redis_key_list_uniform_scroll_handle(tab_id);
        let base_scroll = scroll.0.borrow().base_handle.clone();
        let item_count = page.rows.len();
        let page = page.clone();
        let table_state = table_state.clone();
        let flat_rows = uniform_list(
            ("redis-key-list-flat", tab_id.0),
            item_count,
            cx.processor(move |this, visible_range: std::ops::Range<usize>, window, cx| {
                if visible_range.end.saturating_add(4) >= item_count
                    && page.has_more
                    && !this._data_load_tasks.contains_key(&tab_id.0)
                {
                    cx.defer_in(window, move |this, _, cx| {
                        if !this._data_load_tasks.contains_key(&tab_id.0) {
                            this.redis_key_list_load_more(tab_id, cx);
                        }
                    });
                }

                visible_range
                    .map(|row_ix| {
                        redis_key_list_row(
                            tab_id,
                            &page,
                            &page.rows[row_ix],
                            row_ix,
                            &table_state,
                            refreshed_at,
                            colors,
                            this,
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(&scroll)
        .size_full();
        list = list
            .overflow_hidden()
            .child(flat_rows)
            .vertical_scrollbar(&base_scroll);
    }
    list.when(refreshing, |this| this.child(redis_key_list_loading_overlay(colors)))
}

/// Folder 行的行底色：按缩进深度淡化 + 选中高亮（与平铺行的斑马纹解耦，folder 视图更清爽）。
fn redis_key_tree_row_bg(colors: UiColors, selected: bool, depth: usize) -> Hsla {
    if selected {
        return if colors.is_dark {
            rgb(0x003d52).into()
        } else {
            rgb(0xe8f1ff).into()
        };
    }
    if colors.is_dark {
        hsla(216. / 360., 0.16, 0.12 + depth as f32 * 0.008, 1.)
    } else {
        hsla(210. / 360., 0.20, 0.99 - depth as f32 * 0.008, 1.)
    }
}

fn redis_key_tree_chevron(expanded: bool, colors: UiColors) -> Div {
    div()
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(
            if expanded {
                AppIcon::ChevronDown
            } else {
                AppIcon::ChevronRight
            },
            14.,
            colors.muted,
        ))
}

fn redis_key_tree_spacer(width: f32) -> Div {
    div().w(px(width)).flex_none()
}

/// Folder 节点行：展开箭头 + folder 图标 + 前缀文本 + 叶子数。点击只展开/收起，不打开详情。
fn redis_key_list_folder_row(
    tab_id: TabId,
    row: &RedisKeyVisibleRow,
    colors: UiColors,
    this: &NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let expanded = this.redis_key_list_expanded(tab_id).contains(&row.prefix);
    let prefix = row.prefix.clone();
    let depth = row.depth as f32;
    let hover_bg: Hsla = if colors.is_dark {
        rgb(0x242a32).into()
    } else {
        rgb(0xf1f5fb).into()
    };
    div()
        .id(SharedString::from(format!(
            "redis-key-folder-{}-{}",
            tab_id.0, prefix
        )))
        .h(px(44.))
        .w_full()
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .gap_1()
        .bg(redis_key_tree_row_bg(colors, false, row.depth))
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                // folder 节点只负责展开/收起，不打开详情。
                this.toggle_redis_key_folder(tab_id, prefix.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .text_color(colors.text)
        .child(redis_key_tree_chevron(expanded, colors))
        .child(redis_key_tree_spacer(depth * 16.))
        .child(app_icon(AppIcon::Folder, 15., colors.muted))
        .child(
            redis_key_list_text(redis_ellipsis_text(&row.label, 26), true)
                .flex_1()
                .min_w(px(0.)),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(format!("{}", row.count)),
        )
}

/// Folder 模式叶子行：保持类型 tag、key、刷新时间、大小和删除按钮，按叶子键选中。
fn redis_key_list_folder_leaf_row(
    tab_id: TabId,
    page: &DataPage,
    row: &RedisKeyVisibleRow,
    _visible_ix: usize,
    refreshed_at: Option<Instant>,
    colors: UiColors,
    this: &NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let key = &row.key;
    let label = &row.label;
    let source_row = row.source_row;
    let kind = redis_row_text(page, &page.rows[source_row], "类型").unwrap_or_default();
    let size = redis_row_text(page, &page.rows[source_row], "大小").unwrap_or_default();
    let selected = this.redis_key_list_selected_leaf(tab_id).as_deref() == Some(key.as_str());
    let hovered = this.redis_key_list_folder_hovered(tab_id) == Some(source_row);
    let depth = row.depth as f32;
    let hover_bg: Hsla = if colors.is_dark {
        rgb(0x242a32).into()
    } else {
        rgb(0xf1f5fb).into()
    };
    let key_for_click = key.clone();

    div()
        .id(SharedString::from(format!(
            "redis-key-folder-leaf-{}-{}",
            tab_id.0, key
        )))
        .h(px(44.))
        .w_full()
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .bg(redis_key_tree_row_bg(colors, selected, row.depth))
        .when(!selected, |this| this.hover(move |style| style.bg(hover_bg)))
        .cursor_pointer()
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            // 浮出删除按钮：folder 视图用叶子原始行下标记录 hover（与表格行下标解耦）。
            this.set_redis_key_list_folder_hovered(tab_id, Some(source_row));
            cx.notify();
        }))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            if !*hovered {
                this.set_redis_key_list_folder_hovered(tab_id, None);
                cx.notify();
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.set_redis_key_list_selected_leaf(tab_id, key_for_click.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .text_color(colors.text)
        .child(redis_key_tree_spacer(16. + depth * 16.))
        .child(
            div()
                .w(px(74.))
                .flex_none()
                .child(redis_list_type_tag(kind, colors)),
        )
        .child(redis_key_list_text(redis_ellipsis_text(label, 26), true).flex_1().min_w(px(0.)))
        .child(redis_key_list_text(redis_key_refresh_time_text(refreshed_at), false).w(px(82.)))
        .child(
            redis_key_list_text(size, false)
                .w(px(62.))
                .text_align(gpui::TextAlign::Right),
        )
        .child(redis_key_list_delete_button(
            tab_id, source_row, key.clone(), hovered, colors, this, cx,
        ))
}

fn redis_key_list_loading_overlay(colors: UiColors) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(if colors.is_dark {
            hsla(216. / 360., 0.16, 0.10, 0.76)
        } else {
            hsla(210. / 360., 0.20, 0.98, 0.76)
        })
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(loading_spinner_with_color(18., colors.muted))
        .child("正在刷新...")
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
}

fn redis_key_detail_loading_overlay(colors: UiColors) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .rounded(colors.radius_lg)
        .bg(if colors.is_dark {
            hsla(216. / 360., 0.16, 0.10, 0.70)
        } else {
            hsla(210. / 360., 0.20, 0.98, 0.70)
        })
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(loading_spinner_with_color(18., colors.muted))
        .child("正在刷新...")
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
}

fn redis_key_list_row(
    tab_id: TabId,
    page: &DataPage,
    row: &Row,
    row_ix: usize,
    table_state: &Entity<TableState<DataPageTableDelegate>>,
    refreshed_at: Option<Instant>,
    colors: UiColors,
    this: &NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let key = redis_row_text(page, row, "键").unwrap_or_default();
    let kind = redis_row_text(page, row, "类型").unwrap_or_default();
    let size = redis_row_text(page, row, "大小").unwrap_or_default();
    let (source_row, selected, hovered) = {
        let table = table_state.read(cx);
        let delegate = table.delegate();
        (
            delegate
                .source_row_indexes
                .get(row_ix)
                .copied()
                .unwrap_or(row_ix),
            delegate.has_row_selected(row_ix) || delegate.row_has_selected_cell(row_ix),
            delegate.hovered_cell.is_some_and(|(row, _)| row == row_ix),
        )
    };
    let row_bg = if selected {
        if colors.is_dark {
            rgb(0x003d52).into()
        } else {
            rgb(0xe8f1ff).into()
        }
    } else if row_ix % 2 == 1 {
        if colors.is_dark {
            rgb(0x202020).into()
        } else {
            rgb(0xf7f9fc).into()
        }
    } else {
        colors.panel_bg
    };
    let hover_bg: Hsla = if colors.is_dark {
        rgb(0x242a32).into()
    } else {
        rgb(0xf1f5fb).into()
    };
    let table_for_click = table_state.clone();
    let table_for_hover = table_state.clone();
    let table_for_leave = table_state.clone();

    div()
        .id(("redis-key-row", row_ix))
        .h(px(44.))
        .w_full()
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .bg(row_bg)
        .when(!selected, |this| this.hover(move |style| style.bg(hover_bg)))
        .cursor_pointer()
        .on_mouse_move(cx.listener(move |_, _, _, cx| {
            table_for_hover.update(cx, |table, cx| {
                if table.delegate().hovered_cell != Some((row_ix, 0)) {
                    table.delegate_mut().hovered_cell = Some((row_ix, 0));
                    table.refresh(cx);
                }
            });
            cx.notify();
        }))
        .on_hover(cx.listener(move |_, hovered: &bool, _, cx| {
            if !*hovered {
                table_for_leave.update(cx, |table, cx| {
                    if table.delegate().hovered_cell == Some((row_ix, 0)) {
                        table.delegate_mut().hovered_cell = None;
                        table.refresh(cx);
                    }
                });
                cx.notify();
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, event: &MouseDownEvent, _, cx| {
                table_for_click.update(cx, |table, cx| {
                    let delegate = table.delegate_mut();
                    if event.modifiers.shift {
                        delegate.select_row_range_for_click(row_ix);
                    } else {
                        delegate.select_row_for_click(row_ix, event.modifiers.secondary());
                    }
                    table.refresh(cx);
                });
                cx.notify();
                cx.stop_propagation();
            }),
        )
        .text_color(colors.text)
        .child(
            div()
                .w(px(74.))
                .flex_none()
                .child(redis_list_type_tag(kind, colors)),
        )
        .child(redis_key_list_text(redis_ellipsis_text(&key, 30), true).flex_1().min_w(px(0.)))
        .child(redis_key_list_text(redis_key_refresh_time_text(refreshed_at), false).w(px(82.)))
        .child(
            redis_key_list_text(size, false)
                .w(px(62.))
                .text_align(gpui::TextAlign::Right),
        )
        .child(redis_key_list_delete_button(
            tab_id, source_row, key, hovered, colors, this, cx,
        ))
}

fn redis_list_type_tag(kind: String, colors: UiColors) -> Div {
    let (text, bg, border) = redis_type_tag_colors(kind.as_str(), colors.is_dark);
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(text)
        .child(kind.to_ascii_uppercase())
}

fn redis_key_list_text(text: String, mono: bool) -> Div {
    div()
        .w_full()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .when(mono, |this| this.font_family("Menlo"))
        .child(text)
}

fn redis_key_refresh_time_text(refreshed_at: Option<Instant>) -> String {
    redis_key_refresh_label(refreshed_at)
        .strip_prefix("上次刷新: ")
        .unwrap_or("未刷新")
        .to_string()
}

fn redis_key_list_delete_button(
    tab_id: TabId,
    source_row: usize,
    key: String,
    visible: bool,
    colors: UiColors,
    this: &NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let view = cx.entity().downgrade();
    let pending = this
        .pending_redis_key_delete
        .as_ref()
        .is_some_and(|pending| pending.tab_id == tab_id && pending.source_row == source_row);
    let shown = visible || pending;
    div()
        .id(SharedString::from(format!(
            "redis-key-delete-{}-{}",
            tab_id.0, source_row
        )))
        .h(px(28.))
        .w(px(if shown { 28. } else { 0. }))
        .flex_none()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .relative()
        .opacity(if shown { 1.0 } else { 0.0 })
        .when(shown, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(|window, cx| Tooltip::new("删除").build(window, cx))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let key = key.clone();
                        move |this, _, _, cx| {
                        this.pending_redis_key_delete = Some(RedisKeyDeleteConfirm {
                            tab_id,
                            source_row,
                            key: key.clone(),
                        });
                        cx.notify();
                        cx.stop_propagation();
                        }
                    }),
                )
        })
        // 隐藏态按钮宽度为 0，不能继续挂载 SVG，否则 GPUI 会尝试以零尺寸绘制图标。
        .when(shown, |this| {
            this.child(app_icon(AppIcon::Trash, 15., rgb(0xe5484d)))
        })
        .when(pending, |this| {
            this.child(redis_key_delete_confirm_popover(
                tab_id, source_row, key, view, colors, cx,
            ))
        })
        .with_animation(
            ("redis-key-delete-slide", source_row),
            Animation::new(Duration::from_secs_f64(0.36)).with_easing(ease_in_out),
            move |this, delta| {
                if shown {
                    let progress = delta * delta * delta * (delta * (delta * 6. - 15.) + 10.);
                    this.w(px(28. * progress)).opacity(progress)
                } else {
                    this.w(px(0.)).opacity(0.0)
                }
            },
        )
}

fn redis_key_delete_confirm_popover(
    tab_id: TabId,
    source_row: usize,
    key: String,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .absolute()
        .right(px(34.))
        .top(px(-74.))
        .size(px(1.))
        .child(
            deferred(
                anchored().anchor(Anchor::TopRight).child(
                    div()
                        .occlude()
                        .w(px(238.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .shadow_lg()
                        .bg(colors.panel_bg)
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .on_mouse_down_out(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.pending_redis_key_delete = None;
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .text_size(px(14.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(key.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(colors.muted)
                                        .child("将被删除，此操作不可撤销。"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(redis_key_delete_confirm_button("取消", false, colors).on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.pending_redis_key_delete = None;
                                        cx.notify();
                                        cx.stop_propagation();
                                    }),
                                ))
                                .child(redis_key_delete_confirm_button("确认删除", true, colors).on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.pending_redis_key_delete = None;
                                        this.dispatch(
                                            AppCommand::DeleteDataRow {
                                                tab_id,
                                                result_index: None,
                                                row: source_row,
                                            },
                                            cx,
                                        );
                                        this.refresh_active_data_table(tab_id, cx);
                                        cx.notify();
                                        cx.stop_propagation();
                                    }),
                                )),
                        ),
                ),
            )
            .with_priority(3),
        )
}

fn redis_key_delete_confirm_button(label: &'static str, danger: bool, colors: UiColors) -> Div {
    div()
        .h(px(28.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if danger { rgb(0xe5484d) } else { colors.border })
        .bg(if danger {
            if colors.is_dark {
                rgb(0x4a1f22)
            } else {
                rgb(0xffeeee)
            }
        } else {
            colors.panel_alt
        })
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if danger { rgb(0xe5484d) } else { colors.text })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .when(danger, |this| {
            this.child(app_icon(AppIcon::Trash, 13., rgb(0xe5484d)))
        })
        .child(label)
}

/// 左侧 Key 列表顶部状态栏：最左侧是展示模式双态 segmented control（纯图标，无文字），
/// 右侧展示结果数、已扫描数；Folder 模式还有更多数据时保留「Scan more」按钮。
/// 位于 `redis_split_view` 左侧包裹列顶部，是列表自身的状态栏，独立于主工具栏（搜索栏）。
fn redis_key_list_status_bar(
    tab_id: TabId,
    page: &DataPage,
    refreshing: bool,
    colors: UiColors,
    this: &NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mode = this.redis_key_list_mode(tab_id);
    let result_count = page.rows.len();
    let scanned = this.redis_key_list_loaded(tab_id);
    let has_more = page.has_more;
    div()
        .h(px(40.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .child(redis_key_mode_segmented(tab_id, mode, colors, cx))
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(format!("结果 {result_count} · 已扫描 {scanned}")),
        )
        .when(mode.is_folder() && has_more && !refreshing, |this| {
            this.child(
                div()
                    .id(("redis-key-scan-more", tab_id.0))
                    .h(px(26.))
                    .px_2()
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.input_bg)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.redis_key_list_load_more(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child("Scan more"),
            )
        })
}

#[cfg(test)]
mod redis_key_list_batch_tests {
    use super::*;

    #[test]
    fn refreshed_placeholder_rows_request_metadata_again() {
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "键".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "类型".to_string(),
                    type_name: None,
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![
                Row {
                    values: vec![
                        CellValue::Text("needs-refresh".to_string()),
                        CellValue::Text(String::new()),
                    ],
                },
                Row {
                    values: vec![
                        CellValue::Text("already-loaded".to_string()),
                        CellValue::Text("hash".to_string()),
                    ],
                },
            ],
            offset: 0,
            limit: 200,
            has_more: false,
        };

        assert_eq!(
            redis_visible_missing_metadata_keys(
                &page,
                RedisKeyListMode::Flat,
                BTreeSet::new(),
                BTreeSet::new(),
            ),
            vec!["needs-refresh".to_string()]
        );
    }

    /// Folder 模式每批累加 10000：10000 -> 20000，首次触发从 0 起步到 10000。
    #[test]
    fn folder_batch_advances_by_ten_thousand() {
        assert_eq!(redis_key_list_next_batch(RedisKeyListMode::Folder, 10_000), 20_000);
        assert_eq!(redis_key_list_next_batch(RedisKeyListMode::Folder, 0), 10_000);
    }

    /// 平铺模式每批累加 200 做滚动触底懒加载。
    #[test]
    fn flat_batch_advances_by_lazy_step() {
        assert_eq!(redis_key_list_next_batch(RedisKeyListMode::Flat, 0), 200);
        assert_eq!(redis_key_list_next_batch(RedisKeyListMode::Flat, 200), 400);
        assert_eq!(redis_key_list_next_batch(RedisKeyListMode::Flat, 100), 300);
    }

}
