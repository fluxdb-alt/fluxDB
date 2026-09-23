// ER 表卡片渲染与字段滚动（er-ui-relationship-canvas.md §3）。
//
// 紧凑卡片：4px 顶部色带（按连通分量取色）+ 30px 表头 + 字段三列（名称 | 约束图标 | 类型）
// + 行间极淡分隔线 + 长表页脚。短表自然收短，长表固定 8 行可视区内部滚动。
// 表头按住可拖动该表（阈值与移动在 interaction.rs 处理）；点击（未移动）选择。
// 滚动条为本地轻量实现（gpui-component ScrollHandle 需持久元素，与节点虚拟化冲突）。

use gpui::ScrollDelta;

/// 单个字段行：名称（弹性）| 约束图标槽 14px | 类型右对齐 82px（§3.2）。
/// 行间 1px 极淡分隔线用 border_b 画在行内，不减少有效行高（不底部截字）。
fn field_row(
    tab_id: TabId,
    table: &str,
    col: &NodeColumnDisplay,
    is_last_visible: bool,
    highlighted: bool,
    card_color: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    // 图标：主键 key 优先；仅外键 link；tooltip 写明两者。普通字段不放装饰性圆点。
    let icon = if col.primary {
        Some((AppIcon::Key, card_color))
    } else if col.foreign_key {
        Some((AppIcon::Link, card_color))
    } else {
        None
    };
    // tooltip：完整字段名、类型、已知约束。
    let mut tip = format!("{table}.{}", col.name);
    if let Some(ty) = &col.type_name {
        tip.push_str(" · ");
        tip.push_str(ty);
    } else {
        tip.push_str(" · —");
    }
    if col.primary && col.foreign_key {
        tip.push_str(" · 主键、外键");
    } else if col.primary {
        tip.push_str(" · 主键");
    } else if col.foreign_key {
        tip.push_str(" · 外键");
    }
    let mut row = div()
        .h(px(NODE_FIELD_ROW))
        .pl(px(10.))
        .pr(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .when(highlighted, |d| d.bg(colors.hover))
        .when(!highlighted, |d| d.hover(|s| s.bg(colors.hover)))
        .id(format!("er-field-{}-{}-{}", tab_id.0, table, col.name))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
    // 行间极淡分隔线（最后一行不画，避免压页脚边框）。
    if !is_last_visible {
        row = row.border_b_1().border_color(colors.border_soft);
    }
    // 字段名：弹性区，正常文字色；仅外键可用适中的组件色（§3.2），主键加粗。
    let name_color = if col.primary || !col.foreign_key {
        colors.text
    } else {
        card_color
    };
    row = row.child(
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
            .font_weight(if col.primary {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(name_color)
            .child(col.name.clone()),
    );
    // 约束图标槽 14px（无图标也占位，保证类型列右对齐）。
    row = row.child(match icon {
        Some((ic, color)) => app_icon(ic, 12., color),
        None => div().w(px(14.)).into_any_element(),
    });
    // 类型列：82px 右对齐、弱于字段名；未知类型用「—」不伪造。
    let type_text = col.type_name.clone().unwrap_or_else(|| "—".to_string());
    let mut row = row.child(
        div()
            .w(px(82.))
            .flex_shrink_0()
            .text_right()
            .overflow_hidden()
            .text_ellipsis()
            .text_color(colors.muted)
            .child(type_text),
    );
    // 悬停字段行上报：画布端口层据此浮现该行左/右端口圆点（§手动连线）。
    // 离开时只清自己的记录，避免相邻行 hover 交替时把新行清掉。
    let hov_table = table.to_string();
    let hov_col = col.name.clone();
    row = row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
        let key = (hov_table.clone(), hov_col.clone());
        let changed = {
            let mut canvas = this.er_canvas.borrow_mut();
            let before = canvas.er_row_hover.get(&tab_id).cloned().flatten();
            let after = if *hovered {
                Some(key.clone())
            } else if before.as_ref() == Some(&key) {
                None
            } else {
                before.clone()
            };
            let changed = before != after;
            match after {
                Some(v) => {
                    canvas.er_row_hover.insert(tab_id, Some(v));
                }
                None => {
                    canvas.er_row_hover.remove(&tab_id);
                }
            }
            changed
        };
        if changed {
            cx.notify();
        }
    }));
    row
}

