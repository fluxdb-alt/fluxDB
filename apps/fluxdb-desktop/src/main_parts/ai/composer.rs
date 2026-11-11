use gpui_component::ElementExt as _;

gpui::actions!(ai_composer, [SendAiMessage]);

const AI_MESSAGE_SEND_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd-enter"
} else {
    "ctrl-enter"
};

// 输入区独立组织，消息时间线不承担候选列表与多行编辑布局。
fn ai_chat_composer(
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let chat = this.controller.state().ai_conversation.clone();
    let loading = this.ai_chat.loading;
    let running = this.ai_chat.session_running(chat.session_id.as_deref());
    let cancel_requested = this
        .ai_chat
        .cancel
        .as_ref()
        .is_some_and(|flag| flag.is_cancelled());
    let ready = !loading
        && !this.ai_chat.attachment_loading
        && !this.ai_chat.recovery_pending
        && !this.ai_chat.recovery_failed
        && this.ai_chat._io.is_none();
    let input = this.ai_chat.input.clone();
    let send_shortcut = shortcut_display(AI_MESSAGE_SEND_SHORTCUT);
    let panel_height = if this.ai_chat.panel_height > px(0.) {
        this.ai_chat.panel_height
    } else {
        window.viewport_size().height
    };
    let reference_height = if chat.context_references.is_empty() {
        px(0.)
    } else {
        this.ai_chat.reference_height
    };
    // 12px 正文使用组件 1.5 倍行高；为发送按钮行和引用区预留布局空间。
    let max_rows = ((f32::from(panel_height) * 0.5 - f32::from(reference_height) - 100.) / 18.)
        .floor()
        .max(2.) as usize;
    if max_rows != this.ai_chat.input_max_rows {
        this.ai_chat.input_max_rows = max_rows;
        input.update(cx, |input, cx| input.set_auto_grow(2, max_rows, cx));
    }
    let popup = this
        .ai_chat
        .reference_open
        .then(|| this.ai_reference_list(ready, colors, cx));
    div()
                .id("ai-chat-composer")
                .relative()
                .debug_selector(|| "ai-chat-composer".into())
                .on_prepaint({
                    let owner = cx.entity().downgrade();
                    move |bounds, _, cx| { let _ = owner.update(cx, |this, cx| {
                        if this.ai_chat.composer_bounds != bounds { this.ai_chat.composer_bounds = bounds; cx.notify(); }
                    }); }
                })
                .flex_none().min_h(px(0.)).max_h(px(f32::from(panel_height) * 0.5))
                .child(div()
                .p_3()
                .border_t_1()
                .border_color(colors.border_soft)
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                // 引用较多时整个输入区最多占面板一半：输入框向上挤但不吞掉消息区。
                .max_h(px(f32::from(panel_height) * 0.5))
                .min_h(px(0.))
                .overflow_hidden()
                .when(!chat.context_references.is_empty(), |column| {
                    column.child(
                        div()
                            .id("ai-chat-references-scroll")
                            .debug_selector(|| "ai-chat-references-scroll".into())
                            // 已选引用至少保留一行，不参与输入框的高度压缩。
                            .flex_none()
                            .min_h(px(32.))
                            .max_h(px((f32::from(panel_height) * 0.5 - 154.).clamp(32., 80.)))
                            .on_prepaint({
                                let owner = cx.entity().downgrade();
                                move |bounds, _, cx| { let _ = owner.update(cx, |this, cx| {
                                    if this.ai_chat.reference_height != bounds.size.height { this.ai_chat.reference_height = bounds.size.height; cx.notify(); }
                                }); }
                            })
                            .relative()
                            .overflow_y_scroll()
                            .track_scroll(&this.ai_chat.reference_chips_scroll)
                            .vertical_scrollbar(&this.ai_chat.reference_chips_scroll)
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .children(chat.context_references.iter().map(|reference| {
                                let id = reference.id.clone();
                                let invalid = this.ai_chat.reference_errors.contains_key(&reference.id) || !this.controller.ai_reference_is_valid(reference);
                                let note = this.ai_chat.reference_errors.get(&reference.id).cloned()
                                    .unwrap_or_else(|| format!("{} · {} · {}", reference.label, reference.summary, reference.id));
                                ai_reference_chip(&reference.label, &reference.kind, id, invalid, note, colors, cx)
                            })),
                    )
                })
                .child(
                    div()
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .border_1()
                        .rounded(colors.radius_lg)
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .p_2()
                        .id("ai-chat-input")
                        .debug_selector(|| "ai-chat-input".into())
                        .key_context("AiChatComposer")
                        .capture_action(cx.listener(|this, _: &SendAiMessage, window, cx| {
                            if !this.ai_chat.input.read(cx).focus_handle(cx).is_focused(window) {
                                cx.propagate();
                                return;
                            }
                            let composing = this.ai_chat.input.update(cx, |input, cx| {
                                gpui::EntityInputHandler::marked_text_range(input, window, cx).is_some()
                            });
                            if !composing {
                                this.send_ai_chat(window, cx);
                            }
                            // 无论发送、禁用或输入法组词，都消费快捷键，避免回退为换行或执行 SQL。
                            cx.stop_propagation();
                        }))
                        .child(
                            Textarea::new(&input)
                                .accessibility_id("ai-chat-prompt")
                                .appearance(false)
                                .text_size(ui_text_size(12.))
                                .min_h(px(0.))
                                .aria_label(
                                    tr("app.ai.chat.composer.input_aria")
                                        .replace("%{shortcut}", &send_shortcut),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div().flex_1().min_w(px(0.)).flex().items_center().gap_1()
                                    .child(
                                        Button::new("ai-chat-add")
                                            .debug_selector(|| "ai-chat-add".into())
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .tooltip(if this.ai_chat.attachment_loading {
                                                tr("app.ai.chat.composer.attachment_loading")
                                            } else {
                                                tr("app.ai.chat.composer.add_attachment")
                                            })
                                            .loading(this.ai_chat.attachment_loading)
                                            .disabled(!ready)
                                            .child(app_icon(AppIcon::Plus, 16., colors.muted))
                                            .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, {
                                                let owner = cx.entity().downgrade();
                                                let upload_disabled = this.controller.state().ai_plugin_instances.iter()
                                                    .any(|plugin| plugin.id == "builtin.files" && !plugin.enabled);
                                                move |menu, _, _| {
                                                    let upload_owner = owner.clone();
                                                    let context_owner = owner.clone();
                                                    menu.item(PopupMenuItem::element(|_, _| div()
                                                        .id("ai-chat-upload")
                                                        .child(tr("app.ai.chat.composer.upload")))
                                                        .disabled(upload_disabled)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = upload_owner.update(cx, |this, cx| {
                                                                this.choose_ai_attachment(window, cx);
                                                            });
                                                        }))
                                                        .item(PopupMenuItem::element(|_, _| div()
                                                            .id("ai-chat-add-context")
                                                            .child(tr("app.ai.chat.composer.add_context")))
                                                            .on_click(move |_, window, cx| {
                                                                let _ = context_owner.update(cx, |this, cx| {
                                                                    this.open_ai_reference_menu(window, cx);
                                                                });
                                                            }))
                                                }
                                            }),
                                    )
                                                                        .when(
                                        !chat.context_references.is_empty(),
                                        |row| {
                                            row.child(
                                                div()
                                                    .text_size(ui_text_size(10.))
                                                    .text_color(colors.muted)
                                                    .child(
                                                        tr("app.ai.chat.composer.reference_count")
                                                            .replace(
                                                                "%{count}",
                                                                &chat.context_references.len().to_string(),
                                                            ),
                                                    ),
                                            )
                                        },
                                    ),
                                )
                                .child(this.ai_chat_model_button(
                                    loading || running || this.ai_chat._io.is_some() || chat.active_turn_id.is_some(),
                                    colors, cx,
                                ))
                                .child(if running {
                                    Button::new("ai-chat-stop")
                                        .debug_selector(|| "ai-chat-stop".into())
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .disabled(cancel_requested)
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .child(app_icon(
                                                    AppIcon::Square,
                                                    12.,
                                                    colors.danger,
                                                ))
                                                .child(if cancel_requested {
                                                    tr("app.ai.chat.composer.stopping")
                                                } else {
                                                    tr("app.ai.chat.composer.stop")
                                                }),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let (Some(runtime), Some(job)) =
                                                (&this.ai_chat.runtime, &this.ai_chat.running)
                                            {
                                                match runtime.cancel(&job.turn_id) {
                                                    Ok(()) => {
                                                        // 执行可能已收尾但正文仍在揭示；停止也应立即结束这段展示。
                                                        if let Some(flag) = &this.ai_chat.cancel { flag.request(); }
                                                    }
                                                    Err(error) => this.show_message(error.to_string(), AppMessageKind::Warning, cx),
                                                }
                                            }
                                            cx.notify();
                                        }))
                                        .into_any_element()
                                } else {
                                    Button::new("ai-chat-send")
                                        .accessibility_id("ai-chat-send")
                                        .accessibility_label(tr("app.ai.chat.composer.send_aria"))
                                        .debug_selector(|| "ai-chat-send".into())
                                        .with_size(px(22.))
                                        .w(px(22.))
                                        .h(px(22.))
                                        .p_0()
                                        .flex_none()
                                        .custom(
                                            ButtonCustomVariant::new(cx)
                                                .color(colors.text.into())
                                                .foreground(colors.panel_bg.into())
                                                .hover(Hsla::from(colors.text).opacity(0.9))
                                                .active(Hsla::from(colors.text).opacity(0.82)),
                                        )
                                        .rounded(px(12.))
                                        .cursor_pointer()
                                        .tooltip(
                                            tr("app.ai.chat.composer.send_tooltip")
                                                .replace("%{shortcut}", &send_shortcut),
                                        )
                                        .disabled(
                                            !ready || input.read(cx).value().trim().is_empty(),
                                        )
                                        .child(app_icon(AppIcon::ArrowUp, 14., colors.panel_bg))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send_ai_chat(window, cx)
                                        }))
                                        .into_any_element()
                                }),
                        ),
                )
                )
        .when_some(popup, |column, popup| column.child(div().absolute().top_0().left_0().child(popup)))
        .into_any_element()
}
