// 设置配置提案的非敏感字段白名单。
//
// 这里只处理不含凭据/路径的普通设置字段。AI 工具只提交配置提案，
// 真正保存仍由用户在确认框中批准；已有的设置页草稿受冲突检查保护。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSection {
    Editor,
    Appearance,
    Data,
    System,
}

impl SettingsSection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::Appearance => "appearance",
            Self::Data => "data",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "editor" => Some(Self::Editor),
            "appearance" => Some(Self::Appearance),
            "data" => Some(Self::Data),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsPrefill {
    pub section: SettingsSection,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

pub fn settings_section_fields(section: SettingsSection) -> &'static [&'static str] {
    match section {
        SettingsSection::Editor => &[
            "page_size",
            "results_placement",
            "editor_font_size",
            "editor_line_height",
            "editor_font_family",
            "editor_tab_width",
            "editor_word_wrap",
        ],
        SettingsSection::Appearance => &[
            "theme",
            "show_shadows",
            "focus_ring",
            "scrollbar_mode",
            "ui_density",
            "ui_font_size",
            "show_status_bar",
            "reduce_motion",
            "global_font_family",
            "button_radius",
            "large_radius",
        ],
        SettingsSection::Data => &["data_table_page_size"],
        SettingsSection::System => &["performance_diagnostics", "log_level"],
    }
}

/// 配置工具 schema 中设置字段的统一声明。
pub fn settings_fields_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "page_size": { "type": "integer", "enum": [0, 100, 500, 1000] },
            "results_placement": { "type": "string", "enum": ["bottom", "right"] },
            "editor_font_size": { "type": "integer", "minimum": 10, "maximum": 24 },
            "editor_line_height": { "type": "integer", "minimum": 13, "maximum": 28 },
            "editor_font_family": { "type": "string", "maxLength": 128 },
            "editor_tab_width": { "type": "integer", "enum": [2, 4, 8] },
            "editor_word_wrap": { "type": "boolean" },
            "theme": { "type": "string", "enum": ["system", "light", "dark"] },
            "show_shadows": { "type": "boolean" },
            "focus_ring": { "type": "boolean" },
            "scrollbar_mode": { "type": "string", "enum": ["scrolling", "hover", "always"] },
            "ui_density": { "type": "string", "enum": ["compact", "standard", "comfortable"] },
            "ui_font_size": { "type": "integer", "minimum": 12, "maximum": 18 },
            "show_status_bar": { "type": "boolean" },
            "reduce_motion": { "type": "boolean" },
            "global_font_family": { "type": "string", "maxLength": 128 },
            "button_radius": { "type": "integer", "minimum": 0, "maximum": 24 },
            "large_radius": { "type": "integer", "minimum": 0, "maximum": 32 },
            "data_table_page_size": { "type": "integer", "enum": [100, 500, 1000] },
            "performance_diagnostics": { "type": "boolean" },
            "log_level": { "type": "string", "enum": ["error", "warn", "info", "debug", "trace"] }
        }
    })
}

/// 解析并校验 `config.setting.update` 的设置字段。
///
/// schema 只做第一层过滤；这里仍按 section 校验字段归属和取值，避免模型把
/// “危险 SQL 确认”“客户端路径”等未开放字段夹带进来。
pub fn parse_settings_prefill(args: &serde_json::Value) -> Result<SettingsPrefill, String> {
    let section_name = args
        .get("section")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "缺少设置分区 section".to_string())?;
    let section = SettingsSection::parse(section_name)
        .ok_or_else(|| format!("不支持的设置分区: {section_name}"))?;
    let fields = args
        .get("fields")
        .and_then(|value| value.as_object())
        .ok_or_else(|| "fields 必须是对象".to_string())?;
    if fields.is_empty() {
        return Err("fields 不能为空".to_string());
    }
    let allowed = settings_section_fields(section);
    for key in fields.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "字段 {key} 不属于设置分区 {}；可用字段: {}",
                section.as_str(),
                allowed.join(", ")
            ));
        }
    }
    Ok(SettingsPrefill {
        section,
        fields: fields.clone(),
    })
}

