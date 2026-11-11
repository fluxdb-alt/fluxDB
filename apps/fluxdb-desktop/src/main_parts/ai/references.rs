impl NavicatMain {
    /// `@` 按钮与键盘输入共用同一条候选路径；这里只补一个空查询触发片段。
    fn open_ai_reference_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut text = self.ai_chat.input.read(cx).value().to_string();
        if !text.ends_with('@') {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push('@');
        }
        self.ai_chat.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
        let focus = self.ai_chat.input.read(cx).focus_handle(cx);
        focus.focus(window, cx);
        self.refresh_ai_reference_menu(window, cx);
    }

    fn choose_ai_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_chat.attachment_loading {
            return;
        }
        if self.controller.state().ai_plugin_instances.iter()
            .any(|plugin| plugin.id == "builtin.files" && !plugin.enabled)
        {
            self.show_message(
                tr("app.ai.chat.reference.disabled_plugin").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.ai_chat.attachment_loading = true;
        let epoch = self.ai_chat.epoch;
        let session_id = self.controller.state().ai_conversation.session_id.clone();
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.ai.chat.reference.picker_prompt")),
        });
        let root = self.storage.root_dir().to_path_buf();
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = picker.await;
            let result = match selected {
                Ok(Ok(Some(paths))) => match paths.first() {
                    Some(path) => {
                        let path = path.clone();
                        Some(
                            cx.background_spawn(async move {
                                fluxdb_app::authorize_ai_file(&path, &root)
                            })
                            .await,
                        )
                    }
                    None => None,
                },
                Ok(Ok(None)) => None,
                _ => Some(Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Internal,
                    tr("app.ai.chat.reference.picker_failed").to_string(),
                ))),
            };
            let _ = view.update_in(cx, |this, window, cx| {
                this.ai_chat.attachment_loading = false;
                if epoch != this.ai_chat.epoch
                    || session_id != this.controller.state().ai_conversation.session_id
                {
                    this.show_message(
                        tr("app.ai.chat.reference.session_switched").as_ref(),
                        AppMessageKind::Warning,
                        cx,
                    );
                    return;
                }
                match result {
                    Some(Ok(reference)) => this.ai_chat_request(
                        fluxdb_app::AiConversationCommand::AddReference { reference },
                        window,
                        cx,
                    ),
                    Some(Err(error)) => this.show_message(error.message, AppMessageKind::Error, cx),
                    None => {}
                }
                cx.notify();
            });
        }));
    }

    /// 从输入框文本里取出正在输入的 `@` 查询片段。
    ///
    /// 规则：取词边界后的最后一个 `@`，且它到文本末尾之间没有空白与前导 `@` 本身；
    /// 这样普通的邮箱/单词里的 `@` 不会误触发候选，`@` 后的内容即检索词。
    fn ai_reference_query(text: &str) -> Option<String> {
        let at = text.rfind('@')?;
        if at > 0 && !text[..at].chars().last()?.is_whitespace() {
            return None;
        }
        let tail = &text[at + 1..];
        if tail.chars().any(char::is_whitespace) {
            return None;
        }
        Some(tail.to_string())
    }

    /// SQLite 检索与附件校验在后台执行；查询/会话变更后丢弃迟到结果。
    fn refresh_ai_reference_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_chat.reference_epoch += 1;
        let epoch = self.ai_chat.reference_epoch;
        let Some(query) = Self::ai_reference_query(&self.ai_chat.input.read(cx).value()) else {
            self.ai_chat.reference_query = None;
            self.ai_chat.reference_error = None;
            self.ai_chat.reference_loading = false;
            self.ai_chat.reference_open = false;
            self.ai_chat.reference_options.clear();
            self.ai_chat.reference_index = 0;
            return;
        };
        let controller = self.controller.clone();
        let context = self.capture_ai_active_context(cx);
        let session_epoch = self.ai_chat.epoch;
        self.ai_chat.reference_query = Some(query.clone());
        self.ai_chat.reference_index = 0;
        self.ai_chat.reference_scroll.scroll_to_item(0);
        self.ai_chat.reference_options.clear();
        self.ai_chat.reference_no_match = false;
        self.ai_chat.reference_error = None;
        self.ai_chat.reference_loading = true;
        self.ai_chat.reference_open = true;
        self.ai_chat.reference_task = Some(cx.spawn_in(window, async move |view, cx| {
            let (candidates, errors) = cx
                .background_spawn(async move {
                    (
                        controller.ai_reference_candidates(&query, Some(&context)),
                        controller.ai_reference_errors(),
                    )
                })
                .await;
            let _ = view.update_in(cx, |this, _, cx| {
                if this.ai_chat.reference_epoch != epoch || this.ai_chat.epoch != session_epoch {
                    return;
                }
                this.ai_chat.reference_loading = false;
                this.ai_chat.reference_errors = errors;
                match candidates {
                    Ok(candidates) => {
                        this.ai_chat.reference_no_match = candidates.is_empty();
                        this.ai_chat.reference_options = candidates;
                    }
                    Err(error) => {
                        tracing::warn!("AI 引用候选检索失败");
                        this.ai_chat.reference_error = Some(error.message);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// 选中一个 `@` 候选 → 存引用 + 清掉输入框里的 `@query` 触发片段。
    fn accept_ai_reference(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(reference) = self
            .ai_chat
            .reference_options
            .iter()
            .find(|reference| reference.id == id)
            .cloned()
        else {
            return;
        };
        let mut text = self.ai_chat.input.read(cx).value().to_string();
        if let Some(at) = text.rfind('@') {
            let tail = &text[at + 1..];
            if !tail.chars().any(char::is_whitespace) {
                text.truncate(at);
            }
        }
        self.ai_chat
            .input
            .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        self.ai_chat.reference_open = false;
        self.ai_chat.reference_query = None;
        self.ai_chat.reference_options.clear();
        self.ai_chat
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        self.ai_chat_request(
            fluxdb_app::AiConversationCommand::AddReference { reference },
            window,
            cx,
        );
    }

    // 候选使用 ListItem，不把焦点从 Textarea 抢走；方向键和确认仅在候选打开时接管。
    fn handle_ai_reference_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.ai_chat.reference_open
            || (self.ai_chat.reference_loading && key != "escape")
            || self.ai_chat.loading
            || self.ai_chat.recovery_pending
            || self.ai_chat.recovery_failed
            || self.ai_chat._io.is_some()
            || !self
                .ai_chat
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            return false;
        }
        // 输入法组词期间，回车和方向键应留给系统候选，不确认应用引用。
        if self.ai_chat.input.update(cx, |input, cx| {
            gpui::EntityInputHandler::marked_text_range(input, window, cx).is_some()
        }) {
            return false;
        }
        match key {
            "up" | "down" if !self.ai_chat.reference_options.is_empty() => {
                let count = self.ai_chat.reference_options.len();
                self.ai_chat.reference_index = if key == "up" {
                    (self.ai_chat.reference_index + count - 1) % count
                } else {
                    (self.ai_chat.reference_index + 1) % count
                };
                self.ai_chat
                    .reference_scroll
                    .scroll_to_item(self.ai_chat.reference_index);
            }
            "enter" => {
                let id = self
                    .ai_chat
                    .reference_options
                    .get(self.ai_chat.reference_index)
                    .map(|r| r.id.clone());
                if let Some(id) = id {
                    self.accept_ai_reference(&id, window, cx);
                } else {
                    // 没有候选时不吞掉回车，仍允许正常换行。
                    self.ai_chat.reference_open = false;
                    return false;
                }
            }
            "escape" => {
                self.ai_chat.reference_epoch += 1;
                self.ai_chat.reference_loading = false;
                self.ai_chat.reference_open = false;
                self.ai_chat.reference_query = None;
                self.ai_chat.reference_options.clear();
            }
            _ => return false,
        }
        cx.stop_propagation();
        cx.notify();
        true
    }

    fn ai_reference_list(
        &self,
        ready: bool,
        colors: UiColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let ready = ready && !self.ai_chat.reference_loading;
        let bounds = self.ai_chat.composer_bounds;
        let max_height = (f32::from(self.ai_chat.panel_height) * 0.42).clamp(48., 272.);
        let candidate_count = self
            .ai_chat
            .reference_options
            .len()
            .min(fluxdb_app::AI_REFERENCE_CANDIDATE_LIMIT);
        let mut list = div()
            .id("ai-chat-reference-list")
            .debug_selector(|| "ai-chat-reference-select".into())
            .w(px((f32::from(bounds.size.width) - 24.).max(0.)))
            .overflow_hidden()
            .border_1()
            .border_color(colors.border_soft)
            .rounded(colors.radius_lg)
            .bg(colors.panel_bg)
            .shadow_lg()
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .text_size(ui_text_size(9.))
                    .text_color(colors.muted)
                    .child(tr("app.ai.chat.reference.db_candidates_hint")),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                if !this
                    .ai_chat
                    .input
                    .read(cx)
                    .input_bounds()
                    .contains(&event.position)
                {
                    this.ai_chat.reference_open = false;
                    this.ai_chat.reference_epoch += 1;
                    this.ai_chat.reference_loading = false;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(tr("app.ai.chat.reference.menu_title"))
                    .child(format!(
                        "{candidate_count} / {}",
                        fluxdb_app::AI_REFERENCE_CANDIDATE_LIMIT
                    )),
            )
            .child(
                div()
                    .id("ai-chat-reference-list-scroll")
                    .debug_selector(|| "ai-chat-reference-list-scroll".into())
                    .flex_none()
                    .max_h(px(max_height))
                    .track_scroll(&self.ai_chat.reference_scroll)
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.ai_chat.reference_scroll)
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px_1()
                    .pb_1()
                    .children(
                        self.ai_chat
                            .reference_options
                            .iter()
                            .take(fluxdb_app::AI_REFERENCE_CANDIDATE_LIMIT)
                            .enumerate()
                            .map(|(index, reference)| {
                                let id = reference.id.clone();
                                let (icon, tone) = ai_reference_visual(&reference.kind, colors);
                                gpui_component::list::ListItem::new(SharedString::from(format!(
                                    "ai-reference-candidate:{id}"
                                )))
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("ai-reference-candidate:{id}")
                                })
                                .selected(index == self.ai_chat.reference_index)
                                .disabled(!ready)
                                .cursor_pointer()
                                .rounded(colors.radius)
                                .px_1p5()
                                .py(px(4.))
                                .text_size(ui_text_size(11.))
                                .text_color(colors.text)
                                .child(
                                    div()
                                        .w_full()
                                        .min_w(px(0.))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_none()
                                                .size(px(24.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(colors.radius)
                                                .bg(colors.hover)
                                                .child(app_icon(icon, 13., tone)),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .flex()
                                                .flex_col()
                                                .gap(px(1.))
                                                .child(
                                                    div()
                                                        .min_w(px(0.))
                                                        .truncate()
                                                        .text_color(colors.text)
                                                        .child(reference.label.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .min_w(px(0.))
                                                        .truncate()
                                                        .text_size(ui_text_size(10.))
                                                        .text_color(colors.muted)
                                                        .child(reference.summary.clone()),
                                                ),
                                        )
                                        .child(ai_reference_badge(&reference.kind, colors)),
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.accept_ai_reference(&id, window, cx);
                                    },
                                ))
                            }),
                    ),
            );
        if self.ai_chat.reference_loading {
            list = list.child(
                div()
                    .id("ai-reference-search-loading")
                    .px_2()
                    .py_2()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(tr("app.ai.chat.reference.searching")),
            );
        } else if let Some(error) = self.ai_chat.reference_error.as_deref() {
            list = list.child(
                div()
                    .debug_selector(|| "ai-reference-search-error".into())
                    .px_2()
                    .py_2()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.danger)
                    .child(tr("app.ai.chat.reference.search_failed").replace("%{error}", error)),
            );
        } else if self.ai_chat.reference_options.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(
                        tr("app.ai.chat.reference.no_match").replace(
                            "%{query}",
                            self.ai_chat.reference_query.as_deref().unwrap_or_default(),
                        ),
                    ),
            );
        }
        gpui::deferred(
            gpui::anchored()
                .anchor(gpui::Anchor::BottomLeft)
                .position(gpui::point(bounds.left() + px(12.), bounds.top() - px(4.)))
                .snap_to_window()
                .child(list),
        )
        .into_any_element()
    }
}

