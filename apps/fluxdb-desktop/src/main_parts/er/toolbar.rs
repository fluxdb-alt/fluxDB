// ER 工具栏、搜索、分组切换和导入导出菜单渲染。

/// 顶部工具栏：标题 + 计数 + 关系状态 + 深度 + 回到起点 / 按关系排列 / 重置手动位置。

fn er_toolbar(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let current_depth = this.er_depths.get(&tab_id).copied().unwrap_or(1);
    let table_count = this
        .er_graphs
        .get(&tab_id)
        .map(|g| g.tables.len())
        .unwrap_or(0);
    let relation_status = this
        .er_graphs
        .get(&tab_id)
        .map(|g| g.relation_status)
        .unwrap_or(ErLoadStatus::NotLoaded);
    let interacted = this.er_user_interacted.contains(&tab_id);
    let applied = this.er_layout_applied.contains(&tab_id);

    let mut base = div()
        .h(px(40.))
        .flex()
        .items_center()
        .px(px(12.))
        .gap(px(12.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg);

    let title = match &er.center_table {
        Some(table) => tr("app.er.title_centered")
            .replace("%{database}", &er.database)
            .replace("%{table}", &table.display())
            .replace("%{depth}", &current_depth.to_string()),
        None => tr("app.er.title").replace("%{database}", &er.database),
    };
    base = base.child(
        div()
            .text_size(ui_text_size(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(colors.text)
            .child(title),
    );
    let total_tables = this.er_full_tables.get(&tab_id).map(Vec::len);
    let (loaded, failed, edge_count) = this.er_graphs.get(&tab_id)
        .map(|graph| (
            graph.tables.iter().filter(|t| t.status == ErLoadStatus::Loaded).count(),
            graph.tables.iter().filter(|t| t.status == ErLoadStatus::Failed).count(),
            graph.edges.len(),
        )).unwrap_or((0, 0, 0));
    let range = total_tables
        .map(|total| {
            tr("app.er.table_range")
                .replace("%{loaded}", &table_count.to_string())
                .replace("%{total}", &total.to_string())
        })
        .unwrap_or_else(|| tr("app.er.reading_tables").to_string());
    base = base.child(
        div().text_size(ui_text_size(12.)).text_color(colors.muted).child(range),
    );
    if table_count > 0 {
        base = base.child(
            div().text_size(ui_text_size(11.)).text_color(if failed > 0 { rgb(0xef4444) } else { colors.muted })
                .child(
                    tr("app.er.field_relation")
                        .replace("%{loaded}", &loaded.to_string())
                        .replace("%{total}", &table_count.to_string())
                        .replace("%{edges}", &edge_count.to_string())
                        .replace(
                            "%{failed}",
                            &if failed > 0 {
                                tr("app.er.failed_suffix")
                                    .replace("%{count}", &failed.to_string())
                            } else {
                                String::new()
                            },
                        ),
                ),
        );
    }
    if er.center_table.is_some() {
        let back_path = ObjectPath {
            connection_id: er.connection_id,
            database: Some(er.database.clone()),
            schema: er.schema.clone(),
            name: er.database.clone(),
            kind: ObjectKind::Database,
        };
        base = base.child(
            Button::new(("er-back-full", tab_id.0))
                .ghost().xsmall().h(px(28.))
                .child(app_icon(AppIcon::ChevronLeft, 14., colors.muted))
                .label(tr("app.er.back_full_database"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::OpenErDiagram(back_path.clone()), cx);
                })),
        );
    }
    if relation_status != ErLoadStatus::Loaded {
        let (label, tone) = match relation_status {
            ErLoadStatus::NotLoaded | ErLoadStatus::Loading => {
                (tr("app.er.relations_loading"), colors.muted)
            }
            ErLoadStatus::Failed => (tr("app.er.relations_incomplete"), rgb(0xef4444)),
            _ => unreachable!(),
        };
        base = base.child(div().text_size(ui_text_size(12.)).text_color(tone).child(label));
        if relation_status == ErLoadStatus::Failed {
            // 关系加载失败：提供显式重试入口（作废 Failed 缓存后由 sync_er_relations 重新读取）。
            let retry_conn = er.connection_id;
            let retry_db = er.database.clone();
            let retry_schema = er.schema.clone();
            let retry_tab = tab_id;
            base = base.child(
                Button::new(("er-rel-retry", tab_id.0))
                    .ghost()
                    .xsmall()
                    .h(px(24.))
                    .label(tr("app.er.retry_relations"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(config) = this
                            .controller
                            .connection_configs()
                            .into_iter()
                            .find(|c| c.id == retry_conn)
                        {
                            this.controller.er_relations_invalidate_failed(
                                &config,
                                &retry_db,
                                retry_schema.as_deref(),
                            );
                            // 清 graph 的 Failed，使 sync_er_relations 重新发起读取。
                            if let Some(graph) = this.er_graphs.get_mut(&retry_tab) {
                                graph.relation_status = ErLoadStatus::NotLoaded;
                            }
                            this.er_advance_generation(retry_tab);
                            this.er_relation_tasks.remove(&retry_tab);
                        }
                        cx.notify();
                    })),
            );
        }
    } else if interacted && !applied {
        // 用户已操作且关系布局未应用：轻量提示 + 按钮可用（§6.2）。
        base = base.child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.er.relations_ready")),
        );
    }
    base = base.child(div().flex_1());

    let rel_panel_tab = tab_id;
    base = base.child(
        Button::new(("er-relationships", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.relationships_tooltip"))
            .child(app_icon(AppIcon::PanelRight, 15., colors.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.er_relationship_panel_open.contains(&rel_panel_tab) {
                    this.er_relationship_panel_open.remove(&rel_panel_tab);
                } else {
                    this.er_relationship_panel_open.insert(rel_panel_tab);
                    this.er_relationship_panel_focus.entry(rel_panel_tab)
                        .or_insert_with(|| cx.focus_handle()).focus(window, cx);
                }
                cx.notify();
            })),
    );

    // 缩放控件（视图变换缩放，§六.23-24）：- / % / + / 适配当前范围。
    let zoom = this.er_canvas.borrow().er_viewports.get(&tab_id).map(|v| v.safe_scale()).unwrap_or(1.0);
    let zoom_out_tab = tab_id;
    base = base.child(
        Button::new(("er-zoom-out", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.zoom_out"))
            .child(app_icon(AppIcon::Minus, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.zoom_er_around_center(zoom_out_tab, 1.0 / 1.25);
                cx.notify();
            })),
    );
    base = base.child(
        div()
            .w(px(44.))
            .text_center()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(format!("{:.0}%", zoom * 100.0)),
    );
    let zoom_in_tab = tab_id;
    base = base.child(
        Button::new(("er-zoom-in", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.zoom_in"))
            .child(app_icon(AppIcon::Plus, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.zoom_er_around_center(zoom_in_tab, 1.25);
                cx.notify();
            })),
    );
    let fit_tab = tab_id;
    base = base.child(
        Button::new(("er-fit", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.fit_scope"))
            .child(app_icon(AppIcon::Maximize, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.er_fit_scope(fit_tab);
                cx.notify();
            })),
    );

    // 导出（§十）：打开对话框，选格式生成文本导出到本地文件；另含导入 JSON。
    let export_tab = tab_id;
    base = base.child(
        Button::new(("er-export", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.export_tooltip"))
            // 导出到文件（打开保存对话框），保留 Save 图标语义。
            .child(app_icon(AppIcon::Save, 15., colors.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.er_export_open.contains(&export_tab) {
                    this.er_export_open.remove(&export_tab);
                    this.er_export_focus.remove(&export_tab);
                    this.er_export_sel.remove(&export_tab);
                } else {
                    // 打开对话框并聚焦，使键盘导航（上下/Enter/Esc）路由到对话框。
                    let focus = this
                        .er_export_focus
                        .entry(export_tab)
                        .or_insert_with(|| cx.focus_handle())
                        .clone();
                    this.er_export_open.insert(export_tab);
                    this.er_export_sel.entry(export_tab).or_insert(0);
                    focus.focus(window, cx);
                }
                cx.notify();
            })),
    );

    if er.center_table.is_some() {
        base = base.child(er_depth_selector(tab_id, er, current_depth, colors, cx));
    }

    // 回到起点：回到布局初始视口，不清人工坐标/数据缓存（§6.3）。
    let back_tab = tab_id;
    base = base.child(
        Button::new(("er-back", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.back_to_start"))
            .child(app_icon(AppIcon::Home, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.er_restore_initial_view(back_tab);
                this.mark_er_interacted(back_tab);
                cx.notify();
            })),
    );
    // 按关系排列：保留手动固定节点；不重读数据库、不清缓存（§6.3）。
    let arrange_tab = tab_id;
    base = base.child(
        Button::new(("er-arrange", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.arrange_by_relations"))
            .child(app_icon(AppIcon::Wand, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.er_remember_layout(arrange_tab);
                this.apply_er_layout(arrange_tab);
                this.er_restore_initial_view(arrange_tab);
                this.er_save_view_state(arrange_tab);
                cx.notify();
            })),
    );
    // 重置手动位置并排列：显式清除固定（§6.3）。
    let reset_tab = tab_id;
    base = base.child(
        Button::new(("er-reset-pins", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.reset_pins"))
            .child(app_icon(AppIcon::Undo, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.er_remember_layout(reset_tab);
                this.er_canvas.borrow_mut().er_pinned.remove(&reset_tab);
                this.apply_er_layout(reset_tab);
                this.er_restore_initial_view(reset_tab);
                this.er_save_view_state(reset_tab);
                cx.notify();
            })),
    );

    if this.er_previous_layout.contains_key(&tab_id) {
        let undo_tab = tab_id;
        base = base.child(
            Button::new(("er-undo-layout", tab_id.0)).ghost().xsmall().h(px(28.))
                .tooltip(tr("app.er.undo_layout"))
                .child(app_icon(AppIcon::Undo, 15., colors.muted))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.er_restore_previous_layout(undo_tab) {
                        this.show_message(
                            tr("app.er.layout_restore_failed"),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    cx.notify();
                })),
        );
    }

    // 更新时间（缓存时效反馈，§3.5）。
    if let Some(upd) = this.er_last_updated.get(&tab_id) {
        base = base.child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.er.updated_at").replace("%{time}", &upd)),
        );
    }
    // 刷新进行中：轻量提示（保留当前可用图，只重读元数据，§3.3/§3.5）。
    if this.er_refreshing.contains(&tab_id) {
        base = base.child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.er.refreshing")),
        );
    }
    if this.er_view_save_failed.contains(&tab_id) {
        base = base.child(
            div().text_size(ui_text_size(11.)).text_color(rgb(0xef4444))
                .child(tr("app.er.layout_save_failed")),
        );
    }
    // 手动刷新：重读元数据（表目录+关系），保留当前可用图、视口与人工布局（§3.3）。
    let refresh_er = er.clone();
    let refresh_tab = tab_id;
    base = base.child(
        Button::new(("er-refresh", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip(tr("app.er.refresh_metadata"))
            .disabled(this.er_refreshing.contains(&tab_id))
            .child(app_icon(AppIcon::Refresh, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.er_refreshing.contains(&refresh_tab) {
                    return;
                }
                this.er_refreshing.insert(refresh_tab);
                // 作废 app 层字段 + 关系缓存，使刷新真正重读数据库（外部删列/改列/外键变化
                // 不残留旧结构与旧连线）。保留 er_graphs 当前可用图、坐标/视口/固定。
                if let Some(config) = this
                    .controller
                    .connection_configs()
                    .into_iter()
                    .find(|c| c.id == refresh_er.connection_id)
                {
                    let refs: Vec<fluxdb_core::ErTableRef> = this
                        .er_full_tables
                        .get(&refresh_tab)
                        .map(|t| t.iter().map(|n| n.reference.clone()).collect())
                        .unwrap_or_default();
                    if !refs.is_empty() {
                        this.controller.er_columns_invalidate(&config, &refs);
                    }
                    this.controller.er_relations_invalidate(
                        &config,
                        &refresh_er.database,
                        refresh_er.schema.as_deref(),
                    );
                }
                // 清表目录与关系缓存使重读。
                this.er_advance_generation(refresh_tab);
                this.er_load_tasks.remove(&refresh_tab);
                this.er_full_tables.remove(&refresh_tab);
                this.er_all_edges.remove(&refresh_tab);
                this.er_relation_tasks.remove(&refresh_tab);
                // 记录刷新开始为“加载中”，展示「刷新中…」。
                let er = refresh_er.clone();
                ensure_er_graph_loaded(refresh_tab, &er, this, cx);
                cx.notify();
            })),
    );

    base
}

/// ER 表搜索条（§五.6）：仅输入框行（命中列表作为画布后置兄弟绝对定位浮层渲染，
/// 因 GPUI 无 z-index，后置者在上，见 `er_search_dropdown`）。
/// 无场景（元数据未到）时返回空元素，不占工具栏行。
fn er_search_bar(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if !this.er_scenes.contains_key(&tab_id) {
        return div();
    }
    let Some(search_input) = this.er_search_input.get(&tab_id).cloned() else {
        return div();
    };
    let matches = this.er_search_matches(tab_id);
    let count_text = if this.er_search_query.get(&tab_id).map(|q| !q.trim().is_empty()).unwrap_or(false) {
        tr("app.er.search_matches").replace("%{count}", &matches.len().to_string())
    } else {
        String::new()
    };
    let bar_tab = tab_id;
    div()
        .px(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon(AppIcon::Search, 15., colors.muted))
        .child(
            div()
                .flex_1()
                .h(px(28.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .px_2()
                // 键盘导航：上/下移动选择，Enter 定位选中表，Esc 清空查询并关闭悬浮层。
                .on_key_down(
                    cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                        let key = event.keystroke.key.as_str();
                        if key == "escape" {
                            if let Some(input) = this.er_search_input.get(&bar_tab) {
                                input.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                            this.er_search_query.remove(&bar_tab);
                            this.er_search_sel.remove(&bar_tab);
                            cx.notify();
                            return;
                        }
                        let matches = this.er_search_matches(bar_tab);
                        if matches.is_empty() {
                            return;
                        }
                        let cur = this.er_search_sel.get(&bar_tab).copied().unwrap_or(0);
                        let next = match key {
                            "Down" => Some((cur + 1).min(matches.len() - 1)),
                            "Up" => Some(cur.saturating_sub(1)),
                            "Enter" | "Return" => {
                                this.er_center_on_table(bar_tab, &matches[cur]);
                                cx.notify();
                                return;
                            }
                            _ => None,
                        };
                        if let Some(n) = next {
                            this.er_search_sel.insert(bar_tab, n);
                            cx.notify();
                        }
                    }),
                )
                .child(
                    Input::new(&search_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(ui_text_size(13.)),
                ),
        )
        .child(div().text_size(ui_text_size(11.)).text_color(colors.muted).child(count_text))
}

/// 分组过滤的纯逻辑（§五.10-11）：None=返回全部；Some(schema)=仅保留该 schema 的表，
/// 边只保留两端都属本组者（真实外键，不伪装聚合 JOIN）。孤立表仍保留（始终可达可搜索）。
fn er_group_subset_graph(graph: &ErGraphData, group: Option<&str>) -> ErGraphData {
    let Some(schema) = group else {
        return graph.clone();
    };
    let included: std::collections::BTreeSet<String> = graph
        .tables
        .iter()
        .filter(|t| t.reference.schema.as_deref() == Some(schema))
        .map(|t| t.name.clone())
        .collect();
    let edges = graph
        .edges
        .iter()
        .filter(|e| included.contains(&e.from_table) && included.contains(&e.to_table))
        .cloned()
        .collect();
    let tables = graph
        .tables
        .iter()
        .filter(|t| included.contains(&t.name))
        .cloned()
        .collect();
    ErGraphData {
        tables,
        edges,
        relation_status: graph.relation_status,
    }
}

/// 业务组只过滤画布视图，不复制或更改关系目录，跨组边在组内不绘制。
fn er_custom_group_subset_graph(
    graph: &ErGraphData,
    members: Option<&BTreeSet<String>>,
) -> ErGraphData {
    let tables: Vec<_> = graph.tables.iter()
        .filter(|table| members.is_some_and(|set| set.contains(&table.name)))
        .cloned().collect();
    let included: BTreeSet<_> = tables.iter().map(|table| table.name.as_str()).collect();
    let edges = graph.edges.iter()
        .filter(|edge| included.contains(edge.from_table.as_str()) && included.contains(edge.to_table.as_str()))
        .cloned().collect();
    ErGraphData { tables, edges, relation_status: graph.relation_status }
}

/// ER 导出对话框（§十）：JSON / DBML / Mermaid / SVG 导出到本地文件 + 导入 JSON。
/// 居中弹窗（GPUI 无 z-index，用遮罩弹框避开画布裁剪/层级问题），复用仓库画布内
/// 模态（见 `data_export_modal_shell`）的交互约定：遮罩点击/Esc 关闭、内部点击不穿透。
/// 导出内容仍由 `er/export.rs` 纯函数生成，这里只负责“选路径写文件”。导入另行校验
/// （未做 D1-D9 逻辑关系）。SVG/DBML 为有损导出，行内带「有损」标签。
fn er_export_menu(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if !this.er_export_open.contains(&tab_id) {
        return div();
    }
    let Some(graph) = this.er_graphs.get(&tab_id) else {
        return div();
    };
    let connection_label = this
        .controller
        .connection_configs()
        .into_iter()
        .find(|c| c.id == er.connection_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| tr("app.er.connection").to_string());
    let title = format!("{} · {}", er.database, er.schema.as_deref().unwrap_or(""));
    // 导出范围子标题：连接 / 库 / schema，让用户知道导的是哪张图。
    let scope = format!("{connection_label} / {title}");
    let sel_idx = this
        .er_export_sel
        .get(&tab_id)
        .copied()
        .unwrap_or(0)
        .min(4);
    // 对话框焦点句柄：打开时按钮已聚焦它，键盘上下/Enter/Esc 路由到面板。
    let focus = this.er_export_focus.get(&tab_id).cloned();

    // 导出项定义：格式 / 说明文案 / 是否「有损」。点击或 Enter 走 `er_export_start_save`
    // 生成文本并选保存路径写文件（格式为 JSON/DBML/Mermaid/SVG，内容由 export.rs 生成）。
    let actions: [(ErExportKind, SharedString, bool); 4] = [
        (
            ErExportKind::Json,
            tr("app.er.export_json_desc"),
            false,
        ),
        (
            ErExportKind::Dbml,
            tr("app.er.export_dbml_desc"),
            true,
        ),
        (
            ErExportKind::Mermaid,
            tr("app.er.export_mermaid_desc"),
            false,
        ),
        (
            ErExportKind::Svg,
            tr("app.er.export_svg_desc"),
            true,
        ),
    ];

    let mut panel = div()
        .relative()
        .w(px(360.))
        .max_w(px(360.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(16.),
            px(34.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.44 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .key_context("ErExportDialog")
        .on_action(cx.listener(move |this, _: &CancelDialog, window, cx| {
            this.er_close_export_menu(tab_id, window, cx);
            cx.stop_propagation();
        }))
        .on_key_down(
            cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if key == "escape" {
                    this.er_close_export_menu(tab_id, window, cx);
                    return;
                }
                let cur = this.er_export_sel.get(&tab_id).copied().unwrap_or(0);
                let next = match key {
                    "Down" => Some((cur + 1).min(4)),
                    "Up" => Some(cur.saturating_sub(1)),
                    _ => None,
                };
                if let Some(n) = next {
                    this.er_export_sel.insert(tab_id, n);
                    cx.notify();
                } else if key == "Enter" || key == "Return" {
                    this.er_export_activate(tab_id, cur, cx);
                }
            }),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when_some(focus, |panel, f| panel.track_focus(&f))
        // 头部：标题 + 导出范围子标题 + 关闭按钮。
        .child(
            div()
                .h(px(52.))
                .flex_none()
                .px_4()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .child(
                            div()
                                .text_size(ui_text_size(15.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(tr("app.er.export_dialog_title")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .overflow_hidden()
                                .truncate()
                                .child(scope),
                        ),
                )
                .child(
                    div()
                        .size(px(28.))
                        .rounded(colors.radius)
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |style| style.bg(colors.hover))
                        .child(app_icon(AppIcon::Close, 15., colors.muted))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.er_close_export_menu(tab_id, window, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ),
        )
        .child(div().flex().flex_col().py_1());

    // 导出 4 项（JSON / DBML / Mermaid / SVG），点击导出到本地文件。
    for (idx, (kind, desc, lossy)) in actions.iter().enumerate() {
        let kind = *kind;
        let desc = desc.clone();
        let lossy = *lossy;
        panel = panel.child(
            er_export_action_row(
                tab_id,
                sel_idx,
                idx,
                kind.label(),
                desc,
                lossy,
                colors,
                cx,
            ),
        );
    }
    // 导出与导入的分隔线。
    panel = panel.child(
        div().mx_3().my_1().h(px(1.)).bg(colors.border_soft),
    );
    // 导入 JSON：读剪贴板 → 校验 → 差异预览（应用布局仍须再次核对真实连接身份）。
    panel = panel.child(er_export_import_row(tab_id, sel_idx, 4, colors, cx));
    if let Some(report) = this.er_last_import.get(&tab_id) {
        let matched = report.positions.keys().filter(|name| graph.tables.iter().any(|t| t.name == **name)).count();
        let allowed = er_import_scope_matches(report, er.connection_id.0, &er.database);
        let summary = tr("app.er.import_preview")
            .replace("%{matched}", &matched.to_string())
            .replace("%{added}", &report.added_tables.len().to_string())
            .replace("%{missing}", &report.missing_tables.len().to_string())
            .replace("%{unresolved}", &report.unresolved_edges.len().to_string());
        panel = panel.child(
            div().px_3().py(px(6.)).border_t_1().border_color(colors.border_soft)
                .text_size(ui_text_size(11.)).text_color(colors.muted).child(summary),
        );
        let apply_tab = tab_id;
        let apply_db = er.database.clone();
        let apply_connection = er.connection_id.0;
        panel = panel.child(
            Button::new(("er-import-apply-layout", tab_id.0))
                .ghost().small().w_full()
                .label(if allowed {
                    tr("app.er.apply_matched_layout")
                } else {
                    tr("app.er.connection_mismatch_preview_only")
                })
                .disabled(!allowed || matched == 0)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.er_export_open.remove(&apply_tab);
                    this.er_apply_import_layout(apply_tab, apply_connection, &apply_db, cx);
                })),
        );
    }

    // 遮罩：点击空白关闭；内部面板不穿透。
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.46)
        } else {
            opaque_grey(0.75, 0.22)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.er_close_export_menu(tab_id, window, cx);
                cx.stop_propagation();
            }),
        )
        .child(panel)
}

impl NavicatMain {
    /// ER 导出对话框关闭：清打开态与选中，返回焦点到工具箱根（触发按钮由调用侧再次点击恢复）。
    fn er_close_export_menu(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<NavicatMain>) {
        self.er_export_open.remove(&tab_id);
        self.er_export_sel.remove(&tab_id);
        self.er_export_focus.remove(&tab_id);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// ER 导出对话框键盘/Enter 激活：按下 Enter 对当前高亮项执行。
    fn er_export_activate(&mut self, tab_id: TabId, idx: usize, cx: &mut Context<NavicatMain>) {
        // 第 0..=3 项为导出格式，第 4 项为导入 JSON。
        if idx == 4 {
            self.er_export_import_from_clipboard(tab_id, cx);
            return;
        }
        let kind = match idx {
            0 => ErExportKind::Json,
            1 => ErExportKind::Dbml,
            2 => ErExportKind::Mermaid,
            3 => ErExportKind::Svg,
            _ => return,
        };
        self.er_export_start_save(tab_id, kind, cx);
    }

    /// 取某 tab 的 ER 图状态（连接/库/schema），非 ER 标签返回 None。
    fn er_diagram_state_for_tab(&self, tab_id: TabId) -> Option<ErDiagramState> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| match &t.kind {
                TabKind::ErDiagram(er) => Some(er.clone()),
                _ => None,
            })
    }
}

/// 导出格式枚举：名称 / 后缀 / 建议文件名，文本由 `er/export.rs` 纯函数生成。
#[derive(Clone, Copy)]
enum ErExportKind { Json, Dbml, Mermaid, Svg }

impl ErExportKind {
    fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Dbml => "DBML",
            Self::Mermaid => "Mermaid",
            Self::Svg => "SVG",
        }
    }

    /// 保存对话框的建议文件名（`{库或表}.{ext}`），落盘时补全后缀。
    fn suggested_name(self, er: &ErDiagramState) -> String {
        let base = er.center_table.as_ref().map(|t| t.display()).unwrap_or_else(|| er.database.clone());
        let fn_seg = safe_data_export_filename_segment(&base);
        format!("er-{fn_seg}.{}", self.extension())
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Dbml => "dbml",
            Self::Mermaid => "mmd",
            Self::Svg => "svg",
        }
    }
}