/// 把已经通过校验的预填字段应用到设置草稿。只改内存草稿，不落盘。
pub fn apply_settings_prefill(
    settings: &mut fluxdb_core::Settings,
    prefill: &SettingsPrefill,
) -> Result<(), String> {
    for (key, value) in &prefill.fields {
        match (prefill.section, key.as_str()) {
            (SettingsSection::Editor, "page_size") => {
                let value = value_u64(value, key, 0, 1000)?;
                if !matches!(value, 0 | 100 | 500 | 1000) {
                    return Err("page_size 只能是 0、100、500 或 1000".to_string());
                }
                settings.page_size = value;
            }
            (SettingsSection::Editor, "results_placement") => {
                settings.results_placement = match value_str(value, key)? {
                    "bottom" => fluxdb_core::ResultsPlacement::Bottom,
                    "right" => fluxdb_core::ResultsPlacement::Right,
                    other => {
                        return Err(format!(
                            "results_placement 只能是 bottom 或 right，收到 {other}"
                        ));
                    }
                };
            }
            (SettingsSection::Editor, "editor_font_size") => {
                settings.editor_font_size = value_u32(value, key, 10, 24)?;
            }
            (SettingsSection::Editor, "editor_line_height") => {
                settings.editor_line_height = value_u32(value, key, 13, 28)?;
            }
            (SettingsSection::Editor, "editor_font_family") => {
                let family = value_str(value, key)?;
                if family.chars().count() > 128 {
                    return Err("editor_font_family 不能超过 128 个字符".to_string());
                }
                settings.editor_font_family = family.to_string();
            }
            (SettingsSection::Editor, "editor_tab_width") => {
                let value = value_u32(value, key, 2, 8)?;
                if !matches!(value, 2 | 4 | 8) {
                    return Err("editor_tab_width 只能是 2、4 或 8".to_string());
                }
                settings.editor_tab_width = value;
            }
            (SettingsSection::Editor, "editor_word_wrap") => {
                settings.editor_word_wrap = value_bool(value, key)?;
            }
            (SettingsSection::Appearance, "theme") => {
                settings.theme = match value_str(value, key)? {
                    "system" => fluxdb_core::Theme::System,
                    "light" => fluxdb_core::Theme::Light,
                    "dark" => fluxdb_core::Theme::Dark,
                    other => {
                        return Err(format!(
                            "theme 只能是 system、light 或 dark，收到 {other}"
                        ));
                    }
                };
            }
            (SettingsSection::Appearance, "show_shadows") => {
                settings.show_shadows = value_bool(value, key)?;
            }
            (SettingsSection::Appearance, "focus_ring") => {
                settings.focus_ring = value_bool(value, key)?;
            }
            (SettingsSection::Appearance, "scrollbar_mode") => {
                settings.scrollbar_mode = match value_str(value, key)? {
                    "scrolling" => fluxdb_core::ScrollbarMode::Scrolling,
                    "hover" => fluxdb_core::ScrollbarMode::Hover,
                    "always" => fluxdb_core::ScrollbarMode::Always,
                    other => {
                        return Err(format!(
                            "scrollbar_mode 只能是 scrolling、hover 或 always，收到 {other}"
                        ));
                    }
                };
            }
            (SettingsSection::Appearance, "ui_density") => {
                settings.ui_density = match value_str(value, key)? {
                    "compact" => fluxdb_core::UiDensity::Compact,
                    "standard" => fluxdb_core::UiDensity::Standard,
                    "comfortable" => fluxdb_core::UiDensity::Comfortable,
                    other => {
                        return Err(format!(
                            "ui_density 只能是 compact、standard 或 comfortable，收到 {other}"
                        ));
                    }
                };
            }
            (SettingsSection::Appearance, "ui_font_size") => {
                settings.ui_font_size = value_u32(value, key, 12, 18)?;
            }
            (SettingsSection::Appearance, "show_status_bar") => {
                settings.show_status_bar = value_bool(value, key)?;
            }
            (SettingsSection::Appearance, "reduce_motion") => {
                settings.reduce_motion = value_bool(value, key)?;
            }
            (SettingsSection::Appearance, "global_font_family") => {
                let family = value_str(value, key)?;
                if family.chars().count() > 128 {
                    return Err("global_font_family 不能超过 128 个字符".to_string());
                }
                settings.global_font_family = family.to_string();
            }
            (SettingsSection::Appearance, "button_radius") => {
                settings.button_radius = value_u8(value, key, 0, 24)?;
            }
            (SettingsSection::Appearance, "large_radius") => {
                settings.large_radius = value_u8(value, key, 0, 32)?;
            }
            (SettingsSection::Data, "data_table_page_size") => {
                let max = fluxdb_core::data_table_page_size_max();
                settings.data_table_page_size = value_u64(value, key, 1, max)?;
                if !fluxdb_core::DATA_TABLE_PAGE_SIZE_CHOICES
                    .iter()
                    .any(|(_, choice)| *choice == settings.data_table_page_size)
                {
                    return Err("data_table_page_size 只能是 100、500 或 1000".to_string());
                }
            }
            (SettingsSection::System, "performance_diagnostics") => {
                settings.performance_diagnostics = value_bool(value, key)?;
            }
            (SettingsSection::System, "log_level") => {
                settings.log_level = match value_str(value, key)? {
                    "error" => fluxdb_core::LogLevel::Error,
                    "warn" => fluxdb_core::LogLevel::Warn,
                    "info" => fluxdb_core::LogLevel::Info,
                    "debug" => fluxdb_core::LogLevel::Debug,
                    "trace" => fluxdb_core::LogLevel::Trace,
                    other => {
                        return Err(format!(
                            "log_level 只能是 error、warn、info、debug 或 trace，收到 {other}"
                        ));
                    }
                };
            }
            _ => {
                return Err(format!(
                    "字段 {key} 不属于设置分区 {}",
                    prefill.section.as_str()
                ));
            }
        }
    }
    Ok(())
}

