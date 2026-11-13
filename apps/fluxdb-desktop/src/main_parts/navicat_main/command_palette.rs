#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum CommandPaletteScope {
    All,
    Commands,
    Objects,
    Queries,
    History,
    Connections,
}

impl CommandPaletteScope {
    const ALL: [Self; 6] = [
        Self::All,
        Self::Commands,
        Self::Objects,
        Self::Queries,
        Self::History,
        Self::Connections,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Commands => "commands",
            Self::Objects => "objects",
            Self::Queries => "queries",
            Self::History => "history",
            Self::Connections => "connections",
        }
    }

    fn label(self) -> SharedString {
        tr(match self {
            Self::All => "app.command_palette.scope.all",
            Self::Commands => "app.command_palette.scope.commands",
            Self::Objects => "app.command_palette.scope.objects",
            Self::Queries => "app.command_palette.scope.queries",
            Self::History => "app.command_palette.scope.history",
            Self::Connections => "app.command_palette.scope.connections",
        })
    }

    fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|scope| *scope == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|scope| *scope == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandPaletteCommand {
    NewQuery,
    NewConnection,
    Refresh,
    SaveOrApply,
    CloseTab,
    ToggleSidebar,
    QuickOpenTable,
    QueryHistory,
    OpenSettings,
    ToggleTheme,
    OpenDataSearch,
}

#[derive(Clone, Copy)]
struct CommandPaletteCommandSpec {
    command: CommandPaletteCommand,
    title_key: &'static str,
    detail_key: &'static str,
    icon: AppIcon,
    shortcut_id: Option<&'static str>,
}

