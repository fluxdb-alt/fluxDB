// sql_editor_adapter/execute_shortcuts.rs —— SQL 执行快捷键（适配层）。
//
// fluxdb-editor-core 的通用编辑器本身不感知 SQL 语义，因此 Run/Select/Explain 的
// 快捷键由 SQL 接入层在此注册：绑定到通用编辑器上下文（EditorComponent），
// 携带 `ExecuteMode`，由宿主（查询页面，见单元 D）在焦点编辑器上派发，
// 触发 `EditorEvent::Execute{mode, ..}`。注册无副作用，未命中宿主时为空操作。

use gpui::{Action, App, KeyBinding};

// 注：`ExecuteMode` 已在 mod.rs 顶部 `use fluxdb_editor_core::...` 导入，直接复用。

/// SQL 执行快捷键动作：携带执行模式（Run/Select/Explain）。
#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = sql_editor_adapter, no_json)]
pub struct ExecuteQueryShortcut {
    /// 期望的执行模式。
    pub mode: ExecuteMode,
}

/// 通用编辑器上下文，与 editor_component::CONTEXT 一致。
pub(crate) const EXECUTE_CONTEXT: &str = "EditorComponent";

/// 注册 SQL 执行快捷键（Run/Select/Explain）。
///
/// - Cmd+Enter(Run) 已由全局 `ExecuteOrApply` 提供，此处补充分模式快捷键：
/// - Run：Cmd+R（编辑器内覆盖全局 Refresh）
/// - Select：Cmd+Shift+R
/// - Explain：Cmd+Shift+E
pub(crate) fn register_sql_execute_shortcuts(cx: &mut App) {
    cx.bind_keys(vec![
        KeyBinding::new(
            "cmd-r",
            ExecuteQueryShortcut {
                mode: ExecuteMode::Execute,
            },
            Some(EXECUTE_CONTEXT),
        ),
        KeyBinding::new(
            "cmd-shift-r",
            ExecuteQueryShortcut {
                mode: ExecuteMode::Select,
            },
            Some(EXECUTE_CONTEXT),
        ),
        KeyBinding::new(
            "cmd-shift-e",
            ExecuteQueryShortcut {
                mode: ExecuteMode::Explain,
            },
            Some(EXECUTE_CONTEXT),
        ),
    ]);
}