/// 字段区：固定高度视口（min(N,8)×22），像素滚动 + 虚拟行 + 滚动条。
fn er_field_area(
    tab_id: TabId,
    meta: &ErNodeMeta,
    nv: &ErNodeView,
    highlight: Option<&str>,
    card_color: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> (Div, bool) {
    let n = meta.columns.len();
    let visible_rows = n.min(MAX_FIELD_ROWS);
    let area_h = visible_rows as f32 * NODE_FIELD_ROW;
    let is_long = n > MAX_FIELD_ROWS;
    let max_scroll = if is_long {
        ((n - MAX_FIELD_ROWS) as f32 * NODE_FIELD_ROW).max(0.0)
    } else {
        0.0
    };
    let scroll = nv.scroll_px.clamp(0.0, max_scroll);
    let table = meta.name.clone();
    let scroll_table = table.clone();
    let mut body = div()
        .h(px(area_h))
        .overflow_hidden()
        .relative()
        .on_scroll_wheel(cx.listener(move |this, event: &gpui::ScrollWheelEvent, _, cx| {
            // 滚轮只滚字段，不平移画布、不触发展开（§7）。
            let delta = match event.delta {
                ScrollDelta::Lines(p) => p.y * NODE_FIELD_ROW,
                ScrollDelta::Pixels(p) => p.y.as_f32(),
            };
            let mut canvas = this.er_canvas.borrow_mut();
            let e = canvas
                .er_node_scroll_px
                .entry((tab_id, scroll_table.clone()))
                .or_insert(0.0);
            let before = *e;
            *e = (*e + delta).clamp(0.0, max_scroll);
            // 滚动到顶/底后不再更新状态，避免无效重绘（§4.3 前一轮约定保留）。
            let changed = *e != before;
            drop(canvas);
            if changed {
                cx.notify();
            }
        }));

    let mut rows = div().flex().flex_col().relative();
    // 虚拟化：只渲染可见 + 上下各 1 行 overscan。
    let row0 = (scroll / NODE_FIELD_ROW).floor().max(0.0) as usize;
    let first = row0.saturating_sub(1);
    let last = (row0 + MAX_FIELD_ROWS + 1).min(n);
    let (_, vis1) = visible_row_range(n, scroll);
    for i in first..last {
        let top = FIELD_PAD_Y / 2.0 + i as f32 * NODE_FIELD_ROW - scroll;
        let col = &meta.columns[i];
        let is_last_visible = i == vis1;
        let highlighted = highlight == Some(col.name.as_str());
        rows = rows.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(top))
                .child(field_row(tab_id, &table, col, is_last_visible, highlighted, card_color, colors, cx)),
        );
    }
    body = body.child(rows);

    if is_long {
        let thumb_h = (area_h / (area_h + max_scroll) * area_h).max(20.0);
        let thumb_y = scroll / max_scroll * (area_h - thumb_h);
        let thumb_table = table.clone();
        let scrollbar = div()
            .absolute()
            .right(px(2.))
            .top(px(3.))
            .bottom(px(3.))
            .w(px(5.))
            .rounded_full()
            .bg(colors.border)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.er_canvas.borrow_mut().er_scroll_drag = Some((tab_id, thumb_table.clone()));
                cx.notify();
            }));
        body = body.child(
            scrollbar.child(
                div()
                    .absolute()
                    .left_0()
                    .w(px(5.))
                    .top(px(thumb_y))
                    .h(px(thumb_h))
                    .rounded_full()
                    .bg(colors.muted),
            ),
        );
    }
    (body, is_long)
}

