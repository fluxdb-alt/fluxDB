fn platform_titlebar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some(tr("app.window_title")),
        appears_transparent: true,
        #[cfg(target_os = "macos")]
        traffic_light_position: Some(point(px(17.), px(12.))),
        #[cfg(not(target_os = "macos"))]
        traffic_light_position: None,
    }
}
fn set_app_menus(cx: &mut App) {
    cx.set_menus(vec![Menu {
        name: tr("app.name"),
        disabled: true,
        items: vec![],
    }]);
}

/// List 删除位置：布尔值稳定表示方向，显示文案随语言变化。
#[derive(Clone)]
struct RedisListRemovePositionOption {
    head: bool,
}

impl SelectItem for RedisListRemovePositionOption {
    type Value = bool;

    fn title(&self) -> SharedString {
        if self.head {
            tr("app.redis.list.remove_from_head")
        } else {
            tr("app.redis.list.remove_from_tail")
        }
    }

    fn value(&self) -> &bool {
        &self.head
    }
}

/// 删除位置选项列表；下标 0 对应从尾删除（默认）。
fn redis_list_remove_position_options() -> Vec<RedisListRemovePositionOption> {
    [
        RedisListRemovePositionOption { head: false },
        RedisListRemovePositionOption { head: true },
    ]
    .into_iter()
    .collect()
}

fn query_history_record_to_entry(record: QueryHistoryRecord) -> QueryHistoryEntry {
    let kind = query_history_kind_from_operation(record.operation_kind);
    let success = record.success;
    let text = record.text;
    QueryHistoryEntry {
        session_id: None,
        connection_id: record.connection_id,
        database: record.database,
        schema: record.schema,
        text: text.clone(),
        tables: record.tables,
        kind,
        success,
        summary: QueryExecutionSummary {
            sql: text.clone(),
            kind: query_statement_kind_from_history(kind),
            success,
            message: record
                .message
                .unwrap_or_else(|| {
                    if success {
                        "OK".to_string()
                    } else {
                        tr("app.redis.execution_failed").to_string()
                    }
                }),
            returned_rows: record.returned_rows,
            affected_rows: record.affected_rows,
            elapsed_ms: record.elapsed_ms,
        },
        executed_at_unix_secs: record.executed_at_unix_secs,
        object: record.object,
        rollback_snapshot: record.rollback_snapshot,
        transaction_state: QueryHistoryTransactionState::from_storage(
            record.transaction_state.as_deref(),
        ),
    }
}

fn query_history_entry_to_record(entry: &QueryHistoryEntry) -> QueryHistoryRecord {
    QueryHistoryRecord {
        connection_id: entry.connection_id,
        database: entry.database.clone(),
        schema: entry.schema.clone(),
        text: entry.text.clone(),
        tables: entry.tables.clone(),
        operation_kind: query_history_kind_to_operation(entry.kind),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        object: entry.object.clone(),
        rollback_sql: None,
        rollback_snapshot: entry.rollback_snapshot.clone(),
        transaction_state: Some(entry.transaction_state.as_str().to_string()),
        message: Some(entry.summary.message.clone()),
        returned_rows: entry.summary.returned_rows,
        affected_rows: entry.summary.affected_rows,
        elapsed_ms: entry.summary.elapsed_ms,
    }
}

/// 把 App 层 Redis 历史记录转成持久化记录（source 序列化为枚举名）。
fn redis_workbench_entry_to_record(
    entry: &fluxdb_app::RedisWorkbenchHistoryEntry,
) -> RedisWorkbenchHistoryRecord {
    RedisWorkbenchHistoryRecord {
        id: entry.id,
        connection_id: entry.connection_id,
        database: entry.database,
        text: entry.text.clone(),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        summary: entry.summary.clone(),
        source: format!("{:?}", entry.source),
    }
}

/// 把 source 字符串还原成 `CommandExecutionSource`（枚举名匹配，未知回退 KeyShortcut）。
fn redis_workbench_source_from_str(source: &str) -> fluxdb_core::CommandExecutionSource {
    match source {
        "Workbench" => fluxdb_core::CommandExecutionSource::Workbench,
        "HistoryRerun" => fluxdb_core::CommandExecutionSource::HistoryRerun,
        _ => fluxdb_core::CommandExecutionSource::KeyShortcut,
    }
}