/// 以已保存设置为基线生成提案目标值，供确认时判断草稿是否已经是目标状态。
pub fn settings_after_prefill(
    settings: &fluxdb_core::Settings,
    prefill: &SettingsPrefill,
) -> Result<fluxdb_core::Settings, String> {
    let mut target = settings.clone();
    apply_settings_prefill(&mut target, prefill)?;
    Ok(target)
}

/// 已保存设置的稳定指纹：配置提案用它做并发校验，不依赖页面或进程内 revision。
pub fn settings_fingerprint(settings: &fluxdb_core::Settings) -> u64 {
    let material = serde_json::to_string(settings).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in material.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 当前设置分区的公开值摘要；不包含路径、快捷键、危险操作清单或凭据。
pub fn settings_section_public_json(
    settings: &fluxdb_core::Settings,
    section: SettingsSection,
) -> serde_json::Value {
    let values = match section {
        SettingsSection::Editor => serde_json::json!({
            "page_size": settings.page_size,
            "results_placement": match settings.results_placement {
                fluxdb_core::ResultsPlacement::Bottom => "bottom",
                fluxdb_core::ResultsPlacement::Right => "right",
            },
            "editor_font_size": settings.editor_font_size,
            "editor_line_height": settings.editor_line_height,
            "editor_font_family": settings.editor_font_family,
            "editor_tab_width": settings.editor_tab_width,
            "editor_word_wrap": settings.editor_word_wrap,
        }),
        SettingsSection::Appearance => serde_json::json!({
            "theme": match settings.theme {
                fluxdb_core::Theme::System => "system",
                fluxdb_core::Theme::Light => "light",
                fluxdb_core::Theme::Dark => "dark",
            },
            "show_shadows": settings.show_shadows,
            "focus_ring": settings.focus_ring,
            "scrollbar_mode": match settings.scrollbar_mode {
                fluxdb_core::ScrollbarMode::Scrolling => "scrolling",
                fluxdb_core::ScrollbarMode::Hover => "hover",
                fluxdb_core::ScrollbarMode::Always => "always",
            },
            "ui_density": match settings.ui_density {
                fluxdb_core::UiDensity::Compact => "compact",
                fluxdb_core::UiDensity::Standard => "standard",
                fluxdb_core::UiDensity::Comfortable => "comfortable",
            },
            "ui_font_size": settings.effective_ui_font_size(),
            "show_status_bar": settings.show_status_bar,
            "reduce_motion": settings.reduce_motion,
            "global_font_family": settings.global_font_family,
            "button_radius": settings.button_radius,
            "large_radius": settings.large_radius,
        }),
        SettingsSection::Data => serde_json::json!({
            "data_table_page_size": settings.data_table_page_size,
        }),
        SettingsSection::System => serde_json::json!({
            "performance_diagnostics": settings.performance_diagnostics,
            "log_level": match settings.log_level {
                fluxdb_core::LogLevel::Error => "error",
                fluxdb_core::LogLevel::Warn => "warn",
                fluxdb_core::LogLevel::Info => "info",
                fluxdb_core::LogLevel::Debug => "debug",
                fluxdb_core::LogLevel::Trace => "trace",
            },
        }),
    };
    serde_json::json!({
        "section": section.as_str(),
        "values": values,
        "editable_fields": settings_section_fields(section),
    })
}

/// 确认卡使用的字段差异：`(字段, 当前值, 提案值)`。
///
/// 只比较提案里出现的字段；当前值来自已保存设置，不读取草稿，避免用户未保存
/// 修改被误当成基线。
pub fn settings_prefill_diff(
    settings: &fluxdb_core::Settings,
    prefill: &SettingsPrefill,
) -> Vec<(String, String, String)> {
    let current = settings_section_public_json(settings, prefill.section);
    let values = current.get("values").and_then(|value| value.as_object());
    prefill
        .fields
        .iter()
        .map(|(key, proposed)| {
            let before = values
                .and_then(|values| values.get(key))
                .map(json_display)
                .unwrap_or_else(|| "-".to_string());
            (key.clone(), before, json_display(proposed))
        })
        .collect()
}

fn json_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}

