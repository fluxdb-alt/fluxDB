fn connection_browser_titlebar(
    compact: bool,
    connection_browser_width: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .relative()
        .w(px(if compact {
            connection_browser_width.min(260.)
        } else {
            connection_browser_width
        }))
        .h(px(36.))
        .flex_none()
        .bg(colors.sidebar_bg)
        .border_b_1()
        .border_color(colors.border)
        .px_2()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div().flex().items_center().min_w(px(0.)).child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child(tr("app.sidebar.connections")),
            ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(sidebar_title_button(AppIcon::Folder, tr("app.sidebar.new_group"), false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.create_connection_group_and_rename(window, cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(sidebar_title_button(AppIcon::Refresh, tr("app.common.refresh"), false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        // 侧边栏刷新只刷连接树，不跟随活动标签页（详见 refresh_connection_tree）。
                        this.refresh_connection_tree(cx);
                        cx.stop_propagation();
                    }),
                ))
                // 「添加连接」图标放在标题栏右侧（即原「收起侧边栏」位置）；收起功能由顶部栏切换按钮承担。
                .child(sidebar_title_button(AppIcon::Plug, tr("app.sidebar.add_connection"), false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.show_new_connection(window, cx);
                        cx.stop_propagation();
                    }),
                )),
        )
}

/// 收起状态下的窄侧边栏：固定宽纵向两个图标（首页 / 数据库）。
/// 数据库图标同时负责展开连接浏览器；当前页面态通过图标高亮体现：
/// - 首页：无活动标签（active_tab 为空）时高亮。
/// - 数据库：存在数据库/表相关活动标签（非 Settings）时高亮。
fn connection_browser_restore_button(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let on_home = state.active_tab().is_none();
    let on_database = state
        .active_tab()
        .is_some_and(|tab| tab_workspace_scope(tab).is_some());
    div()
        .w(px(52.))
        .h_full()
        .bg(colors.sidebar_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .pt_3()
        .child(
            sidebar_title_button(AppIcon::Home, tr("app.topbar.home"), on_home, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dispatch(AppCommand::DeactivateTab, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            sidebar_title_button(AppIcon::Database, tr("app.sidebar.databases"), on_database, colors)
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    // 展开侧边栏并显示数据库连接及对象列表（连接浏览器）。
                    this.show_connection_browser = true;
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
}

fn clamp_connection_browser_width(width: f32) -> f32 {
    width.clamp(CONNECTION_BROWSER_MIN_WIDTH, CONNECTION_BROWSER_MAX_WIDTH)
}

fn connection_browser_resize_handle(
    connection_browser_width: f32,
    _colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id("connection-browser-resize-handle")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(px(6.))
        .cursor_ew_resize()
        .flex()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.connection_browser_resize_start = Some(SidebarResizeStart {
                    x: f32::from(event.position.x),
                    width: connection_browser_width,
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(SidebarResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<SidebarResizeDrag>, _, cx| {
                if let Some(start) = this.connection_browser_resize_start {
                    let delta = f32::from(event.event.position.x) - start.x;
                    this.connection_browser_width =
                        clamp_connection_browser_width(start.width + delta);
                    cx.notify();
                }
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn sidebar_title_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    active: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let tooltip = tooltip.into();
    let tooltip_for_id = tooltip.clone();
    div()
        .size(px(30.))
        .rounded(colors.radius_lg)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        // 当前页面态高亮：active 时用选中背景 + 主题色图标，否则 muted 淡显。
        .when(active, |style| style.bg(colors.tree_selected))
        .hover(move |style| style.bg(colors.hover))
        .id(tooltip_for_id)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon_box(
            icon,
            30.,
            16.,
            if active { colors.text } else { colors.muted },
        ))
}

fn tab_view_with_context(
    tab: &TabState,
    active: bool,
    compact: bool,
    show_database_context: bool,
    connection_color_hex: Option<&str>,
    pinned: bool,
    hovered: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let tab_id = tab.id;
    let active_bg = colors.panel_bg;
    let inactive_bg = colors.panel_alt;
    let identity_color = connection_color_hex
        .map(|hex| tab_identity_color(hex, colors))
        .unwrap_or(colors.info);
    let table_name = match &tab.kind {
        TabKind::DataEditor(editor) => Some(editor.object.name.clone()),
        _ => None,
    };
    let is_table_tab = table_name.is_some();
    let title = tab_title(tab, show_database_context);
    let title_tooltip = title.clone();
    let dragged_tab = DraggedTab {
        tab_id,
        title: title.clone(),
    };

    div()
        .id(("tab-view", tab_id.0))
        .debug_selector(move || format!("workspace-tab:{}", tab_id.0))
        .relative()
        .h(px(30.))
        .w(px(tab_width(tab, compact)))
        .flex_none()
        .min_w(px(0.))
        .bg(if active {
            active_bg
        } else {
            inactive_bg
        })
        .border_1()
        .border_color(if active {
            identity_color
        } else {
            colors.border_soft
        })
        .rounded(colors.radius_lg)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .overflow_hidden()
        .cursor_move()
        .hover(move |style| {
            style.bg(if active {
                active_bg
            } else {
                colors.hover
            })
            .border_color(if active {
                identity_color
            } else {
                colors.border
            })
        })
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered {
                this.hovered_tab = Some(tab_id);
            } else if this.hovered_tab == Some(tab_id) {
                this.hovered_tab = None;
            }
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::ActivateTab(tab_id), cx);
            }),
        )
        .on_drag(dragged_tab, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .drag_over::<DraggedTab>(move |style, _, _, _| {
            style.border_color(colors.info).bg(colors.hover)
        })
        .on_drop(cx.listener(move |this, drag: &DraggedTab, _, cx| {
            this.drop_tab_after(drag.tab_id, tab_id, cx);
            cx.stop_propagation();
        }))
        .when(is_table_tab, |this| {
            this.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.show_tab_context_menu(tab_id, event.position, window, cx);
                    cx.stop_propagation();
                }),
            )
        })
        .child(tab_icon(tab, active, colors))
        .child(
            div()
                .id(("tab-title", tab_id.0))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(if active { colors.text } else { colors.muted })
                .when(active, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                .tooltip(move |window, cx| Tooltip::new(title_tooltip.clone()).build(window, cx))
                .child(title),
        )
        .when(hovered, move |this| {
            let action_width = if table_name.is_some() { 62. } else { 42. };
            this.child(
                div()
                    .absolute()
                    .top_1()
                    .right_1()
                    .bottom_1()
                    .w(px(action_width))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_1()
                    .rounded(colors.radius)
                    .bg(if active { active_bg } else { colors.hover })
                    .overflow_hidden()
                    .flex_none()
                    .child(tab_pin_button(tab_id, pinned, colors, cx))
                    .when_some(table_name, |this, table_name| {
                        this.child(tab_copy_table_name_button(
                            tab_id, table_name, colors, cx,
                        ))
                    })
                    .child(tab_close_button(tab_id, colors, cx))
                    .with_animation(
                        ("tab-actions", tab_id.0),
                        Animation::new(Duration::from_millis(120)).with_easing(ease_in_out),
                        move |this, progress| {
                            this.opacity(progress)
                        },
                    ),
            )
        })
}

fn tab_pin_button(
    tab_id: TabId,
    pinned: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let color = if pinned { colors.info } else { colors.muted };
    let tooltip = if pinned {
        tr("app.sidebar.unpin")
    } else {
        tr("app.sidebar.pin")
    };
    div()
        .id(("pin-tab", tab_id.0))
        .size(px(18.))
        .flex_none()
        .rounded(colors.radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_color(color)
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_tab_pin(tab_id, cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Pin, 14., color))
}

fn tab_close_button(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("close-tab", tab_id.0))
        .size(px(18.))
        .flex_none()
        .rounded(colors.radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .tooltip(|window, cx| Tooltip::new(tr("app.window.close")).build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::CloseTab(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(app_icon(AppIcon::Close, 14., colors.muted))
}

fn tab_copy_table_name_button(
    tab_id: TabId,
    table_name: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("copy-table-name-tab", tab_id.0))
        .size(px(18.))
        .flex_none()
        .rounded(colors.radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .tooltip(|window, cx| Tooltip::new(tr("app.sidebar.copy_table_name")).build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(table_name.clone()));
                this.show_message(
                    tr("app.sidebar.table_name_copied"),
                    AppMessageKind::Success,
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Copy, 14., colors.muted))
}

/// 表 Tab 短表名的最大字符数：超过则按长表名处理
const SHORT_TAB_TITLE_MAX_CHARS: usize = 15;
/// 短表名 Tab 宽度
const SHORT_TAB_WIDTH: f32 = 200.;
/// 长表名 Tab 宽度
const LONG_TAB_WIDTH: f32 = 300.;

fn tab_width(tab: &TabState, compact: bool) -> f32 {
    let base: f32 = match tab.kind {
        TabKind::ObjectList(_) => 160.,
        // 表 Tab 根据表名长度自适应两种宽度：短表名用短宽度，长表名用长宽度
        TabKind::DataEditor(_) => {
            if tab.title.chars().count() <= SHORT_TAB_TITLE_MAX_CHARS {
                SHORT_TAB_WIDTH
            } else {
                LONG_TAB_WIDTH
            }
        }
        TabKind::QueryEditor(_) => 210.,
        TabKind::RedisWorkbench(_) => 210.,
        TabKind::RedisCli(_) => 210.,
        TabKind::MongoCli(_) => 210.,
        TabKind::MongoWorkbench(_) => 210.,
        TabKind::RedisPubSub(_) => 210.,
        TabKind::CreateTable(_) => 210.,
        TabKind::UserAdmin(_) => 180.,
        TabKind::ServerPanel(_) => 210.,
        TabKind::RedisServerPanel(_) => 210.,
        TabKind::MongoServerPanel(_) => 210.,
        TabKind::Settings(_) => 160.,
        TabKind::AiChat { .. } => 180.,
        TabKind::BackupList(_) => 210.,
        TabKind::ErDiagram(_) => 210.,
        TabKind::MongoCollection(_) => 210.,
    };

    if compact { base.min(180.) } else { base }
}

fn tab_title(tab: &TabState, show_database_context: bool) -> String {
    let prefix = if tab.dirty { "* " } else { "" };
    let title = localized_tab_title(tab);
    match &tab.kind {
        TabKind::DataEditor(editor) if show_database_context => format!(
            "{prefix}{}@{}",
            title,
            editor.object.database.as_deref().unwrap_or("main")
        ),
        _ => format!("{prefix}{title}"),
    }
}

/// 应用层为内置标签页保存稳定中文标题；展示时按标签类型替换为当前语言。
fn localized_tab_title(tab: &TabState) -> String {
    match &tab.kind {
        TabKind::Settings(_) => tr("app.tabs.settings").to_string(),
        TabKind::AiChat { .. } => tr("app.tabs.ai_chat").to_string(),
        TabKind::UserAdmin(_) => tr("app.tabs.user_admin").to_string(),
        TabKind::ServerPanel(panel) => tr("app.tabs.server_panel")
            .replace("%{name}", &panel.connection_name),
        TabKind::RedisServerPanel(panel) => tr("app.tabs.redis_server_panel")
            .replace("%{name}", &panel.connection_name),
        TabKind::MongoServerPanel(panel) => tr("app.tabs.mongo_server_panel")
            .replace("%{name}", &panel.connection_name),
        TabKind::QueryEditor(_) if tab.title == "无标题 - 查询" => {
            tr("app.tabs.untitled_query").to_string()
        }
        TabKind::CreateTable(state) => {
            let name = state.table_name.trim();
            if name.is_empty() {
                if state.is_design() {
                    tr("app.tabs.design_table").to_string()
                } else {
                    tr("app.tabs.new_table").to_string()
                }
            } else if state.is_design() {
                tr("app.tabs.design_table_named").replace("%{table}", name)
            } else {
                name.to_string()
            }
        }
        TabKind::RedisWorkbench(workbench) => tr("app.tabs.untitled_redis_command")
            .replace("%{database}", &workbench.database.to_string()),
        TabKind::RedisCli(cli) => tr("app.tabs.redis_cli")
            .replace("%{database}", &cli.database.to_string()),
        TabKind::MongoCli(cli) => tr("app.tabs.mongo_cli")
            .replace("%{database}", &cli.database),
        TabKind::MongoWorkbench(workbench) => tr("app.tabs.mongo_workbench")
            .replace("%{database}", &workbench.database),
        TabKind::RedisPubSub(pubsub) => tr("app.tabs.redis_pubsub")
            .replace("%{database}", &pubsub.database.to_string()),
        TabKind::BackupList(list) => tr("app.tabs.backup")
            .replace("%{database}", &list.database),
        TabKind::ErDiagram(er) => match &er.center_table {
            Some(table) => tr("app.tabs.er_related").replace("%{table}", &table.display()),
            None => tr("app.tabs.er").replace("%{database}", &er.database),
        },
        // 集合页标题是「库/集合」，跨库同名集合不至于看起来重复。
        TabKind::MongoCollection(collection) => format!(
            "{}",
            collection
                .object
                .database
                .as_ref()
                .map(|database| format!("{database}/{}", collection.object.name))
                .unwrap_or_else(|| collection.object.name.clone())
        ),
        _ => tab.title.clone(),
    }
}

fn tab_icon(
    tab: &TabState,
    active: bool,
    colors: UiColors,
) -> impl IntoElement {
    let icon = match tab.kind {
        TabKind::DataEditor(_) | TabKind::ObjectList(_) => AppIcon::Table,
        TabKind::CreateTable(_) => AppIcon::Plus,
        TabKind::QueryEditor(_) | TabKind::RedisWorkbench(_) | TabKind::RedisCli(_) => {
            AppIcon::Query
        }
        TabKind::MongoCli(_) | TabKind::MongoWorkbench(_) => AppIcon::Query,
        TabKind::RedisPubSub(_) => AppIcon::Broadcast,
        TabKind::UserAdmin(_) => AppIcon::Users,
        TabKind::ServerPanel(_)
        | TabKind::RedisServerPanel(_)
        | TabKind::MongoServerPanel(_) => AppIcon::Activity,
        TabKind::Settings(_) => AppIcon::Settings,
        TabKind::AiChat { .. } => AppIcon::Bot,
        TabKind::BackupList(_) => AppIcon::Save,
        TabKind::ErDiagram(_) => AppIcon::Schema,
        TabKind::MongoCollection(_) => AppIcon::Table,
    };
    let color = if active { colors.text } else { colors.muted };

    div()
        .size(px(18.))
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(icon, 13., color))
}

impl SidebarTreeCacheKey {
    #[allow(clippy::too_many_arguments)]
    fn matches(
        &self,
        state: &AppState,
        loading_databases: &BTreeSet<String>,
        loaded_database_children: &BTreeSet<String>,
        pinned_databases: &BTreeSet<String>,
        pinned_tables: &BTreeSet<String>,
        table_folders: &BTreeMap<String, Vec<String>>,
        table_folder_assignments: &BTreeMap<String, (String, String)>,
        expanded_databases: &BTreeMap<String, bool>,
        expanded_object_groups: &BTreeMap<String, bool>,
        saved_queries: &[SavedQuery],
        search_query: &str,
    ) -> bool {
        self.connections == state.connections
            && self.sidebar_layout == state.sidebar_layout
            && &self.loading_databases == loading_databases
            && &self.loaded_database_children == loaded_database_children
            && &self.pinned_databases == pinned_databases
            && &self.pinned_tables == pinned_tables
            && &self.table_folders == table_folders
            && &self.table_folder_assignments == table_folder_assignments
            && &self.expanded_databases == expanded_databases
            && &self.expanded_object_groups == expanded_object_groups
            && self.saved_queries == saved_queries
            && self.search_query == search_query
    }
}

impl NavicatMain {
    #[allow(clippy::too_many_arguments)]
    fn sidebar_tree_rows(
        &self,
        state: &AppState,
        loading_databases: &BTreeSet<String>,
        loaded_database_children: &BTreeSet<String>,
        pinned_databases: &BTreeSet<String>,
        pinned_tables: &BTreeSet<String>,
        table_folders: &BTreeMap<String, Vec<String>>,
        table_folder_assignments: &BTreeMap<String, (String, String)>,
        expanded_databases: &BTreeMap<String, bool>,
        expanded_object_groups: &BTreeMap<String, bool>,
        saved_queries: &[SavedQuery],
        search_text: &str,
    ) -> (Rc<Vec<SidebarVisibleRow>>, Rc<Vec<Size<Pixels>>>) {
        let search_query = normalized_sidebar_search(search_text);
        {
            let cache = self.sidebar_tree_cache.borrow();
            if let Some(cache) = cache.as_ref()
                && cache.key.matches(
                    state,
                    loading_databases,
                    loaded_database_children,
                    pinned_databases,
                    pinned_tables,
                    table_folders,
                    table_folder_assignments,
                    expanded_databases,
                    expanded_object_groups,
                    saved_queries,
                    &search_query,
                )
            {
                return (cache.rows.clone(), cache.item_sizes.clone());
            }
        }

        let rows = Rc::new(flatten_sidebar_visible_rows(
            &state.connections,
            &state.sidebar_layout,
            saved_queries,
            &BTreeSet::new(),
            loading_databases,
            loaded_database_children,
            pinned_databases,
            pinned_tables,
            table_folders,
            table_folder_assignments,
            expanded_databases,
            expanded_object_groups,
            &search_query,
        ));
        let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
            rows.iter()
                .map(|row| size(px(0.), px(row.height())))
                .collect(),
        );
        *self.sidebar_tree_cache.borrow_mut() = Some(SidebarTreeCache {
            key: SidebarTreeCacheKey {
                connections: state.connections.clone(),
                sidebar_layout: state.sidebar_layout.clone(),
                loading_databases: loading_databases.clone(),
                loaded_database_children: loaded_database_children.clone(),
                pinned_databases: pinned_databases.clone(),
                pinned_tables: pinned_tables.clone(),
                table_folders: table_folders.clone(),
                table_folder_assignments: table_folder_assignments.clone(),
                expanded_databases: expanded_databases.clone(),
                expanded_object_groups: expanded_object_groups.clone(),
                saved_queries: saved_queries.to_vec(),
                search_query,
            },
            rows: rows.clone(),
            item_sizes: item_sizes.clone(),
        });
        (rows, item_sizes)
    }
}


fn sidebar(
    compact: bool,
    connection_browser_width: f32,
    sidebar_tree_scroll: &VirtualListScrollHandle,
    visible: Rc<Vec<SidebarVisibleRow>>,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    table_folder_rename_input: Entity<InputState>,
    rename_group_input: Entity<InputState>,
    search_input: Entity<InputState>,
    search_text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let search_query = normalized_sidebar_search(search_text);
    // —— 连接浏览器树：虚拟化列表（只渲染视口附近的行），替代整树命令式重建 ——
    // render 阶段不反向 read/update NavicatMain；行数据由 render.rs 的纯数据缓存传入。
    let sidebar_scroll = sidebar_tree_scroll.clone();
    let list_rows = visible.clone();
    let list_search = search_query.clone();
    let list_rename_group = rename_group_input.clone();
    let list_folder_rename = table_folder_rename_input.clone();

    let virtual_tree = v_virtual_list(
        cx.entity(),
        "connection-tree-vlist",
        item_sizes,
        move |this, range, _window, cx| {
            range
                .map(|ix| {
                    let row = &list_rows[ix];
                    build_sidebar_row(
                        row,
                        &*this,
                        this.controller.state(),
                        colors,
                        &list_search,
                        list_rename_group.clone(),
                        list_folder_rename.clone(),
                        cx,
                    )
                })
                .collect()
        },
    )
    .track_scroll(&sidebar_scroll);

    let tree_rows = div()
        .debug_selector(|| "connection-tree-viewport".into())
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .pt_1()
        .drag_over::<DraggedConnection>(move |style, _, _, _| style.bg(colors.hover))
        .on_drop(cx.listener(|this, drag: &DraggedConnection, _, cx| {
            this.move_connection_to_top_level_end(drag.connection_id, cx);
            cx.stop_propagation();
        }))
        .child(virtual_tree)
        .vertical_scrollbar(&sidebar_scroll);

    div()
        .debug_selector(|| "connection-browser".into())
        .relative()
        .w(px(if compact {
            connection_browser_width.min(260.)
        } else {
            connection_browser_width
        }))
        .h_full()
        .bg(colors.sidebar_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(connection_browser_titlebar(
            compact,
            connection_browser_width,
            colors,
            cx,
        ))
        .child(
            div()
                .px_2()
                .py_2()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .gap_2()
                .child(sidebar_search(search_input, colors, cx)),
        )
        .child(tree_rows)
        // 底部常驻「添加连接」大按钮：独立于树形列表，滚动时保持固定在最下方。
        .child(
            div()
                .flex_none()
                .border_t_1()
                .border_color(colors.border)
                .p_2()
                .child(
                    div()
                        .h(px(32.))
                        .w_full()
                        .rounded(colors.radius_lg)
                        .bg(colors.panel_bg)
                        .border_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .id("sidebar-add-connection")
                        .tooltip(move |window, cx| {
                            Tooltip::new(tr("app.sidebar.new_mysql_connection")).build(window, cx)
                        })
                        .child(app_icon_box(AppIcon::Plug, 20., 16., colors.text))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child(tr("app.sidebar.add_connection")),
                        ),
                )
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                    this.show_new_connection(window, cx);
                    cx.stop_propagation();
                })),
        )
        .child(connection_browser_resize_handle(
            connection_browser_width,
            colors,
            cx,
        ))
}

