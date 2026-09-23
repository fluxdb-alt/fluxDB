// ER 画布视口交互层（er-ui-relationship-canvas.md §7/§8）。
//
// 画布 div 树：尺寸探针 + 连线绘制层 + 节点层 + 汇总徽标层。
// - 空白拖动平移画布；表头按住拖动单表（4px 阈值，未移动释放 = 选择）；
//   字段区滚轮只滚字段；滚动条拖拽只调滚动；按钮/徽标点击不穿透。
// - 每帧由 scene + 可变环境（坐标/滚动/选中/固定）物化 ErFrame：
//   节点网格索引查询可见节点，边按两端卡片联合包围盒保守筛选后解析字段端口并路由。
// - 拖动/滚动只更新对应状态并 notify，不重建场景拓扑、不每帧全图布局。

/// 折线拐角圆角半径（px）。
const EDGE_CORNER_R: f32 = 7.0;
/// 字段端口点半径（px）。
const PORT_R: f32 = 3.0;
/// 节点拖动开始阈值（px）。
const NODE_DRAG_THRESHOLD: f32 = 4.0;

/// 画布主视图。scene/frame 为 None 时仍渲染探针以测量画布尺寸并触发建场景。
#[allow(clippy::too_many_arguments)]
fn er_canvas_view(
    tab_id: TabId,
    scene: Option<Rc<ErScene>>,
    frame: Option<ErFrame>,
    viewport: ErViewport,
_canvas_size: Option<(f32, f32)>,
    expandable: bool,
    field_highlight: Option<(String, String)>,
    overlay: ErCanvasOverlay,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let current_tab = tab_id;
    let selected_idx = frame.as_ref().and_then(|f| f.node_views.iter().find(|v| v.selected).map(|v| v.idx));
    let er_scale = viewport.safe_scale();
    let er_inv = 1.0 / er_scale;
    // 可见节点的汇总徽标数据：(世界位置, 表名, 方向, 计数)。
    // 卡片固定屏幕尺寸：内部像素偏移须 *inv 转世界单位（与连线锚点同一换算），否则缩放下徽标漂移。
    let collapsed = er_scale < ER_COLLAPSE_SCALE;
    let badges: Vec<(f32, f32, String, bool, usize)> = frame
        .as_ref()
        .map(|f| {
            let mut out = Vec::new();
            for &i in &f.visible_nodes {
                let v = &f.node_views[i];
                let meta = {
                    // scene 与 frame 同源，按 idx 取回 meta。
                    let s = scene.as_ref().unwrap();
                    &s.nodes[v.idx]
                };
                let bx = v.x + v.width * er_inv;
                if collapsed {
                    // 折叠态：关联字段计数内嵌于紧凑节点内（node_view），不再悬空徽标，
                    // 避免遮挡连线/破坏概览；折叠端点在底边，由连线锚点层表示。
                    continue;
                }
                let (top, bottom) = summary_counts(meta, v.scroll_px);
                if top > 0 {
                    out.push((bx, v.y + field_viewport_offset() * er_inv, v.name.clone(), true, top));
                }
                if bottom > 0 {
                    let rows = meta.columns.len().min(MAX_FIELD_ROWS) as f32;
                    out.push((
                        bx,
                        v.y + (field_viewport_offset() + rows * NODE_FIELD_ROW) * er_inv,
                        v.name.clone(),
                        false,
                        bottom,
                    ));
                }
            }
            out
        })
        .unwrap_or_default();

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .overflow_hidden()
                .bg(colors.canvas_bg)
                // 尺寸探针：回报画布真实可见区域并触发建场景。
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(ErCanvasProbe { tab_id, view: cx.entity().clone() }),
                )
                // 空白：点击先试命中关系线（选中说明），否则取消选择并开始平移。
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        // 世界坐标 = (屏幕坐标 - 画布原点 - pan) / scale（绘制 `world*scale + pan + origin`，命中须一致）。
                        let (ox, oy) = this.er_canvas.borrow().er_canvas_origins.get(&current_tab).copied().unwrap_or((0.0, 0.0));
                        let vp = this.er_canvas.borrow().er_viewports.get(&current_tab).copied().unwrap_or_default();
                        let wx = vp.to_world_x(f32::from(event.position.x) - ox);
                        let wy = vp.to_world_y(f32::from(event.position.y) - oy);
                        // 关系线命中：只扫本帧可见折线（有界）。命中返回说明文本 + 逻辑关系 id
                        // （物理外键 id 为 None）。克隆避免持有对 this 的不可变借用。
                        let hit_edge = this.er_canvas.borrow_mut().er_frame_edges
                            .get(&current_tab)
                            .and_then(|edges| {
                                edges
                                    .iter()
                                    .filter(|e| e.points.windows(2).any(|w| {
                                        dist_point_seg(wx, wy, w[0], w[1]) <= 6.0
                                    }))
                                    .next()
                            })
                            .cloned();
                        this.mark_er_interacted(current_tab);
                        if let Some(edge) = hit_edge {
                            // 本地逻辑关系：点击直接展开抽屉并选中该关系（供编辑/确认）；物理外键仅提示。
                            if let Some(rel_id) = edge.logical_rel_id {
                                this.er_select_relationship(current_tab, rel_id, cx);
                            } else {
                                this.show_message(edge.desc, AppMessageKind::Info, cx);
                            }
                            cx.stop_propagation();
                            cx.notify();
                            return;
                        }
                        if this.er_canvas.borrow().er_selected_table.get(&current_tab).cloned().flatten().is_some() {
                            this.er_canvas.borrow_mut().er_selected_table.insert(current_tab, None);
                        }
                        this.er_canvas.borrow_mut().er_field_highlights.remove(&current_tab);
                        // 借用注意：左值 borrow_mut 与右值内 borrow 同语句=双重借用 panic。
                        // 先取 pan 快照（右值），再写 viewport_drag。
                        let (drag_pan_x, drag_pan_y) = {
                            let c = this.er_canvas.borrow();
                            (
                                c.er_viewports.get(&current_tab).map(|v| v.pan_x).unwrap_or(0.0),
                                c.er_viewports.get(&current_tab).map(|v| v.pan_y).unwrap_or(0.0),
                            )
                        };
                        this.er_canvas.borrow_mut().er_viewport_drag = Some((
                            current_tab,
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                            drag_pan_x,
                            drag_pan_y,
                        ));
                        cx.notify();
                    }),
                )
                .on_mouse_move(
                    cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        // 字段连线拖拽：更新光标 + 吸附目标行；其余拖拽行为在拖线期间全部挂起。
                        let link_tab = this.er_canvas.borrow().er_link_drag.as_ref().map(|d| d.tab);
                        if link_tab == Some(current_tab) {
                            if event.pressed_button != Some(MouseButton::Left) {
                                this.er_canvas.borrow_mut().er_link_drag = None;
                                cx.notify();
                                return;
                            }
                            let (ox, oy) = this.er_canvas.borrow().er_canvas_origins.get(&current_tab).copied().unwrap_or((0.0, 0.0));
                            let vp = this.er_canvas.borrow().er_viewports.get(&current_tab).copied().unwrap_or_default();
                            let sx = f32::from(event.position.x) - ox;
                            let sy = f32::from(event.position.y) - oy;
                            // 吸附：命中真实字段行即吸附该行端口（靠光标近的一侧）；否则跟手。
                            let (nodes, nodes_scene) = {
                                let c = this.er_canvas.borrow();
                                (
                                    c.er_frame_nodes.get(&current_tab).cloned().unwrap_or_default(),
                                    this.er_scenes.get(&current_tab).cloned(),
                                )
                            };
                            let wx = vp.to_world_x(sx);
                            let wy = vp.to_world_y(sy);
                            let target = nodes_scene.as_ref().and_then(|s| {
                                field_row_at(&nodes, &s.nodes, wx, wy, vp.safe_scale()).map(|(t, c)| {
                                    let side = nearest_port_side(&nodes, &t, vp.safe_scale(), wx);
                                    (t, c, side)
                                })
                            });
                            // 过滤：不允许连回源表自身（自关联走已有编辑入口）。
                            let from_table = this.er_canvas.borrow().er_link_drag.as_ref().map(|d| d.from_table.clone());
                            let target = target.filter(|(t, _, _)| from_table.as_deref() != Some(t.as_str()));
                            // 未命中字段行：若落在另一张表卡片上（表头/页脚/字段未加载），
                            // 记录目标表 + 空列名 → 释放后只预填左右表，字段留给表单选。
                            let target = target.or_else(|| {
                                node_at(&nodes, wx, wy, vp.safe_scale())
                                    .filter(|t| from_table.as_deref() != Some(t.as_str()))
                                    .map(|t| (t, String::new(), ErPortSide::Left))
                            });
                            let mut canvas = this.er_canvas.borrow_mut();
                            if let Some(d) = canvas.er_link_drag.as_mut() {
                                d.cursor = (sx, sy);
                                d.target = target;
                            }
                            drop(canvas);
                            cx.notify();
                            return;
                        }
                        // 节点拖动：超阈值才开始移动该表；连线端点/邻接边随坐标每帧重解析。
                        let drag_snapshot = this.er_canvas.borrow_mut().er_node_drag.clone();
                        if let Some((tab, name, dx, dy, ox, oy, moved)) = drag_snapshot {
                            if tab == current_tab {
                                let cxp = f32::from(event.position.x);
                                let cyp = f32::from(event.position.y);
                                let moved_now =
                                    moved || ((cxp - dx).abs() > NODE_DRAG_THRESHOLD || (cyp - dy).abs() > NODE_DRAG_THRESHOLD);
                                if event.pressed_button != Some(MouseButton::Left) {
                                    this.finish_node_drag(current_tab);
                                } else if moved_now {
                                    // 借用注意：viewport 读取放在场景坐标可变借入前，避免 RefCell 冲突。
                                    let vp = this.er_canvas.borrow().er_viewports.get(&tab).copied().unwrap_or_default();
                                    if let Some(pos) = this.er_canvas.borrow_mut().er_scene_positions
                                        .get_mut(&tab)
                                        .and_then(|m| m.get_mut(&name))
                                    {
                                        // 屏幕增量 ÷ scale 才是世界增量（视图变换缩放，§六.24）。
                                        let inv = 1.0 / vp.safe_scale();
                                        *pos = (ox + (cxp - dx) * inv, oy + (cyp - dy) * inv);
                                    }
                                    this.er_canvas.borrow_mut().er_node_drag = Some((tab, name.clone(), dx, dy, ox, oy, true));
                                    this.er_canvas.borrow_mut().er_pinned.entry(tab).or_default().insert(name);
                                    this.mark_er_interacted(tab);
                                }
                                cx.notify();
                            }
                        }
                        let viewport_drag = this.er_canvas.borrow_mut().er_viewport_drag.clone();
                        if let Some((tab, down_x, down_y, pan_x, pan_y)) = viewport_drag {
                            if event.pressed_button != Some(MouseButton::Left) {
                                this.er_canvas.borrow_mut().er_viewport_drag = None;
                                cx.notify();
                            } else if tab == current_tab {
                                let mut canvas = this.er_canvas.borrow_mut();
                                let vp = canvas.er_viewports.entry(tab).or_default();
                                vp.pan_x = pan_x + (f32::from(event.position.x) - down_x);
                                vp.pan_y = pan_y + (f32::from(event.position.y) - down_y);
                                drop(canvas);
                                cx.notify();
                            }
                        }
                        let scroll_drag = this.er_canvas.borrow_mut().er_scroll_drag.clone();
                        if let Some((tab, _name)) = scroll_drag {
                            if tab == current_tab && event.pressed_button != Some(MouseButton::Left) {
                                this.er_canvas.borrow_mut().er_scroll_drag = None;
                                cx.notify();
                            }
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _event: &gpui::MouseUpEvent, window, cx| {
                        // 字段连线释放：命中目标字段端口 → 打开关系表单并预填该字段配对（§手动连线）。
                        let link = this.er_canvas.borrow_mut().er_link_drag.take();
                        if let Some(d) = link {
                            if d.tab == current_tab {
                                let from_t = d.from_table.clone();
                                let from_c = d.from_column.clone();
                                this.mark_er_interacted(current_tab);
                                let self_drop = d
                                    .target
                                    .as_ref()
                                    .is_some_and(|(t, c, _)| t == &from_t && c == &from_c);
                                match d.target.as_ref() {
                                    Some((to_t, to_c, _)) if !self_drop => {
                                        this.er_prepare_new_relationship_from_ports(
                                            current_tab, &from_t, &from_c, to_t, to_c, window, cx,
                                        );
                                    }
                                    _ => {
                                        this.show_message(
                                            String::from("拖到目标字段上松开可创建关系"),
                                            AppMessageKind::Info,
                                            cx,
                                        );
                                    }
                                }
                                // 释放后清理悬停记录，避免源端口圆点残留。
                                let mut canvas = this.er_canvas.borrow_mut();
                                let clear_hover = canvas
                                    .er_row_hover
                                    .get(&current_tab)
                                    .cloned()
                                    .flatten()
                                    .is_some_and(|(t, _)| t == from_t);
                                if clear_hover {
                                    canvas.er_row_hover.remove(&current_tab);
                                }
                                drop(canvas);
                                cx.notify();
                                return;
                            }
                        }
                        // 未移动的表头按下 = 选择该表（切换）。
                        // 借用注意：先克隆快照再判断，不在 if 条件里持有 RefCell 借用。
                        let drag_snapshot = this.er_canvas.borrow_mut().er_node_drag.clone();
                        if let Some((tab, name, _, _, _, _, moved)) = drag_snapshot {
                            if tab == current_tab && !moved {
                                let cur = this.er_canvas.borrow().er_selected_table.get(&tab_id).cloned().flatten();
                                this.er_canvas.borrow_mut().er_selected_table.insert(
                                    tab,
                                    if cur.as_deref() == Some(name.as_str()) { None } else { Some(name.clone()) },
                                );
                                this.mark_er_interacted(tab);
                            }
                            this.finish_node_drag(current_tab);
                        }
                        let clear_viewport_drag = this.er_canvas.borrow().er_viewport_drag.as_ref().is_some_and(|(tab, ..)| *tab == current_tab);
                        if clear_viewport_drag {
                            this.er_canvas.borrow_mut().er_viewport_drag = None;
                        }
                        let clear_scroll_drag = this.er_canvas.borrow().er_scroll_drag.as_ref().is_some_and(|(tab, _)| *tab == current_tab);
                        if clear_scroll_drag {
                            this.er_canvas.borrow_mut().er_scroll_drag = None;
                        }
                        cx.notify();
                    }),
                )
                .on_key_down(
                    cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        let key = &event.keystroke.key;
                        let delta = match key.as_str() {
                            "PageDown" | "Down" => Some(NODE_FIELD_ROW),
                            "PageUp" | "Up" => Some(-NODE_FIELD_ROW),
                            "escape" => {
                                this.er_canvas.borrow_mut().er_field_highlights.remove(&current_tab);
                                // Esc 取消进行中的字段连线拖拽（不落库）。
                                this.er_canvas.borrow_mut().er_link_drag = None;
                                if this.er_canvas.borrow().er_selected_table.get(&current_tab).cloned().flatten().is_some() {
                                    this.er_canvas.borrow_mut().er_selected_table.insert(current_tab, None);
                                    cx.notify();
                                }
                                None
                            }
                            _ => None,
                        };
                        if let Some(d) = delta {
                            scroll_focused_fields(current_tab, this, d);
                            cx.notify();
                        }
                    }),
                )
                // Ctrl/Cmd + 滚轮：围绕光标缩放（视图变换缩放，§六.23-24）。
                // 普通滚轮留给字段区滚动，避免劫持表内字段滚动；工具栏另有 +/- 按钮与百分比。
                .on_scroll_wheel(
                    cx.listener(move |this, event: &gpui::ScrollWheelEvent, _, cx| {
                        // Cmd(mac)/Ctrl(win/linux) 为secondary修饰；此处用它作为缩放快捷键，三平台一致。
                        if !event.modifiers.secondary() {
                            return;
                        }
                        let delta_y = match event.delta {
                            gpui::ScrollDelta::Lines(p) => p.y,
                            gpui::ScrollDelta::Pixels(p) => p.y.as_f32() / NODE_FIELD_ROW,
                        };
                        if delta_y == 0.0 {
                            return;
                        }
                        let (ox, oy) = this.er_canvas.borrow_mut().er_canvas_origins
                            .get(&current_tab)
                            .copied()
                            .unwrap_or((0.0, 0.0));
                        let anchor = (
                            f32::from(event.position.x) - ox,
                            f32::from(event.position.y) - oy,
                        );
                        let factor = if delta_y > 0.0 { 1.15 } else { 1.0 / 1.15 };
                        let mut canvas = this.er_canvas.borrow_mut();
                        let vp = canvas.er_viewports.entry(current_tab).or_default();
                        vp.zoom_around(anchor, factor);
                        drop(canvas);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                // 触摸板双指捏合：围绕捏合中心缩放（PinchEvent.delta 为增量，正=放大）。
                .on_pinch(
                    cx.listener(move |this, event: &gpui::PinchEvent, _, cx| {
                        if event.delta == 0.0 {
                            return;
                        }
                        let (ox, oy) = this.er_canvas.borrow_mut().er_canvas_origins
                            .get(&current_tab)
                            .copied()
                            .unwrap_or((0.0, 0.0));
                        let anchor = (
                            f32::from(event.position.x) - ox,
                            f32::from(event.position.y) - oy,
                        );
                        let factor = 1.0 + event.delta;
                        let mut canvas = this.er_canvas.borrow_mut();
                        let vp = canvas.er_viewports.entry(current_tab).or_default();
                        vp.zoom_around(anchor, factor);
                        drop(canvas);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                // 连线层。
                .child(
                    div().absolute().inset_0().child(ErCanvas {
                        edges: frame.as_ref().map(|f| f.edge_views.clone()).unwrap_or_default(),
                        selected_idx,
                        edge_color: colors.muted,
                        muted: colors.muted,
                        accent: er_accent_color(cx),
                        pan_x: viewport.pan_x,
                        pan_y: viewport.pan_y,
                        scale: viewport.scale,
                    }),
                )
                // 节点层：只挂载可见节点；卡片内点击 stop_propagation 不穿透画布。
                .children(frame.as_ref().map(|f| {
                    let s = scene.as_ref().unwrap();
                    f.visible_nodes.iter().map(|&i| {
                        let nv = &f.node_views[i];
                        let meta = &s.nodes[i];
                        let card_color = er_component_color(colors, meta.color_idx);
                        // 该表的高亮字段（从汇总端口定位到精确字段后高亮该行）。
                        let hi = field_highlight
                            .as_ref()
                            .filter(|(t, _)| t == &meta.name)
                            .map(|(_, fld)| fld.as_str());
                        node_view(tab_id, meta, nv, expandable, viewport, hi, card_color, colors, cx)
                    })
                }).into_iter().flatten())
                // 字段端口层：悬停字段行浮现端口圆点；按住拖出临时连线（§手动连线）。
                // 必须在节点层之后（浮层在上，圆点才能点/拖）。空图/折叠态自动退化为空层。
                .children(er_port_overlay(
                    tab_id,
                    frame.as_ref(),
                    scene.as_deref(),
                    viewport,
                    &overlay,
                    colors,
                    cx,
                ))
                // 汇总徽标层：字段离屏时的上下汇总端口标记（点击定位，§4.2）。
                .children(badges.into_iter().map(|(bx, by, table, is_top, count)| {
                    let badge_tab = tab_id;
                    let tip = if is_top {
                        format!("上方隐藏 {count} 个关联字段，点击滚到顶部")
                    } else {
                        format!("下方隐藏 {count} 个关联字段，点击滚到底部")
                    };
                    div()
                        .absolute()
                        .left(px(bx * viewport.safe_scale() + viewport.pan_x - 2.))
                        .top(px(by * viewport.safe_scale() + viewport.pan_y - 9.))
                        .h(px(18.))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel_bg)
                        .text_size(px(10.))
                        .text_color(colors.muted)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.hover))
                        .id(format!("er-badge-{}-{}-{}", tab_id.0, table, if is_top { "t" } else { "b" }))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                // 精确定位隐藏字段：上端滚上方隐藏字段、下端滚下方隐藏字段。
                                reveal_hidden_fields(badge_tab, this, &table, is_top, !is_top);
                                cx.notify();
                            }),
                        )
                        .child(app_icon(if is_top { AppIcon::ArrowUp } else { AppIcon::ArrowDown }, 11., colors.muted))
                        .child(format!("{count}"))
                })))
}

