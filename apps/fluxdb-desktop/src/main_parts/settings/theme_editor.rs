// 自定义主题编辑：外观页「新建主题」弹框 + 用户主题目录写入。
// 表单覆盖 23 个常用 token（含 SQL 高亮的全部种类）；其余字段可保存后在 JSON 视图/文件里补。

/// 表单 token：主题 JSON 的键路径 + 文案 key。
/// 注意主题 JSON 里 `colors` 与 `highlight` 的多数键是扁平点号键（如 `accent.background`），
/// 不是嵌套对象；路径必须按实际 JSON 结构拆分，否则读写会落到 serde 忽略的字段上。
const THEME_TOKENS: [(&[&str], &str); 23] = [
    (&["colors", "background"], "app.settings.appearance.token_background"),
    (&["colors", "group_box.background"], "app.settings.appearance.token_group_box"),
    (&["colors", "sidebar.background"], "app.settings.appearance.token_sidebar"),
    (&["colors", "popover.background"], "app.settings.appearance.token_popover"),
    (&["colors", "foreground"], "app.settings.appearance.token_text"),
    (&["colors", "muted.foreground"], "app.settings.appearance.token_muted"),
    (&["colors", "border"], "app.settings.appearance.token_border"),
    (&["colors", "input.border"], "app.settings.appearance.token_input_border"),
    (&["colors", "accent.background"], "app.settings.appearance.token_accent"),
    (&["colors", "accent.foreground"], "app.settings.appearance.token_accent_fg"),
    (&["colors", "success.background"], "app.settings.appearance.token_success"),
    (&["colors", "warning.background"], "app.settings.appearance.token_warning"),
    (&["colors", "danger.background"], "app.settings.appearance.token_danger"),
    (&["colors", "info.background"], "app.settings.appearance.token_info"),
    (&["highlight", "editor.background"], "app.settings.appearance.token_editor_bg"),
    (&["highlight", "syntax", "keyword", "color"], "app.settings.appearance.token_keyword"),
    (&["highlight", "syntax", "string", "color"], "app.settings.appearance.token_string"),
    (&["highlight", "syntax", "comment", "color"], "app.settings.appearance.token_comment"),
    // SQL 高亮实际产出的其余种类。`property` 同时驱动表名/别名（identifier）
    // 与列名（field），单独给出 `identifier`/`field` 键没有对应读取方。
    (&["highlight", "syntax", "number", "color"], "app.settings.appearance.token_number"),
    (&["highlight", "syntax", "boolean", "color"], "app.settings.appearance.token_boolean"),
    (&["highlight", "syntax", "type", "color"], "app.settings.appearance.token_type"),
    (&["highlight", "syntax", "property", "color"], "app.settings.appearance.token_property"),
    (&["highlight", "syntax", "function", "color"], "app.settings.appearance.token_function"),
];

/// 表单分组：把 23 个 token 分成表面、文本边框、强调状态、编辑器语法四组。
/// 区间按 `THEME_TOKENS` 顺序切分。
const THEME_TOKEN_GROUPS: [(&str, usize, usize); 4] = [
    ("app.settings.appearance.theme_group_surface", 0, 4),
    ("app.settings.appearance.theme_group_text", 4, 8),
    ("app.settings.appearance.theme_group_accent", 8, 14),
    ("app.settings.appearance.theme_group_editor", 14, 23),
];

#[derive(Clone)]
struct ThemeTokenPicker {
    path: &'static [&'static str],
    label_key: &'static str,
    picker: Entity<ColorPickerState>,
}

#[derive(Clone)]
struct ThemeEditorDialog {
    name_input: Entity<InputState>,
    mode: ThemeMode,
    /// 编辑已有自定义主题时的原始名称；新建为 None。
    editing: Option<String>,
    base_light: ThemeConfig,
    base_dark: ThemeConfig,
    tokens: Vec<ThemeTokenPicker>,
    json_view: bool,
    json_input: Entity<TextareaState>,
    error: Option<SharedString>,
}

