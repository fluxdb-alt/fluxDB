fn tabs(
    compact: bool,
    state: &AppState,
    app: &NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let groups =
        order_first_level_tab_groups(first_level_tab_groups(state), &app.workspace_tab_order);
    let active_scope = active_first_level_scope(state);
    let show_first_level = !groups.is_empty();
    let show_second_level = show_first_level && first_level_has_second_row(active_scope.as_ref());
    let height = if !show_first_level {
        34.
    } else if show_second_level {
        70.
    } else {
        34.
    };
    let available_width = tab_workspace_available_width(app, window);

    let root = div()
        .relative()
        .h(px(height))
        .bg(colors.status_bg)
        .border_b_1()
        .border_color(colors.border)
        .flex();

    let mut workspace = div().flex_1().min_w(px(0.)).h_full().flex().flex_col();

    if show_first_level {
        workspace = workspace
            .child(first_level_tab_row(
                &groups,
                active_scope.as_ref(),
                app.hovered_first_level_tab.as_ref(),
                compact,
                available_width,
                colors,
                cx,
            ));
        if show_second_level {
            workspace = workspace.child(table_tab_row(
                state,
                active_scope.as_ref(),
                compact,
                available_width,
                &app.tab_order,
                &app.pinned_tabs,
                app.hovered_tab,
                colors,
                cx,
            ));
        }
    } else {
        let show_switcher = tab_row_overflows(
            state.tabs.len(),
            tab_width_for_kind(None, compact),
            available_width,
        );
        let mut row = div()
            .id("single-level-tab-scroll-row")
            .h_full()
            .flex_1()
            .flex()
            .min_w(px(0.))
            .overflow_x_scroll();
        let tabs = order_tab_refs(
            state.tabs.iter().collect::<Vec<_>>(),
            &app.tab_order,
            &app.pinned_tabs,
        );
        for tab in tabs {
            row = row.child(tab_view_with_context(
                tab,
                state.active_tab == Some(tab.id),
                compact,
                true,
                tab_connection_color_hex(state, tab),
                app.pinned_tabs.contains(&tab.id),
                app.hovered_tab == Some(tab.id),
                colors,
                cx,
            ));
        }
        workspace = workspace.child(
            div()
                .h_full()
                .min_w(px(0.))
                .flex()
                .child(row.child(query_tab_blank_area(
                    database_scope(state, active_scope.as_ref()),
                    cx,
                )))
                .when(show_switcher, |this| {
                    this.child(tab_switcher_button(
                        TabSwitcherKind::Tables,
                        false,
                        colors,
                        cx,
                    ))
                }),
        );
    }

    root.child(workspace)
}

type WorkspaceScope = TabWorkspace;

/// 一级标签的作用域：数据库工作区或全局设置。
#[derive(Clone, Debug, Eq, PartialEq)]
enum FirstLevelScope {
    Database(WorkspaceScope),
    /// Redis 的稳定工作区边界是连接；DB 编号只作为连接内标签的上下文。
    RedisConnection(ConnectionId),
    Settings,
    AiChat,
}

/// 数据库工作区和 Redis 连接工作区有二级标签；设置和 AI 是完整的全局页面。
fn first_level_has_second_row(scope: Option<&FirstLevelScope>) -> bool {
    !matches!(scope, Some(FirstLevelScope::Settings | FirstLevelScope::AiChat))
}

#[derive(Clone, Debug)]
enum WorkspaceTabGroup {
    Database {
        scope: WorkspaceScope,
        connection_name: String,
        connection_color_hex: String,
        database: String,
        first_tab_id: TabId,
    },
    RedisConnection {
        connection_id: ConnectionId,
        connection_name: String,
        connection_color_hex: String,
        first_tab_id: TabId,
    },
    Settings {
        first_tab_id: TabId,
    },
    AiChat {
        first_tab_id: TabId,
    },
}

impl WorkspaceTabGroup {
    fn scope(&self) -> FirstLevelScope {
        match self {
            Self::Database { scope, .. } => FirstLevelScope::Database(scope.clone()),
            Self::RedisConnection { connection_id, .. } => {
                FirstLevelScope::RedisConnection(*connection_id)
            }
            Self::Settings { .. } => FirstLevelScope::Settings,
            Self::AiChat { .. } => FirstLevelScope::AiChat,
        }
    }

