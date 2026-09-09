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
    CopyDataSelection,
}

pub(crate) const SHORTCUT_DEFINITIONS: &[ShortcutDefinition] = &[
    ShortcutDefinition {
        id: "app.new_query",
        title: "新建查询",
        detail: "打开一个新的 SQL 查询标签页",
        icon: AppIcon::Plus,
        macos: "cmd-y",
        other: "ctrl-y",
        action: ShortcutAction::NewQuery,
        context: None,
    },
    ShortcutDefinition {
        id: "app.refresh",
        title: "刷新",
        detail: "刷新当前连接、对象或数据页",
        icon: AppIcon::Refresh,
        macos: "cmd-r",
        other: "ctrl-r",
        action: ShortcutAction::Refresh,
        context: None,
    },
    ShortcutDefinition {
        id: "app.save_or_apply",
        title: "保存 / 应用",
        detail: "保存查询或应用当前数据修改",
        icon: AppIcon::Save,
        macos: "cmd-s",
        other: "ctrl-s",
        action: ShortcutAction::SaveOrApply,
        context: None,
    },
    ShortcutDefinition {
        id: "app.close_current_tab",
        title: "关闭当前标签",
        detail: "关闭当前打开的标签页",
        icon: AppIcon::Close,
        macos: "cmd-w",
        other: "ctrl-w",
        action: ShortcutAction::CloseCurrentTab,
        context: None,
    },
    ShortcutDefinition {
        id: "app.toggle_connection_browser",
        title: "切换连接侧边栏",
        detail: "显示或隐藏左侧连接浏览器",
        icon: AppIcon::PanelRight,
        macos: "cmd-1",
        other: "ctrl-1",
        action: ShortcutAction::ToggleConnectionBrowser,
        context: None,
    },
    ShortcutDefinition {
        id: "query.execute_or_apply",
        title: "执行 / 应用",
        detail: "执行查询或应用当前编辑内容",
        icon: AppIcon::Play,
        macos: "cmd-enter",
        other: "ctrl-enter",
        action: ShortcutAction::ExecuteOrApply,
        context: None,
    },
    ShortcutDefinition {
        id: "query.search_data",
        title: "搜索数据",
        detail: "打开当前数据页的搜索面板",
        icon: AppIcon::Search,
        macos: "cmd-f",
        other: "ctrl-f",
        action: ShortcutAction::OpenDataSearch,
        context: Some("NavicatMain"),
    },
    ShortcutDefinition {
        id: "query.search_history",
        title: "SQL 历史快搜",
        detail: "快速搜索并重用历史查询",
        icon: AppIcon::FileSearch,
        macos: "shift-cmd-h",
        other: "ctrl-shift-h",
        action: ShortcutAction::OpenQueryHistoryQuickSearch,
        context: None,
    },
    ShortcutDefinition {
        id: "query.copy_data",
        title: "复制数据",
        detail: "复制当前选中的表格数据",
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
    spec.split('-')
        .map(|part| match part {
            "cmd" => "⌘",
            "ctrl" => "⌃",
            "alt" => "⌥",
            "shift" => "⇧",
            "enter" => "↵",
            "escape" => "Esc",
            "backspace" => "⌫",
            "delete" => "⌫",
            "space" => "Space",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("")
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

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn invalid_saved_shortcut_falls_back_to_default() {
        let definition = SHORTCUT_DEFINITIONS[0];
        let mut settings = Settings::default();
        settings
            .custom_keybindings
            .insert(definition.id.to_string(), "not-a-keystroke".to_string());

        assert_eq!(default_shortcut(&definition), current_shortcut(&settings, &definition));
    }

    #[test]
    fn shortcut_display_uses_platform_symbols() {
        assert_eq!("⇧⌘h", shortcut_display("shift-cmd-h"));
    }
}
