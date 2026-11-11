// VS Code 主题导入：把 workbench `colors` + `tokenColors` 转成 fluxdb 自定义主题。
//
// VS Code 主题可以是 JSONC（允许注释、尾逗号），统一走 json5 解析。颜色键是
// camelCase 的 workbench 名，需要映射到我们的扁平点号 token；语法高亮由
// TextMate scope 规则推导，按「scope 前缀越具体越优先、同分取后者」归一。
//
// 已知降级：`tokenColors` 指向 `.tmTheme`（plist）不支持；`include` 继承链、
// `semanticTokenColors`、高对比变体均不解析，缺失颜色沿用默认明/暗主题。

/// VS Code workbench 颜色 → `ThemeConfigColors` 的扁平点号键。
/// 顺序有意义：后面的键覆盖前面的键。
const VSCODE_COLOR_MAP: &[(&str, &str)] = &[
    ("editor.background", "background"),
    ("editor.background", "list.background"),
    // 侧栏：优先侧栏/活动栏，其次标签组标题栏。
    ("editorGroupHeader.tabsBackground", "sidebar.background"),
    ("activityBar.background", "sidebar.background"),
    ("sideBar.background", "sidebar.background"),
    // 分组框/表面：侧栏、状态栏、标题栏。
    ("statusBar.background", "group_box.background"),
    ("titleBar.activeBackground", "group_box.background"),
    ("sideBar.background", "group_box.background"),
    ("editorGroup.border", "border"),
    ("sideBar.border", "sidebar.border"),
    ("contrastBorder", "border"),
    ("input.border", "input.border"),
    ("focusBorder", "ring"),
    ("button.background", "button.background"),
    ("button.background", "accent.background"),
    ("button.hoverBackground", "button.hover.background"),
    ("button.background", "button.active.background"),
    ("button.foreground", "accent.foreground"),
    ("list.hoverBackground", "list.hover.background"),
    ("list.activeSelectionBackground", "list.active.background"),
    ("menu.background", "popover.background"),
    ("editorSuggestWidget.background", "popover.background"),
    ("editorSuggestWidget.foreground", "popover.foreground"),
    ("menu.foreground", "popover.foreground"),
    ("editorGroupHeader.tabsBackground", "tab_bar.background"),
    ("tab.inactiveBackground", "tab.background"),
    ("tab.activeBackground", "tab.active.background"),
    ("tab.inactiveForeground", "tab.foreground"),
    ("tab.activeForeground", "tab.active.foreground"),
    ("scrollbarSlider.background", "scrollbar.thumb.background"),
    ("editor.selectionBackground", "selection.background"),
    ("editorCursor.foreground", "caret"),
    ("errorForeground", "danger.foreground"),
    ("textLink.foreground", "link.foreground"),
    // 文本类放最后：EditorTheme 没有独立取值的区域统一用编辑器前景色。
    ("editor.foreground", "foreground"),
    ("editor.foreground", "sidebar.foreground"),
    ("editor.foreground", "popover.foreground"),
    ("editor.foreground", "button.foreground"),
    ("editor.foreground", "group_box.foreground"),
    ("editor.foreground", "tab.foreground"),
    ("editor.foreground", "tab.active.foreground"),
    ("editorLineNumber.foreground", "muted.foreground"),
    ("descriptionForeground", "muted.foreground"),
];

/// TextMate scope 前缀 → `SyntaxColors` 键。
/// 顺序有意义：`variable.function` 必须先于 `variable`，`storage.type` 先于 `storage`。
const VSCODE_SCOPE_RULES: &[(&str, &str)] = &[
    ("comment", "comment"),
    ("string", "string"),
    ("constant.numeric", "number"),
    ("constant.language", "boolean"),
    ("entity.name.function", "function"),
    ("support.function", "function"),
    ("variable.function", "function"),
    ("entity.name.type", "type"),
    ("support.type", "type"),
    ("support.class", "type"),
    ("entity.name.class", "type"),
    ("entity.name.namespace", "type"),
    ("storage.type", "type"),
    ("entity.other.attribute-name", "attribute"),
    ("variable.other.property", "property"),
    ("meta.object-literal.key", "property"),
    ("support.variable.property", "property"),
    ("variable.other.member", "property"),
    ("variable.other.object.property", "property"),
    // 没有独立 parameter 槽：映射到 variable.special，由宿主同时用于参数与变量。
    ("variable.parameter", "variable.special"),
    ("keyword", "keyword"),
    ("storage", "keyword"),
    ("variable", "variable"),
];