    fn first_tab_id(&self) -> TabId {
        match self {
            Self::Database { first_tab_id, .. }
            | Self::RedisConnection { first_tab_id, .. }
            | Self::Settings { first_tab_id }
            | Self::AiChat { first_tab_id } => *first_tab_id,
        }
    }
}

fn first_level_tab_groups(state: &AppState) -> Vec<WorkspaceTabGroup> {
    let mut groups = Vec::new();
    let mut settings_tab_id = None;
    let mut ai_chat_tab_id = None;
    for tab in &state.tabs {
        match tab_first_level_scope(tab) {
            Some(FirstLevelScope::Database(scope)) => {
                let exists = groups.iter().any(|group| {
                    matches!(
                        group,
                        WorkspaceTabGroup::Database {
                            scope: group_scope,
                            ..
                        } if *group_scope == scope
                    )
                });
                if exists {
                    continue;
                }
                let connection_color_hex = state
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == scope.connection_id)
                    .map(|connection| connection_config_color_hex(&connection.config.options).to_string())
                    .unwrap_or_else(|| DEFAULT_CONNECTION_COLOR.to_string());
                groups.push(WorkspaceTabGroup::Database {
                    connection_name: connection_name(state, scope.connection_id),
                    connection_color_hex,
                    database: scope.database.clone(),
                    scope,
                    first_tab_id: tab.id,
                });
            }
            Some(FirstLevelScope::RedisConnection(connection_id)) => {
                let exists = groups.iter().any(|group| {
                    matches!(
                        group,
                        WorkspaceTabGroup::RedisConnection {
                            connection_id: group_connection_id,
                            ..
                        } if *group_connection_id == connection_id
                    )
                });
                if exists {
                    continue;
                }
                let connection_color_hex = state
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == connection_id)
                    .map(|connection| connection_config_color_hex(&connection.config.options).to_string())
                    .unwrap_or_else(|| DEFAULT_CONNECTION_COLOR.to_string());
                groups.push(WorkspaceTabGroup::RedisConnection {
                    connection_id,
                    connection_name: connection_name(state, connection_id),
                    connection_color_hex,
                    first_tab_id: tab.id,
                });
            }
            Some(FirstLevelScope::Settings) if settings_tab_id.is_none() => {
                settings_tab_id = Some(tab.id);
            }
            Some(FirstLevelScope::Settings) => {}
            Some(FirstLevelScope::AiChat) if ai_chat_tab_id.is_none() => {
                ai_chat_tab_id = Some(tab.id);
            }
            Some(FirstLevelScope::AiChat) => {}
            None => {}
        }
    }
    if let Some(first_tab_id) = settings_tab_id {
        groups.push(WorkspaceTabGroup::Settings { first_tab_id });
    }
    if let Some(ai_tab_id) = ai_chat_tab_id {
        groups.push(WorkspaceTabGroup::AiChat {
            first_tab_id: ai_tab_id,
        });
    }
    groups
}

fn active_first_level_scope(state: &AppState) -> Option<FirstLevelScope> {
    match state.active_tab() {
        // 首页打开的全局标签没有一级归属；设置页是全局单例，但有独立的 Settings 作用域。
        Some(tab) => tab_first_level_scope(tab),
        None => state.tabs.iter().find_map(tab_first_level_scope),
    }
}

#[derive(Clone, Copy)]
enum TabKindDiscriminant {
    DataEditor,
}

fn tab_workspace_available_width(app: &NavicatMain, window: &Window) -> f32 {
    let mut width = f32::from(window.bounds().size.width);
    if app.show_connection_browser {
        width -= app.connection_browser_width;
    } else {
        width -= 32.;
    }
    width.max(240.)
}

fn tab_width_for_kind(kind: Option<TabKindDiscriminant>, compact: bool) -> f32 {
    match kind {
        Some(TabKindDiscriminant::DataEditor) => {
            if compact {
                180.
            } else {
                260.
            }
        }
        None => {
            if compact {
                180.
            } else {
                210.
            }
        }
    }
}

