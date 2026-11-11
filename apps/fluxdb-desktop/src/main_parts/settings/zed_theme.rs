// Zed 主题导入：把 Zed theme family JSON 转成 fluxdb 的自定义主题。
//
// Zed 的 `style` 与我们 `ThemeConfig` 的 `highlight` 同源（`editor.*`、
// `syntax.*` 键名一致），因此编辑器配色可以整体透传；界面颜色只映射
// 有明确语义对应的键，其余沿用该模式的默认主题，避免导入后出现空色块。

/// Zed 界面颜色 → `ThemeConfigColors` 的扁平点号键。
/// 顺序有意义：后面的键覆盖前面的键，用于「具体键优先于兜底键」。
const ZED_COLOR_MAP: &[(&str, &str)] = &[
    ("background", "background"),
    ("background", "list.background"),
    // Zed 顶层 background 是窗口外框色（One Light 里 #dcdcdd），真正画布是
    // editor.background（#fafafa）。有它时优先，否则才回退 background。
    ("editor.background", "background"),
    ("editor.background", "list.background"),
    // 侧栏：优先 panel，其次标题栏/状态栏/表面，最后回退应用背景。
    ("surface.background", "sidebar.background"),
    ("title_bar.background", "sidebar.background"),
    ("status_bar.background", "sidebar.background"),
    ("panel.background", "sidebar.background"),
    // 分组框：表面/标题栏/状态栏。
    ("surface.background", "group_box.background"),
    ("title_bar.background", "group_box.background"),
    ("status_bar.background", "group_box.background"),
    ("border", "border"),
    ("border", "input.border"),
    ("border", "sidebar.border"),
    ("border.variant", "border"),
    ("border.variant", "input.border"),
    ("border.variant", "sidebar.border"),
    ("border.focused", "ring"),
    ("pane.focused_border", "ring"),
    ("panel.focused_border", "ring"),
    ("element.background", "button.background"),
    // ghost_element.background 在多数 Zed 主题里是全透明，用它当强调色会让
    // 强调条/选中描边消失；改用元素底色，并只取非全透明值（见 valid_color）。
    ("element.background", "accent.background"),
    ("element.hover", "button.hover.background"),
    ("element.active", "button.active.background"),
    ("element.selected", "list.active.background"),
    ("ghost_element.hover", "list.hover.background"),
    ("ghost_element.active", "list.active.background"),
    ("elevated_surface.background", "popover.background"),
    ("toolbar.background", "tab_bar.background"),
    ("tab_bar.background", "tab_bar.background"),
    ("tab.inactive_background", "tab.background"),
    ("tab.active_background", "tab.active.background"),
    ("tab_bar.segmented_control.background", "tab_bar.segmented.background"),
    ("scrollbar.thumb.background", "scrollbar.thumb.background"),
    ("scrollbar.track.background", "scrollbar.background"),
    ("search.match_background", "selection.background"),
    ("link_text.hover", "link.hover"),
    ("text", "foreground"),
    ("text.muted", "muted.foreground"),
    ("text.accent", "accent.foreground"),
    ("text", "sidebar.foreground"),
    ("text", "button.foreground"),
    ("text", "popover.foreground"),
    ("text", "group_box.foreground"),
    ("text", "tab.foreground"),
    ("text", "tab.active.foreground"),
];

