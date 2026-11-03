#[derive(Clone, Copy)]
pub(crate) struct ShortcutDefinition {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub icon: AppIcon,
    pub macos: &'static str,
    pub other: &'static str,
    pub action: ShortcutAction,
    pub context: Option<&'static str>,
}

#[derive(Clone, Copy)]
pub(crate) enum ShortcutAction {
    NewQuery,
    Refresh,
    SaveOrApply,
    CloseCurrentTab,
    ToggleConnectionBrowser,
    ExecuteOrApply,
    OpenDataSearch,
    OpenQueryHistoryQuickSearch,
    QuickOpenTable,
    CopyDataSelection,
}

pub(crate) const SHORTCUT_DEFINITIONS: &[ShortcutDefinition] = &[
    ShortcutDefinition {
        id: "app.new_query",
        title: "New query",
        detail: "Open a new SQL query tab",
        icon: AppIcon::Plus,
        macos: "cmd-y",
        other: "ctrl-y",
        action: ShortcutAction::NewQuery,
        context: None,
    },
    ShortcutDefinition {
        id: "app.refresh",
        title: "Refresh",
        detail: "Refresh the current connection, object, or data page",
        icon: AppIcon::Refresh,
        macos: "cmd-r",
        other: "ctrl-r",
        action: ShortcutAction::Refresh,
        context: None,
    },
    ShortcutDefinition {
        id: "app.save_or_apply",
        title: "Save / Apply",
        detail: "Save the query or apply current data changes",
        icon: AppIcon::Save,
        macos: "cmd-s",
        other: "ctrl-s",
        action: ShortcutAction::SaveOrApply,
        context: None,
    },
    ShortcutDefinition {
        id: "app.close_current_tab",
        title: "Close current tab",
        detail: "Close the currently open tab",
        icon: AppIcon::Close,
        macos: "cmd-w",
        other: "ctrl-w",
        action: ShortcutAction::CloseCurrentTab,
        context: None,
    },
    ShortcutDefinition {
        id: "app.toggle_connection_browser",
        title: "Toggle connection sidebar",
        detail: "Show or hide the connection browser",
        icon: AppIcon::PanelRight,
        macos: "cmd-1",
        other: "ctrl-1",
        action: ShortcutAction::ToggleConnectionBrowser,
        context: None,
    },
    ShortcutDefinition {
        id: "query.execute_or_apply",
        title: "Execute / Apply",
        detail: "Execute the query or apply current edits",
        icon: AppIcon::Play,
        macos: "cmd-enter",
        other: "ctrl-enter",
        action: ShortcutAction::ExecuteOrApply,
        context: None,
    },
    ShortcutDefinition {
        id: "query.search_data",
        title: "Search data",
        detail: "Open the search panel for the current data page",
        icon: AppIcon::Search,
        macos: "cmd-f",
        other: "ctrl-f",
        action: ShortcutAction::OpenDataSearch,
        context: Some("NavicatMain"),
    },
    ShortcutDefinition {
        id: "query.search_history",
        title: "SQL history quick search",
        detail: "Quickly search and reuse past queries",
        icon: AppIcon::FileSearch,
        macos: "shift-cmd-h",
        other: "ctrl-shift-h",
        action: ShortcutAction::OpenQueryHistoryQuickSearch,
        context: None,
    },
    ShortcutDefinition {
        id: "app.quick_open_table",
        title: "Quick open table",
        detail: "Search tables in the current database and open one",
        icon: AppIcon::Table,
        macos: "cmd-p",
        other: "ctrl-p",
        action: ShortcutAction::QuickOpenTable,
        context: None,
    },
    ShortcutDefinition {
        id: "query.copy_data",
        title: "Copy data",
        detail: "Copy the selected table data",
        icon: AppIcon::Copy,
        macos: "cmd-c",
        other: "ctrl-c",
        action: ShortcutAction::CopyDataSelection,
        context: Some("NavicatMain"),
    },
];

pub(crate) fn default_shortcut(definition: &ShortcutDefinition) -> &'static str {
    if cfg!(target_os = "macos") {
        definition.macos
    } else {
        definition.other
    }
}

pub(crate) fn current_shortcut(
    settings: &Settings,
    definition: &ShortcutDefinition,
) -> String {
    settings
        .custom_keybindings
        .get(definition.id)
        .filter(|spec| Keystroke::parse(spec).is_ok())
        .cloned()
        .unwrap_or_else(|| default_shortcut(definition).to_string())
}

pub(crate) fn shortcut_display(spec: &str) -> String {
    let parts = spec.split('-').map(|part| {
        if cfg!(target_os = "macos") {
            match part {
                "cmd" => "⌘",
                "ctrl" => "⌃",
                "alt" => "⌥",
                "shift" => "⇧",
                "enter" => "↵",
                "escape" => "Esc",
                "backspace" | "delete" => "⌫",
                "space" => "Space",
                other => other,
            }
        } else {
            match part {
                "cmd" => "Meta",
                "ctrl" => "Ctrl",
                "alt" => "Alt",
                "shift" => "Shift",
                "enter" => "Enter",
                "escape" => "Esc",
                "backspace" => "Backspace",
                "delete" => "Delete",
                "space" => "Space",
                other => other,
            }
        }
    });

    if cfg!(target_os = "macos") {
        parts.collect::<Vec<_>>().join("")
    } else {
        parts.collect::<Vec<_>>().join("+")
    }
}

pub(crate) fn bind_shortcut(
    cx: &mut App,
    spec: &str,
    action: ShortcutAction,
    context: Option<&'static str>,
) {
    match action {
        ShortcutAction::NewQuery => cx.bind_keys([KeyBinding::new(spec, NewQuery, context)]),
        ShortcutAction::Refresh => cx.bind_keys([KeyBinding::new(spec, Refresh, context)]),
        ShortcutAction::SaveOrApply => cx.bind_keys([KeyBinding::new(spec, SaveOrApply, context)]),
        ShortcutAction::CloseCurrentTab => {
            cx.bind_keys([KeyBinding::new(spec, CloseCurrentTab, context)])
        }
        ShortcutAction::ToggleConnectionBrowser => {
            cx.bind_keys([KeyBinding::new(spec, ToggleConnectionBrowser, context)])
        }
        ShortcutAction::ExecuteOrApply => {
            cx.bind_keys([KeyBinding::new(spec, ExecuteOrApply, context)])
        }
        ShortcutAction::OpenDataSearch => {
            cx.bind_keys([KeyBinding::new(spec, OpenDataSearch, context)])
        }
        ShortcutAction::OpenQueryHistoryQuickSearch => {
            cx.bind_keys([KeyBinding::new(spec, OpenQueryHistoryQuickSearch, context)])
        }
        ShortcutAction::QuickOpenTable => {
            cx.bind_keys([KeyBinding::new(spec, QuickOpenTable, context)])
        }
        ShortcutAction::CopyDataSelection => {
            cx.bind_keys([KeyBinding::new(spec, CopyDataSelection, context)])
        }
    }
}

pub(crate) fn shadow_shortcut(
    cx: &mut App,
    spec: &str,
    context: Option<&'static str>,
) {
    cx.bind_keys([KeyBinding::new(spec, NoAction, context)]);
}

