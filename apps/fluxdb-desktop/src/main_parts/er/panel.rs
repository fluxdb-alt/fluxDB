// ER 关系面板与表单渲染；行为与状态编排分别见 model.rs/form.rs。

/// 本地逻辑关系列表。它是画布上的非模态右侧面板：不改变画布 scope，也不因普通画布
/// 点击关闭；关闭只由标题栏按钮完成。列表数据来自应用层 service 的后台读取缓存。

fn er_relationship_panel(
    tab_id: TabId,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let loading = this.er_relationship_loading.contains(&tab_id);
    let error = this.er_relationship_errors.get(&tab_id).cloned();
    let relationships = this.er_relationships.get(&tab_id).cloned().unwrap_or_default();
    if this.er_relationship_form_open.contains(&tab_id) {
        this.ensure_er_relationship_form_controls(tab_id, window, cx);
    }
    let close_tab = tab_id;
    let panel_focus = this.er_relationship_panel_focus.entry(tab_id)
        .or_insert_with(|| cx.focus_handle()).clone();
    // 面板宽度可拖动（左缘），按 tab 记忆。默认 560，受 ER 内容区可用宽度约束：
    // 可用空间小（窄窗）则收缩，大窗口允许拖宽到 MAX（约 800）。
    let available_w = this.er_canvas.borrow().er_canvas_sizes.get(&tab_id).copied().unwrap_or((960.0, 640.0)).0;
    let panel_width = clamp_er_rel_panel_width(
        this.er_relationship_panel_width.get(&tab_id).copied().unwrap_or(ER_REL_PANEL_DEFAULT_W),
        available_w,
    );
    let mut panel = div()
        .absolute()
        .top(px(0.))
        .right(px(0.))
        .bottom(px(0.))
        .w(px(panel_width))
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .shadow_lg()
        // 抽屉是不透明全高浮层：屏蔽其后方（画布字段行/端口/连线）的鼠标命中，
        // 否则鼠标停在面板上时下方字段行仍被判定为 hover（背景高亮、端口圆点、tooltip
        // 都会误亮）。面板自身子元素在其前方，交互与滚动不受影响。
        .occlude()
        .track_focus(&panel_focus)
        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
            let key = event.keystroke.key.as_str();
            if key == "escape" {
                if !this.er_relationship_form_open.remove(&tab_id) {
                    this.er_relationship_panel_open.remove(&tab_id);
                }
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if this.er_relationship_form_open.contains(&tab_id) { return; }
            let Some(relationships) = this.er_relationships.get(&tab_id) else { return; };
            if relationships.is_empty() { return; }
            let selected = this.er_relationship_panel_selected.get(&tab_id)
                .and_then(|id| id.as_ref())
                .and_then(|id| relationships.iter().position(|rel| rel.id == *id));
            let index = match key {
                "Down" => Some(selected.map(|i| (i + 1).min(relationships.len() - 1)).unwrap_or(0)),
                "Up" => Some(selected.map(|i| i.saturating_sub(1)).unwrap_or(0)),
                "Enter" | "Return" => {
                    if let Some(rel) = selected.and_then(|i| relationships.get(i)).cloned() {
                        this.er_begin_edit_relationship(tab_id, &rel, window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    return;
                }
                _ => None,
            };
            if let Some(index) = index {
                let id = relationships[index].id.clone();
                this.er_relationship_panel_selected.insert(tab_id, Some(id));
                cx.stop_propagation();
                cx.notify();
            }
        }))
        // 拦截面板区域内鼠标按下/松开，避免穿透到下方画布触发平移/拖动（面板是兄弟浮层，
        // 若不消费按下，按住移动会带动底下 ER 画布）。内部控件在子层先命中，不受此影响。
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                // 表单中的下拉点击不一定 prevent_default；打开表单时不要让面板
                // 抢走焦点，否则搜索和选项确认会失效。
                if !this.er_relationship_form_open.contains(&tab_id) && !window.default_prevented() {
                    panel_focus.focus(window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            |_, _, cx| cx.stop_propagation(),
        );

    panel = panel.child(
        div()
            .h(px(42.))
            .flex()
            .items_center()
            .px(px(12.))
            .border_b_1()
            .border_color(colors.border_soft)
            .child(
                // 表单打开时统一标题为「新建本地逻辑关系」，避免内层重复标题。
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(colors.text)
                    .child(if this.er_relationship_form_open.contains(&tab_id) {
                        "新建本地逻辑关系"
                    } else {
                        "本地逻辑关系"
                    }),
            )
            .child(
                Button::new(("er-rel-new", tab_id.0))
                    .secondary()
                    .xsmall()
                    .label(if this.er_relationship_form_open.contains(&tab_id) {
                        "关闭新建"
                    } else {
                        "新建关系"
                    })
                    .disabled(loading || this.er_full_tables.get(&tab_id).is_none_or(Vec::is_empty))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.er_relationship_form_open.contains(&tab_id) {
                            this.er_relationship_form_open.remove(&tab_id);
                        } else {
                            this.er_relationship_form_open.insert(tab_id);
                            this.er_prepare_new_relationship_form(tab_id, window, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("er-rel-close", tab_id.0))
                    .ghost()
                    .xsmall()
                    .tooltip("关闭关系面板")
                    .child(app_icon(AppIcon::Close, 15., colors.muted))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // 关闭面板：清理拖拽起点，避免下次打开残留旧状态。
                        this.er_relationship_panel_resize_start = None;
                        this.er_relationship_panel_open.remove(&close_tab);
                        cx.notify();
                    })),
            ),
    );

    // 结构刷新重绑待处理项横幅（§5.2/D9）：unresolved/同名重建/缺列，人工重绑（编辑关系）。
    let pending = this.er_rebind_pending.get(&tab_id).cloned().unwrap_or_default();
    if !this.er_relationship_form_open.contains(&tab_id) && !pending.is_empty() {
        let mut pending_box = div()
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(colors.border_soft)
            .bg(if colors.is_dark { rgb(0x3a2f18) } else { rgb(0xfdf3e3) })
            .flex()
            .flex_col()
            .gap(px(4.));
        pending_box = pending_box.child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if colors.is_dark { rgb(0xe6c27a) } else { rgb(0x8a5a00) })
                .child(format!("{} 项待处理（结构刷新后需人工确认）", pending.len())),
        );
        for item in pending.iter().take(4) {
            pending_box = pending_box.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(format!("{} · {}：{}", item.endpoint, item.rel_id, item.kind)),
            );
        }
        if pending.len() > 4 {
            pending_box = pending_box.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(format!("… 其余 {} 项", pending.len() - 4)),
            );
        }
        panel = panel.child(pending_box);
    }

    if this.er_relationship_form_open.contains(&tab_id) {
        // 表单内容可滚动：默认打开时把提交区推出视口外也能滚到，底部操作始终可达。
        panel = panel.child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(er_relationship_form(tab_id, this, colors, cx)),
        );
    }

    if let Some(original) = this.er_relationship_delete_undo.get(&tab_id).cloned() {
        let undo_tab = tab_id;
        let mut restore = original.clone();
        restore.review.state = fluxdb_core::ErReviewState::Proposed;
        restore.review.confirmed_revision = None;
        restore.review.confirmed_by = None;
        let scope = this.er_relationship_scope_keys.get(&tab_id).cloned().unwrap_or_default();
        panel = panel.child(
            div().px(px(12.)).py(px(6.)).border_b_1().border_color(colors.border_soft)
                .child(Button::new(("er-undo-delete", tab_id.0)).secondary().xsmall()
                    .label("恢复最近删除的关系（待确认）")
                    .disabled(loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_er_relationship_command(undo_tab,
                            AppCommand::CreateErRelationship { scope_key: scope.clone(), relationship: restore.clone() }, cx);
                    }))),
        );
    }

    if loading {
        panel = panel.child(
            div()
                .px(px(14.))
                .py(px(16.))
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("正在加载关系…"),
        );
    } else if let Some(error) = error {
        let retry_tab = tab_id;
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .px(px(14.))
                .py(px(16.))
                .child(div().text_size(px(12.)).text_color(rgb(0xef4444)).child(error))
                .child(
                    Button::new(("er-rel-retry-list", tab_id.0))
                        .secondary()
                        .small()
                        .label("重试")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.retry_er_relationships(retry_tab, cx);
                        })),
                ),
        );
    } else if relationships.is_empty() {
        panel = panel.child(
            div()
                .px(px(14.))
                .py(px(16.))
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("当前作用域没有本地逻辑关系"),
        );
    } else {
        let mut list = div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.));
        for relationship in relationships {
            let edit_relationship = relationship.clone();
            let relationship_id = relationship.id.clone();
            let relationship_revision = relationship.revision;
            let scope_key = this
                .er_relationship_scope_keys
                .get(&tab_id)
                .cloned()
                .unwrap_or_default();
            let busy = loading;
            let delete_pending = this
                .er_relationship_delete_pending
                .get(&tab_id)
                .is_some_and(|(id, revision)| {
                    id == &relationship_id && *revision == relationship_revision
                });
            let delete_action = if delete_pending {
                let cancel_tab = tab_id;
                let cancel_id = relationship_id.clone();
                let confirm_scope = scope_key.clone();
                let confirm_id = relationship_id.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(0xef4444))
                            .child("仅删除本地关系，不修改数据库外键"),
                    )
                    .child(
                        Button::new(format!("er-rel-delete-cancel-{}-{}", tab_id.0, cancel_id))
                            .ghost()
                            .xsmall()
                            .label("取消")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.er_relationship_delete_pending.remove(&cancel_tab);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(format!("er-rel-delete-confirm-{}-{}", tab_id.0, confirm_id))
                            .danger()
                            .xsmall()
                            .label("确认删除")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_er_relationship_command(
                                    tab_id,
                                    AppCommand::DeleteErRelationship {
                                        scope_key: confirm_scope.clone(),
                                        id: confirm_id.clone(),
                                        expected_revision: relationship_revision,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .into_any_element()
            } else {
                let pending_tab = tab_id;
                let pending_id = relationship_id.clone();
                Button::new(format!("er-rel-delete-{}-{}", tab_id.0, pending_id))
                    .ghost()
                    .xsmall()
                    .label("删除")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.er_relationship_delete_pending.insert(
                            pending_tab,
                            (pending_id.clone(), relationship_revision),
                        );
                        cx.notify();
                    }))
                    .into_any_element()
            };
            let needs_rebuild_review = er_relationship_needs_rebuild_review(this, tab_id, &relationship_id);
            let unresolved = relationship.validity.state == fluxdb_core::ErValidityState::Unresolved
                || this.er_rebind_pending.get(&tab_id).is_some_and(|items| items.iter().any(|item| {
                    item.rel_id == relationship_id && (item.endpoint.contains("缺列") || item.endpoint.contains("实体未找到"))
                }));
            let state = if unresolved {
                ("未解析（缺表或缺列）", rgb(0xef4444))
            } else if needs_rebuild_review {
                ("需确认（同名重建）", rgb(0x8a5a00))
            } else { match relationship.review.state {
                fluxdb_core::ErReviewState::Proposed => ("待确认", colors.muted),
                fluxdb_core::ErReviewState::Confirmed => ("已确认", rgb(0x16a34a)),
                fluxdb_core::ErReviewState::Rejected => ("已拒绝", rgb(0xef4444)),
            }};
            let pairs = relationship
                .column_pairs
                .iter()
                .map(|pair| er_relationship_pair_label(this, tab_id, &relationship, pair))
                .collect::<Vec<_>>()
                .join("\n");
            // 高亮：点击画布逻辑边后定位到该关系。
            let rel_selected = this
                .er_relationship_panel_selected
                .get(&tab_id)
                .and_then(|sel| sel.as_ref())
                == Some(&relationship_id);
            let rel_click_tab = tab_id;
            let rel_click_id = relationship_id.clone();
            list = list.child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_soft)
                    .when(rel_selected, |s| s.bg(colors.tree_selected))
                    .cursor_pointer()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            // 点卡片：切换选中。
                            let currently = this
                                .er_relationship_panel_selected
                                .get(&rel_click_tab)
                                .and_then(|sel| sel.as_ref())
                                == Some(&rel_click_id);
                            this.er_relationship_panel_selected.insert(
                                rel_click_tab,
                                if currently { None } else { Some(rel_click_id.clone()) },
                            );
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_size(px(12.)).text_color(colors.text).child(relationship.role))
                            .child(div().text_size(px(11.)).text_color(state.1).child(state.0)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(format!("修订 {} · {}", relationship.revision, relationship.id)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(format!(
                                "{} → {}",
                                this.er_form_table_by_id(tab_id, Some(&relationship.left_entity))
                                    .map(|table| table.reference.display())
                                    .unwrap_or_else(|| relationship.left_entity.clone()),
                                this.er_form_table_by_id(tab_id, Some(&relationship.right_entity))
                                    .map(|table| table.reference.display())
                                    .unwrap_or_else(|| relationship.right_entity.clone())
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.text)
                            .child(pairs),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .pt(px(3.))
                            .child(
                                Button::new(format!("er-rel-edit-{}-{}", tab_id.0, relationship_id))
                                    .ghost()
                                    .xsmall()
                                    .label("编辑")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.er_begin_edit_relationship(
                                            tab_id,
                                            &edit_relationship,
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(format!("er-rel-confirm-{}-{}", tab_id.0, relationship_id))
                                    .ghost()
                                    .xsmall()
                                    .label("确认")
                                    .disabled(busy || unresolved || relationship.review.state == fluxdb_core::ErReviewState::Confirmed)
                                    .on_click(cx.listener({
                                        let scope_key = scope_key.clone();
                                        let relationship_id = relationship_id.clone();
                                        move |this, _, _, cx| {
                                            this.run_er_relationship_command(
                                                tab_id,
                                                AppCommand::ConfirmErRelationship {
                                                    scope_key: scope_key.clone(),
                                                    id: relationship_id.clone(),
                                                    expected_revision: relationship_revision,
                                                    by: "user".into(),
                                                },
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(
                                Button::new(format!("er-rel-reject-{}-{}", tab_id.0, relationship_id))
                                    .ghost()
                                    .xsmall()
                                    .label("拒绝")
                                    .disabled(busy || relationship.review.state == fluxdb_core::ErReviewState::Rejected)
                                    .on_click(cx.listener({
                                        let scope_key = scope_key.clone();
                                        let relationship_id = relationship_id.clone();
                                        move |this, _, _, cx| {
                                            this.run_er_relationship_command(
                                                tab_id,
                                                AppCommand::RejectErRelationship {
                                                    scope_key: scope_key.clone(),
                                                    id: relationship_id.clone(),
                                                    expected_revision: relationship_revision,
                                                },
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(delete_action),
                    ),
            );
        }
        panel = panel.child(list);
    }
    // 左缘拖宽手柄：作为最后一个 child（渲染在最上层，不被内容覆盖；绘制与命中一致）。
    panel = panel.child(er_rel_panel_resize_handle(tab_id, panel_width, available_w, colors, cx));
    panel
}

/// 表单分节：标题（右侧可放操作）+ 内容，用细分隔线（border-top）与间距组织。
fn er_form_section(
    title: &str,
    head_rear: Div,
    body: Div,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(colors.border_soft)
        .py(px(14.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .mb(px(12.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title.to_string()),
                )
                .child(head_rear),
        )
        .child(body)
}

/// 表单字段：标签在控件上方（可选标「可选」），供基础信息分节使用。
fn er_form_field(
    label: &str,
    optional: bool,
    control: impl gpui::IntoElement,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .min_w_0()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(label.to_string()),
                )
                .when(optional, |s| {
                    s.child(
                        div()
                            .text_size(px(10.))
                            .text_color(colors.muted)
                            .child("可选"),
                    )
                }),
        )
        .child(div().w_full().min_w_0().child(control))
}

/// 等号图标：两条短横线堆叠（AppIcon 无 Equal，用最小布局组合，颜色走 UiColors）。
fn er_equal_icon(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .size(px(12.))
        .child(div().w(px(8.)).h(px(1.5)).bg(colors.muted))
        .child(div().w(px(8.)).h(px(1.5)).bg(colors.muted))
}

/// 关系面板左缘拖宽手柄（独立函数，置于面板最上层，避免被内容区覆盖）。
fn er_rel_panel_resize_handle(
    tab_id: TabId,
    panel_width: f32,
    available_w: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl gpui::IntoElement {
    div()
        .id(("er-rel-panel-handle", tab_id.0))
        .absolute()
        .top_0()
        .left_0()
        .bottom_0()
        .w(px(10.))
        .cursor_ew_resize()
        .hover(|s| s.bg(colors.hover))
        .active(|s| s.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                this.er_relationship_panel_resize_start = Some(SidebarResizeStart {
                    x: f32::from(event.position.x),
                    width: panel_width,
                });
                cx.stop_propagation();
            }),
        )
        // 独立拖拽类型：连接栏仅监听 SidebarResizeDrag，二者不再互相抢占。
        .on_drag(ErRelationshipResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &gpui::DragMoveEvent<ErRelationshipResizeDrag>, _, cx| {
                cx.stop_propagation();
                // 松开（结束帧 p1 为 None / 无左键按下）→ 清理拖拽状态。
                if event.event.pressed_button != Some(MouseButton::Left) {
                    this.er_relationship_panel_resize_start = None;
                    cx.notify();
                    return;
                }
                if let Some(start) = this.er_relationship_panel_resize_start {
                    // 手柄在面板左缘：新宽度 = 起始宽度 + 起始X - 当前X（左拖变宽）。
                    let width = clamp_er_rel_panel_width(
                        start.width + (start.x - f32::from(event.event.position.x)),
                        available_w,
                    );
                    this.er_relationship_panel_width
                        .entry(tab_id)
                        .and_modify(|w| *w = width)
                        .or_insert(width);
                    cx.notify();
                }
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.er_relationship_panel_resize_start = None;
                cx.stop_propagation();
            }),
        )
}

fn er_relationship_pair_label(
    this: &NavicatMain,
    tab_id: TabId,
    relationship: &fluxdb_core::ErRelationship,
    pair: &fluxdb_core::ErColumnPair,
) -> String {
    let table = |entity_id: &String| {
        this.er_full_tables
            .get(&tab_id)
            .into_iter()
            .flatten()
            .find(|table| er_entity_id(&table.reference) == *entity_id)
    };
    let left = table(&relationship.left_entity);
    let right = table(&relationship.right_entity);
    let left_column = left
        .into_iter()
        .flat_map(|table| table.columns.iter())
        .find(|column| {
            left.is_some_and(|table| er_column_id(&table.reference, &column.name) == pair.left_column)
        })
        .map(|column| column.name.clone())
        .unwrap_or_else(|| er_column_display_name(&pair.left_column));
    let right_column = right
        .into_iter()
        .flat_map(|table| table.columns.iter())
        .find(|column| {
            right.is_some_and(|table| {
                er_column_id(&table.reference, &column.name) == pair.right_column
            })
        })
        .map(|column| column.name.clone())
        .unwrap_or_else(|| er_column_display_name(&pair.right_column));
    let left_name = left
        .map(|table| table.reference.display())
        .unwrap_or_else(|| relationship.left_entity.clone());
    let right_name = right
        .map(|table| table.reference.display())
        .unwrap_or_else(|| relationship.right_entity.clone());
    format!("{left_name}.{left_column} ↔ {right_name}.{right_column}")
}

fn er_relationship_form(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(role_input) = this.er_relationship_form_role_inputs.get(&tab_id).cloned() else {
        return div();
    };
    let Some(description_input) = this
        .er_relationship_form_description_inputs
        .get(&tab_id)
        .cloned()
    else {
        return div();
    };
    let Some(left_table) = this.er_relationship_form_left_tables.get(&tab_id).cloned() else {
        return div();
    };
    let Some(right_table) = this.er_relationship_form_right_tables.get(&tab_id).cloned() else {
        return div();
    };
    let Some(cardinality) = this.er_relationship_form_cardinality.get(&tab_id).cloned() else {
        return div();
    };
    let pairs = this
        .er_relationship_form_pairs
        .get(&tab_id)
        .map(|pairs| {
            pairs
                .iter()
                .map(|pair| (pair.left.clone(), pair.right.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let filters = this
        .er_relationship_form_filters
        .get(&tab_id)
        .map(|filters| {
            filters
                .iter()
                .map(|filter| {
                    (
                        filter.side.clone(),
                        filter.column.clone(),
                        filter.op.clone(),
                        filter.literal.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let submitting = this.er_relationship_tasks.contains_key(&tab_id);
    // 当前左/右表显示名与基数（供方向文案）。
    let left_name = this
        .er_relationship_form_left_tables
        .get(&tab_id)
        .and_then(|s| s.read(cx).selected_value().cloned())
        .and_then(|id| this.er_form_table_by_id(tab_id, Some(&id)))
        .map(|t| t.reference.display());
    let right_name = this
        .er_relationship_form_right_tables
        .get(&tab_id)
        .and_then(|s| s.read(cx).selected_value().cloned())
        .and_then(|id| this.er_form_table_by_id(tab_id, Some(&id)))
        .map(|t| t.reference.display());
    let card_id = this
        .er_relationship_form_cardinality
        .get(&tab_id)
        .and_then(|s| s.read(cx).selected_value().cloned())
        .unwrap_or_else(|| "unknown".into());
    // 左右表是否都选定：未选时字段选择器禁用并提示，不用硬编码默认表/字段掩盖空状态。
    let has_both_tables = left_name.is_some() && right_name.is_some();
    // 表单直接铺在面板内（面板 header 已承担标题与关闭，不重复卡片）；外层滚动由面板内
    // 的包裹层承担（见 er_relationship_panel），保证底部操作区始终可达。
    let mut form = div()
        .p(px(16.))
        .flex()
        .flex_col();

    // 弱化说明（标题在面板 header）。
    form = form.child(
        div()
            .mb(px(12.))
            .text_size(px(11.))
            .text_color(colors.muted)
            .child("连接两张表的字段，描述它们之间的业务关系。"),
    );

    // 基础信息分节：左表/右表并排，角色/说明独占一行。
    let mut basic = div().flex().flex_col().gap(px(12.));
    basic = basic.child(
        div()
            .flex()
            .gap(px(12.))
            .flex_wrap()
            .child(
                div()
                    .flex_1()
                    .min_w(px(180.))
                    .child(er_form_field("左表", false, Select::new(&left_table).small().placeholder("选择左表").search_placeholder("搜索表"), colors)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(180.))
                    .child(er_form_field("右表", false, Select::new(&right_table).small().placeholder("选择右表").search_placeholder("搜索表"), colors)),
            ),
    );
    basic = basic.child(er_form_field(
        "业务角色",
        false,
        Input::new(&role_input).small(),
        colors,
    ));
    basic = basic.child(er_form_field(
        "说明",
        true,
        Input::new(&description_input).small(),
        colors,
    ));
    form = form.child(er_form_section("基础信息", div(), basic, colors));

    // 关联字段分节：标题 + 基数下拉 + 添加配对；方向文案 + 表头 + 每对（序号/左/等号/右/删除）。
    let mut assoc_head = div().flex().items_center().gap(px(8.));
    assoc_head = assoc_head.child(
        div().w(px(130.)).child(Select::new(&cardinality).small().placeholder("选择基数")),
    );
    let add_pair_tab = tab_id;
    assoc_head = assoc_head.child(
        Button::new(("er-rel-add-pair", tab_id.0))
            .ghost()
            .xsmall()
            .child(app_icon(AppIcon::Plus, 14., colors.text))
            .label("添加配对")
            .disabled(submitting)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.er_add_relationship_pair(add_pair_tab, window, cx);
                cx.notify();
            })),
    );
    let mut assoc_body = div().flex().flex_col().gap(px(8.));
    // 方向与基数说明。
    let direction = match (&left_name, &right_name) {
        (Some(l), Some(r)) => format!(
            "{l} → {r} · {}，所有字段配对共同生效",
            er_cardinality_label(&card_id)
        ),
        _ => "请先选择左右表 · 所有字段配对共同生效".to_string(),
    };
    assoc_body = assoc_body.child(
        div()
            .text_size(px(11.))
            .text_color(colors.muted)
            .child(direction),
    );
    // 表头。
    assoc_body = assoc_body.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(2.))
            .child(div().w(px(18.)).flex_shrink_0().child(div()))
            .child(
                div()
                    .flex_1()
                    .text_size(px(10.))
                    .text_color(colors.muted)
                    .child("左表字段"),
            )
            .child(div().w(px(14.)).flex_shrink_0().child(div()))
            .child(
                div()
                    .flex_1()
                    .text_size(px(10.))
                    .text_color(colors.muted)
                    .child("右表字段"),
            ),
    );
    // 每对字段行。
    let pair_count = pairs.len();
    for (index, (left, right)) in pairs.into_iter().enumerate() {
        let remove_index = index;
        let remove_tab = tab_id;
        assoc_body = assoc_body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(18.))
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{}", index + 1)),
                )
                .child(div().flex_1().min_w_0().child(
                    Select::new(&left)
                        .small()
                        .placeholder(if has_both_tables { "选择字段" } else { "请先选择左表" })
                        .search_placeholder("搜索字段")
                        .disabled(!has_both_tables),
                ))
                .child(er_equal_icon(colors))
                .child(div().flex_1().min_w_0().child(
                    Select::new(&right)
                        .small()
                        .placeholder(if has_both_tables { "选择字段" } else { "请先选择右表" })
                        .search_placeholder("搜索字段")
                        .disabled(!has_both_tables),
                ))
                .child(
                    Button::new(format!("er-rel-del-pair-{}-{}", tab_id.0, index))
                        .ghost()
                        .child(app_icon(AppIcon::Trash, 14., colors.muted))
                        .tooltip("删除本组配对")
                        .disabled(submitting || pair_count <= 1)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.er_remove_relationship_pair(remove_tab, remove_index, cx);
                        })),
                ),
        );
    }
    form = form.child(er_form_section("关联字段", assoc_head, assoc_body, colors));

    // 附加关联条件分节：标题 + 添加条件 + 辅助文案 + 条件行 + 空态。
    let add_filter_tab = tab_id;
    let cond_head = div().child(
        Button::new(("er-rel-add-filter", tab_id.0))
            .ghost()
            .xsmall()
            .child(app_icon(AppIcon::Plus, 14., colors.text))
            .label("添加条件")
            .disabled(submitting)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.er_add_relationship_filter(add_filter_tab, window, cx);
                cx.notify();
            })),
    );
    let mut cond_body = div().flex().flex_col().gap(px(8.));
    cond_body = cond_body.child(
        div()
            .text_size(px(11.))
            .text_color(colors.muted)
            .child("使用这条关系生成 JOIN 时，始终附加以下条件。"),
    );
    if filters.is_empty() {
        cond_body = cond_body.child(
            div()
                .px(px(10.))
                .py(px(8.))
                .rounded_md()
                .bg(colors.canvas_bg)
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("暂无附加条件，例如：customers.is_deleted = 0"),
        );
    }
    for (_index, (side, column, op, literal)) in filters.into_iter().enumerate() {
        cond_body = cond_body.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().w(px(58.)).flex_shrink_0().child(Select::new(&side).small().placeholder("端点")))
                .child(div().flex_1().min_w_0().child(
                    Select::new(&column)
                        .small()
                        .placeholder(if has_both_tables { "选择字段" } else { "请先选择表" })
                        .search_placeholder("搜索字段")
                        .disabled(!has_both_tables),
                ))
                .child(div().w(px(80.)).flex_shrink_0().child(Select::new(&op).small().placeholder("操作")))
                .child(div().flex_1().min_w_0().child(Input::new(&literal).small())),
        );
    }
    form = form.child(er_form_section("附加关联条件", cond_head, cond_body, colors));

    // 底部操作区：左弱化说明 + 右取消/创建。
    let submit_tab = tab_id;
    let cancel_tab = tab_id;
    form = form.child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .border_t_1()
            .border_color(colors.border_soft)
            .pt(px(12.))
            .mt(px(14.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child("仅保存本地逻辑关系"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new(("er-rel-form-cancel", tab_id.0))
                            .ghost()
                            .xsmall()
                            .label("取消")
                            .disabled(submitting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.er_relationship_form_open.remove(&cancel_tab);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("er-rel-submit", tab_id.0))
                            .primary()
                            .xsmall()
                            .label(if submitting { "提交中…" } else { "创建关系" })
                            .disabled(submitting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit_er_relationship_form(submit_tab, cx);
                                cx.notify();
                            })),
                    ),
            ),
    );
    form
}
