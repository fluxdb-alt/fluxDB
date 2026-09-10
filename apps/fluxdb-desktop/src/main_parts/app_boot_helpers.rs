fn set_app_menus(cx: &mut App) {
    cx.set_menus(vec![Menu {
        name: "FluxDB".into(),
        disabled: true,
        items: vec![],
    }]);
}

/// List 删除元素 - 位置下拉选项（对齐 RedisInsight）。
/// 第一个「从尾部移除」为默认选中项（head=false → RPOP）。
const REDIS_LIST_REMOVE_FROM_TAIL: &str = "从尾部移除";
const REDIS_LIST_REMOVE_FROM_HEAD: &str = "从头部移除";

/// 删除位置选项列表；下标 0 对应从尾删除（默认）。
fn redis_list_remove_position_options() -> Vec<String> {
    [REDIS_LIST_REMOVE_FROM_TAIL, REDIS_LIST_REMOVE_FROM_HEAD]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn query_history_record_to_entry(record: QueryHistoryRecord) -> QueryHistoryEntry {
    let kind = query_history_kind_from_storage(&record.kind);
    let success = record.success;
    let text = record.text;
    QueryHistoryEntry {
        connection_id: record.connection_id,
        database: record.database,
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
                .unwrap_or_else(|| if success { "OK" } else { "执行失败" }.to_string()),
            returned_rows: record.returned_rows,
            affected_rows: record.affected_rows,
            elapsed_ms: record.elapsed_ms,
        },
        executed_at_unix_secs: record.executed_at_unix_secs,
        object: record.object,
        rollback_snapshot: record.rollback_snapshot,
    }
}

fn query_history_entry_to_record(entry: &QueryHistoryEntry) -> QueryHistoryRecord {
    QueryHistoryRecord {
        connection_id: entry.connection_id,
        database: entry.database.clone(),
        text: entry.text.clone(),
        tables: entry.tables.clone(),
        kind: query_history_kind_to_storage(entry.kind).to_string(),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        object: entry.object.clone(),
        rollback_sql: None,
        rollback_snapshot: entry.rollback_snapshot.clone(),
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

fn query_history_kind_from_storage(kind: &str) -> QueryHistoryKind {
    match kind {
        "data_change" => QueryHistoryKind::DataChange,
        "schema_change" => QueryHistoryKind::SchemaChange,
        _ => QueryHistoryKind::Query,
    }
}

fn query_history_kind_to_storage(kind: QueryHistoryKind) -> &'static str {
    match kind {
        QueryHistoryKind::Query => "query",
        QueryHistoryKind::DataChange => "data_change",
        QueryHistoryKind::SchemaChange => "schema_change",
    }
}

fn app_assets_base_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(contents_dir) = exe_path.parent().and_then(|macos_dir| macos_dir.parent()) {
                let bundled_assets = contents_dir.join("Resources").join("assets");
                if bundled_assets.exists() {
                    return bundled_assets;
                }
            }
        }
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
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
        tracing::info!(target: "fluxdb_desktop", "收到退出快捷键，退出应用");
        cx.quit();
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
    cx.bind_keys([
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
    ]);
}