/// 单个导出项行：名称 + 说明 + （有损标签）。鼠标点击触发导出到文件。
fn er_export_action_row(
    tab_id: TabId,
    sel_idx: usize,
    idx: usize,
    name: impl Into<SharedString>,
    desc: impl Into<SharedString>,
    lossy: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name = name.into();
    let desc = desc.into();
    let is_sel = idx == sel_idx;
    let row_tab = tab_id;
    let row_kind = match idx {
        0 => ErExportKind::Json,
        1 => ErExportKind::Dbml,
        2 => ErExportKind::Mermaid,
        _ => ErExportKind::Svg,
    };
    div()
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .when(is_sel, |s| s.bg(colors.hover))
        .cursor_pointer()
        .hover(|s| s.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.er_export_sel.insert(row_tab, idx);
                this.er_export_start_save(row_tab, row_kind, cx);
                cx.stop_propagation();
            }),
        )
        .child(div().w(px(58.)).text_size(ui_text_size(12.)).font_weight(gpui::FontWeight::MEDIUM).text_color(colors.text).child(name))
        .child(div().flex_1().min_w(px(0.)).text_size(ui_text_size(11.)).text_color(colors.muted).overflow_hidden().truncate().child(desc))
        // 有损标记：SVG（表级连线简化）与 DBML（不表达确认状态/布局）。
        .when(lossy, |row| {
            row.child(
                div()
                    .h(px(16.))
                    .px_1()
                    .rounded(colors.radius)
                    .bg(colors.panel_alt)
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .flex()
                    .items_center()
                    .child(tr("app.er.lossy")),
            )
        })
}