fn tab_row_overflows(count: usize, tab_width: f32, available_width: f32) -> bool {
    if count == 0 {
        return false;
    }
    let reserved_width = 44.;
    (count as f32 * tab_width) > (available_width - reserved_width).max(0.)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TabDropOrder {
    order: Vec<TabId>,
    pinned: Option<bool>,
}

fn normalized_tab_order(ids: &[TabId], current_order: &[TabId]) -> Vec<TabId> {
    let id_set = ids.iter().copied().collect::<BTreeSet<_>>();
    let mut order = current_order
        .iter()
        .copied()
        .filter(|tab_id| id_set.contains(tab_id))
        .collect::<Vec<_>>();
    for tab_id in ids {
        if !order.contains(tab_id) {
            order.push(*tab_id);
        }
    }
    order
}

fn tab_display_order(
    ids: &[TabId],
    current_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
) -> Vec<TabId> {
    let order = normalized_tab_order(ids, current_order);
    order
        .iter()
        .copied()
        .filter(|tab_id| pinned_tabs.contains(tab_id))
        .chain(
            order
                .iter()
                .copied()
                .filter(|tab_id| !pinned_tabs.contains(tab_id)),
        )
        .collect()
}

fn tab_order_after_pin(
    ids: &[TabId],
    current_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
    tab_id: TabId,
) -> Vec<TabId> {
    let mut order = normalized_tab_order(ids, current_order)
        .into_iter()
        .filter(|order_tab_id| *order_tab_id != tab_id)
        .collect::<Vec<_>>();
    let insert_at = order
        .iter()
        .rposition(|order_tab_id| pinned_tabs.contains(order_tab_id))
        .map(|index| index + 1)
        .unwrap_or(0);
    order.insert(insert_at, tab_id);
    order
}

fn tab_order_after_unpin(
    ids: &[TabId],
    current_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
    tab_id: TabId,
) -> Vec<TabId> {
    if !ids.contains(&tab_id) {
        return normalized_tab_order(ids, current_order);
    }

    let mut remaining_pinned = pinned_tabs.clone();
    remaining_pinned.remove(&tab_id);
    let mut order = tab_display_order(ids, current_order, pinned_tabs)
        .into_iter()
        .filter(|order_tab_id| *order_tab_id != tab_id)
        .collect::<Vec<_>>();
    let insert_at = order
        .iter()
        .rposition(|order_tab_id| remaining_pinned.contains(order_tab_id))
        .map(|index| index + 1)
        .unwrap_or(0);
    order.insert(insert_at, tab_id);
    order
}

fn tab_order_after_tab_drop(
    ids: &[TabId],
    current_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
    dragged_tab_id: TabId,
    target_tab_id: TabId,
) -> TabDropOrder {
    let order = normalized_tab_order(ids, current_order);
    let display_order = tab_display_order(ids, current_order, pinned_tabs);
    if dragged_tab_id == target_tab_id
        || !ids.contains(&dragged_tab_id)
        || !ids.contains(&target_tab_id)
    {
        return TabDropOrder {
            order,
            pinned: None,
        };
    }

    let dragged_pinned = pinned_tabs.contains(&dragged_tab_id);
    let target_pinned = pinned_tabs.contains(&target_tab_id);
    let mut next_order = order
        .into_iter()
        .filter(|order_tab_id| *order_tab_id != dragged_tab_id)
        .collect::<Vec<_>>();

    if dragged_pinned && !target_pinned {
        next_order.push(dragged_tab_id);
        return TabDropOrder {
            order: next_order,
            pinned: Some(false),
        };
    }

    if !dragged_pinned && target_pinned {
        let insert_at = next_order
            .iter()
            .rposition(|order_tab_id| pinned_tabs.contains(order_tab_id))
            .map(|index| index + 1)
            .unwrap_or(0);
        next_order.insert(insert_at, dragged_tab_id);
        return TabDropOrder {
            order: next_order,
            pinned: None,
        };
    }

    let dragging_backward = drag_moves_towards_end(&display_order, dragged_tab_id, target_tab_id);
    let target_index = next_order
        .iter()
        .position(|order_tab_id| *order_tab_id == target_tab_id)
        .unwrap_or(next_order.len());
    let insert_at = if dragging_backward {
        (target_index + 1).min(next_order.len())
    } else {
        target_index
    };
    next_order.insert(insert_at, dragged_tab_id);
    TabDropOrder {
        order: next_order,
        pinned: None,
    }
}

fn drag_moves_towards_end<T: PartialEq>(order: &[T], dragged: T, target: T) -> bool {
    let dragged_index = order.iter().position(|item| *item == dragged);
    let target_index = order.iter().position(|item| *item == target);
    matches!((dragged_index, target_index), (Some(dragged), Some(target)) if dragged < target)
}

fn normalized_workspace_tab_order(
    scopes: &[FirstLevelScope],
    current_order: &[FirstLevelScope],
) -> Vec<FirstLevelScope> {
    let mut order = current_order
        .iter()
        .filter(|scope| scopes.contains(scope))
        .cloned()
        .collect::<Vec<_>>();
    for scope in scopes {
        if !order.contains(scope) {
            order.push(scope.clone());
        }
    }
    order
}

fn workspace_tab_order_after_drop(
    scopes: &[FirstLevelScope],
    current_order: &[FirstLevelScope],
    dragged_scope: &FirstLevelScope,
    target_scope: &FirstLevelScope,
) -> Vec<FirstLevelScope> {
    let order = normalized_workspace_tab_order(scopes, current_order);
    if dragged_scope == target_scope
        || !scopes.contains(dragged_scope)
        || !scopes.contains(target_scope)
    {
        return order;
    }

    let mut next_order = order
        .into_iter()
        .filter(|scope| scope != dragged_scope)
        .collect::<Vec<_>>();
    let display_order = normalized_workspace_tab_order(scopes, current_order);
    let dragging_backward =
        drag_moves_towards_end(&display_order, dragged_scope.clone(), target_scope.clone());
    let target_index = next_order
        .iter()
        .position(|scope| scope == target_scope)
        .unwrap_or(next_order.len());
    let insert_at = if dragging_backward {
        (target_index + 1).min(next_order.len())
    } else {
        target_index
    };
    next_order.insert(insert_at, dragged_scope.clone());
    next_order
}

fn order_first_level_tab_groups(
    groups: Vec<WorkspaceTabGroup>,
    workspace_tab_order: &[FirstLevelScope],
) -> Vec<WorkspaceTabGroup> {
    let scopes = groups
        .iter()
        .map(WorkspaceTabGroup::scope)
        .collect::<Vec<_>>();
    let order = normalized_workspace_tab_order(&scopes, workspace_tab_order);
    order
        .into_iter()
        .filter_map(|scope| groups.iter().find(|group| group.scope() == scope).cloned())
        .collect()
}

fn order_tab_refs<'a>(
    tabs: Vec<&'a TabState>,
    tab_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
) -> Vec<&'a TabState> {
    let ids = tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
    let display_order = tab_display_order(&ids, tab_order, pinned_tabs);
    display_order
        .into_iter()
        .filter_map(|tab_id| tabs.iter().find(|tab| tab.id == tab_id).copied())
        .collect()
}

