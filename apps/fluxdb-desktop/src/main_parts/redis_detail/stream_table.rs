#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisStreamEntryRow {
    id: String,
    time: String,
    fields: BTreeMap<String, String>,
}

struct RedisStreamTableDelegate {
    view: WeakEntity<NavicatMain>,
    tab_id: Option<TabId>,
    key: String,
    entries: Vec<RedisStreamEntryRow>,
    field_columns: Vec<String>,
    columns: Vec<TableColumn>,
    applying: bool,
    hovered_entry: Option<String>,
    pending_entry: Option<String>,
    colors: UiColors,
}

impl RedisStreamTableDelegate {
    fn new(view: WeakEntity<NavicatMain>) -> Self {
        Self {
            view,
            tab_id: None,
            key: String::new(),
            entries: Vec::new(),
            field_columns: Vec::new(),
            columns: Vec::new(),
            applying: false,
            hovered_entry: None,
            pending_entry: None,
            colors: ui_colors(ThemeMode::Light),
        }
    }

    fn set_data(
        &mut self,
        tab_id: TabId,
        key: String,
        entries: Vec<RedisStreamEntryRow>,
        field_columns: Vec<String>,
        applying: bool,
        pending_entry: Option<String>,
        colors: UiColors,
    ) -> bool {
        let changed = self.tab_id != Some(tab_id)
            || self.key != key
            || self.entries != entries
            || self.field_columns != field_columns
            || self.applying != applying
            || self.pending_entry != pending_entry;
        self.tab_id = Some(tab_id);
        self.key = key;
        self.entries = entries;
        self.field_columns = field_columns.clone();
        self.columns = std::iter::once(
            TableColumn::new("entry_id", "Entry ID")
                .width(px(240.))
                .fixed_left()
                .resizable(false)
                .movable(false)
                .selectable(false),
        )
        .chain(field_columns.into_iter().map(|field| {
            TableColumn::new(field.clone(), field)
                .width(px(180.))
                .resizable(false)
        }))
        .collect();
        self.applying = applying;
        self.pending_entry = pending_entry;
        self.colors = colors;
        self.hovered_entry = self.hovered_entry.take().filter(|entry_id| {
            self.entries.iter().any(|entry| &entry.id == entry_id)
        });
        changed
    }
}

impl TableDelegate for RedisStreamTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.entries.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        self.columns[col_ix].clone()
    }

    /// 保留表头下边框，将表头与数据区区分开，避免 Stream 数据行列难以分辨。
    fn render_header(&mut self, _: &mut Window, _: &mut Context<TableState<Self>>) -> Stateful<Div> {
        div()
            .id("redis-stream-table-header")
            .border_b_1()
            .border_color(self.colors.border)
    }

    /// 数据行之间保留极淡的分隔线，便于逐行对齐阅读。
    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id(("redis-stream-table-row", row_ix))
            .border_b_1()
            .border_color(self.colors.border_soft)
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
            // 单元格不画竖线，表头下划线由 render_header 统一绘制
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(self.colors.muted)
            .child(self.columns[col_ix].name.clone())
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(entry) = self.entries.get(row_ix).cloned() else {
            return div()
                .id(("redis-stream-table-empty-cell", row_ix))
                .size_full();
        };
        let colors = self.colors;
        if col_ix > 0 {
            return div()
                .id(SharedString::from(format!(
                    "redis-stream-table-field-{}-{}",
                    row_ix, col_ix
                )))
                .size_full()
                .px_3()
                .flex()
                .items_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // 单元格不画竖线，行分隔由 render_tr 统一绘制
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(13.))
                .text_color(colors.text)
                .child(
                    entry
                        .fields
                        .get(&self.field_columns[col_ix - 1])
                        .cloned()
                        .unwrap_or_default(),
                );
        }

        let tab_id = self.tab_id.expect("stream table tab should be set");
        let key = self.key.clone();
        let entry_id = entry.id.clone();
        let hovered = self.hovered_entry.as_deref() == Some(entry_id.as_str());
        let pending = self.pending_entry.as_deref() == Some(entry_id.as_str());
        let applying = self.applying;
        let view = self.view.clone();
        let hover_entry_id = entry_id.clone();
        let leave_entry_id = entry_id.clone();
        let delete_button = redis_stream_table_delete_button(
            tab_id,
            key.clone(),
            entry_id.clone(),
            hovered,
            pending,
            applying,
            colors,
            view.clone(),
            cx,
        );
        let cell = redis_stream_table_entry_cell(
            row_ix,
            entry,
            colors,
            hover_entry_id,
            leave_entry_id,
            delete_button,
            cx,
        );
        cell
    }
}

