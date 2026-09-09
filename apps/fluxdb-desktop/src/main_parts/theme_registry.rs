fn watch_bundled_themes(settings: Settings, mode: ThemeMode, cx: &mut App) {
    let themes_dir = app_assets_base_path().join("themes");
    let _ = ThemeRegistry::watch_dir(themes_dir, cx, move |cx| {
        apply_registered_theme(&settings, mode, cx);
        apply_component_theme_colors(mode, cx);
        cx.refresh_windows();
    });
}

fn apply_registered_theme(settings: &Settings, mode: ThemeMode, cx: &mut App) {
    let registry = ThemeRegistry::global(cx);
    let light = registry
        .themes()
        .get(settings.light_theme.as_str())
        .filter(|theme| theme.mode == ThemeMode::Light)
        .cloned()
        .unwrap_or_else(|| registry.default_light_theme().clone());
    let dark = registry
        .themes()
        .get(settings.dark_theme.as_str())
        .filter(|theme| theme.mode == ThemeMode::Dark)
        .cloned()
        .unwrap_or_else(|| registry.default_dark_theme().clone());
    {
        let theme = ComponentTheme::global_mut(cx);
        theme.light_theme = light;
        theme.dark_theme = dark;
    }
    ComponentTheme::change(mode, None, cx);
    apply_button_radius(settings.button_radius, cx);
    apply_global_theme_settings(settings, cx);
}

fn theme_configs_for_mode(mode: ThemeMode, cx: &App) -> Vec<std::rc::Rc<ThemeConfig>> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|theme| theme.mode == mode)
        .cloned()
        .collect()
}

/// 由所选主题（ThemeConfig）的 highlight/colors 派生通用编辑器配色。
///
/// 以 mode 默认 `EditorTheme`（dark()/light()）为底，用主题的 `highlight`
/// （Zed 兼容的 `editor.*` / `syntax.*`，gpui-component 的 HighlightThemeStyle）
/// 与 `colors`（caret/selection/warning，hex 色）覆写有直接对应的字段；
/// 无对应或缺失者保持默认，避免主题缺字段时产生不可读配色。
///
/// 放在本模块（而非 editor_component 内核）是为遵守内核“不依赖主题系统”的分层：
/// 这是把外部主题映射进 `EditorTheme` 的宿主侧适配，非编辑器渲染逻辑。
pub(crate) fn editor_theme_from_highlight(
    fallback: editor_component::EditorTheme,
    cfg: &ThemeConfig,
) -> editor_component::EditorTheme {
    let hl = cfg.highlight.as_ref();

    let mut t = fallback;
    // highlight 的 editor.*/syntax.* 字段均为 Option<Hsla>；syntax 经 style(name)
    // 取 HighlightStyle（其 .color 为 Option<Hsla>），缺名/缺色回退。
    let syn = |name: &str| -> Option<gpui::Hsla> { hl?.syntax.style(name)?.color };
    let col = |v: &Option<SharedString>| -> Option<gpui::Rgba> {
        v.as_deref().and_then(parse_theme_hex_color)
    };

    t.background = hl.and_then(|h| h.editor_background).map(Into::into).unwrap_or(t.background);
    t.text = hl.and_then(|h| h.editor_foreground).map(Into::into).unwrap_or(t.text);
    t.active_line = hl.and_then(|h| h.editor_active_line).map(Into::into).unwrap_or(t.active_line);
    t.line_number = hl.and_then(|h| h.editor_line_number).map(Into::into).unwrap_or(t.line_number);
    t.cursor = col(&cfg.colors.caret).unwrap_or(t.cursor);
    t.selection = col(&cfg.colors.selection).unwrap_or(t.selection);
    t.warning = col(&cfg.colors.warning).unwrap_or(t.warning);

    t.syntax_keyword = syn("keyword").map(Into::into).unwrap_or(t.syntax_keyword);
    t.syntax_string = syn("string").map(Into::into).unwrap_or(t.syntax_string);
    t.syntax_number = syn("number").map(Into::into).unwrap_or(t.syntax_number);
    t.syntax_comment = syn("comment").map(Into::into).unwrap_or(t.syntax_comment);
    t.syntax_type = syn("type").map(Into::into).unwrap_or(t.syntax_type);
    t.syntax_boolean = syn("boolean").map(Into::into).unwrap_or(t.syntax_boolean);
    t.syntax_function = syn("function").map(Into::into).unwrap_or(t.syntax_function);
    t.syntax_attribute = syn("attribute").map(Into::into).unwrap_or(t.syntax_attribute);
    // 标识符/字段无 direct syntax 名：用 property 近义色，缺失回退默认。
    t.syntax_identifier = syn("property").or_else(|| syn("variable")).map(Into::into).unwrap_or(t.syntax_identifier);
    t.syntax_field = syn("property").map(Into::into).unwrap_or(t.syntax_field);
    t.syntax_variable = syn("variable").map(Into::into).unwrap_or(t.syntax_variable);
    t.syntax_parameter = syn("variable").or_else(|| syn("variable.special")).map(Into::into).unwrap_or(t.syntax_parameter);
    // error / status_* / completion_* 主题无可靠对应，保留默认。
    t
}

