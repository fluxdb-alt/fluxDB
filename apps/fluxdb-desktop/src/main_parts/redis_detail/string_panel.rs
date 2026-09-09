/// String / JSON 详情值面板：只读展示已加载值，顶部「格式转换 / 复制 / 下载 / 编辑」按钮。
///
/// 「值是否完整」以 `redis_string_values` 里缓存的 `loaded_all` 为准，而不是列表 200 字符
/// preview：未完整加载时仅展示预览并提供「加载全部」，格式转换 / 复制 / 编辑按 `loaded_all`
/// 严格 gating（避免对截断预览格式化、避免把 preview 片段当可编辑真值写回覆盖完整数据）；
/// 下载恒常可点，导出完整原始字节。
fn redis_string_value_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    input: Entity<InputState>,
    applying: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let key = detail.key.clone();
    let state = this.redis_string_values.get(&(tab_id, key.clone())).cloned();
    let loaded_value = state.as_ref().map(|state| state.value.clone());
    let loaded_all = state.as_ref().map(|state| state.loaded_all).unwrap_or(false);
    let len = state.as_ref().map(|state| state.len);
    let binary = redis_value_is_binary(&detail.value);
    let editing = this.redis_string_editing == Some((tab_id, key.clone()));
    let loading = this
        .redis_string_loading
        .as_ref()
        .is_some_and(|loading| loading.0 == tab_id && loading.1 == key);

    // 查看态展示值：已加载用加载值，未加载退回列表 preview；格式菜单只影响展示，不影响可编辑原文。
    let raw_value = loaded_value.clone().unwrap_or_else(|| detail.value.clone());
    let format = this
        .redis_string_format
        .get(&(tab_id, key.clone()))
        .copied()
        .unwrap_or(RedisStringFormat::Unicode);
    let display_value = match format {
        RedisStringFormat::Unicode => raw_value.clone(),
        RedisStringFormat::Json => {
            redis_pretty_json(&raw_value).unwrap_or_else(|_| raw_value.clone())
        }
    };

    // gating：格式转换 / 复制 / 编辑只在「完整加载 + 非二进制」后可用；复制不限制长度（完整加载后
    // 即可复制，大 key 超长也可复制到剪贴板），未完整加载时格式转换禁用，避免对截断预览做格式化。
    let format_enabled = loaded_all && !binary;
    let copy_enabled = loaded_all && !binary;
    let edit_enabled = loaded_all && !binary && !applying && !editing;
    let copy_tooltip = redis_string_copy_disabled_tooltip(binary, loaded_all);
    let downloading = this.redis_string_downloading.contains(&(tab_id, key.clone()));

    let detail_for_body = detail.clone();
    redis_detail_panel(colors)
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(redis_detail_panel_title("值", colors))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(redis_string_value_hint(len, loaded_all, loading, colors))
                        .child(redis_string_format_menu(
                            tab_id,
                            key.clone(),
                            format,
                            format_enabled,
                            colors,
                            cx,
                        ))
                        .child(redis_string_copy_button(
                            tab_id,
                            key.clone(),
                            display_value.clone(),
                            copy_enabled,
                            copy_tooltip,
                            colors,
                            cx,
                        ))
                        .child(redis_string_download_button(
                            tab_id,
                            detail_for_body.clone(),
                            !downloading && !applying,
                            colors,
                            cx,
                        ))
                        .child(redis_string_edit_button(
                            tab_id,
                            detail_for_body.clone(),
                            edit_enabled,
                            colors,
                            cx,
                        )),
                ),
        )
        .child(redis_string_value_body(
            tab_id,
            key,
            detail_for_body,
            input,
            editing,
            loaded_all,
            applying,
            loading,
            display_value,
            colors,
            cx,
        ))
}

