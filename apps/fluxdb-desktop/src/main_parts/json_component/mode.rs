// json_component/mode.rs —— JSON 组件展示模式。
//
// 组件通过一个模式参数决定是「只读预览」还是「可编辑」，调用方无需关心内部
// 是否创建 `InputState`：`Preview` 不创建，`Edit` 由组件内部创建并管理。

/// JSON 组件的展示模式。
///
/// - `Preview`：只读展示（pretty / 语法高亮 / 折叠 / 行号），不创建 `InputState`。
/// - `Edit`：可编辑全文，组件内部维护 `InputState`（不暴露给调用方）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsonMode {
    Preview,
    Edit,
}

impl JsonMode {
    /// 该模式是否可编辑。
    pub(crate) fn is_editable(self) -> bool {
        matches!(self, JsonMode::Edit)
    }
}

#[cfg(test)]
mod json_component_mode_tests {
    use super::*;

    #[test]
    fn preview_is_readonly_edit_is_editable() {
        assert!(!JsonMode::Preview.is_editable());
        assert!(JsonMode::Edit.is_editable());
    }
}
