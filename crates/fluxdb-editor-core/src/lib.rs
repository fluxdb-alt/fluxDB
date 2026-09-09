//! # fluxdb-editor-core
//!
//! 独立、与业务解耦的通用编辑器内核。
//!
//! 只负责文本编辑、选区、撤销、语法结果、可视行映射、布局和通用补全协议；
//! 不依赖 GPUI、`fluxdb-app`、connector、数据库驱动或任何业务类型。可以被 SQL
//! 编辑器、Redis Workbench、JSON 编辑器、代码编辑器等复用：adapter 实现具体的
//! `LanguageDefinition` / `CompletionProvider` / `ExecutionAdapter` 等协议即可。
//!
//! 异步接口使用标准 `Future`，运行与取消由 GPUI/app adapter 负责。

mod block_map;
mod buffer;
mod completion;
mod coordinates;
mod display_map;
mod edit;
mod fold;
mod fold_map;
mod inlay_map;
mod language;
mod layer;
mod line_index;
mod model;
mod perf;
mod snippet;
mod sum_tree;
mod syntax;
mod tab_map;
mod task;
mod wrap_map;

pub use block_map::{Block, BlockId, BlockSnapshot};
pub use buffer::{BufferSnapshot, EditorBuffer, SnapshotChunkIter};
pub use completion::{
    CallSignature, CodeLens, CodeLensProvider, CompletionContinuation, CompletionController,
    CompletionError, CompletionFuture, CompletionProvider, CompletionSession, Decoration,
    DecorationProvider, DecorationSet, DiagnosticProvider, DocumentationRequest,
    DocumentationState, ExecutionAdapter, ExecutionUnit, HoverContent, HoverProvider, InlineHint,
    InlineHintProvider, SignatureInfo, SignatureProvider, TriggerDecision, active_parameter_index,
    completion_prefix, signature_at, sorted_range,
};
pub use coordinates::{
    Biased, BlockPoint, BufferOffset, BufferPoint, DisplayPoint, FoldPoint, InlayPoint, TabPoint,
    WrapPoint,
};
pub use display_map::{DisplayEdit, DisplayMap, DisplayPatch, Fold, SoftWrap, VisualLine};
pub use edit::{DirtyRange, EditTransaction, FullRebuild, FullRebuildReason};
pub use fold::{FoldEntry, FoldId, FoldSet};
pub use fold_map::FoldSnapshot;
pub use inlay_map::{Inlay, InlayId, InlaySnapshot, inlay_text_width, inline_hints_to_snapshot};
pub use language::{
    LanguageRegistration, LanguageRegistry, SyntaxInjection, SyntaxLayer, SyntaxLayerTree,
};
pub use layer::{LayerEdit, LayerPatch, LayerSnapshot};
pub use model::{
    Anchor, AnchorRange, Bias, CompletionItem, CompletionKind, CompletionRequest, CompletionResult,
    CompletionTrigger, Diagnostic, DiagnosticSeverity, Edit, EditorConfig, EditorEvent,
    EditorProfile, ExecuteMode, InsertTextFormat, Offset, Point, Range, Selection,
    SelectionSnapshot, SoftWrapMode, SubmitMode, TextChange,
};
pub use perf::{BACKGROUND_BUDGET_US, FRAME_BUDGET_US, threshold_us};
pub use snippet::{Snippet, SnippetError, parse_snippet};
pub use sum_tree::TextSummary;
pub use syntax::{
    AsyncWork, Highlight, HighlightStore, IndentRequest, InputEdit, LanguageDefinition,
    SyntaxProvider, SyntaxResult, SyntaxSnapshot, build_range_line_index,
};
pub use tab_map::{TabLine, TabSnapshot};
pub use task::{CancellationToken, TaskKey, TaskKind, TaskOutcome, TaskPhase};
pub use wrap_map::{
    LineBreaker, RowWrap, WrapCacheKey, WrapConfig, WrapLine, WrapMap, WrapSnapshot,
};

#[cfg(test)]
mod tests;
