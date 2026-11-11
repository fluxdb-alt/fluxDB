fn settings_appearance_panel(
    settings: &Settings,
    theme_mode: ThemeMode,
    custom_themes: &BTreeSet<String>,
    colors: UiColors,
    font_size_slider: Entity<SliderState>,
    ui_font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    ui_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    code_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let accent: gpui::Rgba = ComponentTheme::global(cx).primary.into();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group(tr("app.settings.appearance.group.language"), colors)
                .child(settings_language_row(&settings.locale, colors, cx)),
        )
        .child(
            settings_panel_group(tr("app.settings.appearance.group.theme"), colors)
                .child(settings_theme_mode_row(theme_mode, colors, cx))
                .child(settings_theme_palette(
                    settings,
                    theme_mode,
                    custom_themes,
                    colors,
                    cx,
                )),
        )
        .child(
            settings_panel_group(tr("app.settings.appearance.group.font"), colors)
                // 与设计稿一致：界面字体与界面字号收在同一行。
                .child(settings_ui_font_row(
                    settings,
                    &ui_font_select,
                    ui_font_size_slider,
                    colors,
                    window,
                    cx,
                ))
                // 与设计稿一致：代码字体、字号滑杆与行高输入收在同一行。
                .child(settings_code_font_row(
                    &code_font_select,
                    settings.editor_font_size,
                    settings.editor_line_height,
                    font_size_slider,
                    &line_height_input,
                    colors,
                    window,
                    cx,
                ))
                .child(settings_font_preview(settings, colors))
                .child(settings_font_note(colors)),
        )
        .child(settings_appearance_preview(colors, accent))
}

fn settings_appearance_header_actions(
    saved: &Settings,
    draft: &Settings,
    font_size_slider: Entity<SliderState>,
    ui_font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let dirty = settings_section_changed(
        SettingsPanelSection::Appearance,
        saved,
        draft,
    );
    let reset_line_height_input = line_height_input.clone();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new("settings-appearance-reset")
                .label(tr("app.settings.reset_defaults"))
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.reset_appearance_settings(window, cx);
                    // 重置草稿后同步字号/行高控件，避免界面继续显示旧值。
                    let font_size = this.settings_editor_draft.editor_font_size;
                    font_size_slider.update(cx, |slider, cx| {
                        slider.set_value(font_size as f32, window, cx)
                    });
                    let ui_font_size = this.settings_editor_draft.effective_ui_font_size();
                    ui_font_size_slider.update(cx, |slider, cx| {
                        slider.set_value(ui_font_size as f32, window, cx)
                    });
                    let line_height =
                        this.settings_editor_draft.editor_line_height.to_string();
                    reset_line_height_input.update(cx, |input, cx| {
                        input.set_value(line_height, window, cx)
                    });
                })),
        )
        .child(
            Button::new("settings-appearance-save")
                .label(tr("app.common.save"))
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .primary()
                .disabled(!dirty)
                .debug_selector(|| "settings-appearance-save".to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    // 行高输入框允许临时为空/非法，保存时统一收敛并回填。
                    if let Ok(line_height) = line_height_input
                        .read(cx)
                        .value()
                        .trim()
                        .parse::<u32>()
                    {
                        let line_height = line_height.clamp(13, 28);
                        this.settings_editor_draft.editor_line_height = line_height;
                        line_height_input.update(cx, |input, cx| {
                            input.set_value(line_height.to_string(), window, cx)
                        });
                    }
                    this.confirm_appearance_settings_saved(cx);
                })),
        )
}

fn settings_section_header_actions(
    section: SettingsPanelSection,
    saved: &Settings,
    draft: &Settings,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let dirty = settings_section_changed(section, saved, draft);
    let save_selector = format!("settings-{section:?}-save");
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new(SharedString::from(format!("settings-{section:?}-reset")))
                .label(tr("app.settings.reset_defaults"))
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .on_click(cx.listener(move |this, _, _, cx| {
                    reset_settings_section(this, section, cx);
                })),
        )
        .child(
            Button::new(SharedString::from(format!("settings-{section:?}-save")))
                .label(tr("app.common.save"))
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .primary()
                .disabled(!dirty)
                .debug_selector(move || save_selector.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    save_settings_section_from_ui(this, section, cx);
                })),
        )
}

fn settings_section_header_actions_disabled(colors: UiColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new("settings-disabled-reset")
                .label(tr("app.settings.appearance.restore_defaults"))
                .small()
                .rounded(colors.radius)
                .disabled(true),
        )
        .child(
            Button::new("settings-disabled-save")
                .label(tr("app.common.save"))
                .small()
                .rounded(colors.radius)
                .primary()
                .disabled(true),
        )
}

