// 查询计划可视化：复用查询结果区，按解析出的节点树渲染卡片与节点检查器。
//
// 数据只来自当前结果页（`query_plan_from_page`），这里不发起 SQL、也不改执行路径。
// 解析失败时页签根本不出现，用户仍能看到原始结果，因此本文件只处理“已有计划”的情况。

fn query_plan_view(
    tab_id: TabId,
    editor: &QueryEditorState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let database_kind = this.query_editor_database_kind(tab_id);
    let Some(page) = editor.results.last() else {
        return query_output_empty_state(colors);
    };
    let Some(plan) = fluxdb_app::query_plan_from_page(database_kind, page) else {
        return query_output_empty_state(colors);
    };

    let mode = this
        .query_plan_view_modes
        .get(&tab_id)
        .copied()
        .unwrap_or_default();
    let selected = this
        .query_plan_selected_nodes
        .get(&tab_id)
        .copied()
        .filter(|id| *id < plan.nodes.len())
        .unwrap_or(plan.root);

    let body = match mode {
        QueryPlanViewMode::Graph => div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .child(query_plan_tree(
                tab_id, &plan, selected, colors, cx,
            ))
            .child(query_plan_inspector(tab_id, &plan, selected, colors, cx))
            .into_any_element(),
        QueryPlanViewMode::Table => query_plan_raw_table(tab_id, page, colors),
    };

    div()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .child(query_plan_toolbar(tab_id, &plan, mode, colors, cx))
        .child(body)
}

fn query_plan_toolbar(
    tab_id: TabId,
    plan: &fluxdb_app::QueryPlan,
    mode: QueryPlanViewMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let warnings = plan.warning_count();
    let mut summary = div()
        .flex()
        .items_center()
        .gap_4()
        .flex_wrap()
        .child(plan_summary_item(
            tr("app.query.plan.total_cost").to_string(),
            plan.total_cost
                .map(plan_format_cost)
                .unwrap_or_else(|| "—".to_string()),
            colors.text,
            colors,
        ))
        .child(plan_summary_item(
            tr("app.query.plan.estimated_rows").to_string(),
            plan.estimated_rows
                .map(plan_format_count)
                .unwrap_or_else(|| "—".to_string()),
            colors.text,
            colors,
        ))
        .child(plan_summary_item(
            tr("app.query.plan.source").to_string(),
            match plan.source {
                fluxdb_app::QueryPlanSource::MySqlJson
                | fluxdb_app::QueryPlanSource::PostgresJson => "EXPLAIN JSON".to_string(),
                fluxdb_app::QueryPlanSource::SqliteQueryPlan => {
                    "EXPLAIN QUERY PLAN".to_string()
                }
                fluxdb_app::QueryPlanSource::TabularExplain => "EXPLAIN".to_string(),
            },
            colors.muted,
            colors,
        ));
    if warnings > 0 {
        summary = summary.child(plan_summary_item(
            tr("app.query.plan.warnings").to_string(),
            warnings.to_string(),
            colors.warning,
            colors,
        ));
    }

    let estimate = div()
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .rounded(colors.radius)
        .bg(colors.panel_bg)
        .border_1()
        .border_color(colors.border)
        .text_size(px(11.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(tr("app.query.plan.estimate_only").to_string());

    let actual = div()
        .id(("query-plan-actual", tab_id.0))
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .rounded(colors.radius)
        .text_size(px(11.5))
        .text_color(colors.muted)
        .child(tr("app.query.plan.actual_locked").to_string())
        .tooltip(move |window, cx| {
            Tooltip::new(tr("app.query.plan.actual_locked_hint")).build(window, cx)
        });

    let mut toggles = div().flex().items_center().gap_1();
    for (view, label) in [
        (QueryPlanViewMode::Graph, tr("app.query.plan.graph").to_string()),
        (QueryPlanViewMode::Table, tr("app.query.plan.table").to_string()),
    ] {
        let active = mode == view;
        toggles = toggles.child(plan_view_toggle(
            tab_id, view, label, active, colors, cx,
        ));
    }

    div()
        .h(px(48.))
        .flex_none()
        .flex()
        .items_center()
        .gap_4()
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(summary)
        .child(div().flex_1())
        .child(div().flex().items_center().gap_2().child(estimate).child(actual))
        .child(toggles)
}

fn plan_summary_item(
    label: String,
    value: String,
    value_color: gpui::Rgba,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap_2()
        .text_size(px(11.5))
        .child(div().text_color(colors.muted).child(label))
        .child(
            div()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(value_color)
                .child(value),
        )
}

fn plan_view_toggle(
    tab_id: TabId,
    view: QueryPlanViewMode,
    label: String,
    active: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(move || {
            format!(
                "query-plan-toggle:{}",
                match view {
                    QueryPlanViewMode::Graph => "graph",
                    QueryPlanViewMode::Table => "table",
                }
            )
        })
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .rounded(colors.radius)
        .cursor_pointer()
        .text_size(px(11.5))
        .when(active, |this| {
            this.bg(colors.panel_bg)
                .border_1()
                .border_color(colors.border)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
        })
        .when(!active, |this| this.text_color(colors.muted))
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.query_plan_view_modes.insert(tab_id, view);
                cx.notify();
                cx.stop_propagation();
            }),
        )
}

