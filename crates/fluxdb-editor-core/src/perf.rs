//! 编辑器性能采样阈值。阈值集中定义，避免各层散落 magic number。

use crate::TaskKind;

pub const FRAME_BUDGET_US: u64 = 16_000;
pub const BACKGROUND_BUDGET_US: u64 = 100_000;

pub fn threshold_us(kind: TaskKind) -> u64 {
    match kind {
        TaskKind::Syntax | TaskKind::Diagnostics | TaskKind::Hover | TaskKind::Completion => {
            BACKGROUND_BUDGET_US
        }
        TaskKind::Inlay | TaskKind::Codelens | TaskKind::Wrap | TaskKind::Signature => {
            FRAME_BUDGET_US
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_keep_frame_and_background_budgets_distinct() {
        assert_eq!(threshold_us(TaskKind::Wrap), FRAME_BUDGET_US);
        assert_eq!(threshold_us(TaskKind::Syntax), BACKGROUND_BUDGET_US);
        assert!(BACKGROUND_BUDGET_US > FRAME_BUDGET_US);
    }
}