fn first_level_tab_entries(
    groups: &[WorkspaceTabGroup],
    active_scope: Option<&FirstLevelScope>,
    search: &str,
) -> Vec<TabSwitcherEntry> {
    let query = normalized_sidebar_search(search);
    groups
        .iter()
        .filter_map(|group| match group {
            WorkspaceTabGroup::Database {
                scope,
                connection_name,
                database,
                first_tab_id,
                ..
            } => {
                if !query.is_empty()
                    && !search_matches_text(database, &query)
                    && !search_matches_text(connection_name, &query)
                {
                    return None;
                }
                Some(TabSwitcherEntry {
                    id: *first_tab_id,
                    title: format!("{database} {connection_name}"),
                    active: matches!(
                        active_scope,
                        Some(FirstLevelScope::Database(active)) if active == scope
                    ),
                    icon: AppIcon::Database,
                })
            }
            WorkspaceTabGroup::RedisConnection {
                connection_id,
                connection_name,
                first_tab_id,
                ..
            } => {
                if !query.is_empty() && !search_matches_text(connection_name, &query) {
                    return None;
                }
                Some(TabSwitcherEntry {
                    id: *first_tab_id,
                    title: connection_name.clone(),
                    active: matches!(
                        active_scope,
                        Some(FirstLevelScope::RedisConnection(active))
                            if active == connection_id
                    ),
                    icon: AppIcon::Database,
                })
            }
            WorkspaceTabGroup::Settings { first_tab_id } => {
                let settings_title = tr("app.tabs.settings");
                if !query.is_empty() && !search_matches_text(&settings_title, &query) {
                    return None;
                }
                Some(TabSwitcherEntry {
                    id: *first_tab_id,
                    title: settings_title.to_string(),
                    active: matches!(active_scope, Some(FirstLevelScope::Settings)),
                    icon: AppIcon::Settings,
                })
            }
            WorkspaceTabGroup::AiChat { first_tab_id } => {
                let title = tr("app.tabs.ai_chat");
                if !query.is_empty() && !search_matches_text(&title, &query) {
                    return None;
                }
                Some(TabSwitcherEntry {
                    id: *first_tab_id,
                    title: title.to_string(),
                    active: matches!(active_scope, Some(FirstLevelScope::AiChat)),
                    icon: AppIcon::Bot,
                })
            }
        })
        .collect()
}