fn query_statement_kind_from_history(kind: QueryHistoryKind) -> fluxdb_core::QueryStatementKind {
    match kind {
        QueryHistoryKind::Query => fluxdb_core::QueryStatementKind::ResultSet,
        QueryHistoryKind::DataChange | QueryHistoryKind::SchemaChange => {
            fluxdb_core::QueryStatementKind::Command
        }
    }
}

/// App 层历史类别 → 规范化操作类别（跨数据库类型的通用动作词）。
fn query_history_kind_to_operation(kind: QueryHistoryKind) -> fluxdb_core::OperationKind {
    match kind {
        QueryHistoryKind::Query => fluxdb_core::OperationKind::Query,
        QueryHistoryKind::DataChange => fluxdb_core::OperationKind::Write,
        QueryHistoryKind::SchemaChange => fluxdb_core::OperationKind::Schema,
    }
}

/// 规范化操作类别 → App 层历史类别；只读/管理/未归类都按普通查询展示。
fn query_history_kind_from_operation(kind: fluxdb_core::OperationKind) -> QueryHistoryKind {
    match kind {
        fluxdb_core::OperationKind::Write => QueryHistoryKind::DataChange,
        fluxdb_core::OperationKind::Schema => QueryHistoryKind::SchemaChange,
        _ => QueryHistoryKind::Query,
    }
}

/// 资源目录按安装布局解析（AI-01，方案 §4.1）：
/// - macOS：bundle `Contents/Resources/assets`（保持不变）
/// - Windows：EXE 同级 `assets`
/// - Linux（DEB）：安装前缀下 `share/fluxdb/assets`（EXE 位于 `<prefix>/bin`）
/// 仅开发构建（debug_assertions）回退源码目录 `CARGO_MANIFEST_DIR/assets`；
/// 发布构建找不到资源时记录错误日志并返回平台布局路径，便于诊断，
/// 不再静默依赖构建机目录。
fn app_assets_base_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    let platform_assets = std::env::current_exe().ok().and_then(|exe_path| {
        exe_path
            .parent()
            .and_then(|macos_dir| macos_dir.parent())
            .map(|contents_dir| contents_dir.join("Resources").join("assets"))
    });
    #[cfg(target_os = "windows")]
    let platform_assets = std::env::current_exe()
        .ok()
        .and_then(|exe_path| exe_path.parent().map(|exe_dir| exe_dir.join("assets")));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let platform_assets = std::env::current_exe().ok().and_then(|exe_path| {
        exe_path
            .parent()
            .and_then(|bin_dir| bin_dir.parent())
            .map(|prefix| prefix.join("share").join("fluxdb").join("assets"))
    });

    if let Some(assets) = platform_assets {
        if assets.exists() {
            return assets;
        }
    }

    if cfg!(not(debug_assertions)) {
        tracing::warn!(
            target: "fluxdb_desktop",
            "发布构建未找到安装布局资源目录，图标等资源可能缺失（当前按开发目录回退）"
        );
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

/// 统一的应用退出入口。
///
/// 把退出请求转给主窗口做未保存检查，由主窗口决定直接退出还是弹确认框；
/// 拿不到主窗口（例如窗口已销毁）时无法确认，直接退出。
fn request_app_quit(cx: &mut App) {
    // macOS 关闭窗口只是隐藏到托盘，此时 active_window 可能为空，回退到窗口栈里的首个窗口。
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        cx.quit();
        return;
    };
    let handled = window.update(cx, |root, _window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return false;
        };
        main.update(cx, |main, cx| main.request_quit(cx));
        true
    });
    if !matches!(handled, Ok(true)) {
        cx.quit();
    }
}

/// 窗口根视图是 `Root`（承载弹框/通知等插件），业务视图 `NavicatMain` 挂在它的 `view()` 上。
fn navicat_main_from_root(root: &Root) -> Option<Entity<NavicatMain>> {
    root.view().clone().downcast::<NavicatMain>().ok()
}

/// 应用级「快速打开表」入口：不依赖当前焦点在 `NavicatMain` 上。
///
/// 刚启动或点击侧边栏后窗口可能没有焦点，此时视图内的 `on_action` 收不到快捷键；
/// 与退出快捷键一样走应用级监听，再定位主窗口视图执行。
fn open_quick_open_table_from_root(cx: &mut App) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| main.open_quick_open_table(window, cx));
    });
}