fn redis_stream_table(
    table_state: &Entity<TableState<RedisStreamTableDelegate>>,
    colors: UiColors,
) -> Div {
    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .child(
            DataTable::new(&table_state)
                .with_size(gpui_component::Size::Large)
                .stripe(false)
                .bordered(false)
                .scrollbar_visible(true, true),
        )
}

fn redis_stream_table_entry_cell(
    row_ix: usize,
    entry: RedisStreamEntryRow,
    colors: UiColors,
    hover_entry_id: String,
    leave_entry_id: String,
    delete_button: Div,
    cx: &mut Context<TableState<RedisStreamTableDelegate>>,
) -> Stateful<Div> {
    div()
        .id(("redis-stream-table-entry", row_ix))
        .relative()
        .size_full()
        .min_h(px(48.))
        .px_3()
        .pt_2()
        .pb_1()
        .flex()
        .flex_col()
        .justify_start()
        .gap_1()
        // 单元格不画竖线，行分隔由 render_tr 统一绘制
        .on_mouse_move(cx.listener(move |table, _, _, cx| {
            if table.delegate().hovered_entry.as_deref() != Some(hover_entry_id.as_str()) {
                table.delegate_mut().hovered_entry = Some(hover_entry_id.clone());
                table.refresh(cx);
            }
        }))
        .on_hover(cx.listener(move |table, hovered_state: &bool, _, cx| {
            if !*hovered_state
                && table.delegate().hovered_entry.as_deref() == Some(leave_entry_id.as_str())
            {
                table.delegate_mut().hovered_entry = None;
                table.refresh(cx);
            }
        }))
        .child(
            div()
                .w_full()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(colors.text)
                .child(entry.time),
        )
        .child(
            div()
                .w_full()
                .font_family("Menlo")
                .text_size(px(11.))
                .line_height(px(14.))
                .text_color(colors.muted)
                .overflow_hidden()
                .text_ellipsis()
                .child(entry.id),
        )
        .child(delete_button)
}

fn redis_stream_table_delete_button(
    tab_id: TabId,
    key: String,
    entry_id: String,
    hovered: bool,
    pending: bool,
    applying: bool,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    cx: &mut Context<TableState<RedisStreamTableDelegate>>,
) -> Div {
    let shown = hovered || pending;
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(5.))
        .w(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id(SharedString::from(format!(
                    "redis-stream-table-delete-{}-{}-{}",
                    tab_id.0, key, entry_id
                )))
                .size(px(24.))
                .rounded(colors.radius * 0.5)
                .flex()
                .items_center()
                .justify_center()
                .opacity(if shown { 1. } else { 0. })
                .text_color(if applying { colors.border } else { rgb(0xe5484d) })
                .when(shown, |this| {
                    this.cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .tooltip(|window, cx| Tooltip::new("删除").build(window, cx))
                        .on_mouse_down(
                            MouseButton::Left,
                            // cx.listener 的回调本身就运行在 TableState 的 lease 内，
                            // 直接改第一个参数（已租用的 &mut TableState）即可，不能再 update 同一实体。
                            cx.listener(move |table, _, _, cx| {
                                if !applying {
                                    let _ = view.update(cx, |this, cx| {
                                        this.pending_redis_stream_entry_delete =
                                            Some(RedisStreamEntryDeleteConfirm {
                                                tab_id,
                                                key: key.clone(),
                                                entry_id: entry_id.clone(),
                                            });
                                        cx.notify();
                                    });
                                    table.delegate_mut().pending_entry = Some(entry_id.clone());
                                    table.refresh(cx);
                                }
                                cx.stop_propagation();
                            }),
                        )
                        .on_click(|_, _, cx| cx.stop_propagation())
                })
                .child(app_icon(
                    AppIcon::Trash,
                    14.,
                    if applying { colors.border } else { rgb(0xe5484d) },
                )),
        )
}

fn redis_stream_table_delete_confirm_overlay(
    tab_id: TabId,
    key: String,
    entry_id: String,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .left(px(18.))
        .top(px(44.))
        .w(px(240.))
        .p_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow_lg()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("确认删除该 Entry？"),
        )
        .child(
            div()
                .font_family("Menlo")
                .text_size(px(11.))
                .text_color(colors.muted)
                .overflow_hidden()
                .text_ellipsis()
                .child(entry_id.clone()),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    redis_detail_action_button("取消", false, !applying, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.pending_redis_stream_entry_delete = None;
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    redis_detail_action_button("删除", false, !applying, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if !applying {
                                this.request_redis_stream_entry_delete(
                                    tab_id,
                                    key.clone(),
                                    entry_id.clone(),
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
}

// Stream 面板「新增」按钮：与 Set 面板复用同一按钮样式，保持两处 UI 一致
