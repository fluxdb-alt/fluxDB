#[derive(Clone, PartialEq)]
struct AiChatModelOption { id: String, label: String, unavailable: bool }

impl SelectItem for AiChatModelOption {
    type Value = String;
    fn title(&self) -> SharedString { self.label.clone().into() }
    fn value(&self) -> &String { &self.id }
    fn disabled(&self) -> bool { self.unavailable }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().debug_selector(|| format!("ai-chat-model-option:{}",self.id)).child(self.label.clone())
    }
}

impl NavicatMain {
    fn ai_chat_model_button(
        &self,
        disabled: bool,
        colors: UiColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selected = self.controller.state().ai_conversation.selected_model_id.clone().unwrap_or_default();
        let label = self.ai_chat.model_options.iter().find(|option| option.id == selected)
            .map(|option| option.label.clone())
            .unwrap_or_else(|| tr("app.ai.chat.model.select").into());
        let options = self.ai_chat.model_options.clone();
        let owner = cx.entity().downgrade();
        Button::new("ai-chat-model-trigger")
            .debug_selector(|| "ai-chat-model-select".into())
            .small().ghost().cursor_pointer().disabled(disabled)
            .tooltip(label.clone())
            .child(div().flex().items_center().gap_1p5().min_w(px(0.))
                .text_size(ui_text_size(11.)).font_weight(gpui::FontWeight::NORMAL).text_color(colors.muted)
                .child(div().max_w(px(160.)).truncate().child(label))
                .child(app_icon(AppIcon::ChevronDown, 10., colors.muted)))
            .dropdown_menu_with_anchor(gpui::Anchor::BottomRight, move |menu, _, _| {
                let mut menu = menu.check_side(gpui_component::Side::Right).min_w(px(200.)).max_w(px(280.));
                for option in &options {
                    let id = option.id.clone();
                    let label = option.label.clone();
                    let render_id = id.clone();
                    let owner = owner.clone();
                    menu = menu.item(PopupMenuItem::element(move |_, _| {
                        // 复用菜单项交互，内容只负责紧凑文字与可访问名称。
                        div().id(SharedString::from(format!("ai-chat-model-label:{render_id}")))
                            .aria_label(label.clone())
                            .debug_selector({ let id = render_id.clone(); move || format!("ai-chat-model-option:{id}") })
                            .text_size(ui_text_size(11.)).child(label.clone())
                    }).checked(option.id == selected).disabled(option.unavailable)
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                let revision = this.controller.state().ai_conversation.model_selection_revision;
                                this.ai_chat_request(fluxdb_app::AiConversationCommand::SelectModel {
                                    model_id: (!id.is_empty()).then(|| id.clone()),
                                    expected_revision: revision,
                                }, window, cx);
                            });
                        }));
                }
                menu
            }).into_any_element()
    }

    /// 底栏当前 Agent 只做状态展示；会话级切换命令尚未开放，避免做出“看似可选但不生效”的控件。
    fn sync_ai_chat_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.controller.state();
        let chat = &state.ai_conversation;
        let session_agent_id = chat
            .session_id
            .as_deref()
            .and_then(|session_id| {
                chat.sessions
                    .iter()
                    .find(|session| session.id == session_id)
            })
            .map(|session| session.agent_id.clone())
            .unwrap_or_else(|| "fluxdb.agent.default".into());
        let mut options = state
            .ai_agents
            .iter()
            .filter(|agent| agent.enabled)
            .map(|agent| AiChatModelOption {
                id: agent.id.clone(),
                label: agent.display_name.clone(),
                unavailable: false,
            })
            .collect::<Vec<_>>();
        if !options.iter().any(|option| option.id == session_agent_id) {
            options.push(AiChatModelOption {
                id: session_agent_id.clone(),
                label: tr("app.ai.chat.model.current_agent").into(),
                unavailable: true,
            });
        }
        let items_changed = self.ai_chat.agent_options != options;
        if items_changed {
            self.ai_chat.agent_options = options.clone();
            self.ai_chat.agent_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(options), window, cx);
            });
        }
        if items_changed
            || self.ai_chat.agent_select.read(cx).selected_value() != Some(&session_agent_id)
        {
            self.ai_chat
                .agent_select
                .update(cx, |select, cx| select.set_selected_value(&session_agent_id, window, cx));
        }
    }

    fn sync_ai_chat_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.controller.state();
        let selected = state.ai_conversation.selected_model_id.clone().unwrap_or_default();
        let mut options = vec![AiChatModelOption {
            id: String::new(),
            label: tr("app.ai.chat.model.default").into(),
            unavailable: false,
        }];
        options.extend(state.ai_models.iter().filter(|m|m.enabled && m.kind==fluxdb_app::AiModelKind::Generation).map(|m|AiChatModelOption{
            id:m.id.clone(), label:format!("{} · {}",m.display_name,m.provider),unavailable:false,
        }));
        if !selected.is_empty() && !options.iter().any(|o|o.id==selected) {
            options.push(AiChatModelOption {
                id: selected.clone(),
                label: tr("app.ai.chat.model.unavailable").into(),
                unavailable: true,
            });
        }
        let items_changed = self.ai_chat.model_options != options;
        if items_changed {
            self.ai_chat.model_options = options.clone();
            self.ai_chat.model_select.update(cx, |select,cx|select.set_items(SearchableVec::new(options),window,cx));
        }
        // 候选重排会让 Select 游标停留在旧索引上：`selected_value` 可能仍相等，
        // 但标题按游标取项就会显示成别的模型。因此这里在同一帧无条件重设一次。
        if items_changed || self.ai_chat.model_select.read(cx).selected_value() != Some(&selected) {
            self.ai_chat.model_select.update(cx,|select,cx|select.set_selected_value(&selected,window,cx));
        }
    }
}
