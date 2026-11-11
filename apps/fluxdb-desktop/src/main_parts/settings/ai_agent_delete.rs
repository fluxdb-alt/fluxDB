// 删除只隐藏可编辑配置，保留会话与审计身份；存储层再次校验不可删除边界。
impl NavicatMain {
    fn confirm_ai_agent_delete(
        &mut self,
        id: String,
        name: String,
        revision: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if id == fluxdb_storage::ai_platform::agents::BUILTIN_AGENT_ID
            || self.ai_agent_editor.as_ref().is_some_and(|e| e.deleting)
        {
            return;
        }
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let id = id.clone();
            dialog
                .title(tr("app.ai.agents.delete_title"))
                .close_button(true)
                .overlay_closable(true)
                .keyboard(true)
                .child(
                    div().text_size(ui_text_size(13.)).child(
                        tr("app.ai.agents.delete_confirm").replace("%{name}", &name),
                    ),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("ai-agent-delete-cancel")
                                .debug_selector(|| "ai-agent-delete-cancel".into())
                                .cursor_pointer()
                                .child(tr("app.common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("ai-agent-delete-confirm")
                                .debug_selector(|| "ai-agent-delete-confirm".into())
                                .danger()
                                .cursor_pointer()
                                .child(tr("app.ai.agents.delete_action"))
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
                    let _ = view.update(cx, |this, cx| {
                        this.start_ai_agent_delete(id.clone(), revision, cx)
                    });
                    true
                })
        });
    }

    fn start_ai_agent_delete(&mut self, id: String, revision: i64, cx: &mut Context<Self>) {
        if let Some(editor) = self.ai_agent_editor.as_mut() {
            if editor.deleting {
                return;
            }
            editor.deleting = true;
        }
        cx.notify();
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let task_id = id.clone();
            let (event, runtime_revision) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::DeleteAiAgent {
                        id: task_id,
                        expected_revision: revision,
                    });
                    (event, controller.state().ai_runtime_revision)
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let result = match event {
                        AppEvent::AiAgentDeleted { .. } => Ok(()),
                        AppEvent::AiSettingsError(message) => Err(message),
                        _ => Err(tr("app.ai.agents.delete_failed").to_string()),
                    };
                    let event = this.controller.dispatch(AppCommand::CompleteAiAgentDelete {
                        id: id.clone(),
                        runtime_revision,
                        result,
                    });
                    if let Some(editor) = this.ai_agent_editor.as_mut() {
                        editor.deleting = false;
                    }
                    match event {
                        AppEvent::AiAgentDeleted { .. } => {
                            if this
                                .ai_agent_editor
                                .as_ref()
                                .is_some_and(|e| e.agent_id.as_deref() == Some(id.as_str()))
                            {
                                this.ai_agent_editor = None;
                                this.ai_agent_delete_just_removed = id;
                            }
                            this.show_message(
                                tr("app.ai.agents.deleted"),
                                AppMessageKind::Success,
                                cx,
                            );
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
    }
}