/// 判断已解析的主题 JSON 是否是 VS Code 风格（以 `colors`/`tokenColors` 为主）。
fn is_vscode_theme(root: &serde_json::Value) -> bool {
    root.get("colors").is_some() || root.get("tokenColors").is_some()
}

/// 把一个 VS Code 主题转成单个 `ThemeConfig`。主题名冲突时追加序号。
fn vscode_themes_to_configs(
    root: &serde_json::Value,
    base_light: &ThemeConfig,
    base_dark: &ThemeConfig,
    taken: &mut BTreeSet<String>,
) -> Result<Vec<ThemeConfig>, String> {
    let raw_name = root
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("VS Code Theme");
    let mode = vscode_theme_mode(root);
    let colors = root.get("colors").cloned().unwrap_or_else(|| serde_json::json!({}));
    let token_colors = match root.get("tokenColors") {
        None => None,
        Some(value @ serde_json::Value::Array(_)) => Some(value),
        // 少数主题把 tokenColors 写成 `.tmTheme` 路径，需要 plist 解析，明确拒绝。
        Some(_) => return Err(tr("app.settings.appearance.theme_import_tm_theme").to_string()),
    };
    let base = if mode == ThemeMode::Dark {
        base_dark
    } else {
        base_light
    };
    let name = unique_theme_name(raw_name, taken);
    let mut config = base.clone();
    config.name = name.into();
    config.mode = mode;
    config.is_default = false;
    config.colors = vscode_colors(&base.colors, &colors)?;
    config.highlight = Some(vscode_highlight(&colors, token_colors)?);
    Ok(vec![config])
}

/// `type` 缺失时按编辑器背景亮度推断明暗，避免浅色主题被当成深色。
fn vscode_theme_mode(root: &serde_json::Value) -> ThemeMode {
    match root.get("type").and_then(|value| value.as_str()) {
        Some("light") | Some("hcLight") => ThemeMode::Light,
        Some("dark") | Some("hc") => ThemeMode::Dark,
        _ => {
            let light = root
                .get("colors")
                .and_then(|colors| colors.get("editor.background"))
                .and_then(|value| valid_color(Some(value)))
                .and_then(|hex| serde_json::from_value::<Hsla>(serde_json::json!(hex)).ok())
                .is_some_and(|hsla| hsla.l > 0.5);
            if light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            }
        }
    }
}

/// 以默认主题颜色为底，叠加 VS Code 里有对应语义的 workbench 颜色。
fn vscode_colors(
    base: &ThemeConfigColors,
    colors: &serde_json::Value,
) -> Result<ThemeConfigColors, String> {
    let mut value = serde_json::to_value(base).map_err(|err| err.to_string())?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "base theme colors are not an object".to_string())?;
    for (vscode_key, target) in VSCODE_COLOR_MAP {
        if let Some(color) = valid_color(colors.get(*vscode_key)) {
            object.insert((*target).into(), serde_json::json!(color));
        }
    }
    serde_json::from_value(value).map_err(|err| err.to_string())
}

/// 编辑器配色：`editor.*` 直接取自 workbench 颜色，语法色由 tokenColors 推导。
fn vscode_highlight(
    colors: &serde_json::Value,
    token_colors: Option<&serde_json::Value>,
) -> Result<HighlightThemeStyle, String> {
    let mut highlight = serde_json::Map::new();
    for (source, target) in [
        ("editor.background", "editor.background"),
        ("editor.foreground", "editor.foreground"),
        ("editor.lineHighlightBackground", "editor.active_line.background"),
        ("editorLineNumber.foreground", "editor.line_number"),
        ("editorLineNumber.activeForeground", "editor.active_line_number"),
        ("errorForeground", "error"),
        ("editorWarning.foreground", "warning"),
        ("editorInfo.foreground", "info"),
    ] {
        if let Some(color) = valid_color(colors.get(source)) {
            highlight.insert(target.into(), serde_json::json!(color));
        }
    }
    let syntax = token_colors
        .map(vscode_syntax_styles)
        .unwrap_or_default();
    // HighlightThemeStyle.syntax 是必填字段，没有规则时也要给空对象。
    highlight.insert("syntax".into(), serde_json::Value::Object(syntax));
    serde_json::from_value(serde_json::Value::Object(highlight)).map_err(|err| err.to_string())
}