/// 单个表卡片。absolute 定位到世界坐标 + 视口平移。
/// 表头按住 → 记录拖动候选（阈值/移动/释放判定在 interaction.rs），未移动释放 = 选择。
fn node_view(
    tab_id: TabId,
    meta: &ErNodeMeta,
    nv: &ErNodeView,
    expandable: bool,
    viewport: ErViewport,
    highlight: Option<&str>,
    card_color: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name = meta.name.clone();
    let collapsed = viewport.safe_scale() < ER_COLLAPSE_SCALE;
    // 选中轮廓约 2px：用边框颜色 + 外描边（不改变内容布局）。
    let mut node = div()
        .absolute()
        .left(px(nv.x * viewport.safe_scale() + viewport.pan_x))
        .top(px(nv.y * viewport.safe_scale() + viewport.pan_y))
        .w(px(nv.width))
        .h(px(nv.height))
        .rounded(colors.radius)
        .border_1()
        .border_color(if nv.selected { card_color } else { colors.border })
        .when(nv.selected, |s| s.border_2())
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col();

    // 顶部 4px 色带：贴合顶部圆角，按连通分量取色，不覆盖标题。
    // 折叠态弱化为半透明强调线（避免窄卡片上突兀的粗色条），选中时仍用实色。
    node = node.child(
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(if collapsed { 2.0 } else { ACCENT_BAR }))
            .bg(if collapsed && !nv.selected { card_color.alpha(0.45) } else { card_color }),
    );

    // 表头：图标 16 + 名称 12 semibold +（局部 ER）展开按钮。
    // 折叠态横向内边距收窄、名称 http 截断，贴合紧凑宽度（不产生大片空白）。
    let drag_name = name.clone();
    let tip_name = name.clone();
    let header_h = if collapsed { card_collapsed_height() - if collapsed { 2.0 } else { ACCENT_BAR } } else { NODE_HEADER };
    let header_mt = if collapsed { 2.0 } else { ACCENT_BAR };
    let mut header = div()
        .h(px(header_h))
        .mt(px(header_mt))
        .pl(px(if collapsed { 8.0 } else { 10.0 }))
        .pr(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(colors.panel_alt)
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                // 拖动候选：interaction 的 move/up 按 4px 阈值判定拖动或选择（§7）。
                let (ox, oy) = this.er_canvas.borrow_mut().er_scene_positions
                    .get(&tab_id)
                    .and_then(|m| m.get(&drag_name).copied())
                    .unwrap_or((0.0, 0.0));
                this.er_canvas.borrow_mut().er_node_drag = Some((
                    tab_id,
                    drag_name.clone(),
                    f32::from(event.position.x),
                    f32::from(event.position.y),
                    ox,
                    oy,
                    false,
                ));
                cx.notify();
            }),
        )
        .id(format!("er-node-header-{}-{}", tab_id.0, name.clone()))
        .tooltip(move |window, cx| Tooltip::new(tip_name.clone()).build(window, cx))
        .child(app_icon(AppIcon::Table, if collapsed { 16. } else { 14. }, card_color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(if collapsed { 13. } else { 12. }))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(name.clone()),
        );

    if expandable && !collapsed {
        let expand_table = name.clone();
        header = header.child(
            div()
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(colors.radius)
                .cursor_pointer()
                .hover(|s| s.bg(colors.hover))
                .id(format!("er-expand-{}-{}", tab_id.0, name.clone()))
                .tooltip(move |window, cx| Tooltip::new("展开关联").build(window, cx))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.er_expanded
                            .entry(tab_id)
                            .or_default()
                            .insert(expand_table.clone());
                        // 同步重算邻域：保留人工坐标/固定，不撕空图；中心身份缺失时回退触发读取。
                        let recomputed = this
                            .er_center_refs
                            .get(&tab_id)
                            .cloned()
                            .map(|center| recompute_local_er_from_center(tab_id, &center, this, cx))
                            .unwrap_or(false);
                        if !recomputed {
                            this.er_graphs.remove(&tab_id);
                            this.er_scenes.remove(&tab_id);
                            this.er_relation_tasks.remove(&tab_id);
                            this.er_errors.remove(&tab_id);
                        }
                        cx.notify();
                    }),
                )
                .child(app_icon(AppIcon::Maximize, 14., colors.muted)),
        );
    }
    // 折叠态：表名后内嵌关联字段计数（含义明确、不遮挡连线、随表头拖动/选择）。
    if collapsed {
        let edge_n = meta.edge_columns.len();
        if edge_n > 0 {
            header = header.child(
                div()
                    .h(px(16.))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .flex_shrink_0()
                    .rounded(colors.radius)
                    .bg(colors.panel_bg)
                    .border_1()
                    .border_color(colors.border_soft)
                    .id(format!("er-edgecount-{}-{}", tab_id.0, name.clone()))
                    .text_size(px(10.))
                    .text_color(colors.muted)
                    .tooltip(move |window, cx| Tooltip::new(format!("{edge_n} 个关联字段")).build(window, cx))
                    .child(app_icon(AppIcon::Link, 10., colors.muted))
                    .child(format!("{edge_n}")),
            );
        }
    }
    node = node.child(header);

    // 缩小折叠（§六.26）：只渲染色带+表头（紧凑节点）；关联字段计数已内嵌表头；
    // 字段滚动状态保留（放大恢复）。
    if collapsed {
        return node;
    }

    match meta.status {
        ErLoadStatus::Loaded => {
            if meta.columns.is_empty() {
                node = node.child(
                    div()
                        .h(px(NODE_FIELD_ROW))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("暂无可见字段"),
                );
            } else {
                let (area, is_long) = er_field_area(tab_id, meta, nv, highlight, card_color, colors, cx);
                node = node.child(area);
                if is_long {
                    let n = meta.columns.len();
                    let (row0, row1) = visible_row_range(n, nv.scroll_px);
                    node = node.child(
                        div()
                            .h(px(LONG_FOOTER))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .border_t_1()
                            .border_color(colors.border_soft)
                            .text_size(px(10.))
                            .text_color(colors.muted)
                            .child(format!("{}–{} / {} 个字段", row0 + 1, row1 + 1, n)),
                    );
                }
            }
        }
        ErLoadStatus::NotLoaded | ErLoadStatus::Loading => {
            // 静态骨架（约 3 行）+ 状态文字；不启动动画定时器。
            node = node.child(
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children((0..3).map(|_| div().h(px(8.)).rounded_full().bg(colors.border_soft))),
            );
            node = node.child(
                div()
                    .px(px(10.))
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(if meta.status == ErLoadStatus::Loading { "字段加载中…" } else { "待读取…" }),
            );
        }
        ErLoadStatus::Failed => {
            let retry_table = name.clone();
            node = node.child(
                div()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.))
                    .text_color(rgb(0xef4444))
                    .child("字段加载失败")
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(colors.radius)
                            .border_1()
                            .border_color(colors.border)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.hover))
                            .text_color(colors.text)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(graph) = this.er_graphs.get_mut(&tab_id) {
                                        if let Some(t) = graph.tables.iter_mut().find(|t| t.name == retry_table) {
                                            if t.status == ErLoadStatus::Failed {
                                                t.status = ErLoadStatus::NotLoaded;
                                                // 用结构化身份入队请求；结果归并也按身份，不从展示名反推。
                                                this.er_pending_columns
                                                    .entry(tab_id)
                                                    .or_default()
                                                    .insert(t.reference.clone());
                                            }
                                        }
                                    }
                                    cx.notify();
                                }),
                            )
                            .child("重试"),
                    ),
            );
        }
    }

    node
}

