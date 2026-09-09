const REDIS_HASH_TABLE_ROW_HEIGHT: f32 = 40.0;
// 值编辑浮层向下覆盖 2 行（总高 120px），容纳多行输入

const REDIS_HASH_VALUE_EDIT_EXTRA_ROWS: usize = 2;
// TTL 编辑浮层向下覆盖 1 行（总高 80px），容纳单行数字输入 + 确认/取消按钮两行布局

const REDIS_HASH_TTL_EDIT_EXTRA_ROWS: usize = 1;

/// 每种编辑类型的浮层向下覆盖的行数：Value 覆盖 2 行，TTL 覆盖 1 行。
fn redis_hash_editor_extra_rows(kind: RedisHashFieldCellKind) -> usize {
    match kind {
        RedisHashFieldCellKind::Value => REDIS_HASH_VALUE_EDIT_EXTRA_ROWS,
        RedisHashFieldCellKind::Ttl => REDIS_HASH_TTL_EDIT_EXTRA_ROWS,
    }
}

/// 编辑浮层锚定行：编辑行向下覆盖 EXTRA_ROWS 行，接近表尾时钳制到末行，
/// 避免锚定到不存在的行导致浮层丢失。
fn redis_hash_editor_anchor_row(editing: &RedisHashFieldEditingState, rows_len: usize) -> usize {
    editing
        .row_index
        .saturating_add(redis_hash_editor_extra_rows(editing.kind))
        .min(rows_len.saturating_sub(1))
}

struct RedisHashTableDelegate {
    view: WeakEntity<NavicatMain>,
    tab_id: Option<TabId>,
    key: String,
    rows: Vec<RedisHashFieldRow>,
    columns: Vec<TableColumn>,
    editing: Option<RedisHashFieldEditingState>,
    hovered: Option<RedisHashFieldEditingState>,
    pending_delete: Option<RedisHashFieldDeleteTarget>,
    applying: bool,
    search_loading: bool,
    value_edit_input: Option<Entity<InputState>>,
    ttl_edit_input: Option<Entity<InputState>>,
    colors: UiColors,
    table_width: Pixels,
    // 服务端版本：>=7.4 时字段级 TTL 可用（大字段被截断也能只改 TTL，不重写 value）；
    // None 表示版本未知（截断行维持禁用，非截断行可编辑）。
    server_version: Option<RedisServerVersion>,
}

impl RedisHashTableDelegate {
    fn new(view: WeakEntity<NavicatMain>) -> Self {
        Self {
            view,
            tab_id: None,
            key: String::new(),
            rows: Vec::new(),
            columns: Vec::new(),
            editing: None,
            hovered: None,
            pending_delete: None,
            applying: false,
            search_loading: false,
            value_edit_input: None,
            ttl_edit_input: None,
            colors: ui_colors(ThemeMode::Light),
            table_width: px(1000.),
            server_version: None,
        }
    }

    fn set_data(
        &mut self,
        tab_id: TabId,
        key: String,
        rows: Vec<RedisHashFieldRow>,
        editing: Option<RedisHashFieldEditingState>,
        hovered: Option<RedisHashFieldEditingState>,
        pending_delete: Option<RedisHashFieldDeleteTarget>,
        applying: bool,
        search_loading: bool,
        value_edit_input: Entity<InputState>,
        ttl_edit_input: Entity<InputState>,
        colors: UiColors,
        server_version: Option<RedisServerVersion>,
    ) -> bool {
        // hover 指向已不存在的行时清空，避免越界悬空
        let hovered = hovered.filter(|h| h.row_index < rows.len());
        // 删除确认目标对应的字段已不在结果集时，撤销悬空的确认浮层
        let pending_delete = pending_delete.filter(|pending| {
            rows.iter().any(|row| row.field == pending.field)
                && pending.tab_id == tab_id
                && pending.key == key
        });
        let changed = self.tab_id != Some(tab_id)
            || self.key != key
            || self.rows != rows
            || self.editing != editing
            || self.hovered != hovered
            || self.pending_delete != pending_delete
            || self.applying != applying
            || self.search_loading != search_loading
            || self.server_version != server_version;
        self.tab_id = Some(tab_id);
        self.key = key;
        self.rows = rows;
        self.editing = editing;
        self.hovered = hovered;
        self.pending_delete = pending_delete;
        self.applying = applying;
        self.search_loading = search_loading;
        self.value_edit_input = Some(value_edit_input);
        self.ttl_edit_input = Some(ttl_edit_input);
        self.colors = colors;
        self.server_version = server_version;
        self.rebuild_columns();
        changed
    }