const COMMAND_PALETTE_COMMANDS: &[CommandPaletteCommandSpec] = &[
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::NewQuery,
        title_key: "app.command_palette.command.new_query",
        detail_key: "app.command_palette.command.new_query_detail",
        icon: AppIcon::Plus,
        shortcut_id: Some("app.new_query"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::NewConnection,
        title_key: "app.command_palette.command.new_connection",
        detail_key: "app.command_palette.command.new_connection_detail",
        icon: AppIcon::Plug,
        shortcut_id: None,
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::Refresh,
        title_key: "app.command_palette.command.refresh",
        detail_key: "app.command_palette.command.refresh_detail",
        icon: AppIcon::Refresh,
        shortcut_id: Some("app.refresh"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::SaveOrApply,
        title_key: "app.command_palette.command.save_or_apply",
        detail_key: "app.command_palette.command.save_or_apply_detail",
        icon: AppIcon::Save,
        shortcut_id: Some("app.save_or_apply"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::CloseTab,
        title_key: "app.command_palette.command.close_tab",
        detail_key: "app.command_palette.command.close_tab_detail",
        icon: AppIcon::Close,
        shortcut_id: Some("app.close_current_tab"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::ToggleSidebar,
        title_key: "app.command_palette.command.toggle_sidebar",
        detail_key: "app.command_palette.command.toggle_sidebar_detail",
        icon: AppIcon::PanelRight,
        shortcut_id: Some("app.toggle_connection_browser"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::QuickOpenTable,
        title_key: "app.command_palette.command.quick_open_table",
        detail_key: "app.command_palette.command.quick_open_table_detail",
        icon: AppIcon::Table,
        shortcut_id: Some("app.quick_open_table"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::QueryHistory,
        title_key: "app.command_palette.command.query_history",
        detail_key: "app.command_palette.command.query_history_detail",
        icon: AppIcon::FileSearch,
        shortcut_id: Some("query.search_history"),
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::OpenSettings,
        title_key: "app.command_palette.command.open_settings",
        detail_key: "app.command_palette.command.open_settings_detail",
        icon: AppIcon::Settings,
        shortcut_id: None,
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::ToggleTheme,
        title_key: "app.command_palette.command.toggle_theme",
        detail_key: "app.command_palette.command.toggle_theme_detail",
        icon: AppIcon::Wand,
        shortcut_id: None,
    },
    CommandPaletteCommandSpec {
        command: CommandPaletteCommand::OpenDataSearch,
        title_key: "app.command_palette.command.search_data",
        detail_key: "app.command_palette.command.search_data_detail",
        icon: AppIcon::Search,
        shortcut_id: Some("query.search_data"),
    },
];

struct CommandPaletteState {
    input: Entity<InputState>,
    _subscription: Subscription,
    scroll: ScrollHandle,
    open: bool,
    search: String,
    scope: CommandPaletteScope,
    selected: usize,
}

#[derive(Clone)]
enum CommandPaletteItem {
    Command(CommandPaletteCommand),
    Object(ObjectPath),
    SavedQuery(SavedQuery),
    History(Box<QueryHistoryEntry>),
    Connection(ConnectionId),
}

#[derive(Clone)]
struct CommandPaletteEntry {
    item: CommandPaletteItem,
    scope: CommandPaletteScope,
    title: String,
    detail: String,
    search_text: String,
    icon: AppIcon,
    badge: String,
    shortcut: Option<String>,
}

impl NavicatMain {
    fn open_command_palette_action(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_command_palette(window, cx);
    }

    fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_connection_kind.is_some() {
            return;
        }
        self.quick_open_table_open = false;
        self.query_history_quick_open = false;
        self.query_history_open = false;
        self.tab_switcher = None;
        self.command_palette.open = true;
        self.command_palette.search.clear();
        self.command_palette.scope = CommandPaletteScope::All;
        self.command_palette.selected = 0;
        self.command_palette.scroll.scroll_to_item(0);
        self.command_palette.input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn close_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_palette.open {
            self.command_palette.open = false;
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    fn command_palette_previous(
        &mut self,
        _: &CommandPalettePrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_command_palette_selection(-1, cx);
    }

    fn command_palette_next(
        &mut self,
        _: &CommandPaletteNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_command_palette_selection(1, cx);
    }

    fn command_palette_next_scope(
        &mut self,
        _: &CommandPaletteNextScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_command_palette_scope(self.command_palette.scope.next(), window, cx);
    }

    fn command_palette_previous_scope(
        &mut self,
        _: &CommandPalettePreviousScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_command_palette_scope(self.command_palette.scope.previous(), window, cx);
    }

    fn command_palette_confirm(
        &mut self,
        _: &CommandPaletteConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_command_palette(window, cx);
    }

    fn command_palette_cancel(
        &mut self,
        _: &CommandPaletteCancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_command_palette(window, cx);
    }

    fn move_command_palette_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.command_palette.open {
            return;
        }
        let entries = self.command_palette_filtered_entries();
        if entries.is_empty() {
            self.command_palette.selected = 0;
        } else {
            let selected = self
                .command_palette
                .selected
                .min(entries.len().saturating_sub(1));
            self.command_palette.selected =
                (selected as isize + delta).rem_euclid(entries.len() as isize) as usize;
            let child_index =
                command_palette_scroll_child_index(&entries, self.command_palette.selected);
            self.command_palette.scroll.scroll_to_item(child_index);
        }
        cx.notify();
    }

    fn set_command_palette_scope(
        &mut self,
        scope: CommandPaletteScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_palette.scope = scope;
        self.command_palette.selected = 0;
        self.command_palette.scroll.scroll_to_item(0);
        self.command_palette.input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn confirm_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.command_palette.open {
            return;
        }
        let entries = self.command_palette_filtered_entries();
        let Some(entry) = entries
            .get(
                self.command_palette
                    .selected
                    .min(entries.len().saturating_sub(1)),
            )
            .cloned()
        else {
            return;
        };
        self.execute_command_palette_item(entry.item, window, cx);
    }

    fn execute_command_palette_item(
        &mut self,
        item: CommandPaletteItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_palette.open = false;
        self.command_palette.selected = 0;
        cx.notify();

        match item {
            CommandPaletteItem::Command(command) => match command {
                CommandPaletteCommand::NewQuery => self.open_new_query(None, cx),
                CommandPaletteCommand::NewConnection => self.show_new_connection(window, cx),
                CommandPaletteCommand::Refresh => self.refresh_active(window, cx),
                CommandPaletteCommand::SaveOrApply => {
                    self.save_or_apply(&SaveOrApply, window, cx)
                }
                CommandPaletteCommand::CloseTab => {
                    self.close_current_tab(&CloseCurrentTab, window, cx)
                }
                CommandPaletteCommand::ToggleSidebar => {
                    self.toggle_connection_browser(&ToggleConnectionBrowser, window, cx)
                }
                CommandPaletteCommand::QuickOpenTable => self.open_quick_open_table(window, cx),
                CommandPaletteCommand::QueryHistory => {
                    self.show_query_history_quick_search(window, cx)
                }
                CommandPaletteCommand::OpenSettings => {
                    self.dispatch(AppCommand::OpenSettings, cx);
                }
                CommandPaletteCommand::ToggleTheme => self.toggle_theme(cx),
                CommandPaletteCommand::OpenDataSearch => {
                    self.open_data_search(&OpenDataSearch, window, cx)
                }
            },
            CommandPaletteItem::Object(object) => {
                self.dispatch(AppCommand::OpenDataEditor(object), cx);
            }
            CommandPaletteItem::SavedQuery(query) => {
                self.open_saved_query(query.id, cx);
            }
            CommandPaletteItem::History(entry) => {
                self.open_query_history_entry(*entry, cx);
            }
            CommandPaletteItem::Connection(connection_id) => {
                self.activate_connection_from_command_palette(connection_id, cx);
            }
        }
        cx.notify();
    }

    fn activate_connection_from_command_palette(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        let state = self.controller.state();
        let active_connection = state
            .active_tab()
            .and_then(|tab| tab.workspace())
            .map(|workspace| workspace.connection_id);
        if active_connection == Some(connection_id) {
            return;
        }

        if let Some(tab_id) = state
            .tabs
            .iter()
            .rev()
            .find(|tab| {
                tab.workspace()
                    .is_some_and(|workspace| workspace.connection_id == connection_id)
            })
            .map(|tab| tab.id)
        {
            self.dispatch(AppCommand::ActivateTab(tab_id), cx);
            return;
        }

        let connected = state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .is_some_and(|connection| connection.connected);
        if connected {
            self.show_message(
                tr("app.command_palette.connection.no_tabs").as_ref(),
                AppMessageKind::Info,
                cx,
            );
        } else {
            self.open_connection_from_sidebar(connection_id, cx);
        }
    }

    fn command_palette_entries(&self) -> Vec<CommandPaletteEntry> {
        let state = self.controller.state();
        let mut entries = Vec::new();
        let command_badge = tr("app.command_palette.badge.command").to_string();

        for spec in COMMAND_PALETTE_COMMANDS {
            if !self.command_palette_command_available(spec.command) {
                continue;
            }
            let title = tr(spec.title_key).to_string();
            let detail = tr(spec.detail_key).to_string();
            entries.push(CommandPaletteEntry {
                item: CommandPaletteItem::Command(spec.command),
                scope: CommandPaletteScope::Commands,
                title: title.clone(),
                detail: detail.clone(),
                search_text: format!("{title} {detail}").to_lowercase(),
                icon: spec.icon,
                badge: command_badge.clone(),
                shortcut: spec
                    .shortcut_id
                    .and_then(|id| self.command_palette_shortcut(id)),
            });
        }

        if let Some((connection_id, database)) = self.quick_open_table_scope() {
            for object in quick_open_table_loaded_entries(self, connection_id, database.as_deref())
            {
                let (badge, icon) = match object.kind {
                    ObjectKind::View => (
                        tr("app.command_palette.badge.view").to_string(),
                        AppIcon::Eye,
                    ),
                    _ => (
                        tr("app.command_palette.badge.table").to_string(),
                        AppIcon::Table,
                    ),
                };
                let mut scope_parts = Vec::new();
                if let Some(database) = object.database.as_deref() {
                    scope_parts.push(database.to_string());
                }
                if let Some(schema) = object.schema.as_deref() {
                    scope_parts.push(schema.to_string());
                }
                let object_scope = scope_parts.join(" / ");
                let detail = if object_scope.is_empty() {
                    badge.clone()
                } else {
                    format!("{object_scope} · {badge}")
                };
                entries.push(CommandPaletteEntry {
                    item: CommandPaletteItem::Object(object.clone()),
                    scope: CommandPaletteScope::Objects,
                    title: object.name.clone(),
                    detail: detail.clone(),
                    search_text: format!(
                        "{} {} {}",
                        object.name,
                        object_scope,
                        badge
                    )
                    .to_lowercase(),
                    icon,
                    badge,
                    shortcut: None,
                });
            }
        }

        for query in &self.saved_queries {
            let connection = state
                .connections
                .iter()
                .find(|connection| connection.config.id == query.connection_id)
                .map(|connection| connection.config.name.clone())
                .unwrap_or_else(|| {
                    tr("app.command_palette.connection.unknown").replace(
                        "%{id}",
                        &query.connection_id.0.to_string(),
                    )
                });
            let mut scope_parts = Vec::new();
            if let Some(database) = query.database.as_deref() {
                scope_parts.push(database.to_string());
            }
            if let Some(schema) = query.schema.as_deref() {
                scope_parts.push(schema.to_string());
            }
            let query_scope = if scope_parts.is_empty() {
                connection.clone()
            } else {
                format!("{connection} / {}", scope_parts.join(" / "))
            };
            let badge = tr("app.command_palette.badge.query").to_string();
            entries.push(CommandPaletteEntry {
                item: CommandPaletteItem::SavedQuery(query.clone()),
                scope: CommandPaletteScope::Queries,
                title: query.name.clone(),
                detail: format!("{query_scope} · {badge}"),
                search_text: format!("{} {}", query.name, query_scope).to_lowercase(),
                icon: AppIcon::FileSql,
                badge,
                shortcut: None,
            });
        }

        let history_badge = tr("app.command_palette.badge.history").to_string();
        for entry in state.query_history.iter().take(40) {
            let title = query_history_line(&entry.text);
            let detail = format!(
                "{} · {} · {}",
                query_history_scope(entry, state),
                query_history_time_label(entry.executed_at_unix_secs),
                query_history_kind_label(entry.kind)
            );
            entries.push(CommandPaletteEntry {
                item: CommandPaletteItem::History(Box::new(entry.clone())),
                scope: CommandPaletteScope::History,
                title: title.clone(),
                detail: detail.clone(),
                search_text: format!("{title} {detail}").to_lowercase(),
                icon: AppIcon::FileSearch,
                badge: history_badge.clone(),
                shortcut: None,
            });
        }

        let active_connection = state
            .active_tab()
            .and_then(|tab| tab.workspace())
            .map(|workspace| workspace.connection_id);
        for connection in &state.connections {
            let connected = if connection.connected {
                tr("app.command_palette.connection.connected")
            } else {
                tr("app.command_palette.connection.disconnected")
            };
            let kind = database_kind_label(connection.config.kind);
            let badge = if active_connection == Some(connection.config.id) {
                tr("app.command_palette.badge.current").to_string()
            } else {
                tr("app.command_palette.badge.connection").to_string()
            };
            entries.push(CommandPaletteEntry {
                item: CommandPaletteItem::Connection(connection.config.id),
                scope: CommandPaletteScope::Connections,
                title: connection.config.name.clone(),
                detail: format!("{kind} · {connected}"),
                search_text: format!("{} {kind} {connected}", connection.config.name)
                    .to_lowercase(),
                icon: AppIcon::Database,
                badge,
                shortcut: None,
            });
        }

        entries
    }

    fn command_palette_filtered_entries(&self) -> Vec<CommandPaletteEntry> {
        let entries = self.command_palette_entries();
        command_palette_filter_entries(
            &entries,
            &self.command_palette.search,
            self.command_palette.scope,
        )
    }

    fn command_palette_command_available(&self, command: CommandPaletteCommand) -> bool {
        let state = self.controller.state();
        let active_tab = state.active_tab();
        match command {
            CommandPaletteCommand::NewQuery => !state.connections.is_empty(),
            CommandPaletteCommand::NewConnection
            | CommandPaletteCommand::ToggleSidebar
            | CommandPaletteCommand::OpenSettings
            | CommandPaletteCommand::ToggleTheme => true,
            CommandPaletteCommand::Refresh | CommandPaletteCommand::CloseTab => {
                active_tab.is_some()
            }
            CommandPaletteCommand::SaveOrApply => {
                active_tab.is_some_and(|tab| {
                    matches!(
                        tab.kind,
                        TabKind::QueryEditor(_) | TabKind::Settings(_)
                    )
                }) || self
                    .active_data_editor_dirty()
                    .is_some_and(|(_, dirty)| dirty)
            }
            CommandPaletteCommand::QuickOpenTable => self.quick_open_table_scope().is_some(),
            CommandPaletteCommand::QueryHistory => !state.query_history.is_empty(),
            CommandPaletteCommand::OpenDataSearch => {
                self.active_searchable_data_tab_id().is_some()
            }
        }
    }

    fn command_palette_shortcut(&self, shortcut_id: &str) -> Option<String> {
        let definition = SHORTCUT_DEFINITIONS
            .iter()
            .find(|definition| definition.id == shortcut_id)?;
        Some(shortcut_display(&current_shortcut(
            &self.controller.state().settings,
            definition,
        )))
    }
}

fn command_palette_search_score(entry: &CommandPaletteEntry, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let title_score = fluxdb_completion::fuzzy_score(&entry.title, query);
    let detail_score = fluxdb_completion::fuzzy_score(&entry.detail, query);
    let text_score = fluxdb_completion::fuzzy_score(&entry.search_text, query);
    match (title_score, detail_score, text_score) {
        (None, None, None) => None,
        (title, detail, text) => Some(
            title.unwrap_or(0) * 2 + detail.unwrap_or(0) + text.unwrap_or(0),
        ),
    }
}

fn command_palette_filter_entries(
    entries: &[CommandPaletteEntry],
    search: &str,
    scope: CommandPaletteScope,
) -> Vec<CommandPaletteEntry> {
    let query = search.trim().to_lowercase();
    let mut scored = entries
        .iter()
        .filter(|entry| scope == CommandPaletteScope::All || entry.scope == scope)
        .enumerate()
        .filter_map(|(index, entry)| {
            command_palette_search_score(entry, &query)
                .map(|score| (score, index, entry.clone()))
        })
        .collect::<Vec<_>>();

    if !query.is_empty() {
        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
        });
    }

    let searching = !query.is_empty();
    let mut counts = BTreeMap::new();
    scored
        .into_iter()
        .map(|(_, _, entry)| entry)
        .filter(|entry| {
            let count = counts.entry(entry.scope).or_insert(0usize);
            let limit = command_palette_scope_limit(scope, entry.scope, searching);
            if *count >= limit {
                return false;
            }
            *count += 1;
            true
        })
        .collect()
}

fn command_palette_scope_limit(
    active: CommandPaletteScope,
    scope: CommandPaletteScope,
    searching: bool,
) -> usize {
    if active != CommandPaletteScope::All {
        return 100;
    }
    if searching {
        return 40;
    }
    match scope {
        CommandPaletteScope::Commands => 11,
        CommandPaletteScope::Objects => 12,
        CommandPaletteScope::Queries => 8,
        CommandPaletteScope::History => 8,
        CommandPaletteScope::Connections => 8,
        CommandPaletteScope::All => usize::MAX,
    }
}

fn command_palette_scope_count(
    entries: &[CommandPaletteEntry],
    scope: CommandPaletteScope,
) -> usize {
    if scope == CommandPaletteScope::All {
        entries.len()
    } else {
        entries
            .iter()
            .filter(|entry| entry.scope == scope)
            .count()
    }
}

/// `scroll_to_item` 按滚动容器的直接子元素计数；分组标题也占一个位置。
fn command_palette_scroll_child_index(
    entries: &[CommandPaletteEntry],
    selected: usize,
) -> usize {
    let mut child_index = 0usize;
    let mut flat_index = 0usize;
    for scope in CommandPaletteScope::ALL {
        if scope == CommandPaletteScope::All {
            continue;
        }
        let count = entries
            .iter()
            .filter(|entry| entry.scope == scope)
            .count();
        if count == 0 {
            continue;
        }
        child_index += 1;
        for _ in 0..count {
            if flat_index == selected {
                return child_index;
            }
            child_index += 1;
            flat_index += 1;
        }
    }
    0
}

fn command_palette_list_height(entries: &[CommandPaletteEntry]) -> f32 {
    if entries.is_empty() {
        return 150.;
    }
    let rows = entries.len() as f32 * 50.;
    let groups = CommandPaletteScope::ALL
        .iter()
        .skip(1)
        .filter(|scope| entries.iter().any(|entry| entry.scope == **scope))
        .count() as f32
        * 28.;
    (rows + groups + 16.).clamp(120., 432.)
}

fn command_palette_selected_index(this: &NavicatMain, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        this.command_palette.selected.min(len - 1)
    }
}

fn command_palette_modal(
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let all_entries = this.command_palette_entries();
    let entries = command_palette_filter_entries(
        &all_entries,
        &this.command_palette.search,
        this.command_palette.scope,
    );
    let list_height = command_palette_list_height(&entries);
    let selected = command_palette_selected_index(this, entries.len());
    let selected_bg = if colors.is_dark {
        rgb(0x263244)
    } else {
        rgb(0xe8f2ff)
    };

    let mut list = div()
        .id("command-palette-scroll")
        .size_full()
        .track_scroll(&this.command_palette.scroll)
        .overflow_y_scroll()
        .p_2();
    if entries.is_empty() {
        list = list.child(
            div()
                .h(px(150.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(app_icon(AppIcon::Search, 20., colors.muted))
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(tr("app.command_palette.empty")),
                ),
        );
    } else {
        let mut flat_index = 0usize;
        for scope in CommandPaletteScope::ALL {
            if scope == CommandPaletteScope::All {
                continue;
            }
            let items = entries
                .iter()
                .filter(|entry| entry.scope == scope)
                .cloned()
                .collect::<Vec<_>>();
            if items.is_empty() {
                continue;
            }
            list = list.child(
                div()
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(scope.label())
                    .child(div().flex_1().h(px(1.)).bg(colors.border_soft))
                    .child(div().text_color(colors.muted).child(items.len().to_string())),
            );
            for entry in items {
                list = list.child(command_palette_row(
                    entry,
                    flat_index,
                    selected == flat_index,
                    selected_bg,
                    colors,
                    cx,
                ));
                flat_index += 1;
            }
        }
    }

    let mut scope_tabs = div()
        .h(px(40.))
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt);
    for scope in CommandPaletteScope::ALL {
        let active = this.command_palette.scope == scope;
        let count = command_palette_scope_count(&all_entries, scope);
        scope_tabs = scope_tabs.child(
            div()
                .id(scope.key())
                .debug_selector({
                    let scope = scope.key();
                    move || format!("command-palette-scope:{scope}")
                })
                .h(px(26.))
                .px_2()
                .rounded(px(5.))
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .text_size(ui_text_size(11.))
                .text_color(if active { colors.text } else { colors.muted })
                .bg(if active {
                    colors.tree_selected
                } else {
                    colors.panel_alt
                })
                .hover(move |style| style.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.set_command_palette_scope(scope, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(scope.label())
                .child(
                    div()
                        .text_size(ui_text_size(9.))
                        .text_color(colors.muted)
                        .child(count.to_string()),
                ),
        );
    }

    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.48)
        } else {
            opaque_grey(0.75, 0.20)
        })
        .flex()
        .items_start()
        .justify_center()
        .pt(px(62.))
        .px_4()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.close_command_palette(window, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w_full()
                .max_w(px(720.))
                .max_h(px(560.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., 0.26),
                )])
                .flex()
                .flex_col()
                .overflow_hidden()
                .key_context("CommandPalette")
                .debug_selector(|| "command-palette".into())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(54.))
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon(AppIcon::Search, 16., colors.muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(Input::new(&this.command_palette.input).small()),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.))
                                .text_color(colors.muted)
                                .child("Esc"),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius_lg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |style| style.bg(colors.hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        this.close_command_palette(window, cx);
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(app_icon(AppIcon::Close, 16., colors.muted)),
                        ),
                )
                .child(scope_tabs)
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .h(px(list_height))
                        .child(list)
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .child(Scrollbar::vertical(&this.command_palette.scroll)),
                        ),
                )
                .child(
                    div()
                        .h(px(34.))
                        .px_3()
                        .border_t_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(format!(
                            "↑↓ {} · ↵ {} · Tab {} · Esc {}",
                            tr("app.command_palette.hint.select"),
                            tr("app.command_palette.hint.open"),
                            tr("app.command_palette.hint.scope"),
                            tr("app.command_palette.hint.close"),
                        ))
                        .child(div().flex_1())
                        .child(
                            tr("app.command_palette.count")
                                .replace("%{count}", &entries.len().to_string()),
                        ),
                ),
        )
}