/// 字段端口浮层：圆点（悬停浮现）+ 连线拖拽临时折线。
/// 端口圆点不能放进字段行（卡片 overflow_hidden 会裁掉），必须在画布层浮层绘制。
/// 圆点位置 = 本帧 class=field_row 的真实几何（resolve_field_port 与锚点同源）。
fn er_port_overlay(
    tab_id: TabId,
    frame: Option<&ErFrame>,
    scene: Option<&ErScene>,
    viewport: ErViewport,
    overlay: &ErCanvasOverlay,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Vec<AnyElement> {
    let scale = viewport.safe_scale();
    if scale < ER_COLLAPSE_SCALE {
        return Vec::new(); // 折叠态无字段行端口
    }
    let (Some(f), Some(s)) = (frame, scene) else {
        return Vec::new();
    };
    let inv = 1.0 / scale;
    let to_screen = |x: f32, y: f32| (x * scale + viewport.pan_x, y * scale + viewport.pan_y);

    // 需要浮现圆点的行：悬停行 / 悬停端口 / 拖拽源行。
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(r) = &overlay.row_hover {
        rows.push(r.clone());
    }
    if let Some(r) = &overlay.port_hover {
        if !rows.contains(r) {
            rows.push(r.clone());
        }
    }
    if let Some(d) = &overlay.link_drag {
        let r = (d.from_table.clone(), d.from_column.clone());
        if !rows.contains(&r) {
            rows.push(r);
        }
    }

    let mut dots: Vec<AnyElement> = Vec::new();
    let mouse_down_side = if let Some(d) = &overlay.link_drag {
        Some(d.from_side)
    } else {
        None
    };
    for (table, column) in rows.iter().map(|(t, c)| (t.clone(), c.clone())) {
        // 该行是否属于可见节点。
        let Some(port_l) = resolve_field_port(&f.node_views, &s.nodes, &table, &column, ErPortSide::Left, scale) else {
            continue;
        };
        let port_r = resolve_field_port(&f.node_views, &s.nodes, &table, &column, ErPortSide::Right, scale);
        for (side, p) in [(ErPortSide::Left, Some(port_l)), (ErPortSide::Right, port_r)] {
            let Some((wx, wy)) = p else { continue };
            let (sx, sy) = to_screen(wx, wy);
            let is_source = overlay
                .link_drag
                .as_ref()
                .is_some_and(|d| d.from_table == table && d.from_column == column && d.from_side == side);
            let is_target = overlay
                .link_drag
                .as_ref()
                .and_then(|d| d.target.as_ref())
                .is_some_and(|(t, c, sd)| *t == table && *c == column && *sd == side);
            let active = is_source || is_target;
            let dragging_this = mouse_down_side == Some(side) && is_source;
            let dot_c = if active { er_accent_color(cx) } else { colors.muted };
            let tip_text = format!("拖出连线：{table}.{column}");
            let hov_table = table.clone();
            let hov_col = column.clone();
            let down_table = table.clone();
            let down_col = column.clone();
            dots.push(
                div()
                    .absolute()
                    .left(px(sx - 7.0))
                    .top(px(sy - 7.0))
                    .size(px(if active { 14.0 } else { 10.0 }))
                    .left(px(if active { sx - 7.0 } else { sx - 5.0 }))
                    .top(px(if active { sy - 7.0 } else { sy - 5.0 }))
                    .rounded_full()
                    .bg(dot_c)
                    .border_1()
                    .border_color(colors.panel_bg)
                    .when(active, |d| d.border_2())
                    .hover(|d| d.bg(er_accent_color(cx)))
                    .when(!dragging_this, |d| d.cursor(gpui::CursorStyle::Crosshair))
                    .id(format!("er-port-{}-{}-{}-{}", tab_id.0, table, column, if side == ErPortSide::Left { "l" } else { "r" }))
                    .tooltip(move |window, cx| Tooltip::new(tip_text.clone()).build(window, cx))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        // 借用：先 clone 目标值，再取可变借用写回（避免临时 RefMut 存活到 if 内）。
                        let value = if *hovered { Some((hov_table.clone(), hov_col.clone())) } else { None };
                        let mut canvas = this.er_canvas.borrow_mut();
                        let entry = canvas.er_port_hover.entry(tab_id).or_default();
                        if *hovered {
                            *entry = value;
                        } else if entry.as_ref() == Some(&(hov_table.clone(), hov_col.clone())) {
                            *entry = None;
                        }
                        drop(canvas);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            // 从端口开始拖线：记录源端口 + 光标，进入拖线态（不触发节点拖动/平移）。
                            let (ox, oy) = this.er_canvas.borrow().er_canvas_origins.get(&tab_id).copied().unwrap_or((0.0, 0.0));
                            this.er_canvas.borrow_mut().er_link_drag = Some(ErLinkDrag {
                                tab: tab_id,
                                from_table: down_table.clone(),
                                from_column: down_col.clone(),
                                from_side: side,
                                cursor: (f32::from(event.position.x) - ox, f32::from(event.position.y) - oy),
                                target: None,
                            });
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
    }

    // 临时连线：源端口 → 光标（未吸附）或目标端口（吸附）。
    if let Some(d) = &overlay.link_drag {
        if let Some((sx, sy)) = resolve_field_port(&f.node_views, &s.nodes, &d.from_table, &d.from_column, d.from_side, scale) {
            let (a_sx, a_sy) = to_screen(sx, sy);
            let (b_sx, b_sy) = match &d.target {
                Some((t, c, sd)) => match resolve_field_port(&f.node_views, &s.nodes, t, c, *sd, scale) {
                    Some((wx, wy)) => to_screen(wx, wy),
                    None => d.cursor,
                },
                None => d.cursor,
            };
            let out = 18.0;
            let ax = if d.from_side == ErPortSide::Right { a_sx + out } else { a_sx - out };
            let pts = [(a_sx, a_sy), (ax, a_sy), (ax, b_sy), (b_sx, b_sy)];
            let _ = inv;
            dots.push(
        ErLinkPreview {
            points: pts,
            color: er_accent_color(cx),
        }
        .into_any_element(),
    );
        }
    }
    let _ = inv;
    dots
}

/// 点到线段距离（关系线命中用）。
fn dist_point_seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ax, ay) = a;
    let (bx, by) = b;
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// 尺寸探针：回报画布真实尺寸；条件满足则触发建场景（见 canvas.rs）。
struct ErCanvasProbe {
    tab_id: TabId,
    view: gpui::Entity<NavicatMain>,
}

impl IntoElement for ErCanvasProbe {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ErCanvasProbe {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> {
        Some(("er-canvas-probe", self.tab_id.0).into())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let tab_id = self.tab_id;
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return;
        }
        self.view.update(cx, |this, cx| {
            {
                // 借用注意：paint 属 GPUI「cannot unwind」路径，任何 RefCell 双重借用都会直接
                // abort（非可捕获 panic）。先只读比较，再分开写，避免条件里临时 RefMut 存活。
                let mut canvas = this.er_canvas.borrow_mut();
                let size_changed = canvas.er_canvas_sizes.get(&tab_id) != Some(&size);
                if size_changed {
                    canvas.er_canvas_sizes.insert(tab_id, size);
                }
                // 记录画布区在窗口中的原点：空白点击命中测试需用它把屏幕坐标换算到世界坐标
                // （绘制已含画布原点偏移，命中不能只减 pan 而遗漏原点，否则画布不在窗口原点时错位）。
                let ori = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                let ori_changed = canvas.er_canvas_origins.get(&tab_id) != Some(&ori);
                if ori_changed {
                    canvas.er_canvas_origins.insert(tab_id, ori);
                }
            } // drop(canvas)：notify 前不持有任何 RefCell 借用
            maybe_build_scene(tab_id, this);
            // 首次显示：尺寸/场景就绪后自动适配居中一次（此后尊重用户视口，§六.23）。
            this.er_fit_once_on_first_show(tab_id);
            cx.notify();
        });
    }
}

/// 连线绘制层：圆角折线 + 端点标记。字段锚点画柔和小圆点；
/// 汇总/待加载/缺失锚点画小方块（与真实字段端口明显区别，§4.2）。
struct ErCanvas {
    edges: Vec<ErEdgeView>,
    selected_idx: Option<usize>,
    edge_color: gpui::Rgba,
    muted: gpui::Rgba,
    accent: gpui::Rgba,
    pan_x: f32,
    pan_y: f32,
    scale: f32,
}

impl IntoElement for ErCanvas {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ErCanvas {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let ox = bounds.origin.x;
        let oy = bounds.origin.y;
        let scale = self.scale.clamp(ER_MIN_SCALE, ER_MAX_SCALE);
        let to_screen = |(x, y): (f32, f32)| {
            point(
                px(x * scale + self.pan_x + f32::from(ox)),
                px(y * scale + self.pan_y + f32::from(oy)),
            )
        };
        for e in &self.edges {
            let highlighted = self
                .selected_idx
                .map(|s| e.from_idx == s || e.to_idx == s)
                .unwrap_or(false);
            // 普通线：展开 1.25px、折叠概览 1.5px（略粗、更可辨但压不过表名）；高亮 2px。
            let collapsed = self.scale < ER_COLLAPSE_SCALE;
            let (width, color) = if highlighted {
                (2.0, self.accent)
            } else if collapsed {
                (1.5, self.edge_color)
            } else {
                (1.25, self.edge_color)
            };
            if e.points.len() < 2 {
                continue;
            }
            let mut builder = gpui::PathBuilder::stroke(px(width));
            let pts: Vec<gpui::Point<gpui::Pixels>> = e.points.iter().map(|p| to_screen(*p)).collect();
            builder.move_to(pts[0]);
            // 正交折线拐角 7px 圆角：拐点前后各截 r，二次曲线过拐点控制点（§5.1）。
            let r = EDGE_CORNER_R;
            for i in 1..pts.len() {
                let prev = pts[i - 1];
                let cur = pts[i];
                let seg_len = (((cur.x - prev.x).as_f32()).powi(2) + ((cur.y - prev.y).as_f32()).powi(2)).sqrt();
                if i >= pts.len() - 1 || seg_len <= f32::EPSILON {
                    builder.line_to(cur);
                    continue;
                }
                let next = pts[i + 1];
                let ux = (cur.x - prev.x).as_f32() / seg_len;
                let uy = (cur.y - prev.y).as_f32() / seg_len;
                let seg2 = (((next.x - cur.x).as_f32()).powi(2) + ((next.y - cur.y).as_f32()).powi(2)).sqrt();
                if seg2 <= f32::EPSILON {
                    builder.line_to(cur);
                    continue;
                }
                let ux2 = (next.x - cur.x).as_f32() / seg2;
                let uy2 = (next.y - cur.y).as_f32() / seg2;
                // 路由可能包含同向的出走段与车道段；只有真正的直角才画圆角。
                // 相邻短段共用半径时也须各限制一半，避免控制点越过下一拐点形成尖刺。
                if (ux * uy2 - uy * ux2).abs() < 0.01 {
                    builder.line_to(cur);
                    continue;
                }
                let rr = r.min(seg_len / 2.0).min(seg2 / 2.0);
                let cut = point(px(cur.x.as_f32() - ux * rr), px(cur.y.as_f32() - uy * rr));
                let enter = point(px(cur.x.as_f32() + ux2 * rr), px(cur.y.as_f32() + uy2 * rr));
                builder.line_to(cut);
                builder.curve_to(enter, cur);
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
            // 端点标记：两端锚点。
            for anchor in [e.from_anchor, e.to_anchor] {
                let c = to_screen((anchor.x, anchor.y));
                // 折叠态（紧凑节点）：全部端点退到卡片底边，统一用柔和圆点（无字段端口可比，
                // 取消深灰方块造成的接缝处突兀）；非折叠态保留方块区分汇总/待加载端口。
                let collapsed = self.scale < ER_COLLAPSE_SCALE;
                match anchor.kind {
                    ErAnchorKind::Field => {
                        // 小圆点：默认柔和（muted），选中表相关用强调色。
                        draw_circle(window, c, PORT_R, if highlighted { self.accent } else { self.muted });
                    }
                    ErAnchorKind::SummaryTop | ErAnchorKind::SummaryBottom | ErAnchorKind::Pending | ErAnchorKind::Missing => {
                        if collapsed {
                            draw_circle(
                                window,
                                c,
                                PORT_R,
                                if highlighted { self.accent } else { self.edge_color },
                            );
                        } else {
                            // 小方块标记：与真实字段端口区别。
                            draw_square(window, c, PORT_R + 1.0, self.muted);
                        }
                    }
                }
            }
        }
    }
}

/// 连线拖拽临时折线：虚线强调色，屏幕坐标（画布内）折线。
/// 与 ErCanvas 同一绘制约定：`bounds.origin + 点`（点已在画布内坐标）。
struct ErLinkPreview {
    points: [(f32, f32); 4],
    color: gpui::Rgba,
}

impl IntoElement for ErLinkPreview {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ErLinkPreview {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let ox = f32::from(bounds.origin.x);
        let oy = f32::from(bounds.origin.y);
        let pts: Vec<gpui::Point<gpui::Pixels>> = self
            .points
            .iter()
            .map(|(x, y)| point(px(ox + x), px(oy + y)))
            .collect();
        let mut builder = gpui::PathBuilder::stroke(px(1.6)).dash_array(&[px(6.), px(4.)]);
        builder.move_to(pts[0]);
        for p in pts.iter().skip(1) {
            builder.line_to(*p);
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, self.color);
        }
    }
}

/// 实心小圆（PathBuilder 填充近似：小正圆用多段弧）。
fn draw_circle(window: &mut Window, c: gpui::Point<gpui::Pixels>, r: f32, color: gpui::Rgba) {
    let mut pb = gpui::PathBuilder::fill();
    pb.move_to(point(px(c.x.as_f32() + r), c.y));
    pb.arc_to(point(px(r), px(r)), px(0.), true, true, point(px(c.x.as_f32() - r), c.y));
    pb.arc_to(point(px(r), px(r)), px(0.), true, true, point(px(c.x.as_f32() + r), c.y));
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

/// 实心小方块（汇总/待加载端口标记）。
fn draw_square(window: &mut Window, c: gpui::Point<gpui::Pixels>, half: f32, color: gpui::Rgba) {
    let x = c.x.as_f32();
    let y = c.y.as_f32();
    let mut pb = gpui::PathBuilder::fill();
    pb.move_to(point(px(x - half), px(y - half)));
    pb.line_to(point(px(x + half), px(y - half)));
    pb.line_to(point(px(x + half), px(y + half)));
    pb.line_to(point(px(x - half), px(y + half)));
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

/// 右下角小地图（§六.25）：显示世界节点分布 + 当前视口矩形，点击/拖动把世界点居中导航。
/// 叠加为画布后置兄弟（GPUI 无 z-index，后置者在上）；world bbox 由 paint 经
/// `er_world_bbox` 计算，绘制与导航共用同一归一化映射，开销有界（节点多时采样）。
fn er_minimap_view(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    const MM_W: f32 = 180.0;
    const MM_H: f32 = 120.0;
    let mtab = tab_id;
    let vp_color = er_accent_color(cx);
    let mm = div()
        .absolute()
        .bottom(px(12.))
        .right(px(12.))
        .w(px(MM_W))
        .h(px(MM_H))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                let (ox, oy, w, h) = this.er_canvas.borrow().er_minimap_rect.get(&mtab).copied().unwrap_or((0.0, 0.0, MM_W, MM_H));
                let nx = (f32::from(event.position.x) - ox) / w.max(1.0);
                let ny = (f32::from(event.position.y) - oy) / h.max(1.0);
                if this.er_minimap_center_world(mtab, nx, ny) {
                    cx.notify();
                }
            }),
        )
        .on_mouse_move(
            cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let (ox, oy, w, h) = this.er_canvas.borrow().er_minimap_rect.get(&mtab).copied().unwrap_or((0.0, 0.0, MM_W, MM_H));
                let nx = (f32::from(event.position.x) - ox) / w.max(1.0);
                let ny = (f32::from(event.position.y) - oy) / h.max(1.0);
                if this.er_minimap_center_world(mtab, nx, ny) {
                    cx.notify();
                }
            }),
        );
    mm.child(ErMinimap {
        tab_id,
        view: cx.entity().clone(),
        bg: colors.canvas_bg,
        node: er_muted_fill(colors),
        vp: vp_color,
    })
}