fn settings_theme_mode_row(
    theme_mode: ThemeMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    settings_action_row_with_id(
        "settings-theme-mode",
        tr("app.settings.appearance.theme_mode"),
        tr("app.settings.appearance.theme_mode_detail"),
        AppIcon::Settings,
        colors,
    )
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .p_1()
            .rounded(colors.radius)
            .bg(colors.panel_alt)
            .child(settings_theme_mode_button(
                tr("app.common.light"),
                theme_mode == ThemeMode::Light,
                ThemeMode::Light,
                colors,
                cx,
            ))
            .child(settings_theme_mode_button(
                tr("app.common.dark"),
                theme_mode == ThemeMode::Dark,
                ThemeMode::Dark,
                colors,
                cx,
            ))
            .child(
                Button::new("settings-theme-system")
                    .label(tr("app.common.system"))
                    .small()
                    .rounded(colors.radius)
                    .disabled(true),
            ),
    )
}

/// 语言列表完全来自已加载语言包；这里不维护语言枚举或固定 branch。
fn settings_language_row(
    current: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let follow_system = current.is_empty();
    let active_locale = fluxdb_i18n::effective_locale(current);
    let current_label = if follow_system {
        tr("app.settings.appearance.follow_system")
    } else {
        fluxdb_i18n::locales()
            .into_iter()
            .find(|locale| locale.locale.eq_ignore_ascii_case(&active_locale))
            .map(|locale| SharedString::from(locale.native_name))
            .unwrap_or_else(|| SharedString::from(active_locale.clone()))
    };
    let owner = cx.entity().downgrade();
    let dropdown = Button::new("settings-language-dropdown")
        .debug_selector(|| "settings-language-dropdown".into())
        .small()
        .cursor_pointer()
        .min_w(px(170.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(current_label)
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .check_side(gpui_component::Side::Right)
                .min_w(px(210.));
            let system_owner = owner.clone();
            menu = menu.item(
                PopupMenuItem::element(move |_, _| {
                    div()
                        .id("settings-language-option:system")
                        .debug_selector(|| "settings-language-option:system".into())
                        .child(tr("app.settings.appearance.follow_system"))
                })
                .checked(follow_system)
                .on_click(move |_, window, cx| {
                    let _ = system_owner.update(cx, |this, cx| {
                        this.settings_editor_draft.locale.clear();
                        this.preview_locale(window, cx);
                    });
                }),
            );
            for locale in fluxdb_i18n::locales() {
                let option_owner = owner.clone();
                let selected = !follow_system
                    && active_locale.eq_ignore_ascii_case(&locale.locale);
                let option_locale = locale.locale.clone();
                let option_id = format!("settings-language-option:{}", option_locale);
                let debug_id = option_id.clone();
                let label = SharedString::from(locale.native_name);
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(option_id.clone())
                            .debug_selector({
                                let id = debug_id.clone();
                                move || id.clone()
                            })
                            .child(label.clone())
                    })
                    .checked(selected)
                    .on_click(move |_, window, cx| {
                        let _ = option_owner.update(cx, |this, cx| {
                            this.settings_editor_draft.locale = option_locale.clone();
                            this.preview_locale(window, cx);
                        });
                    }),
                );
            }
            menu
        });
    settings_action_row_with_id(
        "settings-language",
        tr("app.settings.appearance.language"),
        tr("app.settings.appearance.language_detail"),
        AppIcon::Text,
        colors,
    )
    .child(dropdown)
}