/// 导入 JSON 行：读剪贴板 → 校验 → 差异预览（流程保持现有实现，仅移到弹窗分组内）。
fn er_export_import_row(
    tab_id: TabId,
    sel_idx: usize,
    idx: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_sel = idx == sel_idx;
    let import_tab = tab_id;
    div()
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .when(is_sel, |s| s.bg(colors.hover))
        .cursor_pointer()
        .hover(|s| s.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.er_export_sel.insert(import_tab, 4);
                this.er_export_import_from_clipboard(import_tab, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(62.))
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(tr("app.er.import_json")),
        )
        .child(
            div()
                .flex_1()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.er.import_desc")),
        )
}

impl NavicatMain {
    /// 从剪贴板解析并校验导入 ER JSON，产生差异预览（不应用布局，只读当前模型）。
    fn er_export_import_from_clipboard(&mut self, tab_id: TabId, cx: &mut Context<NavicatMain>) {
        let Some(graph) = self.er_graphs.get(&tab_id).cloned() else {
            return;
        };
        let database = self
            .er_diagram_state_for_tab(tab_id)
            .map(|er| er.database)
            .unwrap_or_default();
        let text = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        if text.trim().is_empty() {
            self.show_message(
                tr("app.er.clipboard_empty"),
                AppMessageKind::Warning,
                cx,
            );
            cx.notify();
            return;
        }
        match er_import_parse(&text, &graph, &database) {
            Ok(report) => {
                if !er_import_database_matches(&report, &database) {
                    self.show_message(
                        tr("app.er.import_database_mismatch")
                            .replace("%{imported}", &report.database)
                            .replace("%{current}", &database),
                        AppMessageKind::Warning,
                        cx,
                    );
                } else {
                    let mut parts = vec![
                        tr("app.er.import_validated")
                            .replace("%{tables}", &report.tables.len().to_string())
                            .replace("%{edges}", &report.edges.len().to_string()),
                    ];
                    if !report.added_tables.is_empty() {
                        parts.push(
                            tr("app.er.import_added_tables")
                                .replace("%{count}", &report.added_tables.len().to_string()),
                        );
                    }
                    if !report.missing_tables.is_empty() {
                        parts.push(
                            tr("app.er.import_missing_tables")
                                .replace("%{count}", &report.missing_tables.len().to_string()),
                        );
                    }
                    if !report.unresolved_edges.is_empty() {
                        parts.push(
                            tr("app.er.import_unresolved_edges")
                                .replace("%{count}", &report.unresolved_edges.len().to_string()),
                        );
                    }
                    self.show_message(parts.join("；"), AppMessageKind::Info, cx);
                }
                self.er_last_import.insert(tab_id, report);
            }
            Err(err) => {
                self.show_message(
                    tr("app.er.import_failed").replace("%{error}", &err.to_string()),
                    AppMessageKind::Error,
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// 生成选定格式文本并弹出保存对话框写入本地文件；用户取消则不落盘。
    fn er_export_start_save(
        &mut self,
        tab_id: TabId,
        kind: ErExportKind,
        cx: &mut Context<NavicatMain>,
    ) {
        let Some(er) = self.er_diagram_state_for_tab(tab_id) else {
            self.show_message(
                tr("app.er.export_no_data"),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        let Some(graph) = self.er_graphs.get(&tab_id).cloned() else {
            return;
        };
        let positions = self.er_canvas.borrow().er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
        let pinned = self.er_canvas.borrow().er_pinned.get(&tab_id).cloned().unwrap_or_default();
        let connection_label = self
            .controller
            .connection_configs()
            .into_iter()
            .find(|c| c.id == er.connection_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("app.er.connection").to_string());
        let title = format!("{} · {}", er.database, er.schema.as_deref().unwrap_or(""));
        // 导出内容仍由 export.rs 纯函数生成（结构快照+外键+坐标），这里不改生成逻辑。
        let content = match kind {
            ErExportKind::Json => er_export_json(&graph, &positions, &pinned, &connection_label, er.connection_id.0, &er.database),
            ErExportKind::Dbml => er_export_dbml(&graph),
            ErExportKind::Mermaid => er_export_mermaid(&graph),
            ErExportKind::Svg => er_export_svg(&graph, &positions, &title),
        };
        self.er_export_open.remove(&tab_id);
        self.er_export_focus.remove(&tab_id);

        let suggested = kind.suggested_name(&er);
        let extension = kind.extension();
        let label = kind.label();
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        // 导出写文件异步任务：GPUI 任务句柄需保持到完成，这里显式丢弃可运行完整。
        let _ = cx.spawn(async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(path))) => Some(safe_data_export_path_with_extension(path, extension)),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            // 后台写文件避免阻塞 UI；完成后再回主线程播报。
            let path_display = path.display().to_string();
            let result = cx.background_spawn(async move {
                std::fs::write(&path, content).map(|_| ())
            }).await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(()) => {
                        this.show_message(
                            tr("app.er.exported_to")
                                .replace("%{format}", &label)
                                .replace("%{path}", &path_display),
                            AppMessageKind::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        this.show_message(
                            tr("app.er.export_failed")
                                .replace("%{format}", &label)
                                .replace("%{error}", &error.to_string()),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                });
            });
        });
    }
}

/// ER 业务分组选择条（§五.9-12）：按 schema 分组的进入/返回全部；折叠进组仅过滤展示
/// 内容，不丢坐标/固定，可随时回全部。无场景或仅一个 schema 时返回空（不占行、不隐藏表）。
fn er_group_bar(
    tab_id: TabId,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let groups = this.er_schema_groups(tab_id);
    if groups.is_empty() {
        return div();
    }
    if !this.er_group_inputs.contains_key(&tab_id) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.er.group_placeholder"))
        });
        this.er_group_inputs.insert(tab_id, input);
    }
    let active = this.er_group.get(&tab_id).cloned().unwrap_or(None);
    let mut row = div()
        .px(px(12.))
        .py(px(4.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.er.group_label")),
        );
    // 「全部」chip。
    let all_tab = tab_id;
    row = row.child(
        er_group_chip(
            tr("app.er.all_groups")
                .replace("%{count}", &groups.iter().map(|(_, c)| c).sum::<usize>().to_string()),
            active.is_none(),
            all_tab,
            None,
            colors,
            cx,
        ),
    );
    // 各 schema chip。
    for (schema, count) in groups.iter() {
        if schema.is_none() {
            continue;
        }
        let label = tr("app.er.group_chip")
            .replace("%{name}", schema.as_deref().unwrap_or(""))
            .replace("%{count}", &count.to_string());
        let chip_schema = schema.clone();
        let chip_tab = tab_id;
        row = row.child(er_group_chip(&label, active.as_deref() == schema.as_deref(), chip_tab, chip_schema, colors, cx));
    }
    if let Some(custom) = this.er_custom_groups.get(&tab_id) {
        for (name, members) in custom {
            let key = format!("\0custom:{name}");
            row = row.child(er_group_chip(
                tr("app.er.group_chip")
                    .replace("%{name}", name)
                    .replace("%{count}", &members.len().to_string()),
                active.as_deref() == Some(&key),
                tab_id, Some(key), colors, cx,
            ));
        }
    }
    let input = this.er_group_inputs.get(&tab_id).cloned().unwrap();
    let add_tab = tab_id;
    row = row.child(
        div().w(px(118.)).h(px(24.)).flex_shrink_0()
            .child(Input::new(&input).small().w_full().h_full()),
    ).child(
        Button::new(("er-add-custom-group", tab_id.0)).ghost().xsmall()
            .label(tr("app.er.add_selected_to_group"))
            .on_click(cx.listener(move |this, _, _, cx| this.er_add_selected_to_custom_group(add_tab, cx))),
    );
    if let Some(name) = active.as_deref().and_then(|group| group.strip_prefix("\0custom:")) {
        let name = name.to_string();
        let can_delete = this.er_custom_groups.get(&tab_id).and_then(|g| g.get(&name))
            .is_some_and(BTreeSet::is_empty);
        let delete_name = name.clone();
        let delete_tab = tab_id;
        row = row.child(
            Button::new(("er-delete-empty-group", tab_id.0)).ghost().xsmall()
                .label(tr("app.er.delete_empty_group")).disabled(!can_delete)
                .on_click(cx.listener(move |this, _, _, cx| this.er_delete_empty_custom_group(delete_tab, &delete_name, cx))),
        );
        let remove_tab = tab_id;
        row = row.child(
            Button::new(("er-remove-custom-member", tab_id.0)).ghost().xsmall()
                .label(tr("app.er.remove_selected"))
                .on_click(cx.listener(move |this, _, _, cx| this.er_remove_selected_from_custom_group(remove_tab, &name, cx))),
        );
    }
    div().w_full().child(row.overflow_x_scrollbar())
}