fn theme_base_config(settings: &Settings, mode: ThemeMode, cx: &App) -> ThemeConfig {
    let registry = ThemeRegistry::global(cx);
    let name = if mode == ThemeMode::Dark {
        settings.dark_theme.as_str()
    } else {
        settings.light_theme.as_str()
    };
    registry
        .themes()
        .get(name)
        .filter(|theme| theme.mode == mode)
        .map(|theme| (**theme).clone())
        .unwrap_or_else(|| {
            if mode == ThemeMode::Dark {
                (**registry.default_dark_theme()).clone()
            } else {
                (**registry.default_light_theme()).clone()
            }
        })
}

fn theme_json_color(value: &serde_json::Value, path: &[&str]) -> Option<Hsla> {
    let mut current = value;
    for segment in path {
        current = current.get(segment)?;
    }
    current
        .as_str()
        .and_then(parse_theme_hex_color)
        .map(Hsla::from)
}

/// 按点号路径写入 JSON；中间对象缺失时自动补空对象。
fn set_theme_json_path(
    value: &mut serde_json::Value,
    segments: &[&str],
    new_value: serde_json::Value,
) {
    let Some((head, tail)) = segments.split_first() else {
        return;
    };
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let object = value.as_object_mut().expect("ensured object");
    if tail.is_empty() {
        object.insert((*head).to_string(), new_value);
        return;
    }
    let child = object
        .entry((*head).to_string())
        .or_insert_with(|| serde_json::json!({}));
    set_theme_json_path(child, tail, new_value);
}