fn table_tab_entries(
    state: &AppState,
    active_scope: Option<&FirstLevelScope>,
    search: &str,
    tab_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
) -> Vec<TabSwitcherEntry> {
    let query = normalized_sidebar_search(search);
    let tabs = state
        .tabs
        .iter()
        .filter(|tab| tab_matches_workspace_scope(tab, active_scope))
        .filter(|tab| query.is_empty() || search_matches_text(&tab_title(tab, true), &query))
        .collect::<Vec<_>>();
    order_tab_refs(tabs, tab_order, pinned_tabs)
        .into_iter()
        .map(|tab| TabSwitcherEntry {
            id: tab.id,
            title: tab_title(tab, true),
            active: state.active_tab == Some(tab.id),
            icon: AppIcon::Table,
        })
        .collect()
}

fn tab_matches_workspace_scope(tab: &TabState, active_scope: Option<&FirstLevelScope>) -> bool {
    match (active_scope, tab_first_level_scope(tab)) {
        (
            Some(FirstLevelScope::Database(active_scope)),
            Some(FirstLevelScope::Database(tab_scope)),
        ) => &tab_scope == active_scope,
        (
            Some(FirstLevelScope::RedisConnection(active_connection)),
            Some(FirstLevelScope::RedisConnection(tab_connection)),
        ) => *active_connection == tab_connection,
        (None, None) => true,
        _ => false,
    }
}

fn tab_workspace_scope(tab: &TabState) -> Option<WorkspaceScope> {
    tab.workspace()
}

fn tab_first_level_scope(tab: &TabState) -> Option<FirstLevelScope> {
    match &tab.kind {
        TabKind::DataEditor(editor)
            if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
        {
            Some(FirstLevelScope::RedisConnection(editor.object.connection_id))
        }
        TabKind::RedisWorkbench(workbench) => {
            Some(FirstLevelScope::RedisConnection(workbench.connection_id))
        }
        TabKind::RedisCli(cli) => Some(FirstLevelScope::RedisConnection(cli.connection_id)),
        TabKind::RedisPubSub(pubsub) => {
            Some(FirstLevelScope::RedisConnection(pubsub.connection_id))
        }
        TabKind::Settings(_) => Some(FirstLevelScope::Settings),
        // AI 是全局一级标签，不继承数据库工作区；业务页面由 context_tab 单独记录。
        TabKind::AiChat { .. } => Some(FirstLevelScope::AiChat),
        _ => tab_workspace_scope(tab).map(FirstLevelScope::Database),
    }
}

fn database_scope(state: &AppState, scope: Option<&FirstLevelScope>) -> Option<WorkspaceScope> {
    match scope? {
        FirstLevelScope::Database(workspace) => Some(workspace.clone()),
        FirstLevelScope::RedisConnection(connection_id) => state
            .active_tab()
            .and_then(TabState::workspace)
            .filter(|workspace| workspace.connection_id == *connection_id),
        FirstLevelScope::Settings | FirstLevelScope::AiChat => None,
    }
}

fn tab_context_menu_close_targets<I>(scopes: I, tab_id: TabId, include_current: bool) -> Vec<TabId>
where
    I: IntoIterator<Item = (TabId, Option<FirstLevelScope>)>,
{
    let scopes = scopes.into_iter().collect::<Vec<_>>();
    let Some(target_scope) = scopes
        .iter()
        .find(|(scope_tab_id, _)| *scope_tab_id == tab_id)
        .and_then(|(_, scope)| scope.clone())
    else {
        return Vec::new();
    };

    scopes
        .into_iter()
        .filter(|(scope_tab_id, scope)| {
            scope.as_ref() == Some(&target_scope) && (include_current || *scope_tab_id != tab_id)
        })
        .map(|(scope_tab_id, _)| scope_tab_id)
        .collect()
}

