const REDIS_LIST_TABLE_ROW_HEIGHT: f32 = 40.0;
// 值编辑浮层向下覆盖 2 行（总高 120px），容纳多行输入

const REDIS_LIST_VALUE_EDIT_EXTRA_ROWS: usize = 2;

/// 编辑浮层锚定行：编辑行向下覆盖 EXTRA_ROWS 行，接近表尾时钳制到末行，
/// 避免锚定到不存在的行导致浮层丢失。
fn redis_list_editor_anchor_row(editing: &RedisListItemEditingState, rows_len: usize) -> usize {
    editing
        .row_index
        .saturating_add(REDIS_LIST_VALUE_EDIT_EXTRA_ROWS)
        .min(rows_len.saturating_sub(1))
}

/// 值编辑浮层是否覆盖到指定行（编辑行及其下 EXTRA_ROWS 行）。
fn redis_list_value_covered(editing: &RedisListItemEditingState, row_ix: usize) -> bool {
    row_ix >= editing.row_index
        && row_ix <= editing.row_index + REDIS_LIST_VALUE_EDIT_EXTRA_ROWS
}

struct RedisListTableDelegate {
    view: WeakEntity<NavicatMain>,
    tab_id: Option<TabId>,
    key: String,
    rows: Vec<(usize, String)>,
    columns: Vec<TableColumn>,
    editing: Option<RedisListItemEditingState>,
    hovered: Option<RedisListItemEditingState>,
    value_edit_input: Option<Entity<InputState>>,
    applying: bool,
    search_loading: bool,
    colors: UiColors,
    table_width: Pixels,
}

impl RedisListTableDelegate {
    fn new(view: WeakEntity<NavicatMain>) -> Self {
        Self {
            view,
            tab_id: None,
            key: String::new(),
            rows: Vec::new(),
            columns: Vec::new(),
            editing: None,
            hovered: None,
            value_edit_input: None,
            applying: false,
            search_loading: false,
            colors: ui_colors(ThemeMode::Light),
            table_width: px(1000.),
        }
    }

    fn set_data(
        &mut self,
        tab_id: TabId,
        key: String,
        rows: Vec<(usize, String)>,
        editing: Option<RedisListItemEditingState>,
        hovered: Option<RedisListItemEditingState>,
        value_edit_input: Entity<InputState>,
        applying: bool,
        search_loading: bool,
        colors: UiColors,
    ) -> bool {
        // hover 指向已不存在的行时清空，避免越界悬空
        let hovered = hovered.filter(|h| h.row_index < rows.len());
        let changed = self.tab_id != Some(tab_id)
            || self.key != key
            || self.rows != rows
            || self.editing != editing
            || self.hovered != hovered
            || self.applying != applying
            || self.search_loading != search_loading;
        self.tab_id = Some(tab_id);
        self.key = key;
        self.rows = rows;
        self.editing = editing;
        self.hovered = hovered;
        self.value_edit_input = Some(value_edit_input);
        self.applying = applying;
        self.search_loading = search_loading;
        self.colors = colors;
        self.rebuild_columns();
        changed
    }