/// 应用级「统一命令面板」入口：与快速打开表相同，保证窗口没有焦点时也能打开。
fn open_command_palette_from_root(cx: &mut App) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| main.open_command_palette(window, cx));
    });
}

fn next_tab_from_root(cx: &mut App) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| main.next_tab_action(&NextTab, window, cx));
    });
}

fn previous_tab_from_root(cx: &mut App) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| main.previous_tab_action(&PreviousTab, window, cx));
    });
}

fn open_tab_switcher_from_root(cx: &mut App) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| {
            main.open_tab_switcher_action(&OpenTabSwitcher, window, cx)
        });
    });
}

/// ⌥1–⌥9 应用级入口：按编号激活当前工作区第 N 个标签。
fn activate_tab_slot_from_root(cx: &mut App, slot: usize) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| {
            main.activate_tab_slot_action(&ActivateTabSlot { slot }, window, cx)
        });
    });
}

/// 表快捷组应用级入口：按设置中的组下标触发表快捷组。
fn open_table_shortcut_from_root(cx: &mut App, slot: usize) {
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
        .and_then(|handle| handle.downcast::<Root>())
    else {
        return;
    };
    let _ = window.update(cx, |root, window, cx| {
        let Some(main) = navicat_main_from_root(root) else {
            return;
        };
        main.update(cx, |main, cx| {
            main.open_table_shortcut_action(&OpenTableShortcut { slot }, window, cx)
        });
    });
}

/// Windows/Linux：系统关窗（如 Alt+F4）时先做未保存检查。
///
/// 有未保存内容则阻止本次关闭并弹确认框，由确认按钮执行真正退出。
#[cfg(not(target_os = "macos"))]
fn install_close_quit_confirm(window: &Window, cx: &mut App) {
    window.on_window_should_close(cx, |window, cx| {
        let Some(root) = window.root::<Root>().flatten() else {
            return true;
        };
        let Some(main) = navicat_main_from_root(&root.read(cx)) else {
            return true;
        };
        if !main.read(cx).has_unsaved_work() {
            return true;
        }
        main.update(cx, |main, cx| {
            main.pending_exit_confirm = true;
            cx.notify();
        });
        false
    });
}