fn data_editor_table_name(state: &AppState, tab_id: TabId) -> Option<String> {
    state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => Some(editor.object.name.clone()),
            _ => None,
        })
}

fn object_path_database_name(path: &ObjectPath) -> &str {
    path.database.as_deref().unwrap_or("main")
}

fn connection_name(state: &AppState, connection_id: ConnectionId) -> String {
    state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
        .map(|connection| connection.config.name.clone())
        .unwrap_or_else(|| {
            tr("app.connection.fallback").replace("%{id}", &connection_id.0.to_string())
        })
}

fn connection_default_database(config: &ConnectionConfig) -> Option<String> {
    match &config.endpoint {
        Endpoint::Tcp { database, .. } => database.clone(),
        Endpoint::SqliteFile { .. } => Some("main".to_string()),
        Endpoint::Uri { .. } => None,
    }
}

fn tab_connection_color_hex<'a>(state: &'a AppState, tab: &TabState) -> Option<&'a str> {
    let scope = tab_workspace_scope(tab)?;
    state
        .connections
        .iter()
        .find(|connection| connection.config.id == scope.connection_id)
        .map(|connection| connection_config_color_hex(&connection.config.options))
}

/// 连接色只用于页签身份标记；近黑色在暗色主题里改用中性前景，避免细线消失。
fn tab_identity_color(hex: &str, colors: UiColors) -> gpui::Rgba {
    if colors.is_dark && hex == DEFAULT_CONNECTION_COLOR {
        colors.muted
    } else {
        connection_color_rgba(hex)
    }
}

fn first_level_tab_row(
    groups: &[WorkspaceTabGroup],
    active_scope: Option<&FirstLevelScope>,
    hovered_scope: Option<&FirstLevelScope>,
    compact: bool,
    available_width: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let show_switcher = tab_row_overflows(
        groups.len(),
        if compact { 172. } else { 224. },
        available_width,
    );
    let mut tab_strip = div()
        .id("first-level-tab-scroll-row")
        .h_full()
        .flex_1()
        .min_w(px(0.))
        .px_1()
        .pt(px(3.))
        .gap_1()
        .flex()
        .items_end()
        .overflow_x_scroll();

    for group in groups {
        let group_scope = group.scope();
        let active = active_scope == Some(&group_scope);
        tab_strip = tab_strip.child(first_level_tab_view(
            group,
            active,
            compact,
            hovered_scope == Some(&group_scope),
            colors,
            cx,
        ));
    }

    div()
        .h(px(34.))
        .min_w(px(0.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.status_bg)
        .flex()
        .overflow_hidden()
        .child(tab_strip.child(div().flex_1()))
        .when(show_switcher, |this| {
            this.child(tab_switcher_button(
                TabSwitcherKind::Databases,
                active_scope.is_some(),
                colors,
                cx,
            ))
        })
}

fn table_tab_row(
    state: &AppState,
    active_scope: Option<&FirstLevelScope>,
    compact: bool,
    available_width: f32,
    tab_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
    hovered_tab: Option<TabId>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let table_count =
        table_tab_entries(state, active_scope, "", tab_order, pinned_tabs).len();
    let show_switcher = tab_row_overflows(
        table_count,
        tab_width_for_kind(Some(TabKindDiscriminant::DataEditor), compact),
        available_width,
    );
    let mut tab_strip = div()
        .id("table-tab-scroll-row")
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .overflow_x_scroll();

    let scoped_tabs = state
        .tabs
        .iter()
        .filter(|tab| tab_matches_workspace_scope(tab, active_scope))
        .collect::<Vec<_>>();

    for tab in order_tab_refs(scoped_tabs, tab_order, pinned_tabs) {
        tab_strip = tab_strip.child(tab_view_with_context(
            tab,
            state.active_tab == Some(tab.id),
            compact,
            false,
            tab_connection_color_hex(state, tab),
            pinned_tabs.contains(&tab.id),
            hovered_tab == Some(tab.id),
            colors,
            cx,
        ));
    }

    div()
        .debug_selector(|| "table-tab-row".into())
        .h(px(36.))
        .min_w(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .overflow_hidden()
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .child(tab_strip.child(query_tab_blank_area(
                    database_scope(state, active_scope),
                    cx,
                )))
                .when(show_switcher, |this| {
                    this.child(tab_switcher_button(
                        TabSwitcherKind::Tables,
                        false,
                        colors,
                        cx,
                    ))
                }),
        )
}

fn query_tab_blank_area(scope: Option<WorkspaceScope>, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .debug_selector(|| "table-tab-blank-area".into())
        .flex_1()
        .h_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if event.click_count >= 2 {
                    this.open_new_query(scope.clone(), cx);
                    cx.stop_propagation();
                }
            }),
        )
}