/// 小地图节点填充色（明暗自适应，弱于正文）。
fn er_muted_fill(colors: UiColors) -> gpui::Rgba {
    if colors.is_dark {
        hsla(0., 0., 0.5, 0.35).into()
    } else {
        hsla(0., 0., 0.35, 0.35).into()
    }
}

/// 小地图绘制元素：世界节点分布（聚合）+ 当前视口矩形。paint 回报自身窗口边界供导航归一化。
/// 尺寸由父 div 固定（w/h layout 由父级决定），此处按 Size::full 填满（同 ErCanvasProbe）。
struct ErMinimap {
    tab_id: TabId,
    view: gpui::Entity<NavicatMain>,
    bg: gpui::Rgba,
    node: gpui::Rgba,
    vp: gpui::Rgba,
}

impl IntoElement for ErMinimap {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ErMinimap {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        Some(("er-minimap", self.tab_id.0).into())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let tab_id = self.tab_id;
        let ox = f32::from(bounds.origin.x);
        let oy = f32::from(bounds.origin.y);
        let ow = f32::from(bounds.size.width).max(1.0);
        let oh = f32::from(bounds.size.height).max(1.0);
        self.view.update(cx, |this, _cx| {
            // 回报小地图窗口边界（点击导航归一化）。
            this.er_canvas.borrow_mut().er_minimap_rect.insert(tab_id, (ox, oy, ow, oh));
            // 背景。
            window.paint_quad(gpui::fill(bounds, self.bg));
            let Some((min_x, min_y, max_x, max_y)) = this.er_world_bbox(tab_id) else {
                return;
            };
            let world_w = (max_x - min_x).max(1.0);
            let world_h = (max_y - min_y).max(1.0);
            let sx = |wx: f32| ox + (wx - min_x) / world_w * ow;
            let sy = |wy: f32| oy + (wy - min_y) / world_h * oh;
            // 节点分布：位置取卡左上；数量多时采样到 ≤2000 个，避免每帧画全部（开销有界）。
            let positions = this.er_canvas.borrow().er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
            let mut pts: Vec<(f32, f32)> = positions.values().map(|&(x, y)| (x, y)).collect();
            if pts.len() > 2000 {
                let step = (pts.len() as f64 / 2000.0).ceil() as usize;
                pts = pts.iter().step_by(step).cloned().collect();
            }
            for (wx, wy) in pts {
                let mmx = sx(wx);
                let mmy = sy(wy);
                if mmx < ox || mmx > ox + ow || mmy < oy || mmy > oy + oh {
                    continue;
                }
                window.paint_quad(gpui::fill(
                    gpui::bounds(
                        gpui::point(px(mmx), px(mmy)),
                        gpui::size(px(3.0), px(3.0)),
                    ),
                    self.node,
                ));
            }
            // 当前视口矩形（世界可视范围映射到小地图；视口大于 bbox 时裁到小地图边界）。
            if let Some(vp) = this.er_canvas.borrow().er_viewports.get(&tab_id).copied() {
                let (cw, ch) = this.er_canvas.borrow().er_canvas_sizes.get(&tab_id).copied().unwrap_or((ow, oh));
                let vwx0 = vp.to_world_x(0.0);
                let vwy0 = vp.to_world_y(0.0);
                let vwx1 = vp.to_world_x(cw);
                let vwy1 = vp.to_world_y(ch);
                let px0 = sx(vwx0).clamp(ox, ox + ow);
                let py0 = sy(vwy0).clamp(oy, oy + oh);
                let px1 = sx(vwx1).clamp(ox, ox + ow);
                let py1 = sy(vwy1).clamp(oy, oy + oh);
                let w_w = (px1 - px0).max(1.0);
                let h_h = (py1 - py0).max(1.0);
                window.paint_quad(gpui::fill(
                    gpui::bounds(
                        gpui::point(px(px0), px(py0)),
                        gpui::size(px(w_w), px(h_h)),
                    ),
                    self.vp.opacity(0.25),
                ));
            }
        });
    }
}
