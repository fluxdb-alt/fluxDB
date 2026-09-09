//! 统一任务协议类型（DM-600）。
//!
//! 六个编辑器任务（syntax/diagnostics/hover/completion/signature/cursor_blink
//! 等）此前各自用 `Arc<AtomicU64>` 令牌 + 版本守卫管理生命周期。本模块把
//! 「任务分类、取消令牌、任务身份、结果结局」固化为零依赖协议类型，供核心
//! 与 UI/adapter 共享：取消与过期判定语义统一，划分日志字段统一。
//!
//! 升级路径：若未来需要把六个独立 `Task<()>` 字段合并为单一有界调度器，本
//! 模块的 [`TaskKey`] / [`CancellationToken`] 即为其通用协议；当前阶段不合并
//! （YAGNI，ponytail）。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 任务类型标签。用于日志字段、协议路由与 in-flight 有界性断言。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskKind {
    Syntax,
    Diagnostics,
    Hover,
    Completion,
    Signature,
    Inlay,
    Codelens,
    Wrap,
}

impl TaskKind {
    /// 稳定的字符串标签，供结构化日志（tracing）与断言共享。
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::Syntax => "syntax",
            TaskKind::Diagnostics => "diagnostics",
            TaskKind::Hover => "hover",
            TaskKind::Completion => "completion",
            TaskKind::Signature => "signature",
            TaskKind::Inlay => "inlay",
            TaskKind::Codelens => "codelens",
            TaskKind::Wrap => "wrap",
        }
    }
}

/// 一个任务的全局身份：host（编辑器实例）+ 每次重派发递增的 task_id + 分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskKey {
    /// 发起任务所在编辑器实例 id（0 表示非编辑器宿主/单例）。
    pub editor_id: u64,
    /// 宿主内递增的任务 id（重派发时新发一个）。
    pub task_id: u64,
    /// 任务分类。
    pub kind: TaskKind,
}

impl TaskKey {
    pub fn new(editor_id: u64, task_id: u64, kind: TaskKind) -> Self {
        Self {
            editor_id,
            task_id,
            kind,
        }
    }
}

/// 可共享的取消令牌。每次重派发调用 [`CancellationToken::cancel`] 令旧的
/// in-flight 请求失效；长任务以捕获的 `request_id` 在检查点调用
/// [`CancellationToken::check`]（等同 `latest_request.load()!=request_id`）
/// 提前 return。
///
/// 语义与既有 `Arc<AtomicU64>` 令牌完全一致，仅类型化。
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicU64>);

impl CancellationToken {
    /// 发一个新请求，返回本次请求 id。
    pub fn request_id(&self) -> u64 {
        self.0.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// 令此前所有请求失效（供重派发前取消旧任务）。
    pub fn cancel(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    /// 检查一个此前捕获的 `request_id` 是否仍是最新（未过期）。
    pub fn check(&self, request_id: u64) -> bool {
        self.0.load(Ordering::Acquire) == request_id
    }

    /// 当前最新请求 id。
    pub fn latest(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    /// 借出底层原子，兼容仍以 `&AtomicU64` 透传的旧式长任务检查点。
    /// `ponytail`: 低层 cancellable helper 仍是原始原子；协议类型只收敛到
    /// adapter 字段与边界。若未来统一 helper 签名可移除本方法。
    pub fn inner(&self) -> &AtomicU64 {
        self.0.as_ref()
    }
}

/// 任务结束结局，用于划分日志与断言统一归类（DM-605）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskOutcome {
    /// 新请求到来，旧任务被显式取消（token bump）。
    CancelRequested,
    /// 长任务在检查点自行发现过期而停止。
    WorkStopped,
    /// 任务完成但结果因版本/token 已过期被丢弃。
    ResultDiscarded,
    /// 结果通过版本/token 校验并提交。
    ResultCommitted,
}

impl TaskOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskOutcome::CancelRequested => "cancel_requested",
            TaskOutcome::WorkStopped => "work_stopped",
            TaskOutcome::ResultDiscarded => "result_discarded",
            TaskOutcome::ResultCommitted => "result_committed",
        }
    }
}

/// 任务阶段标签，供日志区分（可选）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskPhase {
    Debounce,
    Executing,
    Committing,
}

impl TaskPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskPhase::Debounce => "debounce",
            TaskPhase::Executing => "executing",
            TaskPhase::Committing => "committing",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DM-600：request_id 单调递增，cancel 令旧请求失效。
    #[test]
    fn token_issue_and_cancel_invalidate_old_request() {
        let token = CancellationToken::default();
        let first = token.request_id();
        assert!(token.check(first));
        token.cancel();
        // cancel 后旧 request_id 不再是最新 → check 为假。
        assert!(!token.check(first));
        let second = token.request_id();
        // cancel 已 +1，故 second 比 first 大 2。
        assert_eq!(second, first + 2);
        assert!(token.check(second));
    }

    /// DM-600：TaskKey 身份由三元组共同决定。
    #[test]
    fn task_key_equality_and_kind_tag() {
        let a = TaskKey::new(1, 10, TaskKind::Syntax);
        let b = TaskKey::new(1, 10, TaskKind::Syntax);
        let c = TaskKey::new(1, 11, TaskKind::Syntax);
        let d = TaskKey::new(1, 10, TaskKind::Hover);
        assert_eq!(a, b);
        assert_ne!(a, c, "task_id 不同");
        assert_ne!(a, d, "kind 不同");
        assert_eq!(TaskKind::Syntax.as_str(), "syntax");
    }

    /// DM-605：结局标签稳定，供划分日志关键字。
    #[test]
    fn outcome_tags_are_stable() {
        assert_eq!(TaskOutcome::WorkStopped.as_str(), "work_stopped");
        assert_eq!(TaskOutcome::ResultDiscarded.as_str(), "result_discarded");
        assert_eq!(TaskOutcome::ResultCommitted.as_str(), "result_committed");
        assert_eq!(TaskOutcome::CancelRequested.as_str(), "cancel_requested");
    }
}