/// 单个分组 chip：active 高亮，点击进入该组（None=全部）。
fn er_group_chip(
    label: impl Into<SharedString>,
    active: bool,
    tab_id: TabId,
    group: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    let group_tab = tab_id;
    div()
        .h(px(22.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(colors.radius)
        .border_1()
        .border_color(if active { colors.text } else { colors.border })
        .bg(if active { colors.panel_alt } else { colors.panel_bg })
        .text_size(ui_text_size(11.))
        .text_color(if active { colors.text } else { colors.muted })
        .cursor_pointer()
        .hover(|s| s.bg(colors.hover))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let current = this.er_group.get(&group_tab).cloned().unwrap_or(None);
                if current.as_deref() != group.as_deref() {
                    this.er_set_group(group_tab, group.clone());
                    cx.notify();
                }
            }),
        )
}

/// 表搜索命中判定（§五.6）：表名或注释包含查询（均已小写化，忽略大小写）。
/// 空查询不命中任何表（由调用方在查询为空时不展示列表）。
fn er_search_text_matches(name: &str, comment: Option<&str>, normalized_query: &str) -> bool {
    if name.to_ascii_lowercase().contains(normalized_query) {
        return true;
    }
    comment
        .map(|c| c.to_ascii_lowercase().contains(normalized_query))
        .unwrap_or(false)
}

