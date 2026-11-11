/// 用户自定义主题目录（相对数据目录）。用户主题写在这里，应用升级不会覆盖。
pub(crate) const USER_THEMES_DIR: &str = "themes";

/// 随应用发布的主题 JSON 内容。
fn bundled_theme_files() -> Vec<String> {
    let dir = app_assets_base_path().join("themes");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect()
}

fn watch_themes(settings: Settings, mode: ThemeMode, user_themes_dir: PathBuf, cx: &mut App) {
    // 注册表只在注册监听时执行一次 on_load，之后每次文件变更只重载被监听目录。
    // 内置主题必须以文件形式常驻用户目录，否则任何一次读写都会把它们清空。
    install_bundled_theme_file(&user_themes_dir);
    normalize_user_theme_files(&user_themes_dir);
    let bundled = bundled_theme_files();
    let _ = ThemeRegistry::watch_dir(user_themes_dir, cx, move |cx| {
        for content in &bundled {
            let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(content);
        }
        apply_registered_theme(&settings, mode, cx);
        apply_component_theme_colors(mode, cx);
        cx.refresh_windows();
    });
}

/// 把随应用发布的主题合并写入用户主题目录的一个 JSON。
///
/// 注册表重载只扫描被监听目录，且 on_load 只在注册时执行一次；内置主题若只
/// 存在于 assets，首次文件变更后就会从列表消失。每次启动重写该文件，
/// 应用升级后的内置配色也能同步。
fn install_bundled_theme_file(themes_dir: &std::path::Path) {
    install_bundled_theme_file_from(&bundled_theme_files(), themes_dir);
}

fn install_bundled_theme_file_from(bundled: &[String], themes_dir: &std::path::Path) {
    let mut themes = Vec::new();
    for content in bundled {
        if let Ok(set) = serde_json::from_str::<ThemeSet>(content) {
            themes.extend(set.themes);
        }
    }
    if themes.is_empty() {
        return;
    }
    let set = ThemeSet {
        name: "bundled".into(),
        author: None,
        url: None,
        themes,
    };
    let Ok(content) = serde_json::to_string_pretty(&set) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(themes_dir)
        .and_then(|_| std::fs::write(themes_dir.join("bundled-themes.json"), content))
    {
        tracing::warn!(%error, path = %themes_dir.display(), "写入内置主题文件失败");
    }
}

/// 清掉用户主题文件里的 `is_default`。
///
/// 注册表重载会用该标记覆盖对应模式的默认主题；早期版本把继承来的 true
/// 写进了自定义主题，会让内置默认主题被挤掉。
fn normalize_user_theme_files(themes_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(themes_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut set) = serde_json::from_str::<ThemeSet>(&content) else {
            continue;
        };
        if !set.themes.iter().any(|theme| theme.is_default) {
            continue;
        }
        for theme in &mut set.themes {
            theme.is_default = false;
        }
        match serde_json::to_string_pretty(&set) {
            Ok(content) => {
                if let Err(error) = std::fs::write(&path, content) {
                    tracing::warn!(%error, path = %path.display(), "清理主题 is_default 失败");
                }
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "序列化主题失败");
            }
        }
    }
}

/// 随应用发布的主题名。用于外观页区分内置主题与已从磁盘删除、
/// 但注册表还没异步重载掉的残留条目。
pub(crate) fn bundled_theme_names() -> &'static BTreeSet<String> {
    static NAMES: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        bundled_theme_files()
            .iter()
            .filter_map(|content| serde_json::from_str::<ThemeSet>(content).ok())
            .flat_map(|set| set.themes.into_iter().map(|theme| theme.name.to_string()))
            .collect()
    })
}

