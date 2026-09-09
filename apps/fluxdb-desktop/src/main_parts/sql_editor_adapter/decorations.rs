// sql_editor_adapter/decorations.rs —— SQL 装饰（可选的语句级装饰）。
//
// fluxdb-editor-core 的 `DecorationProvider` 由 `SqlAdapter` 实现（见 mod.rs），
// 最小实现返回空装饰集。本文件提供纯逻辑辅助函数，用于后续基于语句范围
// 生成行首「执行」按钮等装饰，保持模块职责清晰且可单测。

/// 根据语句区间生成行首执行按钮装饰（每个非空语句首行一个）。
///
/// 目前仅用于预留/辅助；`SqlAdapter::decorations` 默认为空集。
/// `buffer_row` 需要调用方由 offset 换算。若需启用可在此展开。
#[allow(dead_code)]
pub fn statement_action_decorations(
    statement_starts: &[usize],
    _offset_to_row: impl Fn(usize) -> usize,
) -> DecorationSet {
    let mut decorations = Vec::new();
    for &start in statement_starts {
        let row = _offset_to_row(start);
        decorations.push(Decoration::StatementAction(
            row,
            "▶".to_string(),
            "sql.execute".to_string(),
        ));
    }
    DecorationSet { decorations }
}

#[cfg(test)]
mod decoration_tests {
    use super::*;

    #[test]
    fn statement_action_decorations_builds() {
        let set = statement_action_decorations(&[0, 10, 20], |o| o / 5);
        assert_eq!(set.decorations.len(), 3);
        match &set.decorations[0] {
            Decoration::StatementAction(row, label, action) => {
                assert_eq!(*row, 0);
                assert_eq!(label, "▶");
                assert_eq!(action, "sql.execute");
            }
            _ => panic!("expected StatementAction"),
        }
    }
}