fn settings_theme_palette(
    settings: &fluxdb_core::Settings,
    theme_mode: ThemeMode,
    custom_themes: &BTreeSet<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = if theme_mode == ThemeMode::Dark {
        settings.dark_theme.as_str()
    } else {
        settings.light_theme.as_str()
    };
    let configs = theme_configs_for_mode(theme_mode, cx);
    let bundled_names = bundled_theme_names();
    let (custom, rest): (Vec<_>, Vec<_>) = configs
        .into_iter()
        .partition(|config| custom_themes.contains(config.name.as_ref()));
    // 删除/改名后注册表条目会保留到文件监听重载完成，这里按磁盘与内置名单
    // 过滤残留，避免旧主题短暂出现在「内置主题」里。
    let builtin: Vec<_> = rest
        .into_iter()
        .filter(|config| config.is_default || bundled_names.contains(config.name.as_ref()))
        .collect();
    let custom_count = custom.len();
    let custom_cards: Vec<Div> = if custom.is_empty() {
        vec![div()
            .w_full()
            .py(px(14.))
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(tr("app.settings.appearance.theme_custom_empty"))]
    } else {
        custom
            .into_iter()
            .map(|config| theme_palette_card(config, selected, theme_mode, colors, cx, true))
            .collect()
    };
    // 主题管理动作收进「自定义主题」分组标题右侧：紧贴它们作用的对象，
    // 也不再挤在模式说明和内置主题之间形成多余的一条横带。
    // 编辑/删除都放在每张自定义主题卡的悬停角标上，分组标题只保留目录与新建操作。
    let custom_actions = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            Button::new("settings-theme-open-dir")
                .small()
                .ghost()
                .label(tr("app.settings.appearance.theme_open_dir"))
                .on_click(cx.listener(|this, _, _, cx| this.reveal_themes_dir(cx))),
        )
        .child(
            Button::new("settings-theme-rescan")
                .small()
                .ghost()
                .label(tr("app.settings.appearance.theme_rescan"))
                .on_click(cx.listener(|this, _, _, cx| this.rescan_custom_themes(cx))),
        )
        .child(
            Button::new("settings-theme-import")
                .small()
                .ghost()
                .debug_selector(|| "settings-theme-import".into())
                .label(tr("app.settings.appearance.theme_import"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.import_theme(window, cx);
                })),
        )
        .child(
            Button::new("settings-theme-new")
                .small()
                .primary()
                .debug_selector(|| "settings-theme-new".into())
                .label(tr("app.settings.appearance.theme_new"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_theme_editor(window, cx);
                })),
        );
    div()
        .flex()
        .flex_col()
        .gap_3()
        // 与设置行保持同样的 12px 横向内边距；上下留白把主题区和模式行、下一个分组分开。
        .px_3()
        .pt(px(12.))
        .pb(px(12.))
        .child(theme_palette_section(
            tr("app.settings.appearance.theme_builtin"),
            builtin.len(),
            colors,
            None,
            builtin
                .into_iter()
                .map(|config| theme_palette_card(config, selected, theme_mode, colors, cx, false))
                .collect(),
        ))
        .child(theme_palette_section(
            tr("app.settings.appearance.theme_custom"),
            custom_count,
            colors,
            Some(custom_actions.into_any_element()),
            custom_cards,
        ))
}

fn theme_palette_section(
    title: SharedString,
    count: usize,
    colors: UiColors,
    actions: Option<gpui::AnyElement>,
    cards: Vec<Div>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(title),
                )
                .child(
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded_full()
                        .bg(colors.panel_alt)
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(count.to_string()),
                )
                .child(div().flex_1().h(px(1.)).bg(colors.border_soft))
                .when_some(actions, |this, actions| this.child(actions)),
        )
        .child(div().flex().flex_wrap().gap_3().children(cards))
}