fn ai_reference_visual(kind: &str, colors: UiColors) -> (AppIcon, gpui::Rgba) {
    match kind {
        "connection" | "database" | "redis_db" => (AppIcon::Database, colors.info),
        "table" | "view" | "schema" => (AppIcon::Table, colors.success),
        "saved_query" => (AppIcon::Query, colors.info),
        "selection" => (AppIcon::Select, colors.warning),
        "result_set" => (AppIcon::List, colors.success),
        _ => (AppIcon::FileSearch, colors.info),
    }
}

fn ai_reference_badge(kind: &str, colors: UiColors) -> gpui::AnyElement {
    let label = match kind {
        "connection" => tr("app.ai.chat.reference.kind.connection"),
        "database" => tr("app.ai.chat.reference.kind.database"),
        "schema" => tr("app.ai.chat.reference.kind.schema"),
        "table" => tr("app.ai.chat.reference.kind.table"),
        "view" => tr("app.ai.chat.reference.kind.view"),
        "collection" => tr("app.ai.chat.reference.kind.collection"),
        "redis_db" => tr("app.ai.chat.reference.kind.redis_db"),
        "redis_key" => tr("app.ai.chat.reference.kind.redis_key"),
        "file" => tr("app.ai.chat.reference.kind.file"),
        "saved_query" => tr("app.ai.chat.reference.kind.saved_query"),
        "selection" => tr("app.ai.chat.reference.kind.selection"),
        "result_set" => tr("app.ai.chat.reference.kind.result_set"),
        _ => tr("app.ai.chat.reference.kind.fallback"),
    };
    let (_, tone) = ai_reference_visual(kind, colors);
    div()
        .flex_none()
        .px_1p5()
        .py(px(2.))
        .rounded(px(999.))
        .bg(rgba_with_alpha(tone, 0.11))
        .text_size(ui_text_size(10.))
        .text_color(tone)
        .child(label)
        .into_any_element()
}