/// 从 `tokenColors` 推导每种语法风格的 JSON，按 scope 具体程度择优。
fn vscode_syntax_styles(token_colors: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut color_rules: BTreeMap<&'static str, (ScopeScore, &str)> = BTreeMap::new();
    let mut font_rules: BTreeMap<&'static str, (ScopeScore, &str)> = BTreeMap::new();
    let Some(entries) = token_colors.as_array() else {
        return serde_json::Map::new();
    };
    for (index, entry) in entries.iter().enumerate() {
        let settings = entry.get("settings").unwrap_or(&serde_json::Value::Null);
        let scopes: Vec<&str> = match entry.get("scope") {
            Some(serde_json::Value::String(scope)) => vec![scope.as_str()],
            Some(serde_json::Value::Array(scopes)) => {
                scopes.iter().filter_map(|scope| scope.as_str()).collect()
            }
            _ => continue,
        };
        let color = valid_color(settings.get("foreground"));
        let font = font_style_json(settings);
        for selector in scopes {
            for part in selector.split(',').flat_map(str::split_whitespace) {
                let Some((kind, prefix)) = classify_scope(part) else {
                    continue;
                };
                let score = ScopeScore {
                    prefix_segments: prefix.split('.').count(),
                    exact: part == prefix,
                    less_dots: std::cmp::Reverse(part.split('.').count()),
                    index,
                };
                if let Some(color) = color {
                    color_rules
                        .entry(kind)
                        .and_modify(|(best, value)| {
                            if score >= *best {
                                *best = score;
                                *value = color;
                            }
                        })
                        .or_insert((score, color));
                }
                if let Some(font) = font {
                    font_rules
                        .entry(kind)
                        .and_modify(|(best, value)| {
                            if score >= *best {
                                *best = score;
                                *value = font;
                            }
                        })
                        .or_insert((score, font));
                }
            }
        }
    }

    let mut syntax = serde_json::Map::new();
    let kinds: BTreeSet<&'static str> = color_rules
        .keys()
        .chain(font_rules.keys())
        .copied()
        .collect();
    for kind in kinds {
        let mut style = serde_json::Map::new();
        if let Some((_, color)) = color_rules.get(kind) {
            style.insert("color".into(), serde_json::json!(color));
        }
        // 只取字体样式，不取字重：导入主题的 bold 面向代码字号，SQL 编辑器里会过重。
        if let Some((_, font)) = font_rules.get(kind) {
            style.insert("font_style".into(), serde_json::json!(font));
        }
        syntax.insert(kind.into(), serde_json::Value::Object(style));
    }
    syntax
}

/// scope 具体程度排序键：前缀越具体越优先，其次精确命中，再其次多余层级更少
/// （语言专属的深层 scope 不该盖过通用规则），最后取靠后的规则。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ScopeScore {
    prefix_segments: usize,
    exact: bool,
    less_dots: std::cmp::Reverse<usize>,
    index: usize,
}

fn classify_scope(scope: &str) -> Option<(&'static str, &'static str)> {
    VSCODE_SCOPE_RULES
        .iter()
        .find(|(prefix, _)| scope == *prefix || scope.starts_with(&format!("{prefix}.")))
        .map(|(prefix, kind)| (*kind, *prefix))
}

/// `fontStyle` 可能是 "italic bold underline" 组合；只保留 italic，其余丢弃。
fn font_style_json(settings: &serde_json::Value) -> Option<&'static str> {
    let style = settings.get("fontStyle").and_then(|value| value.as_str())?;
    style
        .split_whitespace()
        .any(|part| part.eq_ignore_ascii_case("italic"))
        .then_some("italic")
}

#[cfg(test)]
mod vscode_theme_tests {
    use super::*;

    /// 真实主题：One Dark Pro（MIT，Binaryify/OneDark-Pro）。
    /// 用于验证 workbench 颜色映射与 TextMate scope 归类在真实数据上的结果。
    const ONE_DARK_PRO: &str = include_str!("testdata/one-dark-pro.json");

