/// 工作区标签编号直达槽位数量（⌥1–⌥9）。
pub(crate) const TAB_SLOT_COUNT: usize = 9;
/// 表快捷组总数上限。超过后设置页新增入口置灰。
pub(crate) const TABLE_SHORTCUT_GROUP_LIMIT: usize = 32;
/// 单组表数量上限，对齐选择器的 1–9 数字直选。
pub(crate) const TABLE_SHORTCUT_TARGET_LIMIT: usize = 9;

/// 按编号激活当前工作区第 N 个可见标签页。
#[derive(Clone, PartialEq, Eq, Deserialize, gpui_kit::Action)]
#[action(namespace = gdb, no_json)]
pub(crate) struct ActivateTabSlot {
    pub slot: usize,
}

/// 触发表快捷组。`slot` 是组在设置中的下标；动作时刻再读设置，避免持有陈旧数据。
#[derive(Clone, PartialEq, Eq, Deserialize, gpui_kit::Action)]
#[action(namespace = gdb, no_json)]
pub(crate) struct OpenTableShortcut {
    pub slot: usize,
}

/// 表快捷组选择器：按编号直选（1 基）。
#[derive(Clone, PartialEq, Eq, Deserialize, gpui_kit::Action)]
#[action(namespace = gdb, no_json)]
pub(crate) struct SelectTableShortcut {
    pub slot: usize,
}

/// 表级快捷键校验失败原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableShortcutSpecError {
    /// 无法解析为合法 keystroke。
    Parse,
    /// 只有字母/数字/功能键，或只按了修饰键。
    NeedsModifier,
}