/// ER 搜索命中下拉浮层（§五.6）：命中表按名称/注释包含查询（忽略大小写），点击命中表
/// 居中并选中（`er_center_on_table`）。作为画布后置兄弟绝对定位叠放其上（GPUI 无
/// z-index，后置者在上）。空查询/无匹配返回空元素，不遮挡画布。
fn er_search_dropdown(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if this.er_search_query.get(&tab_id).map(|q| q.trim()).unwrap_or("").is_empty() {
        return div();
    }
    let matches = this.er_search_matches(tab_id);
    if matches.is_empty() {
        return div();
    }
    let sel_idx = this
        .er_search_sel
        .get(&tab_id)
        .copied()
        .unwrap_or(0)
        .min(matches.len() - 1);
    let mut list = div()
        .mx(px(12.))
        .mt(px(6.))
        .max_h(px(280.))
        .overflow_y_scrollbar()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(4.),
            px(12.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.45 } else { 0.14 }),
        )])
        .flex()
        .flex_col()
        .py_1();
    for (idx, name) in matches.iter().enumerate() {
        let sel_name = name.clone();
        let sel_tab = tab_id;
        let display = name.clone();
        let is_sel = idx == sel_idx;
        let table_path = this.er_graphs.get(&tab_id).into_iter().flat_map(|g| &g.tables)
            .find(|t| t.name == *name)
            .map(|t| ObjectPath {
                connection_id: er.connection_id,
                database: Some(er.database.clone()),
                schema: t.reference.schema.clone(),
                name: t.reference.name.clone(),
                kind: ObjectKind::Table,
            });
        list = list.child(
            div()
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .text_size(ui_text_size(12.))
                .text_color(if is_sel { colors.text } else { colors.muted })
                .when(is_sel, |s| s.bg(colors.hover))
                .cursor_pointer()
                .hover(|s| s.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this.er_center_on_table(sel_tab, &sel_name) {
                            cx.notify();
                        }
                    }),
                )
                .child(div().flex_1().child(display))
                .when_some(table_path, |row, path| row.child(
                    Button::new(format!("er-search-local-{}-{}", tab_id.0, idx))
                        .ghost().xsmall().label(tr("app.er.associate_er"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.dispatch(AppCommand::OpenErDiagram(path.clone()), cx);
                        }))
                )),
        );
    }
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .child(list)
}