fn command_palette_row(
    entry: CommandPaletteEntry,
    index: usize,
    selected: bool,
    selected_bg: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let trailing = if selected {
        "↵".to_string()
    } else {
        entry.shortcut.clone().unwrap_or_default()
    };
    let row_selector = format!("command-palette-row:{}:{index}", entry.scope.key());
    div()
        .id(("command-palette-row", index))
        .debug_selector(move || row_selector.clone())
        .h(px(50.))
        .rounded(colors.radius_lg)
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .bg(if selected {
            selected_bg
        } else {
            colors.panel_bg
        })
        .hover(move |style| style.bg(if selected { selected_bg } else { colors.hover }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.execute_command_palette_item(entry.item.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .size(px(28.))
                .flex_none()
                .rounded(colors.radius_lg)
                .bg(if colors.is_dark {
                    rgb(0x202a36)
                } else {
                    rgb(0xf0f4f8)
                })
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon(entry.icon, 15., colors.muted)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .line_height(ui_text_size(18.))
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(entry.title.clone()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.5))
                        .line_height(ui_text_size(14.))
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(entry.detail.clone()),
                ),
        )
        .child(
            div()
                .px_2()
                .h(px(20.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .text_size(ui_text_size(9.5))
                .text_color(colors.muted)
                .bg(colors.panel_alt)
                .child(entry.badge.clone()),
        )
        .when(!trailing.is_empty(), |this| {
            this.child(
                div()
                    .min_w(px(24.))
                    .h(px(20.))
                    .px_1()
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(ui_text_size(9.5))
                    .text_color(if selected { colors.text } else { colors.muted })
                    .bg(if selected {
                        colors.tree_selected
                    } else {
                        colors.panel_alt
                    })
                    .child(trailing),
            )
        })
}