/// 扫描用户主题目录，返回其中声明的主题名（用于外观页分组展示）。
pub(crate) fn custom_theme_names(themes_dir: &std::path::Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(themes_dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(set) = serde_json::from_str::<ThemeSet>(&content) else {
            continue;
        };
        for theme in set.themes {
            names.insert(theme.name.to_string());
        }
    }
    // 内置主题以 bundled-themes.json 常驻用户目录，不能算作自定义主题，
    // 否则会多出删除/编辑入口并被归到「自定义主题」分组。
    let bundled = bundled_theme_names();
    names.retain(|name| !bundled.contains(name));
    names
}

/// 把新建主题写入用户主题目录。文件名由主题名派生，同名主题覆盖同一文件。
pub(crate) fn save_custom_theme(
    themes_dir: &std::path::Path,
    theme: &ThemeConfig,
) -> Result<(), String> {
    std::fs::create_dir_all(themes_dir).map_err(|err| err.to_string())?;
    // 用户主题继承自内置主题时会带上 is_default=true，注册表重载会据此覆盖
    // 对应模式的默认主题，必须落盘前清掉。
    let mut theme = theme.clone();
    theme.is_default = false;
    let file_name = theme_file_name(theme.name.as_ref());
    let set = ThemeSet {
        name: theme.name.clone(),
        author: Some("fluxdb".into()),
        url: None,
        themes: vec![theme],
    };
    let content = serde_json::to_string_pretty(&set).map_err(|err| err.to_string())?;
    std::fs::write(themes_dir.join(file_name), content).map_err(|err| err.to_string())
}

pub(crate) fn theme_file_name(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if !slug.is_empty() {
        return format!("{slug}.json");
    }
    // 纯中文/符号名称 slug 为空：用名称哈希区分，避免多个自定义主题写到同一文件互相覆盖。
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    format!("theme-{:016x}.json", hasher.finish())
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
mod theme_file_name_tests {
    use super::*;

    #[test]
    fn theme_file_names_are_distinct() {
        assert_eq!(theme_file_name("My Theme"), "my-theme.json");
        // 纯中文名称不能都退化成同一个文件。
        assert_ne!(theme_file_name("暗色主题"), theme_file_name("亮色主题"));
    }

    #[test]
    fn saved_custom_theme_never_claims_default() {
        // 从内置主题继承来的 is_default=true 会让注册表重载覆盖该模式的默认主题。
        let mut theme: ThemeConfig = serde_json::from_value(serde_json::json!({
            "is_default": true,
            "name": "My Theme",
            "mode": "light"
        }))
        .expect("minimal theme");
        theme.is_default = true;

        let dir = tempfile::tempdir().expect("temp themes dir");
        save_custom_theme(dir.path(), &theme).expect("save theme");
        let saved: ThemeSet = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("my-theme.json")).expect("read theme"),
        )
        .expect("parse theme set");
        assert!(!saved.themes[0].is_default);
    }

    #[test]
    fn bundled_theme_file_keeps_themes_inside_watched_dir() {
        let dir = tempfile::tempdir().expect("temp themes dir");
        let bundled = vec![
            serde_json::json!({
                "name": "github",
                "themes": [
                    {"name": "GitHub Light", "mode": "light"},
                    {"name": "GitHub Dark", "mode": "dark"}
                ]
            })
            .to_string(),
        ];

        install_bundled_theme_file_from(&bundled, dir.path());

        let set: ThemeSet = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("bundled-themes.json"))
                .expect("read bundled theme file"),
        )
        .expect("parse bundled theme set");
        assert_eq!(set.themes.len(), 2);
    }

    #[test]
    fn normalize_clears_legacy_is_default() {
        let dir = tempfile::tempdir().expect("temp themes dir");
        let path = dir.path().join("legacy.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "name": "legacy",
                "themes": [{"is_default": true, "name": "Legacy", "mode": "light"}]
            })
            .to_string(),
        )
        .expect("write legacy theme");

        normalize_user_theme_files(dir.path());

        let set: ThemeSet = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("read normalized theme"),
        )
        .expect("parse theme set");
        assert!(!set.themes[0].is_default);
    }
}
