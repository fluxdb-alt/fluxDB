// 会话重命名弹框与后台 CAS 保存；由 ai.rs include! 引入。
impl NavicatMain {
    /// 打开会话重命名弹框（Esc/遮罩/关闭按钮均取消，不写入）。
    fn open_ai_session_rename(
        &mut self,
        id: String,
        title: String,
        expected_revision: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_ai_session_rename = Some(AiSessionRename {
            id: id.clone(),
            title: title.clone(),
            expected_revision,
        });
        self.ai_session_rename_input
            .update(cx, |input, cx| input.set_value(title, window, cx));
        let input = self.ai_session_rename_input.clone();
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let view = view.clone();
            let focused = input.focus_handle(cx).is_focused(window);
            let focus_border = if colors.is_dark {
                rgb(0x8ab4ff)
            } else {
                rgb(0x111111)
            };
            dialog
                .title(tr("app.ai.sessions.rename_title"))
                .close_button(true)
                .overlay_closable(true)
                .keyboard(true)
                .child(
                    div()
                        .w_full()
                        .h(px(34.))
                        .rounded_md()
                        .border_1()
                        .border_color(if focused { focus_border } else { colors.border })
                        .hover(move |style| {
                            style.border_color(if focused { focus_border } else { colors.muted })
                        })
                        .bg(colors.input_bg)
                        .child(
                            Input::new(&input)
                                .id("ai-session-rename-input")
                                .aria_label(tr("app.ai.sessions.title_field"))
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(ui_text_size(13.)),
                        ),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("ai-session-rename-cancel")
                                .debug_selector(|| "ai-session-rename-cancel".into())
                                .cursor_pointer()
                                .child(tr("app.common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("ai-session-rename-confirm")
                                .debug_selector(|| "ai-session-rename-confirm".into())
                                .primary()
                                .cursor_pointer()
                                .child(tr("app.common.save"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(gpui_component::dialog::Confirm {
                                            secondary: false,
                                        }),
                                        cx,
                                    )
                                }),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| this.confirm_ai_session_rename(cx))
                        .unwrap_or(true)
                })
        });
        self.ai_session_rename_input
            .focus_handle(cx)
            .focus(window, cx);
    }

    fn confirm_ai_session_rename(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pending) = self.pending_ai_session_rename.as_ref() else {
            return true;
        };
        let title = pending.title.trim().to_string();
        if title.is_empty() {
            self.show_message(
                tr("app.ai.sessions.name_required"),
                AppMessageKind::Warning,
                cx,
            );
            return false;
        }
        let pending = self
            .pending_ai_session_rename
            .take()
            .expect("已校验待保存会话");
        let mut controller = self.controller.clone();
        let id = pending.id;
        let expected_revision = pending.expected_revision;
        cx.spawn(async move |view, cx| {
            // 后台在控制器副本上写库；结果经主窗口控制器的 Complete 命令合并，
            // 否则副本里的 state 变更不会回到 UI（与删除 Agent 同一模式）。
            let (revision, result) = cx
                .background_spawn({
                    let id = id.clone();
                    let title = title.clone();
                    async move {
                        let event = controller.dispatch(AppCommand::RenameAiSession {
                            id: id.clone(),
                            title,
                            expected_revision,
                        });
                        let revision = controller
                            .state()
                            .ai_sessions
                            .iter()
                            .find(|s| s.id == id)
                            .map(|s| s.revision)
                            .unwrap_or(expected_revision + 1);
                        let result = match event {
                            AppEvent::AiSessionRenamed { .. } => Ok(()),
                            AppEvent::AiSettingsError(message) => Err(message),
                            _ => Err(tr("app.ai.sessions.rename_failed").to_string()),
                        };
                        (revision, result)
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let event = this
                        .controller
                        .dispatch(AppCommand::CompleteAiSessionRename {
                            id: id.clone(),
                            title: title.clone(),
                            revision,
                            result,
                        });
                    match event {
                        AppEvent::AiSessionRenamed { .. } => {
                            this.show_message(
                                tr("app.ai.sessions.renamed"),
                                AppMessageKind::Success,
                                cx,
                            )
                        }
                        AppEvent::AiSettingsError(message) => {
                            this.show_message(message, AppMessageKind::Error, cx)
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            });
        })
        .detach();
        true
    }
}
