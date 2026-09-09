//! 可复用编辑器的语言适配器边界。
//!
//! 该 crate 只组合通用 core 协议，不包含 SQL、GPUI、connector 或数据库类型。
//! 语言实现可以在桌面应用、独立语言服务或其他宿主中复用。

use fluxdb_editor_core::{CompletionProvider, LanguageDefinition, SyntaxProvider};

/// 一套可注入编辑器的语言能力。
pub trait LanguageAdapter: LanguageDefinition + SyntaxProvider + CompletionProvider {}

impl<T> LanguageAdapter for T where T: LanguageDefinition + SyntaxProvider + CompletionProvider {}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_adapter<T: LanguageAdapter>() {}

    #[test]
    fn marker_composes_core_language_traits() {
        assert_adapter::<TestAdapter>();
    }

    struct TestAdapter;

    impl LanguageDefinition for TestAdapter {
        fn language_id(&self) -> &str {
            "test"
        }
    }

    impl SyntaxProvider for TestAdapter {
        fn parse(
            &self,
            snapshot: &fluxdb_editor_core::BufferSnapshot,
            _changed: fluxdb_editor_core::InputEdit,
        ) -> fluxdb_editor_core::AsyncWork<fluxdb_editor_core::SyntaxResult> {
            let version = snapshot.version();
            Box::pin(async move {
                fluxdb_editor_core::SyntaxResult {
                    buffer_version: version,
                    highlights: Vec::new(),
                    dirty_ranges: Vec::new(),
                    needs_refinement: false,
                }
            })
        }
    }

    impl CompletionProvider for TestAdapter {
        fn should_trigger(
            &self,
            _request: &fluxdb_editor_core::CompletionRequest,
        ) -> fluxdb_editor_core::TriggerDecision {
            fluxdb_editor_core::TriggerDecision::No
        }

        fn complete(
            &self,
            _request: fluxdb_editor_core::CompletionRequest,
        ) -> fluxdb_editor_core::CompletionFuture {
            Box::pin(async { Ok(fluxdb_editor_core::CompletionResult::default()) })
        }
    }
}