/// 展开深度选择按钮组。
fn er_depth_selector(
    tab_id: TabId,
    er: &ErDiagramState,
    current: u8,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let er = er.clone();
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.er.depth_label")),
        )
        .children((1..=3).map(|depth| {
            let active = depth == current;
            let depth_tab = tab_id;
            let depth_er = er.clone();
            div()
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(colors.radius)
                .border_1()
                .border_color(if active { colors.text } else { colors.border })
                .bg(if active { colors.panel_alt } else { colors.panel_bg })
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if active { colors.text } else { colors.muted })
                .cursor_pointer()
                .hover(|style| style.bg(colors.hover))
                .child(
                    tr("app.er.hops").replace("%{count}", &depth.to_string()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if this.er_depths.get(&depth_tab).copied().unwrap_or(1) != depth {
                            this.er_depths.insert(depth_tab, depth);
                            // 同步重算邻域：保留人工坐标/滚动/固定，不撕图、不产生空画布窗口。
                            // 边集未就绪时才回退触发后台读取。
                            if !recompute_local_er_graph(depth_tab, &depth_er, this, cx) {
                                this.er_advance_generation(depth_tab);
                                this.er_graphs.remove(&depth_tab);
                                this.er_scenes.remove(&depth_tab);
                                this.er_relation_tasks.remove(&depth_tab);
                                this.er_errors.remove(&depth_tab);
                            }
                            cx.notify();
                        }
                    }),
                )
        }))
}