fn register_shortcuts(cx: &mut App, settings: &Settings) {
    // 新的通用编辑器与 SQL 连接层：注册通用编辑器快捷键 + SQL 执行快捷键。
    editor_component::register_editor_shortcuts(cx);
    sql_editor_adapter::register_sql_execute_shortcuts(cx);
    // 旧 SQL 编辑器快捷键已随旧 sql_editor 模块移除，不再注册，避免与新的
    // EditorComponent 快捷键绑定冲突。
    register_terminal_shortcuts(cx);

    // 应用级退出快捷键绑定在无上下文层级，确保编辑器或表格聚焦时也能退出。
    cx.on_action(|_: &Quit, cx| {
        // 快捷键在窗口事件处理期间触发，此刻该窗口正被借用；直接更新同一窗口会
        // 返回 "window not found"。延后到本轮更新结束再检查并弹确认框。
        cx.defer(request_app_quit);
    });
    // 快速打开表必须在窗口无焦点时也能触发（例如刚启动或点击侧边栏后），
    // 因此同样走应用级监听；停掉传播避免视图内的同名处理再执行一次。
    cx.on_action(|_: &QuickOpenTable, cx| {
        cx.stop_propagation();
        cx.defer(open_quick_open_table_from_root);
    });
    cx.on_action(|_: &CommandPalette, cx| {
        cx.stop_propagation();
        cx.defer(open_command_palette_from_root);
    });
    cx.on_action(|_: &NextTab, cx| {
        cx.stop_propagation();
        cx.defer(next_tab_from_root);
    });
    cx.on_action(|_: &PreviousTab, cx| {
        cx.stop_propagation();
        cx.defer(previous_tab_from_root);
    });
    cx.on_action(|_: &OpenTabSwitcher, cx| {
        cx.stop_propagation();
        cx.defer(open_tab_switcher_from_root);
    });
    cx.on_action(|action: &ActivateTabSlot, cx| {
        cx.stop_propagation();
        let slot = action.slot;
        cx.defer(move |cx| activate_tab_slot_from_root(cx, slot));
    });
    cx.on_action(|action: &OpenTableShortcut, cx| {
        cx.stop_propagation();
        let slot = action.slot;
        cx.defer(move |cx| open_table_shortcut_from_root(cx, slot));
    });
    cx.bind_keys([KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-q"
        } else {
            "ctrl-q"
        },
        Quit,
        None,
    )]);

    for definition in SHORTCUT_DEFINITIONS {
        bind_shortcut(
            cx,
            &current_shortcut(settings, definition),
            definition.action,
            definition.context,
        );
    }
    // 输入框专用绑定晚于通用快捷键注册，避免同一键位被 SQL 执行抢占。
    cx.bind_keys([
        KeyBinding::new(
            AI_MESSAGE_SEND_SHORTCUT,
            SendAiMessage,
            Some("AiChatComposer > Input"),
        ),
        KeyBinding::new(
            "up",
            QueryHistoryQuickSearchPrevious,
            Some("QueryHistoryQuickSearch"),
        ),
        KeyBinding::new(
            "down",
            QueryHistoryQuickSearchNext,
            Some("QueryHistoryQuickSearch"),
        ),
        KeyBinding::new(
            "enter",
            QueryHistoryQuickSearchConfirm,
            Some("QueryHistoryQuickSearch"),
        ),
        KeyBinding::new("up", QuickOpenTablePrevious, Some("QuickOpenTable")),
        KeyBinding::new("down", QuickOpenTableNext, Some("QuickOpenTable")),
        KeyBinding::new("enter", QuickOpenTableConfirm, Some("QuickOpenTable")),
        KeyBinding::new("up", CommandPalettePrevious, Some("CommandPalette")),
        KeyBinding::new("down", CommandPaletteNext, Some("CommandPalette")),
        KeyBinding::new("enter", CommandPaletteConfirm, Some("CommandPalette")),
        KeyBinding::new("tab", CommandPaletteNextScope, Some("CommandPalette")),
        KeyBinding::new(
            "shift-tab",
            CommandPalettePreviousScope,
            Some("CommandPalette"),
        ),
        KeyBinding::new("escape", CommandPaletteCancel, Some("CommandPalette")),
        KeyBinding::new("up", TabSwitcherPrevious, Some("TabSwitcher")),
        KeyBinding::new("down", TabSwitcherNext, Some("TabSwitcher")),
        KeyBinding::new("enter", TabSwitcherConfirm, Some("TabSwitcher")),
        KeyBinding::new("escape", TabSwitcherCancel, Some("TabSwitcher")),
        KeyBinding::new("escape", CancelDialog, None),
        KeyBinding::new(
            "delete",
            DeleteConnectionShortcut,
            Some("ConnectionContextMenu"),
        ),
        KeyBinding::new(
            "backspace",
            DeleteConnectionShortcut,
            Some("ConnectionContextMenu"),
        ),
        KeyBinding::new(
            "delete",
            DeleteConnectionShortcut,
            Some("DeleteConnectionModal"),
        ),
        KeyBinding::new(
            "backspace",
            DeleteConnectionShortcut,
            Some("DeleteConnectionModal"),
        ),
        KeyBinding::new(
            "enter",
            DeleteConnectionShortcut,
            Some("DeleteConnectionModal"),
        ),
        KeyBinding::new("up", TableShortcutPickerPrevious, Some("TableShortcutPicker")),
        KeyBinding::new("down", TableShortcutPickerNext, Some("TableShortcutPicker")),
        KeyBinding::new(
            "enter",
            TableShortcutPickerConfirm,
            Some("TableShortcutPicker"),
        ),
        KeyBinding::new(
            "escape",
            TableShortcutPickerCancel,
            Some("TableShortcutPicker"),
        ),
    ]);

    // 编号直达与表快捷组：放在应用快捷键之后注册，冲突组在绑定前已过滤。
    bind_tab_slot_shortcuts(cx);
    bind_table_shortcut_groups(cx, settings);
    let digit_bindings = (1..=TABLE_SHORTCUT_TARGET_LIMIT)
        .map(|slot| {
            KeyBinding::new(
                slot.to_string().as_str(),
                SelectTableShortcut { slot },
                Some("TableShortcutPicker"),
            )
        })
        .collect::<Vec<_>>();
    cx.bind_keys(digit_bindings);
}