fn query_plan_tree(
    tab_id: TabId,
    plan: &fluxdb_app::QueryPlan,
    selected: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .id(("query-plan-tree-scroll", tab_id.0))
        // 父容器是 flex 行，默认交叉轴拉伸让高度跟随视口，内容超出才有滚动范围。
        .overflow_scrollbar()
        .p_4()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .min_w(px(420.))
                .child(plan_branch(
                    tab_id, plan, plan.root, selected, colors, cx,
                )),
        )
        .into_any_element()
}

fn plan_branch(
    tab_id: TabId,
    plan: &fluxdb_app::QueryPlan,
    node_id: usize,
    selected: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(node) = plan.node(node_id) else {
        return div();
    };
    let mut branch = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(plan_node_card(
            tab_id, plan, node, selected, colors, cx,
        ));
    if !node.children.is_empty() {
        let mut children = div()
            .flex()
            .flex_col()
            .gap_2()
            .ml(px(16.))
            .pl(px(14.))
            .border_l_1()
            .border_color(colors.border);
        for child in &node.children {
            children = children.child(plan_branch(
                tab_id, plan, *child, selected, colors, cx,
            ));
        }
        branch = branch.child(children);
    }
    branch
}

fn plan_node_card(
    tab_id: TabId,
    plan: &fluxdb_app::QueryPlan,
    node: &fluxdb_app::PlanNode,
    selected: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_selected = node.id == selected;
    let (icon, icon_color) = plan_op_icon(node.op, colors);
    let subtitle = plan_node_subtitle(node);
    let mut top = div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .child(
            div()
                .size(px(26.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(colors.radius)
                .bg(colors.panel_alt)
                .child(app_icon(icon, 14., icon_color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(plan_op_label(node)),
                )
                .when_some(subtitle, |this, subtitle| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .font_family(crate::PLATFORM_MONOSPACE_FONT)
                            .text_color(colors.muted)
                            .text_ellipsis()
                            .max_w(px(360.))
                            .child(subtitle),
                    )
                }),
        );

    for issue in &node.issues {
        top = top.child(plan_issue_badge(*issue, colors));
    }

    top = top
        .child(div().flex_1())
        .child(plan_node_metric(
            tr("app.query.plan.cost").to_string(),
            node.cost
                .map(plan_format_cost)
                .unwrap_or_else(|| "—".to_string()),
            colors,
        ))
        .child(plan_node_metric(
            tr("app.query.plan.rows").to_string(),
            node.rows
                .map(plan_format_count)
                .unwrap_or_else(|| "—".to_string()),
            colors,
        ));

    let mut card = div()
        .flex()
        .flex_col()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(if is_selected {
            colors.info
        } else {
            colors.border
        })
        .bg(if is_selected {
            colors.hover
        } else {
            colors.panel_bg
        })
        .cursor_pointer()
        .hover(move |style| style.border_color(colors.info))
        .child(top);

    if let Some(width) = plan_cost_ratio(plan, node) {
        card = card.child(
            div()
                .h(px(3.))
                .w_full()
                .bg(colors.panel_alt)
                .child(div().h_full().w(gpui::relative(width)).bg(icon_color)),
        );
    }

    let node_id = node.id;
    card.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.query_plan_selected_nodes.insert(tab_id, node_id);
            cx.notify();
            cx.stop_propagation();
        }),
    )
}