fn value_str<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("字段 {key} 必须是字符串"))
}

fn value_bool(value: &serde_json::Value, key: &str) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("字段 {key} 必须是布尔值"))
}

fn value_u64(
    value: &serde_json::Value,
    key: &str,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    let number = value
        .as_u64()
        .ok_or_else(|| format!("字段 {key} 必须是非负整数"))?;
    if !(min..=max).contains(&number) {
        return Err(format!("字段 {key} 必须在 {min}..={max} 之间"));
    }
    Ok(number)
}

fn value_u32(
    value: &serde_json::Value,
    key: &str,
    min: u32,
    max: u32,
) -> Result<u32, String> {
    Ok(value_u64(value, key, min as u64, max as u64)? as u32)
}

fn value_u8(value: &serde_json::Value, key: &str, min: u8, max: u8) -> Result<u8, String> {
    Ok(value_u64(value, key, min as u64, max as u64)? as u8)
}

#[cfg(test)]
mod settings_surface_tests {
    use super::*;

    fn args(section: &str, fields: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "section": section, "fields": fields })
    }

    #[test]
    fn applies_editor_fields_to_draft_only() {
        let mut settings = fluxdb_core::Settings::default();
        let prefill = parse_settings_prefill(&args(
            "editor",
            serde_json::json!({
                "editor_font_size": 18,
                "editor_word_wrap": true,
                "results_placement": "right"
            }),
        ))
        .expect("prefill");
        apply_settings_prefill(&mut settings, &prefill).expect("apply");
        assert_eq!(settings.editor_font_size, 18);
        assert!(settings.editor_word_wrap);
        assert_eq!(
            settings.results_placement,
            fluxdb_core::ResultsPlacement::Right
        );
    }

    #[test]
    fn rejects_fields_from_another_section() {
        let error = parse_settings_prefill(&args(
            "appearance",
            serde_json::json!({ "editor_font_size": 18 }),
        ))
        .expect_err("cross-section field");
        assert!(error.contains("不属于设置分区"));
    }

    #[test]
    fn rejects_sensitive_or_unlisted_fields() {
        let error = parse_settings_prefill(&args(
            "system",
            serde_json::json!({ "log_path": "/tmp/fluxdb.log" }),
        ))
        .expect_err("path is not editable");
        assert!(error.contains("不属于设置分区"));
    }

    #[test]
    fn rejects_values_not_offered_by_the_settings_ui() {
        let prefill = parse_settings_prefill(&args(
            "editor",
            serde_json::json!({ "page_size": 250 }),
        ))
        .expect("prefill");
        let mut settings = fluxdb_core::Settings::default();
        let error = apply_settings_prefill(&mut settings, &prefill).expect_err("page size");
        assert!(error.contains("100"));
    }

    #[test]
    fn public_json_does_not_expose_paths() {
        let settings = fluxdb_core::Settings::default();
        let value = settings_section_public_json(&settings, SettingsSection::System);
        let text = value.to_string();
        assert!(!text.contains("log_path"));
        assert!(!text.contains("pg_dump_path"));
        assert!(text.contains("performance_diagnostics"));
    }

    #[test]
    fn diff_uses_saved_values_as_baseline() {
        let settings = fluxdb_core::Settings {
            editor_font_size: 14,
            ..Default::default()
        };
        let prefill = parse_settings_prefill(&args(
            "editor",
            serde_json::json!({ "editor_font_size": 18 }),
        ))
        .expect("prefill");
        let diff = settings_prefill_diff(&settings, &prefill);
        assert_eq!(
            diff,
            vec![("editor_font_size".to_string(), "14".to_string(), "18".to_string())]
        );
    }

    #[test]
    fn target_settings_can_match_a_prefilled_draft() {
        let settings = fluxdb_core::Settings::default();
        let prefill = parse_settings_prefill(&args(
            "editor",
            serde_json::json!({ "editor_font_size": 18 }),
        ))
        .expect("prefill");
        let target = settings_after_prefill(&settings, &prefill).expect("target");
        assert_eq!(target.editor_font_size, 18);
        let draft = fluxdb_core::Settings {
            editor_font_size: 18,
            ..settings.clone()
        };
        assert_eq!(target, draft);
    }
}