/// 键盘滚动：滚动「当前选中表」的字段列表（PageUp/PageDown/↑/↓）。
fn scroll_focused_fields(tab_id: TabId, this: &mut NavicatMain, delta_px: f32) {
    let Some(name) = this.er_canvas.borrow().er_selected_table.get(&tab_id).cloned().flatten() else {
        return;
    };
    let Some(graph) = this.er_graphs.get(&tab_id) else {
        return;
    };
    let Some(table) = graph.tables.iter().find(|t| t.name == name) else {
        return;
    };
    let n = table.columns.len();
    if n <= MAX_FIELD_ROWS {
        return;
    }
    let max_scroll = (n - MAX_FIELD_ROWS) as f32 * NODE_FIELD_ROW;
    let mut canvas = this.er_canvas.borrow_mut();
    let e = canvas.er_node_scroll_px.entry((tab_id, name)).or_insert(0.0);
    *e = (*e + delta_px).clamp(0.0, max_scroll);
}

/// 计算把 `field_idx` 精确送入可视区（尽量居中、不越界）的目标滚动偏移（px）。
/// 与「滚到最顶/最底」不同：定位应让目标行进入可见区并尽量居中（§4.2）。
fn hidden_field_scroll(field_idx: usize, ncols: usize) -> f32 {
    let max_scroll = (ncols.saturating_sub(MAX_FIELD_ROWS) as f32 * NODE_FIELD_ROW).max(0.0);
    let center_scroll = field_idx as f32 * NODE_FIELD_ROW
        - (MAX_FIELD_ROWS as f32 / 2.0) * NODE_FIELD_ROW
        + NODE_FIELD_ROW / 2.0;
    center_scroll.clamp(0.0, max_scroll)
}

