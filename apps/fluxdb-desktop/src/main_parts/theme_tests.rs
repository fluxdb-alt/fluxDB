// 主题配色的成对约束：组件按 `tokens.accent` 画悬停底色、按 `accent_foreground` 画悬停
// 文字。两者只改一半就会同色——ghost 按钮（MongoDB 查询面板的「添加条件」「条件构建」）
// 悬停时文字直接消失。
//
// 只读 `gpui_component::init` 装好的浅色主题，不走 `Theme::change`：切换主题会触发字体
// 解析与全窗口刷新，在无头用例里把测试进程卡在平台层。

mod theme_tests {
    use gpui::TestAppContext;
    use gpui_component::Theme as ComponentTheme;

    use crate::{apply_component_theme_colors, extend_component_i18n};

    /// 粗粒度亮度：sRGB 分量加权，只用来判断「亮底配暗字」还是反过来。
    fn luminance(color: gpui::Hsla) -> f32 {
        let rgba: gpui::Rgba = color.into();
        0.2126 * rgba.r + 0.7152 * rgba.g + 0.0722 * rgba.b
    }

    #[gpui::test]
    fn ghost_hover_pairs_accent_with_readable_foreground(cx: &mut TestAppContext) {
        let contrast = cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
            apply_component_theme_colors(gpui_component::ThemeMode::Light, cx);

            let theme = ComponentTheme::global(cx);
            (luminance(theme.tokens.accent.color) - luminance(theme.accent_foreground)).abs()
        });

        assert!(
            contrast > 0.4,
            "ghost 悬停文字与底色太接近（Δ={contrast:.2}），悬停会看不见"
        );
    }
}