fn tab_switcher_button(
    kind: TabSwitcherKind,
    active: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w(px(38.))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(colors.border)
        .bg(if active {
            colors.panel_alt
        } else {
            rgba_with_alpha(colors.panel_bg, 0.)
        })
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.toggle_tab_switcher(kind, window, cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon_box(AppIcon::ChevronDown, 22., 18., colors.muted))
}

fn tab_switcher_popup(
    kind: TabSwitcherKind,
    state: &AppState,
    active_scope: Option<&FirstLevelScope>,
    search_input: Entity<InputState>,
    search: &str,
    two_level: bool,
    workspace_tab_order: &[FirstLevelScope],
    tab_order: &[TabId],
    pinned_tabs: &BTreeSet<TabId>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let entries = match kind {
        TabSwitcherKind::Databases => first_level_tab_entries(
            &order_first_level_tab_groups(first_level_tab_groups(state), workspace_tab_order),
            active_scope,
            search,
        ),
        TabSwitcherKind::Tables => {
            table_tab_entries(state, active_scope, search, tab_order, pinned_tabs)
        }
    };
    let top = tab_switcher_popup_top(kind, two_level);

    let mut list = div()
        .max_h(px(330.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_1();
    if entries.is_empty() {
        list = list.child(
            div()
                .h(px(38.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(tr("app.tabs.no_match")),
        );
    } else {
        for entry in entries {
            list = list.child(tab_switcher_item(entry, colors, cx));
        }
    }

    div()
        .absolute()
        .top(px(top))
        .right(px(10.))
        .w(px(360.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .occlude()
        .shadow(vec![box_shadow(
            px(0.),
            px(14.),
            px(30.),
            px(0.),
            hsla(0., 0., 0., 0.16),
        )])
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(Input::new(&search_input).small())
        .child(list)
}

fn tab_switcher_popup_top(kind: TabSwitcherKind, two_level: bool) -> f32 {
    match kind {
        TabSwitcherKind::Databases => 34.,
        TabSwitcherKind::Tables if two_level => 70.,
        TabSwitcherKind::Tables => 34.,
    }
}

fn tab_switcher_item(
    entry: TabSwitcherEntry,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("tab-switcher-item", entry.id.0))
        .h(px(38.))
        .rounded(colors.radius_lg)
        .px_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_3()
        .text_color(colors.text)
        .bg(if entry.active {
            colors.hover
        } else {
            gpui::Rgba {
                r: 0.,
                g: 0.,
                b: 0.,
                a: 0.,
            }
        })
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.tab_switcher = None;
                this.dispatch(AppCommand::ActivateTab(entry.id), cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon_box(
            entry.icon,
            20.,
            16.,
            tab_switcher_icon_color(entry.icon, colors),
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(entry.title),
        )
}

fn tab_switcher_icon_color(icon: AppIcon, colors: UiColors) -> gpui::Rgba {
    match icon {
        AppIcon::Database => rgb(0xf0b400),
        AppIcon::Table => rgb(0x00c985),
        AppIcon::Settings => colors.info,
        _ => colors.text,
    }
}

fn first_level_tab_view(
    group: &WorkspaceTabGroup,
    active: bool,
    compact: bool,
    hovered: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let tab_id = group.first_tab_id();
    let drop_scope = group.scope();
    let close_scope = group.scope();
    let title = match group {
        WorkspaceTabGroup::Database { database, .. } => database.clone(),
        WorkspaceTabGroup::RedisConnection { connection_name, .. } => connection_name.clone(),
        WorkspaceTabGroup::Settings { .. } => tr("app.tabs.settings").to_string(),
        WorkspaceTabGroup::AiChat { .. } => tr("app.tabs.ai_chat").to_string(),
    };
    let subtitle = match group {
        WorkspaceTabGroup::Database { connection_name, .. } => connection_name.clone(),
        WorkspaceTabGroup::RedisConnection { .. } => tr("app.tabs.redis").to_string(),
        WorkspaceTabGroup::Settings { .. } => tr("app.tabs.global").to_string(),
        WorkspaceTabGroup::AiChat { .. } => tr("app.tabs.global").to_string(),
    };
    let (icon, accent) = match group {
        WorkspaceTabGroup::Database {
            connection_color_hex,
            ..
        } => (
            AppIcon::Database,
            tab_identity_color(connection_color_hex, colors),
        ),
        WorkspaceTabGroup::RedisConnection {
            connection_color_hex,
            ..
        } => (
            AppIcon::Database,
            tab_identity_color(connection_color_hex, colors),
        ),
        WorkspaceTabGroup::Settings { .. } => (AppIcon::Settings, colors.info),
        WorkspaceTabGroup::AiChat { .. } => (AppIcon::Bot, colors.info),
    };
    let dragged_workspace_tab = DraggedWorkspaceTab {
        scope: group.scope(),
        title: title.clone(),
    };
    let hover_scope = group.scope();
    div()
        .id(("first-level-tab", tab_id.0))
        .debug_selector(move || format!("first-level-tab:{}", tab_id.0))
        .relative()
        .h(px(30.))
        .w(px(if compact { 172. } else { 224. }))
        .flex_none()
        .min_w(px(0.))
        .border_1()
        .border_color(if active {
            colors.border
        } else {
            colors.border_soft
        })
        .rounded(colors.radius_lg)
        .bg(if active {
            colors.panel_bg
        } else {
            colors.panel_alt
        })
        .flex()
        .items_center()
        .overflow_hidden()
        .cursor_move()
        .hover(move |style| {
            style.bg(if active {
                colors.panel_bg
            } else {
                colors.hover
            })
            .border_color(colors.border)
        })
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered {
                this.hovered_first_level_tab = Some(hover_scope.clone());
            } else if this.hovered_first_level_tab.as_ref() == Some(&hover_scope) {
                this.hovered_first_level_tab = None;
            }
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::ActivateTab(tab_id), cx);
            }),
        )
        .on_drag(dragged_workspace_tab, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .drag_over::<DraggedWorkspaceTab>(move |style, _, _, _| {
            style.border_color(colors.info).bg(colors.hover)
        })
        .on_drop(cx.listener(move |this, drag: &DraggedWorkspaceTab, _, cx| {
            this.drop_workspace_tab_after(drag.scope.clone(), drop_scope.clone(), cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .px_2()
                .whitespace_nowrap()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .size(px(6.))
                        .flex_none()
                        .rounded_full()
                        .bg(if active { accent } else { colors.muted }),
                )
                .child(app_icon_box(
                    icon,
                    18.,
                    15.,
                    if active { colors.text } else { colors.muted },
                ))
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .whitespace_nowrap()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(px(13.))
                                .when(active, |this| {
                                    this.font_weight(gpui::FontWeight::SEMIBOLD)
                                })
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(subtitle),
                        ),
                ),
        )
        .when(hovered, |this| {
            this.child(
                div()
                    .absolute()
                    .right_1()
                    .top(px(2.))
                    .bottom(px(2.))
                    .w(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(colors.radius)
                    .bg(if active {
                        colors.panel_bg
                    } else {
                        colors.hover
                    })
                    .child(first_level_tab_close_button(
                        tab_id, close_scope, colors, cx,
                    )),
            )
        })
}

fn first_level_tab_close_button(
    tab_id: TabId,
    scope: FirstLevelScope,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("close-first-level-tab", tab_id.0))
        .size(px(20.))
        .flex_none()
        .rounded(colors.radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .tooltip(|window, cx| Tooltip::new(tr("app.common.close")).build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.request_close_workspace_scope(scope.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(app_icon(AppIcon::Close, 14., colors.muted))
}