fn plan_node_metric(label: String, value: String, colors: UiColors) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .items_end()
        .gap_1()
        .child(div().text_size(px(10.)).text_color(colors.muted).child(label))
        .child(
            div()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(value),
        )
}

fn plan_issue_badge(issue: fluxdb_app::PlanIssue, colors: UiColors) -> Div {
    let (label, color) = match issue {
        fluxdb_app::PlanIssue::FullTableScan => (
            tr("app.query.plan.issue.full_scan").to_string(),
            colors.danger,
        ),
        fluxdb_app::PlanIssue::Filesort => (
            tr("app.query.plan.issue.filesort").to_string(),
            colors.warning,
        ),
        fluxdb_app::PlanIssue::NoIndex => (
            tr("app.query.plan.issue.no_index").to_string(),
            colors.warning,
        ),
    };
    div()
        .flex_none()
        .h(px(19.))
        .px_2()
        .flex()
        .items_center()
        .rounded(px(10.))
        .bg(colors.panel_alt)
        .text_size(px(10.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(color)
        .child(label)
}

fn query_plan_inspector(
    tab_id: TabId,
    plan: &fluxdb_app::QueryPlan,
    selected: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(node) = plan.node(selected) else {
        return div();
    };
    let (icon, icon_color) = plan_op_icon(node.op, colors);
    let copy_text = plan_node_copy_text(node);
    let advice = plan_advice(node);

    let mut body = div()
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(colors.radius)
                        .bg(colors.panel_bg)
                        .child(app_icon(icon, 15., icon_color)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(plan_op_label(node)),
                        )
                        .when_some(node.object.clone(), |this, object| {
                            this.child(
                                div()
                                    .text_size(px(11.))
                                    .font_family(crate::PLATFORM_MONOSPACE_FONT)
                                    .text_color(colors.muted)
                                    .text_ellipsis()
                                    .child(object),
                            )
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(plan_stat(
                    tr("app.query.plan.cost").to_string(),
                    node.cost
                        .map(plan_format_cost)
                        .unwrap_or_else(|| "—".to_string()),
                    colors,
                ))
                .child(plan_stat(
                    tr("app.query.plan.rows").to_string(),
                    node.rows
                        .map(plan_format_count)
                        .unwrap_or_else(|| "—".to_string()),
                    colors,
                ))
                .child(plan_stat(
                    tr("app.query.plan.filtered").to_string(),
                    node.filtered
                        .map(|value| format!("{value:.0}%"))
                        .unwrap_or_else(|| "—".to_string()),
                    colors,
                ))
                .child(plan_stat(
                    tr("app.query.plan.loops").to_string(),
                    node.loops.unwrap_or(1).to_string(),
                    colors,
                )),
        )
        .child(plan_advice_box(&advice, colors))
        .child(plan_field(
            tr("app.query.plan.condition").to_string(),
            node.condition.clone().unwrap_or_else(|| "—".to_string()),
            true,
            colors,
        ))
        .child(
            div()
                .flex()
                .gap_2()
                .child(plan_field(
                    tr("app.query.plan.index_used").to_string(),
                    node.index.clone().unwrap_or_else(|| {
                        tr("app.query.plan.none").to_string()
                    }),
                    false,
                    colors,
                ))
                .child(plan_field(
                    tr("app.query.plan.possible_keys").to_string(),
                    node.possible_keys.clone().unwrap_or_else(|| {
                        tr("app.query.plan.none").to_string()
                    }),
                    false,
                    colors,
                )),
        );

    if let Some(detail) = node.detail.clone() {
        body = body.child(plan_field(
            tr("app.query.plan.detail").to_string(),
            detail,
            false,
            colors,
        ));
    }
    if let Some(extra) = node.extra.clone() {
        body = body.child(plan_field(
            tr("app.query.plan.extra").to_string(),
            extra,
            false,
            colors,
        ));
    }

    div()
        .w(px(300.))
        .flex_none()
        .min_h(px(0.))
        .border_l_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(colors.muted)
                        .child(tr("app.query.plan.node_details").to_string()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .h(px(24.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(colors.radius)
                        .cursor_pointer()
                        .text_size(px(11.5))
                        .text_color(colors.muted)
                        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
                        .child(app_icon(AppIcon::Copy, 12., colors.muted))
                        .child(tr("app.query.plan.copy").to_string())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    copy_text.clone(),
                                ));
                                this.show_message(
                                    tr("app.query.plan.copied").to_string(),
                                    AppMessageKind::Success,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .id(("query-plan-inspector-scroll", tab_id.0))
                .overflow_y_scrollbar()
                .child(body),
        )
}

fn plan_stat(label: String, value: String, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded(colors.radius)
        .bg(colors.panel_bg)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .text_size(px(10.))
                .text_color(colors.muted)
                .text_ellipsis()
                .child(label),
        )
        .child(
            div()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(value),
        )
}

fn plan_field(label: String, value: String, mono: bool, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_size(px(11.)).text_color(colors.muted).child(label))
        .child(
            div()
                .p_2()
                .rounded(colors.radius)
                .bg(colors.panel_bg)
                .border_1()
                .border_color(colors.border)
                .text_size(px(11.5))
                .text_color(colors.text)
                .when(mono, |this| this.font_family(crate::PLATFORM_MONOSPACE_FONT))
                .child(value),
        )
}

struct PlanAdvice {
    title: String,
    text: String,
    danger: bool,
    warning: bool,
}

fn plan_advice(node: &fluxdb_app::PlanNode) -> PlanAdvice {
    let first = node.issues.first();
    let table = node.object.clone().unwrap_or_default();
    match first {
        Some(fluxdb_app::PlanIssue::FullTableScan) => PlanAdvice {
            title: tr("app.query.plan.issue.full_scan").to_string(),
            text: tr("app.query.plan.advice.full_scan")
                .replace("%{table}", &table)
                .replace(
                    "%{rows}",
                    &node
                        .rows
                        .map(plan_format_count)
                        .unwrap_or_else(|| "—".to_string()),
                ),
            danger: true,
            warning: false,
        },
        Some(fluxdb_app::PlanIssue::Filesort) => PlanAdvice {
            title: tr("app.query.plan.issue.filesort").to_string(),
            text: tr("app.query.plan.advice.filesort").to_string(),
            danger: false,
            warning: true,
        },
        Some(fluxdb_app::PlanIssue::NoIndex) => PlanAdvice {
            title: tr("app.query.plan.issue.no_index").to_string(),
            text: tr("app.query.plan.advice.no_index")
                .replace("%{table}", &table),
            danger: false,
            warning: true,
        },
        None => match node.detail.as_ref().filter(|detail| !detail.trim().is_empty()) {
            Some(detail) => PlanAdvice {
                title: tr("app.query.plan.note_title").to_string(),
                text: detail.clone(),
                danger: false,
                warning: false,
            },
            None => PlanAdvice {
                title: tr("app.query.plan.advice.ok_title").to_string(),
                text: tr("app.query.plan.advice.ok").to_string(),
                danger: false,
                warning: false,
            },
        },
    }
}

fn plan_advice_box(advice: &PlanAdvice, colors: UiColors) -> Div {
    let accent = if advice.danger {
        colors.danger
    } else if advice.warning {
        colors.warning
    } else {
        colors.success
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded(colors.radius)
        .bg(colors.panel_bg)
        .border_1()
        .border_color(accent)
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(accent)
                .child(if advice.danger || advice.warning {
                    app_icon(AppIcon::TriangleAlert, 12., accent)
                } else {
                    app_icon(AppIcon::CircleCheck, 12., accent)
                })
                .child(advice.title.clone()),
        )
        .child(
            div()
                .text_size(px(11.5))
                .line_height(px(18.))
                .text_color(colors.muted)
                .child(advice.text.clone()),
        )
}

fn query_plan_raw_table(tab_id: TabId, page: &DataPage, colors: UiColors) -> AnyElement {
    // MySQL / PostgreSQL 的结构化 EXPLAIN 只有一列 JSON；直接当表格画会变成
    // 一列窄文本、无高亮且难以阅读。复用 JSON 组件的只读预览（高亮 + 折叠）。
    if let Some(source) = single_json_cell(page) {
        let mut component = JsonComponent::new(JsonEditorConfig::default());
        component.load(source);
        let rows = json_editor_rows(component.state());
        let theme = JsonEditorTheme::from_colors(colors);
        // 直接按行渲染，保持内容自然高度；`render_preview` 的 `size_full`
        // 会把内容压回视口高度，导致外层滚动容器没有可滚动区域。
        return div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .id(("query-plan-raw-scroll", tab_id.0))
            .debug_selector(|| "query-plan-raw-scroll".into())
            // 父级是 flex 列：不取消交叉轴拉伸，滚动包装层高度会跟着视口走、
            // 内容没有可滚动范围。这与执行摘要表格的处理保持一致。
            .self_start()
            .overflow_scrollbar()
            .p_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(640.))
                    .children(rows.iter().map(|row| {
                        redis_json_text_row(row, theme, 2, true)
                    })),
            )
            .into_any_element();
    }

    let mut table = div().flex().flex_col().min_w(px(720.)).child(raw_plan_row(
        page.columns.iter().map(|column| column.name.clone()).collect(),
        true,
        colors,
    ));
    for row in page.rows.iter().take(200) {
        let values = row
            .values
            .iter()
            .map(|value| {
                let mut text = value.display_label();
                if text.chars().count() > 400 {
                    text = text.chars().take(400).collect::<String>();
                    text.push('…');
                }
                text
            })
            .collect::<Vec<_>>();
        table = table.child(raw_plan_row(values, false, colors));
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .id(("query-plan-raw-scroll", tab_id.0))
        .debug_selector(|| "query-plan-raw-scroll".into())
        .self_start()
        .overflow_scrollbar()
        .p_3()
        .child(table)
        .into_any_element()
}

fn single_json_cell(page: &DataPage) -> Option<&str> {
    if page.rows.len() != 1 || page.rows[0].values.len() != 1 {
        return None;
    }
    match &page.rows[0].values[0] {
        CellValue::Text(text) | CellValue::Json(text) => {
            let trimmed = text.trim_start();
            (trimmed.starts_with('{') || trimmed.starts_with('[')).then_some(text.as_str())
        }
        _ => None,
    }
}

fn raw_plan_row(values: Vec<String>, header: bool, colors: UiColors) -> Div {
    let mut row = div().flex().flex_none().items_stretch();
    for value in values {
        row = row.child(
            div()
                .w(px(160.))
                .flex_none()
                .px_2()
                .py_1()
                .border_b_1()
                .border_r_1()
                .border_color(colors.border)
                .bg(if header {
                    colors.panel_alt
                } else {
                    colors.panel_bg
                })
                .text_size(px(11.5))
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .when(header, |this| {
                    this.font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                })
                .when(!header, |this| this.text_color(colors.text))
                .child(value),
        );
    }
    row
}

fn plan_op_label(node: &fluxdb_app::PlanNode) -> String {
    match node.op {
        fluxdb_app::PlanOpKind::Query => tr("app.query.plan.op.query").to_string(),
        fluxdb_app::PlanOpKind::Join => tr("app.query.plan.op.join").to_string(),
        fluxdb_app::PlanOpKind::Scan => tr("app.query.plan.op.scan").to_string(),
        fluxdb_app::PlanOpKind::IndexLookup => {
            tr("app.query.plan.op.index_lookup").to_string()
        }
        fluxdb_app::PlanOpKind::Sort => tr("app.query.plan.op.sort").to_string(),
        fluxdb_app::PlanOpKind::Aggregate => tr("app.query.plan.op.aggregate").to_string(),
        fluxdb_app::PlanOpKind::Limit => tr("app.query.plan.op.limit").to_string(),
        fluxdb_app::PlanOpKind::Union => tr("app.query.plan.op.union").to_string(),
        fluxdb_app::PlanOpKind::Subquery => tr("app.query.plan.op.subquery").to_string(),
        fluxdb_app::PlanOpKind::Other => {
            if node.operation.is_empty() {
                tr("app.query.plan.op.other").to_string()
            } else {
                node.operation.clone()
            }
        }
    }
}

fn plan_op_icon(op: fluxdb_app::PlanOpKind, colors: UiColors) -> (AppIcon, gpui::Rgba) {
    match op {
        fluxdb_app::PlanOpKind::Query => (AppIcon::Workflow, colors.info),
        fluxdb_app::PlanOpKind::Join => (AppIcon::Link, colors.info),
        fluxdb_app::PlanOpKind::Scan => (AppIcon::Table, colors.danger),
        fluxdb_app::PlanOpKind::IndexLookup => (AppIcon::Search, colors.success),
        fluxdb_app::PlanOpKind::Sort => (AppIcon::ArrowUpDown, colors.warning),
        fluxdb_app::PlanOpKind::Aggregate => (AppIcon::List, colors.muted),
        fluxdb_app::PlanOpKind::Limit => (AppIcon::Filter, colors.muted),
        fluxdb_app::PlanOpKind::Union => (AppIcon::Workflow, colors.info),
        fluxdb_app::PlanOpKind::Subquery => (AppIcon::FileSql, colors.muted),
        fluxdb_app::PlanOpKind::Other => (AppIcon::Activity, colors.muted),
    }
}

fn plan_node_subtitle(node: &fluxdb_app::PlanNode) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(object) = node.object.as_ref() {
        parts.push(object.clone());
    }
    if let Some(detail) = node.detail.as_ref()
        && !detail.is_empty()
        && !parts.iter().any(|part| part == detail)
    {
        parts.push(detail.clone());
    }
    if let Some(index) = node.index.as_ref()
        && !parts.iter().any(|part| part == index)
    {
        parts.push(index.clone());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn plan_cost_ratio(
    plan: &fluxdb_app::QueryPlan,
    node: &fluxdb_app::PlanNode,
) -> Option<f32> {
    let total = plan.total_cost?;
    let cost = node.cost?;
    if total <= 0. {
        return None;
    }
    Some((cost / total).clamp(0.02, 1.) as f32)
}

fn plan_format_cost(value: f64) -> String {
    if value >= 1000. {
        format!("{:.1}k", value / 1000.)
    } else if (value.fract()).abs() < 0.05 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

fn plan_format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn plan_node_copy_text(node: &fluxdb_app::PlanNode) -> String {
    let mut lines = vec![format!("{} {}", plan_op_label(node), node.object.clone().unwrap_or_default())];
    if let Some(cost) = node.cost {
        lines.push(format!("cost: {}", plan_format_cost(cost)));
    }
    if let Some(rows) = node.rows {
        lines.push(format!("rows: {}", plan_format_count(rows)));
    }
    if let Some(condition) = node.condition.as_ref() {
        lines.push(format!("condition: {condition}"));
    }
    if let Some(index) = node.index.as_ref() {
        lines.push(format!("index: {index}"));
    }
    for issue in &node.issues {
        let label = match issue {
            fluxdb_app::PlanIssue::FullTableScan => {
                tr("app.query.plan.issue.full_scan").to_string()
            }
            fluxdb_app::PlanIssue::Filesort => {
                tr("app.query.plan.issue.filesort").to_string()
            }
            fluxdb_app::PlanIssue::NoIndex => {
                tr("app.query.plan.issue.no_index").to_string()
            }
        };
        lines.push(format!("issue: {label}"));
    }
    lines.join("\n")
}