    fn base_theme(mode: ThemeMode) -> ThemeConfig {
        serde_json::from_value(serde_json::json!({
            "name": if mode == ThemeMode::Dark { "Default Dark" } else { "Default Light" },
            "mode": if mode == ThemeMode::Dark { "dark" } else { "light" },
            "colors": {"background": "#ffffff", "foreground": "#000000"},
            "highlight": {"syntax": {}}
        }))
        .expect("base theme")
    }

    fn import_real_theme() -> ThemeConfig {
        let root: serde_json::Value = json5::from_str(ONE_DARK_PRO).expect("parse real theme");
        let mut taken = BTreeSet::new();
        let configs = vscode_themes_to_configs(
            &root,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect("convert real theme");
        assert_eq!(configs.len(), 1);
        configs[0].clone()
    }

    #[test]
    fn imports_real_one_dark_pro_theme() {
        let theme = import_real_theme();
        assert_eq!(theme.name.as_ref(), "One Dark Pro");
        assert_eq!(theme.mode, ThemeMode::Dark);
        assert_eq!(theme.colors.background.as_deref(), Some("#282c34"));
        assert_eq!(theme.colors.foreground.as_deref(), Some("#abb2bf"));
        assert_eq!(theme.colors.sidebar.as_deref(), Some("#21252b"));
        assert_eq!(theme.colors.caret.as_deref(), Some("#528bff"));

        let highlight = theme.highlight.as_ref().expect("highlight");
        let syn = |name: &str| highlight.syntax.style(name).and_then(|style| style.color);
        // Hsla→Rgba 有浮点取整，比较顺序与主题解析路径一致，避免 +1 通道误差。
        let rgb = |hex: &str| Hsla::from(parse_theme_hex_color(hex).expect("parse hex"));
        assert!(highlight.editor_background.is_some());
        // 以下取值来自真实 tokenColors：宽泛 scope 优先于语言专属深 scope。
        for (kind, hex) in [
            ("keyword", "#c678dd"),
            ("string", "#98c379"),
            ("comment", "#7f848e"),
            ("type", "#e5c07b"),
            ("function", "#61afef"),
            ("property", "#e06c75"),
        ] {
            assert_eq!(syn(kind), Some(rgb(hex)), "{kind} 颜色与真实 tokenColors 不符");
        }
    }

    #[test]
    fn light_theme_mode_and_mapping() {
        let root: serde_json::Value = json5::from_str(
            r##"{
                // 真实主题常见 JSONC 写法
                "name": "Light Example",
                "type": "light",
                "colors": {"editor.background": "#ffffff", "editor.foreground": "#111111",
                           "editorCursor.foreground": "#00000000"},
                "tokenColors": [{"scope": "keyword", "settings": {"foreground": "#AA0000"}}],
            }"##,
        )
        .expect("parse jsonc");
        let mut taken = BTreeSet::new();
        let configs = vscode_themes_to_configs(
            &root,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect("convert theme");
        let theme = &configs[0];
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.colors.background.as_deref(), Some("#ffffff"));
        // 全透明光标色不覆盖基础主题。
        assert_ne!(theme.colors.caret.as_deref(), Some("#00000000"));
        assert_eq!(
            theme
                .highlight
                .as_ref()
                .and_then(|hl| hl.syntax.style("keyword"))
                .and_then(|style| style.color)
                .map(|color| color.to_rgb()),
            Some(
                parse_theme_hex_color("#aa0000")
                    .expect("parse hex")
            )
        );
    }

    #[test]
    fn tm_theme_reference_reports_error() {
        let root: serde_json::Value = serde_json::json!({
            "name": "Legacy",
            "type": "dark",
            "tokenColors": "./legacy.tmTheme"
        });
        let error = vscode_themes_to_configs(
            &root,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut BTreeSet::new(),
        )
        .expect_err("tmTheme must be rejected");
        // 单元测试里 i18n 可能未初始化，tr() 会退回 key 本身。
        assert!(
            error.contains("tmTheme") || error.contains("theme_import_tm_theme"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn conflicting_names_get_a_suffix() {
        let root: serde_json::Value = serde_json::json!({"name": "One Dark Pro", "type": "dark"});
        let mut taken = BTreeSet::from(["One Dark Pro".to_string()]);
        let configs = vscode_themes_to_configs(
            &root,
            &base_theme(ThemeMode::Light),
            &base_theme(ThemeMode::Dark),
            &mut taken,
        )
        .expect("convert theme");
        assert_eq!(configs[0].name.as_ref(), "One Dark Pro (2)");
    }
}