/// 值面板主体：加载中 → 编辑态 → 未完整加载（预览 + Load all/下载）→ 完整加载（只读文本）。
fn redis_string_value_body(
    tab_id: TabId,
    key: String,
    detail: RedisKeyDetail,
    input: Entity<InputState>,
    editing: bool,
    loaded_all: bool,
    applying: bool,
    loading: bool,
    display_value: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = div().flex_1().min_h(px(0.)).overflow_hidden();
    if loading {
        return body.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(loading_spinner_with_color(22., colors.muted)),
        );
    }

    if editing {
        // 编辑态：共享多行输入展示完整原文，底部提供 取消/保存。
        let detail_for_cancel = detail.clone();
        let detail_for_save = detail.clone();
        return body.child(
            div()
                .h_full()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .px_3()
                .py_2()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .w_full()
                        .child(
                            div()
                                .w_full()
                                .h_full()
                                .min_h(px(0.))
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.input_bg)
                                .overflow_hidden()
                                .child(
                                    Input::new(&input)
                                        .appearance(false)
                                        .focus_bordered(false)
                                        .disabled(applying)
                                        .w_full()
                                        .h_full()
                                        .px_2()
                                        .py_2()
                                        .font_family("Menlo")
                                        .text_size(px(13.))
                                        .line_height(px(19.)),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("redis-string-value-cancel-edit")
                                .label("取消编辑")
                                .small()
                                .outline()
                                .w(px(82.))
                                .disabled(applying)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !applying {
                                        this.cancel_redis_string_edit(
                                            tab_id,
                                            detail_for_cancel.clone(),
                                            cx,
                                        );
                                    }
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("redis-string-value-save")
                                .label("保存")
                                .small()
                                .primary()
                                .w(px(72.))
                                .disabled(applying)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !applying {
                                        this.request_redis_key_value_apply(
                                            tab_id,
                                            detail_for_save.clone(),
                                            cx,
                                        );
                                    }
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        );
    }

    if !loaded_all {
        // 未完整加载：展示预览 + 省略号，底部提供「加载全部」（下载已在顶栏工具栏常驻）。
        return body.child(
            div()
                .h_full()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("redis-string-value-preview-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_x_scroll()
                        .overflow_y_scrollbar()
                        .font_family("Menlo")
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .text_color(colors.text)
                        .child(
                            div().px_3().py_2().child(if display_value.is_empty() {
                                div().text_color(colors.muted).child("(空值)").into_any_element()
                            } else {
                                format!("{display_value}\n…").into_any_element()
                            }),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("redis-string-value-load-all")
                                .label("加载全部")
                                .small()
                                .outline()
                                .w(px(88.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_redis_string_value_load(
                                        tab_id,
                                        key.clone(),
                                        true,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        );
    }

    // 完整加载：只读滚动文本（长文本按面板宽度折行，纵向滚动；超长无断词串保留横向滚动兜底）。
    body.child(
        div()
            .id("redis-string-value-scroll")
            .size_full()
            .overflow_x_scroll()
            .overflow_y_scrollbar()
            .font_family("Menlo")
            .text_size(px(13.))
            .line_height(px(19.))
            .text_color(colors.text)
            .child(
                div().px_3().py_2().child(if display_value.is_empty() {
                    div().text_color(colors.muted).child("(空值)").into_any_element()
                } else {
                    display_value.into_any_element()
                }),
            ),
    )
}

/// 复制按钮：`enabled` 决定样式与点击行为；禁用时用 tooltip 说明原因。
fn redis_string_copy_button(
    tab_id: TabId,
    key: String,
    value: String,
    enabled: bool,
    tooltip: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!(
            "redis-string-value-copy-{}-{}",
            tab_id.0, key
        )))
        .h(px(24.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .when_some(tooltip, |this, message| {
            this.tooltip(move |window, cx| Tooltip::new(message.clone()).build(window, cx))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if enabled {
                    cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
                    this.show_message("值已复制", AppMessageKind::Success, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(
            AppIcon::Copy,
            13.,
            if enabled { colors.muted } else { colors.border },
        ))
        .child("复制")
}

/// 下载按钮：持久可点，把 String / JSON 值导出为文件（大 key 顶栏始终可用，不受完整加载限制）。
fn redis_string_download_button(
    tab_id: TabId,
    detail: RedisKeyDetail,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(24.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let detail_for_download = detail.clone();
                if !this
                    .redis_string_downloading
                    .contains(&(tab_id, detail_for_download.key.clone()))
                {
                    this.request_redis_string_download(tab_id, detail_for_download, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(
            AppIcon::Save,
            13.,
            if enabled { colors.muted } else { colors.border },
        ))
        .child("下载")
}

/// 编辑按钮：`enabled` 决定是否进入编辑态。
fn redis_string_edit_button(
    tab_id: TabId,
    detail: RedisKeyDetail,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(24.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.begin_redis_string_edit(tab_id, detail.clone(), window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(
            AppIcon::Edit,
            13.,
            if enabled { colors.muted } else { colors.border },
        ))
        .child("编辑")
}

/// 复制按钮禁用原因：二进制 / 未完整加载 → 提示先加载全部或改用下载。
fn redis_string_copy_disabled_tooltip(binary: bool, loaded_all: bool) -> Option<String> {
    if binary {
        return Some("二进制值无法复制".to_string());
    }
    if !loaded_all {
        return Some("请先加载全部后再复制".to_string());
    }
    None
}

/// 格式转换下拉：Unicode（原文）/ JSON（pretty，仅合法 JSON 生效）。
/// `enabled=false`（未完整加载）时禁用下拉，避免对截断预览做格式化。
fn redis_string_format_menu(
    tab_id: TabId,
    key: String,
    format: RedisStringFormat,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Popover {
    let view = cx.entity().downgrade();
    Popover::new((
        gpui::ElementId::Name(format!("redis-string-format-popover-{}", tab_id.0).into()),
        key.clone(),
    ))
    .appearance(false)
    .anchor(Anchor::TopRight)
    .trigger(
        Button::new((
            gpui::ElementId::Name(format!("redis-string-format-trigger-{}", tab_id.0).into()),
            key.clone(),
        ))
        .ghost()
        .xsmall()
        .h(px(24.))
        .disabled(!enabled)
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_1()
                .text_size(px(12.))
                .text_color(if enabled { colors.text } else { colors.muted })
                .child(app_icon(AppIcon::AlignLeft, 13., colors.muted))
                .child("格式转换")
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        ),
    )
    .content(move |_, _, cx| {
        let popover = cx.entity();
        let mut menu = div()
            .w(px(140.))
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel_bg)
            .shadow_md()
            .overflow_hidden();
        for (label, value) in [
            ("Unicode", RedisStringFormat::Unicode),
            ("JSON", RedisStringFormat::Json),
        ] {
            menu = menu.child(redis_string_format_menu_item(
                view.clone(),
                popover.clone(),
                tab_id,
                key.clone(),
                label,
                value,
                format == value,
                colors,
            ));
        }
        menu
    })
}

fn redis_string_format_menu_item(
    view: WeakEntity<NavicatMain>,
    popover: Entity<gpui_component::popover::PopoverState>,
    tab_id: TabId,
    key: String,
    label: &'static str,
    value: RedisStringFormat,
    active: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if active { colors.text } else { colors.muted })
        .bg(if active { colors.hover } else { colors.panel_bg })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .on_mouse_down(
            MouseButton::Left,
            move |_, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.redis_string_format.insert((tab_id, key.clone()), value);
                    cx.notify();
                });
                popover.update(cx, |state, cx| {
                    state.dismiss(window, cx);
                });
                cx.stop_propagation();
            },
        )
        .child(label)
}

fn redis_string_value_hint(
    len: Option<u64>,
    loaded_all: bool,
    loading: bool,
    colors: UiColors,
) -> Div {
    if loading {
        return div()
            .text_size(px(11.))
            .text_color(colors.muted)
            .child("加载中…");
    }
    match len {
        Some(len) => {
            let label = if loaded_all {
                format!("{len} 字节")
            } else {
                format!("{len} 字节 · 未完整加载")
            };
            div().text_size(px(11.)).text_color(colors.muted).child(label)
        }
        None => div()
            .text_size(px(11.))
            .text_color(colors.muted)
            .child("加载中…"),
    }
}

fn redis_string_value_load_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 50)
}

/// 下载任务 id（与加载用不同高位位掩码，避免同一 tab 的下载/加载任务互相覆盖）。
fn redis_string_download_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 51)
}
