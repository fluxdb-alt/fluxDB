// Included in crate-root scope by ../main.rs; 只读 SQL / DDL 预览编辑器工厂。
//
// 「设计表」的 SQL 预览 / DDL 预览，以及「表属性」抽屉的 DDL 页签，三处预览共用本工厂，
// 宿主只负责外层布局与工具条。
//
// 为什么不用 gpui-component 的 `input::Editor`：它的高亮取组件自带的 `HighlightTheme`，
// 拿不到本项目主题名（ThemeRegistry）派生的 syntax 配色，预览会退化成无高亮的纯文本。
// 底层 `editor_component::Editor` 的配色由宿主按主题名注入（`editor_theme_for`），
// 与查询 / Redis 编辑器同源。

/// 预览字号（pt）。
const SQL_PREVIEW_FONT_SIZE: f32 = editor_component::EDITOR_TEXT_SIZE;
/// 预览行高（px）：1.5 倍字号，与原 gpui-component 预览观感一致。
const SQL_PREVIEW_LINE_HEIGHT: f32 = SQL_PREVIEW_FONT_SIZE * 1.5;

/// 创建（或按 key 复用）只读高亮预览编辑器。
///
/// - `editor_key` 决定缓存粒度：同 key 跨帧复用同一编辑器，key 变化（如切换标签页）才重建。
/// - `dialect` 决定语法 provider 的方言；高亮查询各方言一致，折叠 / 注释标记随方言。
/// - `soft_wrap` 可在运行期通过 `apply_settings` 切换，不重建编辑器、不丢滚动位置。
///
/// 只在每次渲染时同步外部文本与配色，不做编辑回写：`read_only` 只拒绝文本修改，
/// 选择 / 复制 / 滚动仍然可用。
///
/// 宿主注意：返回的编辑器根元素是 `size_full()`、内部滚动容器全部绝对定位，
/// **必须作为高度确定的容器的直接子元素**（例如 flex 列里 `flex_1` 的子项）。
/// 高度塌成 auto 时编辑器会缩成 0 高，表现就是预览一片空白 —— 不要再用 `div()`
/// 之类的默认 `Display::Block` 容器在中间包一层（`flex_1` 在块布局里不生效）。
fn sql_preview_editor(
    editor_key: impl Into<gpui::ElementId>,
    text: &str,
    dialect: sql_editor_adapter::SqlDialect,
    soft_wrap: bool,
    editor_theme: editor_component::EditorTheme,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Entity<editor_component::Editor> {
    let editor = window.use_keyed_state(editor_key, cx, {
        let text = text.to_string();
        move |window, cx| {
            // 只装配语法 provider：预览是只读展示，不注入补全 / 诊断 / 执行，
            // 避免为了渲染拉起无意义的异步任务。
            let adapter = std::sync::Arc::new(sql_editor_adapter::SqlAdapter::new(dialect));
            let registry = std::sync::Arc::new(fluxdb_editor_core::LanguageRegistry::new());
            registry.register(
                dialect.language_id(),
                std::sync::Arc::new(sql_editor_adapter::SqlLanguage::new(dialect)),
                Some(adapter.clone() as _),
            );
            let providers = editor_component::Providers {
                language_registry: Some(registry),
                syntax: Some(adapter as _),
                ..Default::default()
            };
            let config = fluxdb_editor_core::EditorConfig {
                profile: fluxdb_editor_core::EditorProfile {
                    language_id: dialect.language_id().to_string(),
                    // 只读：拒绝文本修改，但保留选择 / 复制与滚动。
                    read_only: true,
                    show_line_numbers: false,
                    show_folding: false,
                    // 预览的可滚范围恰好到内容末尾：短 DDL / SQL 不该还能滚出一屏空白。
                    scroll_beyond_last_line: fluxdb_editor_core::ScrollBeyondLastLine::None,
                    soft_wrap: sql_preview_soft_wrap(soft_wrap),
                    ..Default::default()
                },
                font: editor_component::EDITOR_FONT.to_string(),
                font_size: SQL_PREVIEW_FONT_SIZE,
                line_height: SQL_PREVIEW_LINE_HEIGHT,
                gutter_line_numbers: false,
            };
            editor_component::Editor::new(text.clone(), providers, Some(config), window, cx)
        }
    });
    editor.update(cx, |editor, cx| {
        // 外部模型变化才同步（长度不同立即判定变化），不会每帧重建 buffer。
        editor.sync_text_silent(text, cx);
        // 预览不跟踪多 Tab 宽度 settings，保持构造时的默认制表宽（4）。
        editor.apply_settings(SQL_PREVIEW_FONT_SIZE, SQL_PREVIEW_LINE_HEIGHT, soft_wrap, 0);
        // 主题未变化时 `set_theme` 内部短路，逐帧调用无额外开销。
        editor.set_theme(editor_theme, cx);
    });
    editor
}

/// 换行开关 → 编辑器软换行模式。
fn sql_preview_soft_wrap(soft_wrap: bool) -> fluxdb_editor_core::SoftWrapMode {
    if soft_wrap {
        fluxdb_editor_core::SoftWrapMode::EditorWidth
    } else {
        fluxdb_editor_core::SoftWrapMode::None
    }
}
