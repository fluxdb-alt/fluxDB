// SQL 编辑器接入层：把 fluxdb-editor-core 的通用编辑器接到 SQL 业务上下文。
// 该层通过 provider/adapter 注入 connection / database / schema 信息，
// 复用现有 sql_editor/ 与 fluxdb-app 的查询补全与语句拆分逻辑。
pub(crate) mod sql_editor_adapter {
    include!("sql_editor_adapter/mod.rs");
    include!("sql_editor_adapter/dialect.rs");
    include!("sql_editor_adapter/syntax.rs");
    include!("sql_editor_adapter/completion.rs");
    include!("sql_editor_adapter/semantic.rs");
    include!("sql_editor_adapter/decorations.rs");
    include!("sql_editor_adapter/execution.rs");
    include!("sql_editor_adapter/statements.rs");
    include!("sql_editor_adapter/execute_shortcuts.rs");
}