    fn rebuild_columns(&mut self) {
        let width = self.table_width;
        self.columns = vec![
            TableColumn::new("seq", "序列")
                .width(redis_hash_table_column_width("seq", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
            TableColumn::new("field", "Field")
                .width(redis_hash_table_column_width("field", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
            TableColumn::new("value", "Value")
                .width(redis_hash_table_column_width("value", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
            TableColumn::new("ttl", "TTL")
                .width(redis_hash_table_column_width("ttl", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
            // 删除列为固定操作列，表头留空，按钮在单元格内居中显示
            TableColumn::new("delete", "")
                .width(redis_hash_table_column_width("delete", width))
                .resizable(false)
                .movable(false)
                .selectable(false),
        ];
    }

    fn sync_redis_hash_table_width(&mut self, table_width: Pixels) -> bool {
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
    /// 锚定行在编辑行之后绘制，因此浮层整体位于被覆盖行之上，天然不被其背景/分隔线遮挡；
    /// 鼠标事件经 block_mouse_except_scroll 阻断穿透（滚轮仍可滚动表格），输入框与按钮可正常交互。
    /// 浮层随锚定行滚动自动跟随，仅 viewport 边缘会被 ContentMask 裁剪。
    fn render_redis_hash_value_editor(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Div {
        let Some(input) = self.value_edit_input.clone() else {
            return div();
        };
        let Some(editing) = self.editing.clone() else {
            return div();
        };
        let row_height = px(REDIS_HASH_TABLE_ROW_HEIGHT);
        let editor_height = row_height * (REDIS_HASH_VALUE_EDIT_EXTRA_ROWS as f32 + 1.0);
        // 锚定行固定为传入的 row_ix（render_tr 已按 anchor 行调用本函数），
        // 回退距离 = 锚定行与编辑行的行数差
        let extra_rows = row_ix.saturating_sub(editing.row_index) as f32;
        // 浮层左边缘 = 序列列 + Field 列宽度之和，宽度 = Value 列宽
        let value_left = self.columns[0].width + self.columns[1].width;
        let value_width = self.columns[2].width;
        let view = self.view.clone();
        let colors = self.colors;
        div()
            .absolute()
            .top(-(row_height * extra_rows))
            .left(value_left)
            .w(value_width)
            .h(editor_height)
            .child(redis_hash_table_value_edit_cell(
                row_ix,
                input,
                colors,
                view,
                window,
                cx,
            ))
    }

    /// TTL 编辑浮层比 Value 更紧凑：只向下覆盖 1 行（总高 80px），
    /// 单行数字输入区在上、确认/取消按钮独立成行在下。
    fn render_redis_hash_ttl_editor(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Div {
        let Some(input) = self.ttl_edit_input.clone() else {
            return div();
        };
        let Some(editing) = self.editing.clone() else {
            return div();
        };
        let row_height = px(REDIS_HASH_TABLE_ROW_HEIGHT);
        let editor_height = row_height * (REDIS_HASH_TTL_EDIT_EXTRA_ROWS as f32 + 1.0);
        // 回退距离 = 锚定行与编辑行的行数差
        let extra_rows = row_ix.saturating_sub(editing.row_index) as f32;
        // 浮层左边缘 = 序列列 + Field 列 + Value 列宽度之和，宽度 = TTL 列宽
        let ttl_left = self.columns[0].width + self.columns[1].width + self.columns[2].width;
        let ttl_width = self.columns[3].width;
        let view = self.view.clone();
        let colors = self.colors;
        div()
            .absolute()
            .top(-(row_height * extra_rows))
            .left(ttl_left)
            .w(ttl_width)
            .h(editor_height)
            .child(redis_hash_table_ttl_edit_cell(
                row_ix,
                input,
                colors,
                view,
                window,
                cx,
            ))
    }
}

impl TableDelegate for RedisHashTableDelegate {
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
            .id("redis-hash-table-header")
            .border_b_1()
            .border_color(self.colors.border)
    }

    /// 数据行之间保留极淡的分隔线。
    /// 编辑浮层占用的行（编辑行及其下方覆盖行）整段高亮、不画分隔线，
    /// 避免横线穿过浮层；浮层锚定在「最后被覆盖的行」上，位于这些行之上。
    /// 其余行始终保持普通行高与分隔线。
    fn render_tr(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let in_edit_zone = self.editing.as_ref().is_some_and(|editing| {
            row_ix >= editing.row_index
                && row_ix <= editing.row_index + redis_hash_editor_extra_rows(editing.kind)
        });
        let editor_anchor_row = self.editing.as_ref().is_some_and(|editing| {
            row_ix == redis_hash_editor_anchor_row(editing, self.rows.len())
        });
        let mut tr = div()
            .id(("redis-hash-table-row", row_ix))
            .relative()
            .when(in_edit_zone, |this| this.bg(self.colors.hover).border_b_0())
            .when(!in_edit_zone, |this| {
                this.border_b_1().border_color(self.colors.border_soft)
            });
        if editor_anchor_row {
            // 浮层锚定在最后被覆盖的行，随滚动自动移动
            let kind = self.editing.as_ref().map(|editing| editing.kind);
            tr = tr.child(match kind {
                Some(RedisHashFieldCellKind::Value) => {
                    self.render_redis_hash_value_editor(row_ix, window, cx)
                }
                _ => self.render_redis_hash_ttl_editor(row_ix, window, cx),
            });
        }
        tr
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

    fn render_empty(&mut self, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(self.colors.muted)
            .child("暂无字段")
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return div()
                .id(("redis-hash-table-empty-cell", row_ix))
                .size_full()
                .into_any_element();
        };
        // 编辑浮层锚定在编辑行并向下覆盖若干行，被覆盖行中浮层所在列留空，
        // 避免单元格内容绘制在浮层之上（值编辑浮层→Value 列，TTL 浮层→TTL 列）
        let cell_covered = self.editing.as_ref().is_some_and(|editing| {
            row_ix >= editing.row_index
                && row_ix <= editing.row_index + redis_hash_editor_extra_rows(editing.kind)
        });
        match col_ix {
            0 => redis_hash_table_seq_cell(row_ix, self.colors).into_any_element(),
            1 => redis_hash_table_field_cell(row_ix, row, self.colors).into_any_element(),
            2 => {
                let value_covered = cell_covered
                    && self
                        .editing
                        .as_ref()
                        .is_some_and(|e| e.kind == RedisHashFieldCellKind::Value);
                if value_covered {
                    div()
                        .id(("redis-hash-table-value-blank", row_ix))
                        .size_full()
                        .into_any_element()
                } else {
                    redis_hash_table_value_cell(row_ix, row, self, cx).into_any_element()
                }
            }
            3 => {
                let ttl_covered = cell_covered
                    && self
                        .editing
                        .as_ref()
                        .is_some_and(|e| e.kind == RedisHashFieldCellKind::Ttl);
                if ttl_covered {
                    div()
                        .id(("redis-hash-table-ttl-blank", row_ix))
                        .size_full()
                        .into_any_element()
                } else {
                    redis_hash_table_ttl_cell(row_ix, row, self, cx).into_any_element()
                }
            }
            _ => redis_hash_table_delete_cell(row_ix, row, self, cx).into_any_element(),
        }
    }
}

fn redis_hash_table_column_width(name: &str, table_width: Pixels) -> Pixels {
    // 权重：序列 10 / Field 31 / Value 31 / TTL 20 / 删除 6
    // 权重和为 100，直接按可用宽度归一化
    const TOTAL_WEIGHT: f32 = 100.0;
    let weight = match name {
        "field" | "value" => 31.0,
        "ttl" => 20.0,
        "delete" => 6.0,
        _ => 10.0,
    };
    px(f32::from(table_width) * weight / TOTAL_WEIGHT)
}

fn redis_hash_table_seq_cell(row_ix: usize, colors: UiColors) -> impl IntoElement {
    div()
        .id(("redis-hash-table-seq", row_ix))
        .size_full()
        .min_w(px(0.))
        .px_3()
        .flex()
        // 所有行高度恒定（编辑浮层不改变行高），序号始终垂直居中，独立于编辑态
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family("Menlo")
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(format!("{:02}", row_ix + 1))
}

fn redis_hash_table_field_cell(
    row_ix: usize,
    row: RedisHashFieldRow,
    colors: UiColors,
) -> impl IntoElement {
    // 字段名可能较长，截断时 hover 显示全文（对齐 Redis Insight FormattedValue）
    let field_tooltip = row.field.clone();
    div()
        .id(("redis-hash-table-field", row_ix))
        .size_full()
        .min_w(px(0.))
        .px_3()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .font_family("Menlo")
        .text_color(colors.text)
        .tooltip(move |window, cx| Tooltip::new(field_tooltip.clone()).build(window, cx))
        .child(row.field)
}

fn redis_hash_table_value_cell(
    row_ix: usize,
    row: RedisHashFieldRow,
    delegate: &mut RedisHashTableDelegate,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    let colors = delegate.colors;
    let tab_id = delegate.tab_id.unwrap_or(TabId(0));
    let key = delegate.key.clone();
    let target = RedisHashFieldEditingState {
        tab_id,
        key: key.clone(),
        row_index: row_ix,
        kind: RedisHashFieldCellKind::Value,
    };
    // 编辑行与覆盖行的 Value 列由 render_td 留空，编辑浮层在 render_tr 中渲染
    let hovered = delegate.hovered.as_ref() == Some(&target);
    let applying = delegate.applying;
    let view = delegate.view.clone();
    // 大值被截断时禁编辑（截断串只是片段，回写等于覆盖完整数据）。
    let truncated = redis_hash_value_is_truncated(&row.value);

    let hover_view = view.clone();
    let click_view = view.clone();
    let hover_target = target.clone();
    let click_target = target;
    let click_field = row.field.clone();
    let click_key = key.clone();
    // value 可能很长，截断时 hover 显示查看/编辑完整值入口提示（对齐 Redis Insight FormattedValue）。
    let value_tooltip = if truncated {
        "值超过 1MB 已被截断；点击查看/编辑完整值".to_string()
    } else {
        row.value.clone()
    };
    let eye_field = row.field.clone();
    div()
        .id(("redis-hash-table-value", row_ix))
        .size_full()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap_1()
        // 截断行点击打开完整值内嵌面板；非截断行行内编辑，均为可点击区域
        .cursor_pointer()
        .overflow_hidden()
        .when(hovered, |this| this.bg(colors.hover))
        .on_hover(cx.listener(move |_, is_hovered: &bool, _, cx| {
            let _ = hover_view.clone().update(cx, |this, cx| {
                redis_hash_table_set_navicat_hovered(this, hover_target.clone(), *is_hovered, cx);
            });
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, _, window, cx| {
                if !applying {
                    if truncated {
                        // 截断值：打开完整值内嵌面板（懒加载非截断原始值）
                        let _ = click_view.clone().update(cx, |this, cx| {
                            this.open_redis_hash_full_value_viewer(
                                tab_id,
                                click_key.clone(),
                                click_field.clone(),
                                window,
                                cx,
                            );
                        });
                    } else {
                        let _ = click_view.clone().update(cx, |this, cx| {
                            this.begin_redis_hash_field_edit(click_target.clone(), window, cx);
                        });
                    }
                }
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id(("redis-hash-table-value-text", row_ix))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .font_family("Menlo")
                .text_color(if truncated { colors.muted } else { colors.text })
                .tooltip(move |window, cx| Tooltip::new(value_tooltip.clone()).build(window, cx))
                .child(row.value),
        )
        // 非截断行：hover 显示编辑图标（行内编辑，拒截断值回写）。
        .when(hovered && !applying && !truncated, |this| {
            this.child(redis_hash_field_edit_icon(AppIcon::Edit, "编辑", colors))
        })
        // 截断行：hover 显示查看完整值图标（打开弹框，懒加载完整值后可编辑写回）。
        .when(hovered && !applying && truncated, |this| {
            this.child(
                redis_hash_field_view_icon(
                    AppIcon::Eye,
                    "查看完整值",
                    colors,
                    view,
                    tab_id,
                    key,
                    eye_field,
                    cx,
                ),
            )
        })
        .into_any_element()
}

fn redis_hash_table_ttl_cell(
    row_ix: usize,
    row: RedisHashFieldRow,
    delegate: &mut RedisHashTableDelegate,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    let colors = delegate.colors;
    let tab_id = delegate.tab_id.unwrap_or(TabId(0));
    let key = delegate.key.clone();
    let target = RedisHashFieldEditingState {
        tab_id,
        key: key.clone(),
        row_index: row_ix,
        kind: RedisHashFieldCellKind::Ttl,
    };
    let hovered = delegate.hovered.as_ref() == Some(&target);
    let applying = delegate.applying;
    let view = delegate.view.clone();
    let ttl_display = redis_hash_field_ttl_display_value(row.ttl.as_str());
    let truncated = redis_hash_value_is_truncated(&row.value);
    // 版本 ≥7.4 时字段级 TTL 走纯 TTL 命令（HPEXPIRE/HPERSIST），只改 TTL 不重写 value，
    // 因此大字段（截断）也能只改 TTL；<7.4 或版本未知时截断行维持禁用。
    let server_version = delegate.server_version.as_ref();
    let ttl_editable = redis_hash_field_ttl_editable(server_version, truncated);
    // 截断且不可编辑时才需要禁用提示；可编辑时不显示阻断 tooltip
    let disabled_tooltip = (!ttl_editable && truncated)
        .then(|| redis_hash_field_ttl_disabled_tooltip(server_version));

    let hover_view = view.clone();
    let click_view = view.clone();
    let hover_target = target.clone();
    let click_target = target;
    div()
        .id(("redis-hash-table-ttl", row_ix))
        .size_full()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap_1()
        .when(ttl_editable, |this| this.cursor_pointer())
        .overflow_hidden()
        .when(hovered, |this| this.bg(colors.hover))
        .on_hover(cx.listener(move |_, is_hovered: &bool, _, cx| {
            let _ = hover_view.clone().update(cx, |this, cx| {
                redis_hash_table_set_navicat_hovered(this, hover_target.clone(), *is_hovered, cx);
            });
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, _, window, cx| {
                if !applying && ttl_editable {
                    let _ = click_view.clone().update(cx, |this, cx| {
                        this.begin_redis_hash_field_edit(click_target.clone(), window, cx);
                    });
                }
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id(("redis-hash-table-ttl-text", row_ix))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .font_family("Menlo")
                .text_color(if truncated { colors.muted } else { colors.text })
                .when_some(disabled_tooltip, move |this, message| {
                    this.tooltip(move |window, cx| {
                        Tooltip::new(message.clone()).build(window, cx)
                    })
                })
                .child(ttl_display),
        )
        .into_any_element()
}

/// 独立删除列：单元格内居中渲染删除按钮；删除请求自带任务级防重入，
/// 且先弹二次确认浮层（对齐 Redis Insight ConfirmationPopover）确认后才真正删除
fn redis_hash_table_delete_cell(
    row_ix: usize,
    row: RedisHashFieldRow,
    delegate: &mut RedisHashTableDelegate,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    let colors = delegate.colors;
    let tab_id = delegate.tab_id.unwrap_or(TabId(0));
    let key = delegate.key.clone();
    let view = delegate.view.clone();
    let target = RedisHashFieldDeleteTarget {
        tab_id,
        key: key.clone(),
        field: row.field.clone(),
    };
    let confirm_pending = delegate.pending_delete.as_ref() == Some(&target);
    div()
        .id(("redis-hash-table-delete", row_ix))
        .size_full()
        .min_w(px(0.))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .child(redis_hash_table_delete_button(
            tab_id,
            key.clone(),
            row.field.clone(),
            colors,
            view.clone(),
            cx,
        ))
        .when(confirm_pending, |this| {
            this.child(redis_hash_field_delete_confirm_popover(
                target,
                colors,
                view,
                cx,
            ))
        })
    .into_any_element()
}

fn redis_hash_table_value_edit_cell(
    row_ix: usize,
    input: Entity<InputState>,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    window: &mut Window,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    // 焦点边框使用主题强调色（替代过重的黑色），未聚焦时用较浅的分隔色
    let focus_border = cx.theme().primary;
    let cancel_view = view.clone();
    let cancel_button_view = view.clone();
    let confirm_button_view = view;
    div()
        .id(("redis-hash-table-value-edit-cell", row_ix))
        .size_full()
        .min_w(px(0.))
        .cursor_text()
        // 阻断鼠标穿透到被覆盖行的单元格（避免误触开始新编辑），滚轮滚动仍可穿透
        .block_mouse_except_scroll()
        .on_action(cx.listener(move |_, _: &CancelDialog, _, cx| {
            let _ = cancel_view.update(cx, |this, cx| {
                this.cancel_redis_hash_field_edit(cx);
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
                    // 按钮独立成行、右对齐，贴合输入区底部，不再悬浮于输入框之上
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
                                            this.cancel_redis_hash_field_edit(cx);
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
                                            this.confirm_redis_hash_field_edit(cx);
                                        });
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn redis_hash_table_ttl_edit_cell(
    row_ix: usize,
    input: Entity<InputState>,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    window: &mut Window,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    // 焦点边框使用主题强调色（替代过重的黑色），未聚焦时用较浅的分隔色
    let focus_border = cx.theme().primary;
    let cancel_view = view.clone();
    let cancel_button_view = view.clone();
    let confirm_button_view = view;
    div()
        .id(("redis-hash-table-ttl-edit-cell", row_ix))
        .size_full()
        .min_w(px(0.))
        .cursor_text()
        // 阻断鼠标穿透到被覆盖行的单元格（避免误触开始新编辑），滚轮滚动仍可穿透
        .block_mouse_except_scroll()
        .on_action(cx.listener(move |_, _: &CancelDialog, _, cx| {
            let _ = cancel_view.update(cx, |this, cx| {
                this.cancel_redis_hash_field_edit(cx);
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
                // 两行式布局：单行数字输入区在上（flex_1），确认/取消按钮独立成行在下
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    // 单行数字输入区占满浮层上部，贴近左上角对齐，数字垂直居中
                    div()
                        .rounded(colors.radius * 0.5)
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .flex_1()
                        .w_full()
                        .px_2()
                        .py(px(1.))
                        .flex()
                        .items_center()
                        .child(
                            Input::new(&input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
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
                                            this.cancel_redis_hash_field_edit(cx);
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
                                            this.confirm_redis_hash_field_edit(cx);
                                        });
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn redis_hash_value_edit_icon_button(
    icon: AppIcon,
    colors: UiColors,
) -> Div {
    div()
        .size(px(26.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover).border_color(colors.border))
        .child(app_icon(icon, 13., colors.text))
}

fn redis_hash_table_delete_button(
    tab_id: TabId,
    key: String,
    field: String,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!(
            "redis-hash-table-delete-{}-{}-{}",
            tab_id.0, key, field
        )))
        .size(px(22.))
        .flex_none()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new("删除").build(window, cx))
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, _, _, cx| {
                // 先弹二次确认浮层，确认后再真正删除（对齐 Redis Insight ConfirmationPopover）
                let _ = view.clone().update(cx, |this, cx| {
                    this.pending_redis_hash_field_delete = Some(RedisHashFieldDeleteTarget {
                        tab_id,
                        key: key.clone(),
                        field: field.clone(),
                    });
                    cx.notify();
                });
                cx.stop_propagation();
            }),
        )
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(app_icon(AppIcon::Trash, 14., rgb(0xe5484d)))
}

// Hash 字段删除二次确认浮层：锚定在删除单元格内，确认后执行真正删除（对齐 Redis Insight ConfirmationPopover）

fn redis_hash_field_delete_confirm_popover(
    target: RedisHashFieldDeleteTarget,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    _cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> impl IntoElement {
    // 面板行删除会随保存落库，删除不可撤销，提示字段内容
    let title = target.field.clone();
    let message = "将被删除，此操作不可撤销。";
    let cancel_view = view.clone();
    let confirm_view = view.clone();
    let confirm_target = target.clone();
    let dismiss_view = view.clone();
    div()
        .absolute()
        .right(px(34.))
        .top(px(-74.))
        .size(px(1.))
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopRight)
                    .child(
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
                                let _ = dismiss_view.clone().update(cx, |this, cx| {
                                    this.pending_redis_hash_field_delete = None;
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
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(colors.muted)
                                            .child(message),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        redis_key_delete_confirm_button("取消", false, colors)
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |_, _, cx| {
                                                    let _ = cancel_view.clone().update(cx, |this, cx| {
                                                        this.pending_redis_hash_field_delete = None;
                                                        cx.notify();
                                                    });
                                                    cx.stop_propagation();
                                                },
                                            ),
                                    )
                                    .child(
                                        redis_key_delete_confirm_button("确认删除", true, colors)
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                move |_, _, cx| {
                                                    let _ = confirm_view.clone().update(cx, |this, cx| {
                                                        this.request_redis_hash_field_delete(
                                                            confirm_target.tab_id,
                                                            confirm_target.key.clone(),
                                                            confirm_target.field.clone(),
                                                            cx,
                                                        );
                                                        this.pending_redis_hash_field_delete = None;
                                                        cx.notify();
                                                    });
                                                    cx.stop_propagation();
                                                },
                                            ),
                                    ),
                            ),
                    ),
            )
            .with_priority(1),
        )
        .into_any_element()
}

fn redis_hash_table_set_navicat_hovered(
    this: &mut NavicatMain,
    target: RedisHashFieldEditingState,
    is_hovered: bool,
    cx: &mut Context<NavicatMain>,
) {
    if is_hovered {
        if this.redis_hash_field_hovered.as_ref() != Some(&target) {
            this.redis_hash_field_hovered = Some(target);
            cx.notify();
        }
    } else if this.redis_hash_field_hovered.as_ref() == Some(&target) {
        this.redis_hash_field_hovered = None;
        cx.notify();
    }
}

fn redis_hash_table(
    table_state: &Entity<TableState<RedisHashTableDelegate>>,
) -> Div {
    let measured_table = table_state.clone();
    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_hidden()
        .child(
            DataTable::new(&table_state)
                .with_size(gpui_component::Size::Large)
                .stripe(false)
                .bordered(false)
                .scrollbar_visible(true, true),
        )
        .child(
            // 覆盖层 canvas 负责测量容器宽度，把比例列宽换算成像素
            canvas(
                move |bounds, _, cx| {
                    measured_table.update(cx, |table, cx| {
                        if table
                            .delegate_mut()
                            .sync_redis_hash_table_width(redis_table_fit_width(bounds.size.width))
                        {
                            table.refresh(cx);
                        }
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
}