/// 依据当前明暗模式 + 用户所选主题名，从主题注册表解析编辑器配色。
///
/// 主题名找不到或 mode 不匹配时回退到对应 mode 的内置默认两套。
pub(crate) fn editor_theme_for(mode: ThemeMode, theme_name: &str, cx: &App) -> editor_component::EditorTheme {
    let fallback = if mode == ThemeMode::Dark {
        editor_component::EditorTheme::dark()
    } else {
        editor_component::EditorTheme::light()
    };
    let registry = ThemeRegistry::global(cx);
    let mut theme = match registry.themes().get(theme_name).filter(|t| t.mode == mode) {
        Some(cfg) => editor_theme_from_highlight(fallback, cfg),
        None => fallback,
    };
    // 补全浮层配色无 Zed highlight 直达字段（completion_* 均保留 EditorTheme 默认）。
    // 改从已解析的组件主题 ThemeColor 取（popover/list_hover/muted 已含语义 token 解析），
    // 使补全浮层与普通菜单/弹层随所选主题名一致变化，缺字段不在此处兜底：
    // 组件主题始终有这些字段，无 None 之虞。
    let tc = &ComponentTheme::global(cx).colors;
    theme.completion_bg = tc.popover.into();
    theme.completion_text = tc.popover_foreground.into();
    theme.completion_detail = tc.muted_foreground.into();
    theme.completion_selected_bg = tc.list_hover.into();
    theme
}

#[cfg(test)]
mod theme_registry_tests {
    use super::*;
    use gpui_component::theme::ThemeConfigColors;

    /// 构造含 highlight 的主题。syntax/editor 的颜色经 ThemeStyle 反序列化（其字段私有），
    /// 故用 serde_json 拼 HighlightThemeStyle。
    fn cfg(hl_json: &str) -> ThemeConfig {
        let hl = serde_json::from_str(hl_json).expect("highlight json");
        ThemeConfig {
            is_default: false,
            name: "测试主题".into(),
            mode: ThemeMode::Dark,
            font_size: None,
            font_family: None,
            mono_font_family: None,
            mono_font_size: None,
            radius: None,
            radius_lg: None,
            shadow: None,
            colors: ThemeConfigColors::default(),
            highlight: Some(hl),
        }
    }

    fn red() -> gpui::Rgba {
        gpui::rgba(0xff0000ff)
    }

    /// 有 editor.* / syntax.* 时对应字段被主题覆写（#ff0000 → R=255）；
    /// 无对应名（property/variable）、缺 editor.active_line 时回退 fallback 原值。
    #[test]
    fn maps_known_fields_and_falls_back_on_missing() {
        let json = r##"{
            "editor.background": "#ff0000",
            "editor.foreground": "#ff0000",
            "syntax": { "keyword": { "color": "#ff0000" } }
        }"##;
        let out = editor_theme_from_highlight(editor_component::EditorTheme::dark(), &cfg(json));
        let dark = editor_component::EditorTheme::dark();

        assert_eq!(out.background, red(), "background 应被主题覆写");
        assert_eq!(out.text, red(), "text 应被主题覆写");
        assert_eq!(out.syntax_keyword, red(), "keyword 应被主题覆写");
        assert_eq!(out.syntax_identifier, dark.syntax_identifier, "无 property 时 identifier 回退 default");
        assert_eq!(out.active_line, dark.active_line, "active_line 缺失回退 default");
        assert_eq!(out.line_number, dark.line_number, "line_number 缺失回退 default");
    }

    /// 空 highlight（syntax 全空）时完整回退，不 panic。
    #[test]
    fn empty_highlight_falls_back_gracefully() {
        let fallback = editor_component::EditorTheme::dark();
        let out = editor_theme_from_highlight(fallback, &cfg(r#"{"syntax": {}}"#));
        assert_eq!(out.background, fallback.background);
        assert_eq!(out.syntax_string, fallback.syntax_string);
        assert_eq!(out.cursor, fallback.cursor);
    }
}