/// 与已有快捷键冲突的来源。
#[derive(Clone)]
pub(crate) enum ShortcutConflict {
    /// 与 `SHORTCUT_DEFINITIONS` 中的应用快捷键冲突。
    AppShortcut(&'static ShortcutDefinition),
    /// 与另一个表快捷组冲突；携带对方展示名（组名或快捷键文本）。
    TableGroup(String),
    /// 与固定保留键位冲突（标签编号、退出）。
    Reserved(String),
}

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
    CommandPalette,
    NextTab,
    PreviousTab,
    OpenTabSwitcher,
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
        id: "app.command_palette",
        title: "Command palette",
        detail: "Search commands, tables, queries, history and connections",
        icon: AppIcon::Search,
        macos: "cmd-shift-p",
        other: "ctrl-shift-p",
        action: ShortcutAction::CommandPalette,
        context: None,
    },
    ShortcutDefinition {
        id: "app.next_tab",
        title: "Next tab",
        detail: "Switch to the next tab in the current workspace",
        icon: AppIcon::ChevronRight,
        macos: "ctrl-tab",
        other: "ctrl-tab",
        action: ShortcutAction::NextTab,
        context: None,
    },
    ShortcutDefinition {
        id: "app.previous_tab",
        title: "Previous tab",
        detail: "Switch to the previous tab in the current workspace",
        icon: AppIcon::ChevronLeft,
        macos: "ctrl-shift-tab",
        other: "ctrl-shift-tab",
        action: ShortcutAction::PreviousTab,
        context: None,
    },
    ShortcutDefinition {
        id: "app.tab_switcher",
        title: "Switch tab",
        detail: "Search and switch to a tab in the current workspace",
        icon: AppIcon::List,
        macos: "cmd-shift-e",
        other: "ctrl-shift-e",
        action: ShortcutAction::OpenTabSwitcher,
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
        ShortcutAction::CommandPalette => {
            cx.bind_keys([KeyBinding::new(spec, CommandPalette, context)])
        }
        ShortcutAction::NextTab => cx.bind_keys([KeyBinding::new(spec, NextTab, context)]),
        ShortcutAction::PreviousTab => {
            cx.bind_keys([KeyBinding::new(spec, PreviousTab, context)])
        }
        ShortcutAction::OpenTabSwitcher => {
            cx.bind_keys([KeyBinding::new(spec, OpenTabSwitcher, context)])
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

/// 解析后的按键签名：修饰键顺序无关，参与比较的键名小写。
type KeystrokeSignature = (bool, bool, bool, bool, String);

fn keystroke_signature(spec: &str) -> Option<KeystrokeSignature> {
    let keystroke = Keystroke::parse(spec).ok()?;
    Some((
        keystroke.modifiers.control,
        keystroke.modifiers.alt,
        keystroke.modifiers.shift,
        keystroke.modifiers.platform,
        keystroke.key.to_ascii_lowercase(),
    ))
}

fn modifiers(keystroke: &Keystroke) -> (bool, bool, bool, bool) {
    (
        keystroke.modifiers.control,
        keystroke.modifiers.alt,
        keystroke.modifiers.shift,
        keystroke.modifiers.platform,
    )
}

/// 两个规范比较是否是同一按键（忽略修饰键书写顺序）。
pub(crate) fn shortcut_specs_match(left: &str, right: &str) -> bool {
    match (keystroke_signature(left), keystroke_signature(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// 表级快捷键只接受带 ⌘ / ⌃ / ⌥ 的组合，避免抢走编辑器输入与单纯功能键。
pub(crate) fn validate_table_shortcut_spec(spec: &str) -> Result<(), TableShortcutSpecError> {
    let keystroke = Keystroke::parse(spec).map_err(|_| TableShortcutSpecError::Parse)?;
    if matches!(
        keystroke.key.as_str(),
        "ctrl" | "control" | "alt" | "shift" | "cmd" | "win"
    ) {
        return Err(TableShortcutSpecError::NeedsModifier);
    }
    let (control, alt, _, platform) = modifiers(&keystroke);
    if !(control || alt || platform) {
        return Err(TableShortcutSpecError::NeedsModifier);
    }
    Ok(())
}

/// 固定保留键位：⌥1–⌥9 用于标签编号直达，⌘Q / Ctrl+Q 退出。
fn reserved_shortcut_label(spec: &str) -> Option<String> {
    let (control, alt, shift, platform, key) = keystroke_signature(spec)?;
    if alt
        && !control
        && !shift
        && !platform
        && let Ok(slot) = key.parse::<usize>()
        && (1..=TAB_SLOT_COUNT).contains(&slot)
    {
        return Some(if cfg!(target_os = "macos") {
            format!("⌥1–⌥{TAB_SLOT_COUNT}")
        } else {
            format!("Alt+1–Alt+{TAB_SLOT_COUNT}")
        });
    }
    let quit = if cfg!(target_os = "macos") {
        "cmd-q"
    } else {
        "ctrl-q"
    };
    if shortcut_specs_match(spec, quit) {
        return Some(shortcut_display(quit));
    }
    None
}

/// 表快捷组的展示名：用户填了组名就用组名，否则回落到快捷键文本。
pub(crate) fn table_shortcut_group_name(group: &TableShortcutGroup) -> String {
    let name = group.name.trim();
    if name.is_empty() {
        shortcut_display(&group.keys)
    } else {
        name.to_string()
    }
}

/// 检测表级快捷键冲突。
///
/// 冲突范围：
/// 1. 固定保留键位（标签编号、退出）；
/// 2. `SHORTCUT_DEFINITIONS` 中当前生效的应用快捷键（含用户自定义覆盖）；
/// 3. 其它表快捷组（`ignore_group_id` 用于编辑自身时排除自己）。
pub(crate) fn table_shortcut_conflict(
    settings: &Settings,
    spec: &str,
    ignore_group_id: Option<&str>,
) -> Option<ShortcutConflict> {
    if let Some(label) = reserved_shortcut_label(spec) {
        return Some(ShortcutConflict::Reserved(label));
    }
    for definition in SHORTCUT_DEFINITIONS {
        let current = current_shortcut(settings, definition);
        if shortcut_specs_match(&current, spec) {
            return Some(ShortcutConflict::AppShortcut(definition));
        }
    }
    for group in &settings.table_shortcut_groups {
        if ignore_group_id == Some(group.id.as_str()) {
            continue;
        }
        if shortcut_specs_match(&group.keys, spec) {
            return Some(ShortcutConflict::TableGroup(table_shortcut_group_name(group)));
        }
    }
    None
}

/// 该组能否真正注册按键：自身合法、不与应用键冲突、且不与前面的组重复。
///
/// 同一键位重复时只有第一组生效；后续组保留配置并在设置页标记冲突，避免静默丢配置。
pub(crate) fn table_shortcut_group_bindable(settings: &Settings, index: usize) -> bool {
    let Some(group) = settings.table_shortcut_groups.get(index) else {
        return false;
    };
    if group.targets.is_empty() || validate_table_shortcut_spec(&group.keys).is_err() {
        return false;
    }
    for earlier in settings.table_shortcut_groups.iter().take(index) {
        if shortcut_specs_match(&earlier.keys, &group.keys) {
            return false;
        }
    }
    match table_shortcut_conflict(settings, &group.keys, Some(group.id.as_str())) {
        None => true,
        // 与应用键/保留键冲突则不可绑定。
        Some(ShortcutConflict::Reserved(_) | ShortcutConflict::AppShortcut(_)) => false,
        // 与后面的组重复由前面的组优先绑定；前面的重复已在上面检查过。
        Some(ShortcutConflict::TableGroup(_)) => true,
    }
}

/// ⌥1–⌥9：按编号切到当前工作区第 N 个可见标签页。无对应标签时无操作。
pub(crate) fn bind_tab_slot_shortcuts(cx: &mut App) {
    let bindings = (1..=TAB_SLOT_COUNT)
        .map(|slot| KeyBinding::new(format!("alt-{slot}").as_str(), ActivateTabSlot { slot }, None))
        .collect::<Vec<_>>();
    cx.bind_keys(bindings);
}

/// 注册可用的表快捷组；冲突/非法的组跳过，配置保留。
pub(crate) fn bind_table_shortcut_groups(cx: &mut App, settings: &Settings) {
    for index in 0..settings.table_shortcut_groups.len() {
        if !table_shortcut_group_bindable(settings, index) {
            continue;
        }
        cx.bind_keys([KeyBinding::new(
            &settings.table_shortcut_groups[index].keys,
            OpenTableShortcut { slot: index },
            None,
        )]);
    }
}

/// 设置保存后重绑表快捷组。
///
/// 先遮蔽全部旧组键位，避免删除/重排后旧槽位的动作继续生效；再按新顺序注册可用组。
pub(crate) fn rebind_table_shortcut_groups(
    cx: &mut App,
    before: &Settings,
    after: &Settings,
) {
    for (index, old) in before.table_shortcut_groups.iter().enumerate() {
        // 只遮蔽“当时确实注册过”的键位；冲突/非法组从未占用按键，
        // 若一并遮蔽会把同键位的应用快捷键压成 NoAction。
        if table_shortcut_group_bindable(before, index) {
            shadow_shortcut(cx, &old.keys, None);
        }
    }
    bind_table_shortcut_groups(cx, after);
}