/// 主题卡：中性底色做卡片框架，内部嵌一块按该主题配色绘制的迷你界面，
/// 名称用应用正文色，避免五张卡各染一块背景色、看起来像拼贴。
fn theme_palette_card(
    config: std::rc::Rc<ThemeConfig>,
    selected: &str,
    theme_mode: ThemeMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
    deletable: bool,
) -> Div {
    let is_selected = config.name == selected;
    let name = config.name.to_string();
    let pick = |value: &Option<SharedString>, fallback: gpui::Rgba| -> Hsla {
        value
            .as_ref()
            .and_then(|hex| parse_theme_hex_color(hex))
            .map(Hsla::from)
            .unwrap_or_else(|| fallback.into())
    };
    let background = pick(&config.colors.background, colors.panel_bg);
    let sidebar = pick(&config.colors.sidebar, colors.sidebar_bg);
    let border = pick(&config.colors.border, colors.border);
    let text = pick(&config.colors.foreground, colors.text);
    let muted = pick(&config.colors.muted_foreground, colors.muted);
    let accent = pick(&config.colors.accent, colors.text);
    let accent_foreground = pick(&config.colors.accent_foreground, colors.panel_bg);

    let line = |width: f32, color: Hsla, opacity: f32| {
        div()
            .h(px(3.))
            .w(px(width))
            .rounded_full()
            .bg(color.opacity(opacity))
    };
    // 迷你界面：左窄侧栏 + 右内容行，内容整体垂直居中，不留大片空白。
    let preview = div()
        .flex()
        .h(px(54.))
        .w_full()
        .overflow_hidden()
        .rounded(px(4.))
        .border(px(1.))
        .border_color(border)
        .bg(background)
        .child(
            div()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(4.))
                .w(px(42.))
                .h_full()
                .px(px(6.))
                .bg(sidebar)
                .border_r(px(1.))
                .border_color(border)
                .child(line(22., muted, 0.5))
                .child(line(16., accent, 0.85))
                .child(line(24., muted, 0.5)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(4.))
                .flex_1()
                .h_full()
                .px(px(7.))
                .child(line(46., text, 0.7))
                .child(line(60., muted, 0.35))
                .child(
                    div()
                        .mt(px(2.))
                        .h(px(6.))
                        .w(px(26.))
                        .rounded_full()
                        .bg(accent),
                ),
        );

    let card = Button::new(SharedString::from(format!("theme-palette-{}", config.name)))
        .w_full()
        .h(px(94.))
        .p_0()
        .rounded(colors.radius)
        .border(px(if is_selected { 1.5 } else { 1. }))
        .border_color(if is_selected {
            accent
        } else {
            colors.border.into()
        })
        .bg(colors.panel_bg)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_theme_name(theme_mode, name.clone(), cx);
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .size_full()
                .p(px(8.))
                .child(preview)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(ui_text_size(11.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(config.name.clone()),
                        )
                        .when(is_selected, |this| {
                            this.child(
                                div()
                                    .size(px(14.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(accent)
                                    .child(app_icon(AppIcon::Check, 9., accent_foreground.into())),
                            )
                        }),
                ),
        );

    // 选中态再加一圈淡淡的强调色光晕；未选中保留同尺寸透明边距，网格不跳动。
    let mut wrapper = div()
        .w(px(172.))
        .p(px(2.))
        .rounded(px(6.))
        .bg(if is_selected {
            accent.opacity(0.14)
        } else {
            gpui::transparent_black()
        })
        .child(card);
    if deletable {
        // 悬停整张卡才显示编辑/删除角标：group_hover 由卡片容器驱动，
        // 角标是容器的兄弟节点，点它不会冒泡到选中卡片的点击处理。
        let group = SharedString::from(format!("theme-card-{}", config.name));
        let edit_name = config.name.to_string();
        let delete_name = config.name.to_string();
        wrapper = wrapper.relative().group(group.clone()).child(
            div()
                .absolute()
                .top(px(7.))
                .right(px(7.))
                .flex()
                .items_center()
                .gap(px(2.))
                .opacity(0.)
                .group_hover(group, |this| this.opacity(1.))
                .child(
                    Button::new(SharedString::from(format!(
                        "theme-card-edit-{}",
                        config.name
                    )))
                    .small()
                    .ghost()
                    .p(px(4.))
                    .rounded(px(4.))
                    .bg(colors.panel_bg.opacity(0.9))
                    .tooltip(tr("app.settings.appearance.theme_edit"))
                    .debug_selector(|| "settings-theme-card-edit".into())
                    .child(app_icon(AppIcon::Edit, 12., colors.text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_theme_editor_for(Some(edit_name.clone()), window, cx);
                        cx.stop_propagation();
                    })),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "theme-card-delete-{}",
                        config.name
                    )))
                    .small()
                    .ghost()
                    .p(px(4.))
                    .rounded(px(4.))
                    .bg(colors.panel_bg.opacity(0.9))
                    .tooltip(tr("app.settings.appearance.theme_delete"))
                    .debug_selector(|| "settings-theme-card-delete".into())
                    .child(app_icon(AppIcon::Trash, 12., colors.danger))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_custom_theme(delete_name.clone(), cx);
                        cx.stop_propagation();
                    })),
                ),
        );
    }
    wrapper
}

fn settings_font_select_widget(
    select: &Entity<SelectState<SearchableVec<FontChoice>>>,
    colors: UiColors,
) -> Div {
    div().w(px(210.)).child(
        Select::new(select)
            .small()
            .w_full()
            .menu_width(px(280.))
            .menu_max_h(px(360.))
            .search_placeholder(tr("app.settings.appearance.font_search_placeholder"))
            .text_color(colors.text),
    )
}

