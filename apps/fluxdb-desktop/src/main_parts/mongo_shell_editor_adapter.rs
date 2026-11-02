// mongo_shell_editor_adapter.rs —— MongoDB 命令工作台的编辑器补全接入层。
//
// 候选形状与过滤全部复用 fluxdb-app 的 `mongo_shell_completions`（与 Mongo CLI 同一份实现）；
// 这里只做两件事：把补全范围收窄到光标所在行，以及映射成 editor-core 的候选类型。
//
// 库名 / 集合名通过 `MongoCliSession` 共享：adapter 在每次补全时请求一次后台刷新，
// session 内部记账保证同一库不会每次按键都重新建连。
//
// 本文件的 `include!` 作用域与其它 main_parts 文件共享：`CompletionItem` /
// `CompletionProvider` / `Range` 等名字已被 lsp_types / gpui 占用，因此这里全部用
// 全限定路径，不做顶层 `use`，否则会改变那些文件的解析结果。

/// MongoDB 命令工作台的补全适配器。
pub(crate) struct MongoShellEditorAdapter {
    session: fluxdb_app::MongoCliSession,
    config: fluxdb_core::ConnectionConfig,
}

impl MongoShellEditorAdapter {
    pub(crate) fn new(config: fluxdb_core::ConnectionConfig, database: String) -> Self {
        Self {
            session: fluxdb_app::MongoCliSession::new(database),
            config,
        }
    }

    /// 同步工作台当前库：执行 `use <db>` 后集合候选要跟着换库。
    pub(crate) fn set_database(&self, database: &str) {
        if self.session.database() != database {
            self.session.set_database(database.to_string());
        }
    }
}

/// 光标所在行的补全文本与替换区间。
///
/// 返回 `(起点, 终点, 光标前文本)`：起点跳过行首缩进（候选整行替换时不吞缩进），
/// 终点取整行末尾——候选是整行文本，光标右侧的残留也一并换掉。
fn mongo_shell_completion_line(text: &str, cursor: usize) -> (usize, usize, &str) {
    let mut cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let line_start = text[..cursor].rfind('\n').map(|index| index + 1).unwrap_or(0);
    let line_end = text[cursor..]
        .find('\n')
        .map(|index| cursor + index)
        .unwrap_or(text.len());
    let indent = text[line_start..line_end].len()
        - text[line_start..line_end].trim_start().len();
    // 光标停在缩进里时没有可补的正文：起点夹到光标，返回空前缀。
    let content_start = (line_start + indent).min(cursor);
    (content_start, line_end, &text[content_start..cursor])
}

impl fluxdb_editor_core::CompletionProvider for MongoShellEditorAdapter {
    /// 与 CLI 一致：边打字边给候选（本地关键字 + 已缓存的库/集合名，不需要等请求）。
    fn should_trigger(
        &self,
        _request: &fluxdb_editor_core::CompletionRequest,
    ) -> fluxdb_editor_core::TriggerDecision {
        fluxdb_editor_core::TriggerDecision::Yes
    }

    fn complete(
        &self,
        request: fluxdb_editor_core::CompletionRequest,
    ) -> fluxdb_editor_core::CompletionFuture {
        // 列表刷新只影响后续候选：本次立刻用缓存返回，绝不阻塞输入。
        self.session.request_refresh(&self.config);
        let cursor = request.cursor;
        let (line, line_start, line_end) = request
            .document
            .as_ref()
            .map(|snapshot| {
                let text = snapshot.to_string();
                let (start, end, content) = mongo_shell_completion_line(&text, cursor);
                (content.to_string(), start, end)
            })
            .unwrap_or((String::new(), 0, 0));
        let databases = self.session.databases();
        let collections = self.session.collections();
        Box::pin(async move {
            // 语句按行执行，补全也只认光标所在行。
            let items = fluxdb_app::mongo_shell_completions(&line, &databases, &collections)
                .into_iter()
                .map(|candidate| {
                    let mut item = fluxdb_editor_core::CompletionItem::new(
                        candidate,
                        fluxdb_editor_core::CompletionKind::Command,
                    );
                    // 候选是整行文本，替换范围覆盖光标所在行（含光标右侧残留）。
                    item.replace_range =
                        Some(fluxdb_editor_core::Range::new(line_start, line_end));
                    item
                })
                .collect();
            Ok(fluxdb_editor_core::CompletionResult {
                items,
                has_more: false,
                continuation: None,
            })
        })
    }
}

#[cfg(test)]
mod mongo_shell_editor_adapter_tests {
    use super::mongo_shell_completion_line;

    #[test]
    fn completion_stays_on_the_cursor_line_and_skips_indent() {
        let text = "db.a.find({})\n  use fl\nshow dbs";
        let cursor = text.find("use fl").expect("second line") + "use fl".len();
        let (start, end, content) = mongo_shell_completion_line(text, cursor);
        assert_eq!(content, "use fl");
        assert_eq!(&text[start..end], "use fl", "缩进不进替换范围");
        assert_eq!(&text[start..cursor], "use fl", "缩进不进候选前缀");

        // 光标停在行中间：候选按光标前文本匹配，但替换到行尾，右侧残留一并清掉。
        let text = "db.evXX\n";
        let (start, end, content) = mongo_shell_completion_line(text, "db.ev".len());
        assert_eq!((start, end, content), (0, 7, "db.ev"));
    }
}