fn theme_token_pickers_from_value(
    value: &serde_json::Value,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Vec<ThemeTokenPicker> {
    THEME_TOKENS
        .iter()
        .map(|(path, label_key)| {
            let color = theme_json_color(value, path).unwrap_or_else(|| hsla(0., 0., 0.5, 1.));
            let picker = cx.new(|cx| ColorPickerState::new(window, cx).default_value(color));
            ThemeTokenPicker {
                path,
                label_key,
                picker,
            }
        })
        .collect()
}

fn theme_token_pickers(
    config: &ThemeConfig,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Vec<ThemeTokenPicker> {
    let value = serde_json::to_value(config).unwrap_or_default();
    theme_token_pickers_from_value(&value, window, cx)
}

/// 表单当前值 → 单个 ThemeConfig。名称缺失、颜色未提交时返回可展示的错误。
fn theme_config_from_editor(
    dialog: &ThemeEditorDialog,
    cx: &App,
) -> Result<ThemeConfig, String> {
    let name = dialog.name_input.read(cx).value().trim().to_string();
    if name.is_empty() {
        return Err(tr("app.settings.appearance.theme_name_required").to_string());
    }
    let base = if dialog.mode == ThemeMode::Dark {
        &dialog.base_dark
    } else {
        &dialog.base_light
    };
    let mut value = serde_json::to_value(base).map_err(|err| err.to_string())?;
    value["name"] = serde_json::json!(name);
    value["mode"] = serde_json::to_value(dialog.mode).map_err(|err| err.to_string())?;
    // 基础主题没有 highlight 时不能凭空造部分字段：HighlightThemeStyle 的 syntax 是必填，
    // 缺了会反序列化失败。此时跳过编辑器配色，其余颜色照常保存。
    let has_highlight = value.get("highlight").is_some();
    for token in &dialog.tokens {
        if !has_highlight && token.path.first() == Some(&"highlight") {
            continue;
        }
        let color = token
            .picker
            .read(cx)
            .displayed_color()
            .ok_or_else(|| tr("app.settings.appearance.theme_color_invalid").to_string())?;
        set_theme_json_path(&mut value, token.path, serde_json::json!(color.to_hex()));
    }
    serde_json::from_value(value).map_err(|err| err.to_string())
}

fn theme_config_json(config: &ThemeConfig) -> String {
    serde_json::to_string_pretty(config).unwrap_or_default()
}

impl NavicatMain {
    fn open_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_theme_editor_for(None, window, cx);
    }

    /// 打开主题编辑器。`existing` 为已有自定义主题名时进入编辑态：
    /// 表单回填该主题的颜色与名称，保存时同名覆盖、改名时删除旧文件。
    fn open_theme_editor_for(
        &mut self,
        existing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.controller.state().settings.clone();
        let existing_config = existing
            .as_ref()
            .and_then(|name| ThemeRegistry::global(cx).themes().get(name.as_str()).cloned());
        let mode = existing_config
            .as_ref()
            .map(|config| config.mode)
            .unwrap_or(self.theme_mode);
        let base_light = theme_base_config(&settings, ThemeMode::Light, cx);
        let base_dark = theme_base_config(&settings, ThemeMode::Dark, cx);
        let base = if mode == ThemeMode::Dark {
            &base_dark
        } else {
            &base_light
        };
        let initial_name = existing.clone();
        let name_input = cx.new(|cx| {
            let input =
                InputState::new(window, cx).placeholder(tr("app.settings.appearance.theme_name"));
            match &initial_name {
                Some(name) => input.default_value(name.clone()),
                None => input,
            }
        });
        let json_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(16)
                .placeholder(tr("app.settings.appearance.theme_json_hint"))
        });
        let tokens = match existing_config {
            Some(config) => theme_token_pickers(&config, window, cx),
            None => theme_token_pickers(base, window, cx),
        };
        self.theme_editor = Some(ThemeEditorDialog {
            name_input,
            mode,
            editing: existing,
            base_light,
            base_dark,
            tokens,
            json_view: false,
            json_input,
            error: None,
        });
        cx.notify();
    }

    fn close_theme_editor(&mut self, cx: &mut Context<Self>) {
        self.theme_editor = None;
        cx.notify();
    }

    fn set_theme_editor_mode(
        &mut self,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.theme_editor.as_mut() else {
            return;
        };
        if dialog.mode == mode {
            return;
        }
        dialog.mode = mode;
        let base = if mode == ThemeMode::Dark {
            &dialog.base_dark
        } else {
            &dialog.base_light
        };
        let value = serde_json::to_value(base).unwrap_or_default();
        dialog.tokens = theme_token_pickers_from_value(&value, window, cx);
        dialog.json_view = false;
        dialog.error = None;
        cx.notify();
    }

    fn toggle_theme_editor_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.theme_editor.as_mut() else {
            return;
        };
        if dialog.json_view {
            // JSON → 表单：解析失败时留在 JSON 视图并提示。
            let text = dialog.json_input.read(cx).value().to_string();
            match serde_json::from_str::<ThemeConfig>(&text) {
                Ok(config) => {
                    dialog.mode = config.mode;
                    dialog.name_input.update(cx, |input, cx| {
                        input.set_value(config.name.to_string(), window, cx)
                    });
                    let value = serde_json::to_value(&config).unwrap_or_default();
                    dialog.tokens = theme_token_pickers_from_value(&value, window, cx);
                    dialog.json_view = false;
                    dialog.error = None;
                }
                Err(err) => dialog.error = Some(SharedString::from(err.to_string())),
            }
        } else {
            // 表单 → JSON：名称还没填时用占位名生成，方便直接编辑完整 JSON。
            let json = theme_config_from_editor(dialog, cx)
                .map(|config| theme_config_json(&config))
                .unwrap_or_else(|_| {
                    let base = if dialog.mode == ThemeMode::Dark {
                        &dialog.base_dark
                    } else {
                        &dialog.base_light
                    };
                    theme_config_json(base)
                });
            dialog
                .json_input
                .update(cx, |input, cx| input.set_value(json, window, cx));
            dialog.json_view = true;
            dialog.error = None;
        }
        cx.notify();
    }

    fn submit_theme_editor(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.theme_editor.as_ref() else {
            return;
        };
        let config = if dialog.json_view {
            let text = dialog.json_input.read(cx).value().to_string();
            match serde_json::from_str::<ThemeConfig>(&text) {
                Ok(config) => config,
                Err(err) => {
                    let error = SharedString::from(err.to_string());
                    if let Some(dialog) = self.theme_editor.as_mut() {
                        dialog.error = Some(error);
                    }
                    cx.notify();
                    return;
                }
            }
        } else {
            match theme_config_from_editor(dialog, cx) {
                Ok(config) => config,
                Err(error) => {
                    let error = SharedString::from(error);
                    if let Some(dialog) = self.theme_editor.as_mut() {
                        dialog.error = Some(error);
                    }
                    cx.notify();
                    return;
                }
            }
        };
        if config.name.trim().is_empty() {
            let error = SharedString::from(tr("app.settings.appearance.theme_name_required"));
            if let Some(dialog) = self.theme_editor.as_mut() {
                dialog.error = Some(error);
            }
            cx.notify();
            return;
        }
        // 编辑态允许保留原名覆盖；只有改成别的已存在主题名才拦。
        let editing = self
            .theme_editor
            .as_ref()
            .and_then(|dialog| dialog.editing.clone());
        let name_taken = ThemeRegistry::global(cx)
            .themes()
            .contains_key(config.name.as_str());
        if name_taken && editing.as_deref() != Some(config.name.as_str()) {
            let error = SharedString::from(tr("app.settings.appearance.theme_name_duplicate"));
            if let Some(dialog) = self.theme_editor.as_mut() {
                dialog.error = Some(error);
            }
            cx.notify();
            return;
        }
        let themes_dir = self.storage.root_dir().join(USER_THEMES_DIR);
        if let Err(error) = save_custom_theme(&themes_dir, &config) {
            tracing::warn!(%error, "自定义主题保存失败");
            let error = SharedString::from(error);
            if let Some(dialog) = self.theme_editor.as_mut() {
                dialog.error = Some(error);
            }
            cx.notify();
            return;
        }
        // 改名保存后旧文件不再被引用，删掉避免目录里留一份同内容旧主题。
        if let Some(old_name) = &editing {
            let old_path = themes_dir.join(theme_file_name(old_name));
            let new_path = themes_dir.join(theme_file_name(config.name.as_ref()));
            if old_path != new_path {
                let _ = std::fs::remove_file(old_path);
            }
        }
        // 改名后先把新主题同步注入，保证立即选中生效；注册表对同名主题不覆盖，
        // 同色改名/调色由应用启动时注册的目录监听在文件落盘后异步重载。
        if let Ok(content) = std::fs::read_to_string(
            themes_dir.join(theme_file_name(config.name.as_ref())),
        ) {
            let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(&content);
        }
        self.custom_theme_names = custom_theme_names(&themes_dir);
        self.theme_editor = None;
        let mode = config.mode;
        let name = config.name.to_string();
        self.set_theme_name(mode, name, cx);
        self.show_message(
            tr("app.settings.appearance.theme_saved"),
            AppMessageKind::Info,
            cx,
        );
    }

    /// 选择 Zed / VS Code 主题 JSON 并导入为自定义主题。
    fn import_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.settings.appearance.theme_import_prompt").into()),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Ok(None)) => return,
                Ok(Err(error)) => {
                    report_theme_import_error(&view, cx, &error.to_string());
                    return;
                }
                Err(error) => {
                    report_theme_import_error(&view, cx, &error.to_string());
                    return;
                }
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(content) => content,
                Err(error) => {
                    report_theme_import_error(&view, cx, &error.to_string());
                    return;
                }
            };
            let _ = cx.update(|_, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    match this.import_theme_content(&content, cx) {
                        Ok(0) => this.show_message(
                            tr("app.settings.appearance.theme_import_empty"),
                            AppMessageKind::Info,
                            cx,
                        ),
                        Ok(count) => this.show_message(
                            tr("app.settings.appearance.theme_imported")
                                .replace("%{count}", &count.to_string()),
                            AppMessageKind::Info,
                            cx,
                        ),
                        Err(error) => this.show_message(
                            tr("app.settings.appearance.theme_import_failed")
                                .replace("%{error}", &error),
                            AppMessageKind::Error,
                            cx,
                        ),
                    }
                });
            });
        }));
    }

    /// 把外部主题文件内容写成自定义主题并立即注入注册表。
    ///
    /// 按 JSON 结构判别来源：顶层 `themes` 数组是 Zed，`colors`/`tokenColors` 是
    /// VS Code；名称与现有主题冲突时由对应转换器追加序号，导入后选中第一个。
    fn import_theme_content(
        &mut self,
        content: &str,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let (base_light, base_dark) = {
            let registry = ThemeRegistry::global(cx);
            (
                (**registry.default_light_theme()).clone(),
                (**registry.default_dark_theme()).clone(),
            )
        };
        let mut taken: BTreeSet<String> = ThemeRegistry::global(cx)
            .themes()
            .keys()
            .map(|name| name.to_string())
            .collect();
        taken.extend(self.custom_theme_names.iter().cloned());
        // VS Code 主题允许 JSONC（注释、尾逗号），json5 兼容两者，统一用它判别结构。
        let root: serde_json::Value = serde_json::from_str(content)
            .or_else(|_| json5::from_str(content))
            .map_err(|err| err.to_string())?;
        let configs = if root.get("themes").is_some() {
            zed_themes_to_configs(content, &base_light, &base_dark, &mut taken)?
        } else if is_vscode_theme(&root) {
            vscode_themes_to_configs(&root, &base_light, &base_dark, &mut taken)?
        } else {
            return Err(tr("app.settings.appearance.theme_import_unknown_format").to_string());
        };
        if configs.is_empty() {
            return Ok(0);
        }
        let themes_dir = self.storage.root_dir().join(USER_THEMES_DIR);
        let mut first = None;
        for config in &configs {
            save_custom_theme(&themes_dir, config)?;
            if first.is_none() {
                first = Some((config.mode, config.name.to_string()));
            }
            if let Ok(saved) =
                std::fs::read_to_string(themes_dir.join(theme_file_name(config.name.as_ref())))
            {
                let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(&saved);
            }
        }
        self.custom_theme_names = custom_theme_names(&themes_dir);
        if let Some((mode, name)) = first {
            // 配色列表只展示当前明暗模式的主题；导入深色主题时若停留在浅色模式，
            // 界面看起来像「导入没生效」。先切到主题所属模式再选中。
            if mode != self.theme_mode {
                self.set_theme_mode(mode, cx);
            }
            self.set_theme_name(mode, name, cx);
        }
        Ok(configs.len())
    }

    /// 删除自定义主题：文件移入 `themes/.trash/`（可手工恢复），再从注册表移除。
    fn delete_custom_theme(&mut self, name: String, cx: &mut Context<Self>) {
        let themes_dir = self.storage.root_dir().join(USER_THEMES_DIR);
        let source = themes_dir.join(theme_file_name(&name));
        let Some(removed) = ThemeRegistry::global(cx).themes().get(name.as_str()).cloned() else {
            return;
        };
        let mode = removed.mode;
        let trash_dir = themes_dir.join(".trash");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let file_name = source
            .file_name()
            .and_then(|file| file.to_str())
            .unwrap_or("theme.json");
        let target = trash_dir.join(format!("{stamp}-{file_name}"));
        let moved = std::fs::create_dir_all(&trash_dir)
            .and_then(|_| std::fs::rename(&source, &target));
        if let Err(error) = moved {
            tracing::warn!(%error, path = %source.display(), "移动自定义主题到回收目录失败");
            self.show_message(
                tr("app.settings.appearance.theme_delete_failed"),
                AppMessageKind::Error,
                cx,
            );
            return;
        }
        self.custom_theme_names = custom_theme_names(&themes_dir);
        // 删的是当前模式选中的主题时，回退到该模式的默认主题。
        let selected_is_removed = if mode == ThemeMode::Dark {
            self.settings_editor_draft.dark_theme.as_str() == name
        } else {
            self.settings_editor_draft.light_theme.as_str() == name
        };
        if selected_is_removed {
            let fallback = {
                let registry = ThemeRegistry::global(cx);
                let theme = if mode == ThemeMode::Dark {
                    registry.default_dark_theme()
                } else {
                    registry.default_light_theme()
                };
                theme.name.to_string()
            };
            self.set_theme_name(mode, fallback, cx);
        }
        // 注册表里的旧条目由应用启动时注册的目录监听重载清理；
        // 在它到达前，配色列表用磁盘上的主题名过滤掉这个残留条目。
        self.show_message(
            tr("app.settings.appearance.theme_deleted"),
            AppMessageKind::Info,
            cx,
        );
    }

    fn rescan_custom_themes(&mut self, cx: &mut Context<Self>) {
        let themes_dir = self.storage.root_dir().join(USER_THEMES_DIR);
        normalize_user_theme_files(&themes_dir);
        self.custom_theme_names = custom_theme_names(&themes_dir);
        if let Ok(entries) = std::fs::read_dir(&themes_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(&content);
                }
            }
        }
        self.preview_settings(cx);
        cx.notify();
    }

    fn reveal_themes_dir(&mut self, cx: &mut Context<Self>) {
        let themes_dir = self.storage.root_dir().join(USER_THEMES_DIR);
        if let Err(error) = std::fs::create_dir_all(&themes_dir) {
            tracing::warn!(%error, path = %themes_dir.display(), "创建主题目录失败");
            return;
        }
        reveal_directory(&themes_dir);
        cx.notify();
    }
}