/// 把一个 Zed 主题文件转换成若干 `ThemeConfig`。
///
/// `taken` 是已存在的主题名（内置 + 自定义），转换时会为冲突项追加序号，
/// 返回前会把新名字写回 `taken`。
fn zed_themes_to_configs(
    content: &str,
    base_light: &ThemeConfig,
    base_dark: &ThemeConfig,
    taken: &mut BTreeSet<String>,
) -> Result<Vec<ThemeConfig>, String> {
    let root: serde_json::Value =
        serde_json::from_str(content).map_err(|err| err.to_string())?;
    let entries: Vec<&serde_json::Value> = match root.get("themes").and_then(|value| value.as_array())
    {
        Some(themes) => themes.iter().collect(),
        // 兼容单主题文件（顶层直接就是 theme 对象）。
        None => vec![&root],
    };

    let mut configs = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(raw_name) = entry.get("name").and_then(|value| value.as_str()) else {
            return Err(format!("theme #{index} missing name"));
        };
        let mode = match entry.get("appearance").and_then(|value| value.as_str()) {
            Some("dark") => ThemeMode::Dark,
            Some("light") => ThemeMode::Light,
            other => {
                return Err(format!(
                    "theme {raw_name} has unsupported appearance: {}",
                    other.unwrap_or("missing")
                ))
            }
        };
        let style = entry
            .get("style")
            .ok_or_else(|| format!("theme {raw_name} missing style"))?;
        let name = unique_theme_name(raw_name, taken);
        let base = if mode == ThemeMode::Dark {
            base_dark
        } else {
            base_light
        };
        configs.push(convert_zed_style(&name, mode, style, base)?);
    }
    Ok(configs)
}