    fn rebuild_columns(&mut self) {
        let width = self.table_width;
        self.columns = vec![
            TableColumn::new("seq", "#")
                .width(redis_list_table_column_width("seq", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
            TableColumn::new("value", "Value")
                .width(redis_list_table_column_width("value", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
        ];
    }

    fn sync_redis_list_table_width(&mut self, table_width: Pixels) -> bool {
        if table_width <= px(0.)
            || (f32::from(self.table_width) - f32::from(table_width)).abs() < 1.
        {
            return false;
        }
        self.table_width = table_width;
        self.rebuild_columns();
        true
    }

    /// 值编辑浮层锚定在「最后被覆盖的行」上（而非编辑行本身），以负 top 向上回退覆盖编辑行。
    /// 锚定行在编辑行之后绘制，因此浮层整体位于被覆盖行之上；鼠标事件经 block_mouse_except_scroll
    /// 阻断穿透（滚轮仍可滚动表格），输入框与按钮可正常交互。
    fn render_redis_list_value_editor(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Div {
        let Some(input) = self.value_edit_input.clone() else {
            return div();
        };
        // 若当前行并无编辑目标（例如切换 key 后状态残留被过滤），直接返回空
        let Some(editing) = self.editing.clone().filter(|e| {
            e.row_index + REDIS_LIST_VALUE_EDIT_EXTRA_ROWS >= row_ix
                && e.row_index <= row_ix
        }) else {
            return div();
        };
        let row_height = px(REDIS_LIST_TABLE_ROW_HEIGHT);
        let editor_height = row_height * (REDIS_LIST_VALUE_EDIT_EXTRA_ROWS as f32 + 1.0);
        // 回退距离 = 锚定行与编辑行的行数差
        let extra_rows = row_ix.saturating_sub(editing.row_index) as f32;
        // 浮层左边缘 = 序号列宽，宽度 = Value 列宽
        let value_left = self.columns[0].width;
        let value_width = self.columns[1].width;
        let view = self.view.clone();
        let colors = self.colors;
        div()
            .absolute()
            .top(-(row_height * extra_rows))
            .left(value_left)
            .w(value_width)
            .h(editor_height)
            .child(redis_list_table_value_edit_cell(
                row_ix,
                input,
                colors,
                view,
                window,
                cx,
            ))
    }
}

impl TableDelegate for RedisListTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        self.columns[col_ix].clone()
    }

    fn loading(&self, _: &App) -> bool {
        self.search_loading && self.rows.is_empty()
    }

    /// 表头下边框，与数据区区分开
    fn render_header(&mut self, _: &mut Window, _: &mut Context<TableState<Self>>) -> Stateful<Div> {
        div()
            .id("redis-list-table-header")
            .border_b_1()
            .border_color(self.colors.border)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .px_3()
            .flex()
            .items_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(self.colors.muted)
            .child(self.columns[col_ix].name.clone())
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        // 编辑浮层占用的行（编辑行及其下方覆盖行）整段高亮、不画分隔线，
        // 避免横线穿过浮层；浮层锚定在「最后被覆盖的行」上，位于这些行之上。
        let in_edit_zone = self
            .editing
            .as_ref()
            .is_some_and(|editing| redis_list_value_covered(editing, row_ix));
        let editor_anchor_row = self.editing.as_ref().is_some_and(|editing| {
            row_ix == redis_list_editor_anchor_row(editing, self.rows.len())
        });
        let mut tr = div()
            .id(("redis-list-table-row", row_ix))
            .relative()
            .when(in_edit_zone, |this| this.bg(self.colors.hover).border_b_0())
            .when(!in_edit_zone, |this| {
                this.border_b_1().border_color(self.colors.border_soft)
            });
        if editor_anchor_row {
            tr = tr.child(self.render_redis_list_value_editor(row_ix, window, cx));
        }
        tr
    }

    fn render_empty(&mut self, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(self.colors.muted)
            .child("暂无条目")
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some((idx, value)) = self.rows.get(row_ix).cloned() else {
            return div()
                .id(("redis-list-table-empty-cell", row_ix))
                .size_full()
                .into_any_element();
        };
        match col_ix {
            0 => redis_list_table_seq_cell(row_ix, idx, self.colors).into_any_element(),
            _ => {
                // 编辑浮层覆盖区内的 Value 列留空，避免单元格内容绘制在浮层之上
                let covered = self
                    .editing
                    .as_ref()
                    .is_some_and(|editing| redis_list_value_covered(editing, row_ix));
                if covered {
                    div()
                        .id(("redis-list-table-value-blank", row_ix))
                        .size_full()
                        .into_any_element()
                } else {
                    redis_list_table_value_cell(row_ix, idx, value, self, cx).into_any_element()
                }
            }
        }
    }
}

fn redis_list_table_column_width(name: &str, table_width: Pixels) -> Pixels {
    // 权重：序号 12 / Value 88，权重和为 100，按可用宽度归一化
    const TOTAL_WEIGHT: f32 = 100.0;
    let weight = match name {
        "value" => 88.0,
        _ => 12.0,
    };
    px(f32::from(table_width) * weight / TOTAL_WEIGHT)
}

/// 序号单元格：显示该元素在列表中的下标。
fn redis_list_table_seq_cell(row_ix: usize, index: usize, colors: UiColors) -> impl IntoElement {
    div()
        .id(("redis-list-table-seq", row_ix))
        .size_full()
        .min_w(px(0.))
        .px_3()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family("Menlo")
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(format!("{:02}", index))
}

/// 值单元格：只读纯文本展示，超长截断时 hover 显示全文（对齐 Hash 字段值展示风格）。
fn redis_list_table_value_cell(
    row_ix: usize,
    _index: usize,
    value: String,
    delegate: &mut RedisListTableDelegate,
    cx: &mut Context<TableState<RedisListTableDelegate>>,
) -> impl IntoElement {
    let colors = delegate.colors;
    let tab_id = delegate.tab_id.unwrap_or(TabId(0));
    let key = delegate.key.clone();
    let target = RedisListItemEditingState {
        tab_id,
        key: key.clone(),
        row_index: row_ix,
    };
    let hovered = delegate.hovered.as_ref() == Some(&target);
    let applying = delegate.applying;
    let view = delegate.view.clone();

    let hover_view = view.clone();
    let click_view = view.clone();
    let hover_target = target.clone();
    let click_target = target;
    let tooltip = value.clone();
    div()
        .id(("redis-list-table-value", row_ix))
        .size_full()
        .min_w(px(0.))
        .px_3()
        .flex()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .overflow_hidden()
        .when(hovered, |this| this.bg(colors.hover))
        .on_hover(cx.listener(move |_, is_hovered: &bool, _, cx| {
            let _ = hover_view.clone().update(cx, |this, cx| {
                redis_list_table_set_navicat_hovered(this, hover_target.clone(), *is_hovered, cx);
            });
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, _, window, cx| {
                if !applying {
                    let _ = click_view.clone().update(cx, |this, cx| {
                        this.begin_redis_list_item_edit(click_target.clone(), window, cx);
                    });
                }
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id(("redis-list-table-value-text", row_ix))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family("Menlo")
                .text_size(px(13.))
                .text_color(colors.text)
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .child(value),
        )
        // hover 时在右侧显示编辑图标；点击整格进入编辑（LSET）
        .when(hovered && !applying, |this| {
            this.child(redis_hash_field_edit_icon(AppIcon::Edit, "编辑", colors))
        })
        .into_any_element()
}

/// 值编辑浮层单元格：多行输入在上、确认/取消按钮在下，交互与 Hash 值编辑一致。
fn redis_list_table_value_edit_cell(
    row_ix: usize,
    input: Entity<InputState>,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    window: &mut Window,
    cx: &mut Context<TableState<RedisListTableDelegate>>,
) -> impl IntoElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    // 焦点边框使用主题强调色（替代过重的黑色），未聚焦时用较浅的分隔色
    let focus_border = cx.theme().primary;
    let cancel_view = view.clone();
    let outside_view = view.clone();
    let cancel_button_view = view.clone();
    let confirm_button_view = view;
    div()
        .id(("redis-list-table-value-edit-cell", row_ix))
        .size_full()
        .min_w(px(0.))
        .cursor_text()
        // 阻断鼠标穿透到被覆盖行的单元格（避免误触开始新编辑），滚轮滚动仍可穿透
        .block_mouse_except_scroll()
        // 点击浮层以外区域时退出编辑模式（对齐「点击他处取消」的交互预期）
        .on_mouse_down_out(move |_, _, cx| {
            let view = outside_view.clone();
            let _ = view.update(cx, |this, cx| {
                this.cancel_redis_list_item_edit(cx);
            });
        })
        .on_action(cx.listener(move |_, _: &CancelDialog, _, cx| {
            let _ = cancel_view.update(cx, |this, cx| {
                this.cancel_redis_list_item_edit(cx);
            });
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .size_full()
                .rounded(colors.radius)
                .border_1()
                .border_color(if focused { focus_border } else { colors.border.into() })
                .bg(colors.panel_bg)
                .shadow_lg()
                .p_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    // 输入区占满浮层上部，贴近左上角对齐，不再垂直居中
                    div()
                        .rounded(colors.radius * 0.5)
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .flex_1()
                        .w_full()
                        .px_0()
                        .py(px(1.))
                        .child(
                            Input::new(&input)
                                .xsmall()
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .font_family("Menlo")
                                .line_height(px(19.))
                                .text_size(px(13.)),
                        ),
                )
                .child(
                    // 按钮独立成行、右对齐，贴合输入区底部
                    div()
                        .flex()
                        .justify_end()
                        .gap_1()
                        .child(
                            redis_hash_value_edit_icon_button(AppIcon::Close, colors)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |_, _, _, cx| {
                                        let _ = cancel_button_view.update(cx, |this, cx| {
                                            this.cancel_redis_list_item_edit(cx);
                                        });
                                        cx.stop_propagation();
                                    }),
                                ),
                        )
                        .child(
                            redis_hash_value_edit_icon_button(AppIcon::Check, colors)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |_, _, _, cx| {
                                        let _ = confirm_button_view.update(cx, |this, cx| {
                                            this.confirm_redis_list_item_edit(cx);
                                        });
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

/// 更新 NavicatMain 中当前 hover 的 List 编辑行状态（用于 Value 单元格背景高亮与编辑图标显示）。
fn redis_list_table_set_navicat_hovered(
    this: &mut NavicatMain,
    target: RedisListItemEditingState,
    is_hovered: bool,
    cx: &mut Context<NavicatMain>,
) {
    if is_hovered {
        if this.redis_list_item_hovered.as_ref() != Some(&target) {
            this.redis_list_item_hovered = Some(target);
            cx.notify();
        }
    } else if this.redis_list_item_hovered.as_ref() == Some(&target) {
        this.redis_list_item_hovered = None;
        cx.notify();
    }
}