/// 加载中状态。
fn er_loading_state(colors: UiColors) -> Div {
    center_loading_message(tr("app.er.loading_tables"), colors)
}

/// 空库/无关系等明确空状态：给可理解文案 + 刷新入口，不伪装成空白画布（§3.5）。
fn er_empty_state(
    message: impl Into<SharedString>,
    er: &ErDiagramState,
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let msg = message.into();
    let refresh_er = er.clone();
    let refresh_tab = tab_id;
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .text_color(colors.muted)
        .text_size(ui_text_size(13.))
        .child(div().child(msg))
        .child(
            Button::new(("er-empty-refresh", tab_id.0))
                .ghost()
                .xsmall()
                .label(tr("app.common.refresh"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.er_refreshing.contains(&refresh_tab) {
                        return;
                    }
                    this.er_refreshing.insert(refresh_tab);
                    this.er_advance_generation(refresh_tab);
                    this.er_load_tasks.remove(&refresh_tab);
                    this.er_full_tables.remove(&refresh_tab);
                    this.er_all_edges.remove(&refresh_tab);
                    this.er_relation_tasks.remove(&refresh_tab);
                    let er = refresh_er.clone();
                    ensure_er_graph_loaded(refresh_tab, &er, this, cx);
                    cx.notify();
                })),
        )
}

/// 加载失败状态。
fn er_error_state(
    tab_id: TabId,
    er: &ErDiagramState,
    message: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let retry_tab_id = tab_id;
    let retry_er = er.clone();
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .text_color(colors.text)
        .child(
            div()
                .max_w(px(480.))
                .text_center()
                .text_color(colors.muted)
                .text_size(ui_text_size(13.))
                .child(tr("app.er.load_failed").replace("%{error}", &message)),
        )
        .child(
            Button::new("er_retry")
                .label(tr("app.common.retry"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.er_errors.remove(&retry_tab_id);
                    this.er_graphs.remove(&retry_tab_id);
                    this.er_advance_generation(retry_tab_id);
                    this.er_full_tables.remove(&retry_tab_id);
                    this.er_scenes.remove(&retry_tab_id);
                    this.er_canvas.borrow_mut().er_scene_positions.remove(&retry_tab_id);
                    this.er_layout_applied.remove(&retry_tab_id);
                    let er = retry_er.clone();
                    ensure_er_graph_loaded(retry_tab_id, &er, this, cx);
                    cx.notify();
                })),
        )
}