/// 定位隐藏字段：若该方向确实有隐藏的关系字段，则滚动该表让目标字段**精确进入可视区**
/// （居中，非仅滚到最顶/最底），并高亮该字段行（§4.2）。
/// `to_top=true` 处理上方隐藏字段，`to_bottom=true` 处理下方隐藏字段。
/// 无对应方向字段时不做任何滚动。
fn reveal_hidden_fields(tab_id: TabId, this: &mut NavicatMain, table: &str, to_top: bool, to_bottom: bool) {
    let Some(graph) = this.er_graphs.get(&tab_id) else {
        return;
    };
    let Some(t) = graph.tables.iter().find(|t| t.name == table) else {
        return;
    };
    let n = t.columns.len();
    if n == 0 || n <= MAX_FIELD_ROWS {
        return;
    }
    // 找出该方向上「隐藏且有关联」的字段在下标集合。无字段时不动。
    // 端点字段集合（edge_columns）存在 scene，这里从图列 + 状态推导：直接取可见区间边界外的列。
    let current_scroll = this.er_canvas.borrow().er_node_scroll_px.get(&(tab_id, table.to_string())).copied().unwrap_or(0.0);
    let (row0, row1) = visible_row_range(n, current_scroll);
    let candidates: Vec<usize> = (0..n)
        .filter(|&ci| {
            if to_top {
                ci < row0
            } else if to_bottom {
                ci > row1
            } else {
                false
            }
        })
        .collect();
    let Some(&field_idx) = candidates.first() else {
        return; // 该方向没有隐藏字段
    };
    // 精确滚动到目标字段（尽量居中，但不越界）：目标行进入可视区即满足定位要求。
    let target = hidden_field_scroll(field_idx, n);
    this.er_canvas.borrow_mut().er_node_scroll_px.insert((tab_id, table.to_string()), target);
    // 记录高亮字段（渲染时该行高亮；空白/Esc/其它交互后清除）。
    this.er_canvas.borrow_mut().er_field_highlights
        .insert(tab_id, (table.to_string(), t.columns[field_idx].name.clone()));
}