/// 界面字体与界面字号共用一行：字体下拉 + 字号滑杆 + 当前值。
fn settings_ui_font_row(
    settings: &Settings,
    select: &Entity<SelectState<SearchableVec<FontChoice>>>,
    slider: Entity<SliderState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let size = settings.effective_ui_font_size();
    // 旧配置没有显式字号时，滑杆按回退后的实际值展示，避免手柄停在 0。
    if (slider.read(cx).value().end() - size as f32).abs() > f32::EPSILON {
        slider.update(cx, |state, cx| state.set_value(size as f32, window, cx));
    }
    settings_row_container("settings-ui-font", colors)
        // 字号可放大到 18px，行高留最小高度并允许内容撑开，避免说明文字被裁切。
        .min_h(px(52.))
        .child(settings_row_label(
            tr("app.settings.appearance.ui_font_and_size"),
            tr("app.settings.appearance.ui_font_and_size_detail"),
            AppIcon::Text,
            colors,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(settings_font_select_widget(select, colors))
                .child(div().w(px(120.)).child(Slider::new(&slider).horizontal()))
                .child(
                    div()
                        .w(px(42.))
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(format!("{size} px")),
                ),
        )
}

/// 代码字体、字号与行高共用一行：字体下拉、字号滑杆、行高输入依次排列。
#[allow(
    clippy::too_many_arguments,
    reason = "设置行参数逐项对应 GPUI 控件契约，打包成结构体反而隐藏调用点差异"
)]
fn settings_code_font_row(
    code_font_select: &Entity<SelectState<SearchableVec<FontChoice>>>,
    size: u32,
    line_height: u32,
    slider: Entity<SliderState>,
    line_height_input: &Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let size = size.clamp(10, 24);
    if (slider.read(cx).value().end() - size as f32).abs() > f32::EPSILON {
        slider.update(cx, |state, cx| state.set_value(size as f32, window, cx));
    }
    // 输入框允许编辑中为空或非法值，失焦且草稿变化时才回填合法值。
    let expected = line_height.clamp(13, 28).to_string();
    let focused = line_height_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    if !focused && line_height_input.read(cx).value().to_string() != expected {
        line_height_input.update(cx, |input, cx| {
            input.set_value(expected.clone(), window, cx)
        });
    }
    let focus_border = if colors.is_dark {
        rgb(0x8ab4ff)
    } else {
        rgb(0x111111)
    };
    settings_row_container("settings-code-font-size", colors)
        .min_h(px(68.))
        .py_2()
        .child(settings_row_label(
            tr("app.settings.appearance.code_font_and_size"),
            tr("app.settings.appearance.code_font_detail"),
            AppIcon::FileSql,
            colors,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(settings_font_select_widget(code_font_select, colors))
                .child(div().w(px(200.)).child(Slider::new(&slider).horizontal()))
                .child(
                    div()
                        .w(px(42.))
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(format!("{size} px")),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(tr("app.settings.appearance.code_line_height")),
                )
                .child(
                    div()
                        .w(px(64.))
                        .h(px(28.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(if focused { focus_border } else { colors.border })
                        .bg(colors.input_bg)
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .cursor_text()
                        .child(
                            Input::new(line_height_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .px_2()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.text),
                        ),
                ),
        )
}

/// 字体预览直接读取草稿，选择字体/字号后无需保存即可看到效果。
fn settings_font_preview(settings: &Settings, colors: UiColors) -> Div {
    let ui_family = if settings.global_font_family.trim().is_empty() {
        ".SystemUIFont".to_string()
    } else {
        settings.global_font_family.trim().to_string()
    };
    let code_family = code_font_family(settings);
    let code_size = settings.editor_font_size.clamp(10, 24);
    let code_line_height = settings.editor_line_height.clamp(13, 28);
    div()
        // 设计稿把两段预览收在带边框的圆角框里，和上方设置行区分开。
        .mx_4()
        .my_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .child(
            div()
                .p_3()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.settings.appearance.ui_font_preview")),
                )
                .child(
                    div()
                        .font_family(ui_family)
                        .text_size(ui_text_size(13.))
                        .line_height(ui_text_size(20.))
                        .text_color(colors.text)
                        .child(
                            tr("app.settings.appearance.ui_font_preview_text"),
                        ),
                ),
        )
        .child(
            div()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.settings.appearance.code_font_preview")),
                )
                .child(
                    div()
                        .font_family(code_family)
                        .text_size(px(code_size as f32))
                        .line_height(px(code_line_height as f32))
                        .text_color(colors.text)
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(div().text_color(colors.info).child("SELECT"))
                                .child("id, name, created_at")
                                .child(div().text_color(colors.info).child("FROM"))
                                .child("customers"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(div().text_color(colors.info).child("WHERE"))
                                .child("status =")
                                .child(div().text_color(colors.success).child("'active'")),
                        ),
                ),
        )
}

/// 说明字体按名称匹配与回退行为，避免用户误以为缺字是渲染故障。
fn settings_font_note(colors: UiColors) -> Div {
    div()
        .px_4()
        .pb_3()
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon(AppIcon::Info, 12., colors.muted))
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.settings.appearance.font_note")),
        )
}

