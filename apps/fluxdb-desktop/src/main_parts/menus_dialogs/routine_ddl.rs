fn routine_ddl_modal(
    form: PendingRoutineDdl,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let ddl = form.ddl.clone();
    let editor = window.use_keyed_state("routine-ddl-content", cx, {
        let ddl = ddl.clone().unwrap_or_default();
        move |window, cx| {
            gpui_component::input::TextareaState::new(window, cx).default_value(ddl)
        }
    });
    if let Some(ddl) = ddl.as_ref() {
        editor.update(cx, |state, cx| {
            if state.value().as_ref() != ddl {
                state.set_value(ddl.clone(), window, cx);
            }
        });
    }
    let title = tr("app.routine_ddl.title").replace("%{name}", &form.object_path.name);
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.6)
        } else {
            opaque_grey(0.75, 0.3)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_routine_ddl(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id("RoutineDdlModal")
                .w(px(920.))
                .h(px(620.))
                .rounded(px(12.))
                .border_1()
                .border_color(colors.border)
                .bg(menu_surface_bg(colors))
                .shadow(vec![box_shadow(
                    px(0.),
                    px(24.),
                    px(64.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.5 } else { 0.2 }),
                )])
                .flex()
                .flex_col()
                .overflow_hidden()
                .key_context("RoutineDdlModal")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_routine_ddl(cx);
                    cx.stop_propagation();
                }))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(app_icon(AppIcon::FileSql, 17., colors.text))
                                .child(
                                    div()
                                        .text_size(ui_text_size(15.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(title),
                                ),
                        )
                        .child(
                            user_admin_icon_button(AppIcon::Close, colors)
                                .id("routine-ddl-close")
                                .aria_label(tr("app.common.close"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.close_routine_ddl(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .p_4()
                        .child(if let Some(error) = form.error {
                            div()
                                .debug_selector(|| "routine-ddl-error".into())
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.danger)
                                .p_3()
                                .text_color(colors.danger)
                                .child(error)
                                .into_any_element()
                        } else if ddl.is_none() {
                            div()
                                .debug_selector(|| "routine-ddl-loading".into())
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_color(colors.muted)
                                .child(loading_spinner_with_color(14., colors.muted))
                                .child(tr("app.table_info.loading"))
                                .into_any_element()
                        } else {
                            div()
                                .size_full()
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.border_soft)
                                .bg(colors.panel_alt)
                                .overflow_hidden()
                                .child(
                                    div()
                                        .debug_selector(|| "routine-ddl-content".into())
                                        .size_full()
                                        .child(
                                            gpui_component::input::Textarea::new(&editor)
                                                .appearance(false)
                                                .bordered(false)
                                                .readonly(true)
                                                .text_size(ui_text_size(12.))
                                                .font_family(EDITOR_FONT)
                                                .p_2()
                                                .size_full(),
                                        ),
                                )
                                .into_any_element()
                        }),
                )
                .child(
                    div()
                        .h(px(60.))
                        .px_5()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("routine-ddl-copy")
                                .label(tr("app.table_info.copy_ddl"))
                                .outline()
                                .disabled(ddl.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(ddl) = this
                                        .pending_routine_ddl
                                        .as_ref()
                                        .and_then(|form| form.ddl.clone())
                                    {
                                        cx.write_to_clipboard(ClipboardItem::new_string(ddl));
                                        this.show_message(
                                            tr("app.table_info.copied_ddl"),
                                            AppMessageKind::Success,
                                            cx,
                                        );
                                    }
                                })),
                        )
                        .child(
                            Button::new("routine-ddl-close-footer")
                                .label(tr("app.common.close"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.close_routine_ddl(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}