fn sidebar_search(
    search_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_value = !search_input.read(cx).value().is_empty();
    let search_input_for_clear = search_input.clone();

    div()
        .h(px(34.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.muted)
        .child(app_icon(AppIcon::Search, 16., colors.muted))
        .child(
            Input::new(&search_input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(px(14.)),
        )
        .when(has_value, |this| {
            this.child(
                div()
                    .size(px(20.))
                    .rounded(colors.radius * 0.5)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .child(app_icon(AppIcon::Close, 13., colors.muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            search_input_for_clear.update(cx, |input, cx| {
                                input.set_value("", window, cx);
                            });
                            // set_value 走 silent 替换不触发 InputEvent::Change，订阅不会更新 sidebar_search，
                            // 需在此同步清空搜索词，否则下方列表仍按旧关键词过滤
                            this.sidebar_search = String::new();
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            )
        })
}

fn connection_should_show_children(expanded: bool, _search_active: bool) -> bool {
    expanded
}

fn tree_expanded_for_search(expanded: Option<bool>, search_active: bool) -> bool {
    expanded.unwrap_or(search_active)
}

fn normalized_sidebar_search(search: &str) -> String {
    search.trim().to_ascii_lowercase()
}

fn search_matches_text(text: &str, query: &str) -> bool {
    query.is_empty() || text.to_ascii_lowercase().contains(query)
}

fn connection_matches_sidebar_search(
    connection: &ConnectionState,
    saved_queries: &[SavedQuery],
    query: &str,
) -> bool {
    if query.is_empty() {
        return true;
    }

    (connection.connected
        && connection
            .objects
            .iter()
            .any(|object| object_matches_sidebar_search(object, query)))
        || saved_queries.iter().any(|saved| {
            saved.connection_id == connection.config.id && search_matches_text(&saved.name, query)
        })
}

fn object_matches_sidebar_search(object: &ObjectSummary, query: &str) -> bool {
    matches!(
        object.path.kind,
        ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::Procedure
            | ObjectKind::Function
            | ObjectKind::Collection
            | ObjectKind::RedisKey
    ) && search_matches_text(&object.path.name, query)
}
