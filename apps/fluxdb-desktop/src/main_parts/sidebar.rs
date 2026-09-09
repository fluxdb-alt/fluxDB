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
                    .child("连接"),
            ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(sidebar_title_button(AppIcon::Folder, "新建分组", false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.create_connection_group_and_rename(window, cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(sidebar_title_button(AppIcon::Refresh, "刷新", false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.refresh_active(window, cx);
                        cx.stop_propagation();
                    }),
                ))
                // 「添加连接」图标放在标题栏右侧（即原「收起侧边栏」位置）；收起功能由顶部栏切换按钮承担。
                .child(sidebar_title_button(AppIcon::Plug, "添加连接", false, colors).on_mouse_down(
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
            sidebar_title_button(AppIcon::Home, "首页", on_home, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dispatch(AppCommand::DeactivateTab, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            sidebar_title_button(AppIcon::Database, "数据库", on_database, colors)
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
    tooltip: &'static str,
    active: bool,
    colors: UiColors,
) -> Stateful<Div> {
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
        .id(tooltip)
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
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
    let active_bg = connection_color_hex
        .map(|hex| connection_color_row_bg(hex, colors))
        .unwrap_or(colors.panel_bg);
    let active_border = connection_color_hex
        .map(connection_color_rgba)
        .unwrap_or(colors.border);
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
        .h(px(if active { 28. } else { 26. }))
        .w(px(tab_width(tab, compact)))
        .flex_none()
        .min_w(px(0.))
        .bg(if active {
            active_bg
        } else if colors.is_dark {
            rgb(0x171a1f)
        } else {
            rgb(0xf7f8fa)
        })
        .border_1()
        .border_color(if active { active_border } else { colors.border })
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
            } else if colors.is_dark {
                rgb(0x232934)
            } else {
                rgb(0xffffff)
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
            style.border_color(rgb(0xf5a400)).bg(colors.hover)
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
        .child(tab_icon(tab, colors))
        .child(
            div()
                .id(("tab-title", tab_id.0))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(colors.text)
                .tooltip(move |window, cx| Tooltip::new(title_tooltip.clone()).build(window, cx))
                .child(title),
        )
        .when(hovered, move |this| {
            let action_width = if table_name.is_some() { 62. } else { 40. };
            this.child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_1()
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
                        Animation::new(Duration::from_millis(140)).with_easing(ease_in_out),
                        move |this, progress| {
                            this.w(px(action_width * progress)).opacity(progress)
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
    let color = if pinned { rgb(0x1677ff) } else { colors.muted };
    let tooltip = if pinned { "取消置顶" } else { "置顶" };
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
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
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
        .tooltip(|window, cx| Tooltip::new("关闭").build(window, cx))
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
        .tooltip(|window, cx| Tooltip::new("复制表名").build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(table_name.clone()));
                this.show_message("已复制表名", AppMessageKind::Success, cx);
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
        TabKind::RedisPubSub(_) => 210.,
        TabKind::CreateTable(_) => 210.,
        TabKind::UserAdmin(_) => 180.,
        TabKind::Settings => 160.,
        TabKind::BackupList(_) => 210.,
    };

    if compact { base.min(180.) } else { base }
}

fn tab_title(tab: &TabState, show_database_context: bool) -> String {
    let prefix = if tab.dirty { "* " } else { "" };
    match &tab.kind {
        TabKind::DataEditor(editor) if show_database_context => format!(
            "{prefix}{}@{}",
            tab.title,
            editor.object.database.as_deref().unwrap_or("main")
        ),
        _ => format!("{prefix}{}", tab.title),
    }
}

fn tab_icon(tab: &TabState, colors: UiColors) -> impl IntoElement {
    let (bg, border, color, icon, size) = match tab.kind {
        TabKind::DataEditor(_) | TabKind::ObjectList(_) => (
            rgb(0xa8ddfb),
            Some(rgb(0x118ee9)),
            rgb(0xeaf7ff),
            AppIcon::Table,
            12.,
        ),
        TabKind::CreateTable(_) => (
            rgb(0xa8ddfb),
            Some(rgb(0x118ee9)),
            rgb(0xeaf7ff),
            AppIcon::Plus,
            12.,
        ),
        TabKind::QueryEditor(_) => (rgb(0xff7252), None, rgb(0xffffff), AppIcon::Query, 12.),
        // Redis 命令执行器沿用 Query 图标，配色偏 Redis 经典红。
        TabKind::RedisWorkbench(_) => (rgb(0xea4a1f), None, rgb(0xffffff), AppIcon::Query, 12.),
        // Redis CLI 终端沿用 Redis 经典红 + Query 图标，与 Workbench 保持一致。
        TabKind::RedisCli(_) => (rgb(0xea4a1f), None, rgb(0xffffff), AppIcon::Query, 12.),
        // Redis Pub/Sub 沿用 Redis 经典红 + 广播图标，与 Workbench / CLI 视觉统一。
        TabKind::RedisPubSub(_) => (
            rgb(0xea4a1f),
            None,
            rgb(0xffffff),
            AppIcon::Broadcast,
            12.,
        ),
        TabKind::UserAdmin(_) => (rgb(0x2563eb), None, rgb(0xffffff), AppIcon::Users, 12.),
        TabKind::Settings => (rgb(0xc8ccd2), None, rgb(0x4f5661), AppIcon::Settings, 12.),
        // 备份列表 tab：与侧边栏备份节点一致的保存图标，配色用墨绿区分数据表。
        TabKind::BackupList(_) => (rgb(0x0f9d78), None, rgb(0xffffff), AppIcon::Save, 12.),
    };

    div()
        .size(px(18.))
        .rounded(colors.radius_lg)
        .bg(bg)
        .when_some(border, |this, border| {
            this.border_t_8().border_color(border)
        })
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(icon, size, color))
}

impl SidebarTreeCacheKey {
    #[allow(clippy::too_many_arguments)]
    fn matches(
        &self,
        state: &AppState,
        loading_databases: &BTreeSet<String>,
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
            &BTreeSet::new(),
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
                            Tooltip::new("新建 MySQL 连接").build(window, cx)
                        })
                        .child(app_icon_box(AppIcon::Plug, 20., 16., colors.text))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child("添加连接"),
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
                        cx.listener(move |_, _, window, cx| {
                            search_input_for_clear.update(cx, |input, cx| {
                                input.set_value("", window, cx);
                            });
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
        ObjectKind::Table | ObjectKind::View | ObjectKind::Collection | ObjectKind::RedisKey
    ) && search_matches_text(&object.path.name, query)
}