/// 文件选择/读取失败时统一提示，避免每个分支重复 upgrade 与 show_message。
fn report_theme_import_error(
    view: &gpui::WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncWindowContext,
    error: &str,
) {
    let _ = cx.update(|_, cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            this.show_message(
                tr("app.settings.appearance.theme_import_failed").replace("%{error}", error),
                AppMessageKind::Error,
                cx,
            );
        });
    });
}

/// 在系统文件管理器中打开目录（三平台各自的原生命令）。
fn reveal_directory(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(target_os = "windows")]
    let command = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let command = "xdg-open";
    if let Err(error) = std::process::Command::new(command).arg(path).spawn() {
        tracing::warn!(%error, path = %path.display(), "打开主题目录失败");
    }
}

fn theme_editor_dialog(
    dialog: Option<ThemeEditorDialog>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Option<gpui::AnyElement> {
    let dialog = dialog.as_ref()?;
    let mask = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_theme_editor(cx);
                cx.stop_propagation();
            }),
        );

    let mode_label = if dialog.mode == ThemeMode::Dark {
        tr("app.settings.appearance.theme_dark_variant")
    } else {
        tr("app.settings.appearance.theme_light_variant")
    };
    let base_name = if dialog.mode == ThemeMode::Dark {
        dialog.base_dark.name.clone()
    } else {
        dialog.base_light.name.clone()
    };
    let body = if dialog.json_view {
        div()
            .w_full()
            .h(px(420.))
            .child(Textarea::new(&dialog.json_input).h_full())
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(THEME_TOKEN_GROUPS.iter().map(|(group_key, start, end)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.muted)
                            .child(tr(*group_key)),
                    )
                    .child(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap_2()
                            .children(dialog.tokens[*start..*end].iter().map(|token| {
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .px_2()
                                    .py_1()
                                    .rounded(colors.radius)
                                    .bg(colors.panel_alt)
                                    .child(
                                        div()
                                            .text_size(ui_text_size(11.))
                                            .text_color(colors.text)
                                            .child(tr(token.label_key)),
                                    )
                                    .child(ColorPicker::new(&token.picker).small())
                            })),
                    )
            }))
            .into_any_element()
    };

    // 面板必须有确定高度：正文区用 flex_1 + min_h(0) 承接滚动，
    // 父容器高度为 auto 时 flex-basis 0 会把整个表单压成 0 高。
    let panel = div()
        .w(px(760.))
        .h(px(640.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(18.),
            px(48.),
            px(0.),
            hsla(220. / 360., 0.3, 0.1, 0.28),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.close_theme_editor(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .px_5()
                .pt_4()
                .pb_3()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(ui_text_size(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(if dialog.editing.is_some() {
                            tr("app.settings.appearance.theme_edit_title")
                        } else {
                            tr("app.settings.appearance.theme_editor_title")
                        }),
                )
                .child(
                    Button::new("theme-editor-close")
                        .small()
                        .ghost()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _, cx| this.close_theme_editor(cx))),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .px_5()
                .pb_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .debug_selector(|| "theme-editor-name".into())
                                .child(Input::new(&dialog.name_input)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new("theme-editor-light")
                                        .small()
                                        .label(tr("app.settings.appearance.theme_light_variant"))
                                        .when(dialog.mode == ThemeMode::Light, |button| {
                                            button.primary()
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_theme_editor_mode(
                                                ThemeMode::Light,
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new("theme-editor-dark")
                                        .small()
                                        .label(tr("app.settings.appearance.theme_dark_variant"))
                                        .when(dialog.mode == ThemeMode::Dark, |button| {
                                            button.primary()
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_theme_editor_mode(ThemeMode::Dark, window, cx)
                                        })),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(format!(
                            "{}: {} · {mode_label}",
                            tr("app.settings.appearance.theme_base"),
                            base_name
                        )),
                )
                .child(body)
                .when_some(dialog.error.clone(), |this, error| {
                    this.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.danger)
                            .child(error),
                    )
                }),
        )
        .child(
            div()
                .px_5()
                .py_3()
                .border_t_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    Button::new("theme-editor-json-toggle")
                        .small()
                        .label(if dialog.json_view {
                            tr("app.settings.appearance.theme_form_view")
                        } else {
                            tr("app.settings.appearance.theme_json_view")
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_theme_editor_json(window, cx)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("theme-editor-cancel")
                                .small()
                                .label(tr("app.common.cancel"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.close_theme_editor(cx)),
                                ),
                        )
                        .child(
                            Button::new("theme-editor-save")
                                .small()
                                .primary()
                                .debug_selector(|| "theme-editor-save".into())
                                .label(tr("app.common.save"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.submit_theme_editor(cx)),
                                ),
                        ),
                ),
        );

    Some(
        mask.child(panel)
            .into_any_element(),
    )
}

#[cfg(test)]
mod theme_token_tests {
    use super::*;

    /// 表单 token 必须写进主题 JSON 的扁平点号键，否则 serde 会静默丢弃。
    #[test]
    fn theme_tokens_round_trip_through_theme_config() {
        let mut value = serde_json::to_value(ThemeConfig::default()).expect("serialize base");
        set_theme_json_path(&mut value, &["colors", "background"], serde_json::json!("#123456"));
        set_theme_json_path(&mut value, &["colors", "accent.background"], serde_json::json!("#abcdef"));
        set_theme_json_path(&mut value, &["name"], serde_json::json!("Test Theme"));

        let config: ThemeConfig = serde_json::from_value(value.clone()).expect("deserialize");
        assert_eq!(config.name.as_ref(), "Test Theme");
        assert_eq!(config.colors.background.as_deref(), Some("#123456"));
        assert_eq!(config.colors.accent.as_deref(), Some("#abcdef"));
        assert!(theme_json_color(&value, &["colors", "accent.background"]).is_some());
    }

    /// 表单里所有语法 token 必须真正进到编辑器配色：写 JSON 路径能反序列化回
    /// `ThemeConfig`，再经 `editor_theme_from_highlight` 落到 SQL 高亮的各类 kind。
    #[test]
    fn syntax_tokens_reach_editor_theme() {
        let base: ThemeConfig = serde_json::from_value(serde_json::json!({
            "name": "Base",
            "mode": "light",
            "colors": {},
            "highlight": {"syntax": {}}
        }))
        .expect("base theme with highlight");
        let mut value = serde_json::to_value(&base).expect("serialize base");
        for (path, _) in THEME_TOKENS.iter() {
            if path.first() == Some(&"highlight") {
                set_theme_json_path(&mut value, path, serde_json::json!("#123456"));
            }
        }

        let config: ThemeConfig = serde_json::from_value(value).expect("deserialize theme");
        let fallback = editor_component::EditorTheme::light();
        let theme = crate::editor_theme_from_highlight(fallback, &config);
        for kind in [
            "keyword", "string", "comment", "number", "boolean", "type", "function",
            // identifier/field 都由 syntax.property 驱动。
            "identifier", "field",
        ] {
            // Hsla→Rgba 有浮点取整，不断言精确值，只要求脱离写死的兜底色。
            assert_ne!(
                theme.syntax_color(kind),
                fallback.syntax_color(kind),
                "{kind} 未跟随主题"
            );
        }
    }
}