fn unique_theme_name(name: &str, taken: &mut BTreeSet<String>) -> String {
    let mut candidate = name.to_string();
    let mut suffix = 2;
    while taken.contains(&candidate) {
        candidate = format!("{name} ({suffix})");
        suffix += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

fn convert_zed_style(
    name: &str,
    mode: ThemeMode,
    style: &serde_json::Value,
    base: &ThemeConfig,
) -> Result<ThemeConfig, String> {
    let mut config = base.clone();
    config.name = name.into();
    config.mode = mode;
    config.is_default = false;
    config.colors = merged_colors(&base.colors, style)?;
    if let Some(highlight) = zed_highlight(style) {
        config.highlight = Some(highlight);
    }
    Ok(config)
}

/// 以默认主题颜色为底，叠加 Zed 里有对应语义的界面颜色。
fn merged_colors(
    base: &ThemeConfigColors,
    style: &serde_json::Value,
) -> Result<ThemeConfigColors, String> {
    let mut value = serde_json::to_value(base).map_err(|err| err.to_string())?;
    let colors = value
        .as_object_mut()
        .ok_or_else(|| "base theme colors are not an object".to_string())?;
    for (zed_key, target) in ZED_COLOR_MAP {
        if let Some(color) = valid_color(style.get(*zed_key)) {
            colors.insert((*target).into(), serde_json::json!(color));
        }
    }
    // Zed 的光标/选区在 players 里，取第一个玩家的颜色。
    if let Some(player) = style
        .get("players")
        .and_then(|value| value.as_array())
        .and_then(|players| players.first())
    {
        if let Some(color) = valid_color(player.get("selection")) {
            colors.insert("selection.background".into(), serde_json::json!(color));
        }
        if let Some(color) = valid_color(player.get("cursor")) {
            colors.insert("caret".into(), serde_json::json!(color));
        }
    }
    serde_json::from_value(value).map_err(|err| err.to_string())
}

fn valid_color(value: Option<&serde_json::Value>) -> Option<&str> {
    let color = value?.as_str()?;
    // 非 hex（如 "transparent"）解析失败；全透明色（#00000000）虽合法但落到
    // 背景类 token 上等于没颜色，同样跳过，保留基础主题的可见配色。
    let alpha = serde_json::from_value::<Hsla>(serde_json::json!(color))
        .ok()
        .map(|hsla| hsla.a)?;
    (alpha > 0.0).then_some(color)
}

/// Zed 的 style 与 `HighlightThemeStyle` 键名一致，可整体透传；
/// 数字字重不受支持时去掉字重字段再解析，保留颜色。
fn zed_highlight(style: &serde_json::Value) -> Option<HighlightThemeStyle> {
    if style.get("syntax").is_none() && style.get("editor.background").is_none() {
        return None;
    }
    if let Ok(highlight) = serde_json::from_value(style.clone()) {
        return Some(highlight);
    }
    let mut fallback = style.clone();
    if let Some(syntax) = fallback.get_mut("syntax").and_then(|value| value.as_object_mut()) {
        for entry in syntax.values_mut() {
            if let Some(entry) = entry.as_object_mut() {
                entry.remove("font_weight");
            }
        }
    }
    serde_json::from_value(fallback).ok()
}

#[cfg(test)]
mod zed_theme_tests {
    use super::*;

    fn base_theme(mode: ThemeMode) -> ThemeConfig {
        serde_json::from_value(serde_json::json!({
            "name": if mode == ThemeMode::Dark { "Default Dark" } else { "Default Light" },
            "mode": if mode == ThemeMode::Dark { "dark" } else { "light" },
            "colors": {
                "background": "#ffffff",
                "foreground": "#000000",
                "danger.background": "#ff0000"
            }
        }))
        .expect("base theme")
    }

    const ONE_LIGHT: &str = r##"{
        "name": "One",
        "author": "zed",
        "themes": [{
            "name": "One Light",
            "appearance": "light",
            "style": {
                "background": "#fafafa",
                "text": "#383a42",
                "text.muted": "#696c77",
                "border": "#d0d0d0",
                "panel.background": "#f0f0f0",
                "tab.active_background": "#ffffff",
                "players": [{"selection": "#b4d8fd", "cursor": "#526fff"}],
                "editor.background": "#fafafa",
                "editor.foreground": "#383a42",
                "syntax": {
                    "keyword": {"color": "#a626a4"},
                    "string": {"color": "#50a14f"},
                    "comment": {"color": "#a0a1a7", "font_style": "italic"}
                }
            }
        }]
    }"##;

    #[test]
    fn converts_zed_theme_colors_and_highlight() {
        let mut taken = BTreeSet::new();
        let configs = zed_themes_to_configs(
            ONE_LIGHT,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect("convert theme");
        assert_eq!(configs.len(), 1);
        let theme = &configs[0];
        assert_eq!(theme.name.as_ref(), "One Light");
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.colors.background.as_deref(), Some("#fafafa"));
        assert_eq!(theme.colors.foreground.as_deref(), Some("#383a42"));
        assert_eq!(theme.colors.sidebar.as_deref(), Some("#f0f0f0"));
        assert_eq!(theme.colors.selection.as_deref(), Some("#b4d8fd"));
        // 未映射的状态色沿用基础主题。
        assert_eq!(theme.colors.danger.as_deref(), Some("#ff0000"));
        let highlight = theme.highlight.as_ref().expect("highlight copied");
        assert!(highlight.editor_background.is_some());
        assert!(highlight.syntax.keyword.is_some());
    }

    #[test]
    fn conflicting_names_get_a_suffix() {
        let mut taken = BTreeSet::from(["One Light".to_string()]);
        let configs = zed_themes_to_configs(
            ONE_LIGHT,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect("convert theme");
        assert_eq!(configs[0].name.as_ref(), "One Light (2)");
    }

    #[test]
    fn canvas_color_prefers_editor_background() {
        // Zed 顶层 background 是窗口外框色，画布取 editor.background。
        let mut taken = BTreeSet::new();
        let configs = zed_themes_to_configs(
            r##"{"themes":[{"name":"Canvas","appearance":"light","style":{
                "background":"#dcdcddff",
                "editor.background":"#fafafaff",
                "ghost_element.background":"#00000000"}}]}"##,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect("convert theme");
        assert_eq!(configs[0].colors.background.as_deref(), Some("#fafafaff"));
        // 全透明色不能覆盖强调色，否则强调条/选中描边消失。
        assert_ne!(configs[0].colors.accent.as_deref(), Some("#00000000"));
    }

    #[test]
    fn invalid_theme_reports_error() {
        let mut taken = BTreeSet::new();
        let error = zed_themes_to_configs(
            r#"{"name":"Broken","themes":[{"name":"Bad"}]}"#,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect_err("missing appearance/style must fail");
        assert!(error.contains("appearance"));
    }
}