fn settings_theme_mode_button(
    label: impl Into<SharedString>,
    active: bool,
    mode: ThemeMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    let label = label.into();
    Button::new(SharedString::from(format!(
        "settings-theme-mode-{mode:?}"
    )))
        .label(label)
        .small()
        .rounded(colors.radius)
        .when(active, |button| button.primary())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_theme_mode(mode, cx);
            this.show_message(
                if mode == ThemeMode::Dark {
                    tr("app.settings.appearance.dark_preview")
                } else {
                    tr("app.settings.appearance.light_preview")
                },
                AppMessageKind::Success,
                cx,
            );
        }))
}

fn settings_appearance_preview(colors: UiColors, accent: gpui::Rgba) -> GroupBox {
    settings_panel_group(tr("app.settings.appearance.group.preview"), colors).child(
        div()
            .border_t_1()
            .border_color(colors.border_soft)
            .p_3()
            .flex()
            .gap_3()
            .child(
                div()
                    .w(px(150.))
                    .h(px(176.))
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border_soft)
                    .bg(colors.panel_alt)
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.muted)
                            .child(tr("app.settings.appearance.preview_connections")),
                    )
                    .child(settings_preview_tree_item("Local MySQL", true, accent, colors))
                    .child(settings_preview_tree_item("orders", false, accent, colors))
                    .child(settings_preview_tree_item("users", false, accent, colors)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(176.))
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border_soft)
                    .bg(colors.panel_bg)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(div().h(px(4.)).w_full().bg(accent))
                    .child(
                        div()
                            .h(px(38.))
                            .px_3()
                            .border_b_1()
                            .border_color(colors.border_soft)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(ui_text_size(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(colors.text)
                                    .child("orders"),
                            )
                            .child(
                                div()
                                    .h(px(24.))
                                    .px_2()
                                    .rounded(colors.radius)
                                    .bg(accent)
                                    .flex()
                                    .items_center()
                                    .text_size(ui_text_size(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xffffff))
                                    .child(tr("app.settings.appearance.preview_filter")),
                            ),
                    )
                    .child(settings_preview_table(colors)),
            ),
    )
}

fn settings_preview_tree_item(
    label: &'static str,
    active: bool,
    accent: gpui::Rgba,
    colors: UiColors,
) -> Div {
    div()
        .h(px(28.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .bg(if active {
            colors.tree_selected
        } else {
            colors.panel_alt
        })
        .child(
            div()
                .size(px(7.))
                .rounded_full()
                .bg(if active { accent } else { colors.muted }),
        )
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_weight(if active {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if active { colors.text } else { colors.muted })
                .child(label),
        )
}

fn settings_preview_table(colors: UiColors) -> Div {
    let rows = [
        ("1001", "Fusuwei", "paid", "$120.00"),
        ("1002", "Cloud", "pending", "$88.40"),
        ("1003", "Studio", "paid", "$310.20"),
    ];
    let mut table = div().flex().flex_col().p_3();
    table = table.child(settings_preview_table_row(
        ("id", "customer", "status", "total"),
        true,
        colors,
    ));
    for row in rows {
        table = table.child(settings_preview_table_row(row, false, colors));
    }
    table
}

fn settings_preview_table_row(
    row: (&'static str, &'static str, &'static str, &'static str),
    header: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(if header { 30. } else { 28. }))
        .border_1()
        .border_color(colors.border_soft)
        .bg(if header { colors.panel_alt } else { colors.panel_bg })
        .flex()
        .items_center()
        .text_size(ui_text_size(11.))
        .text_color(if header { colors.muted } else { colors.text })
        .child(settings_preview_table_cell(row.0, header, colors))
        .child(settings_preview_table_cell(row.1, header, colors))
        .child(settings_preview_table_cell(row.2, header, colors))
        .child(settings_preview_table_cell(row.3, header, colors))
}

fn settings_preview_table_cell(text: &'static str, header: bool, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .px_2()
        .font_family(if header { "JetBrains Mono" } else { "Inter" })
        .font_weight(if header {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(if header { colors.muted } else { colors.text })
        .child(text)
}

