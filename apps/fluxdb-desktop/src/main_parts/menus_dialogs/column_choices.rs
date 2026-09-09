const COLUMN_CHOICES_OPTION: &str = "column_choices.v1";

type ColumnChoicesConfig = BTreeMap<String, Vec<ColumnChoice>>;

impl NavicatMain {
    fn show_column_choices_modal(
        &mut self,
        tab_id: TabId,
        column_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((object_path, type_name)) = self.data_editor_column_for_choices(tab_id, &column_name) else {
            self.show_message("当前字段不支持设置枚举值", AppMessageKind::Warning, cx);
            return;
        };
        let choices = self.column_choices_for_object_column(&object_path, &column_name);
        self.column_choice_value_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.column_choice_label_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.pending_column_choices = Some(PendingColumnChoices {
            tab_id,
            object_path,
            column_name,
            type_name,
            choices,
            adding: false,
        });
        cx.notify();
    }

    fn cancel_column_choices_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_column_choices = None;
        cx.notify();
    }

    fn save_column_choices_modal(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_column_choices.take() else {
            return;
        };
        let choices = pending.choices.clone();
        let Some(connection) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == pending.object_path.connection_id)
        else {
            self.show_message("找不到当前连接，无法保存枚举值", AppMessageKind::Error, cx);
            return;
        };

        let mut config = connection.config.clone();
        let mut all_choices = column_choices_config(&config.options);
        let key = column_choices_key(&pending.object_path, &pending.column_name);
        if choices.is_empty() {
            all_choices.remove(&key);
        } else {
            all_choices.insert(key, choices);
        }
        if all_choices.is_empty() {
            config.options.remove(COLUMN_CHOICES_OPTION);
        } else if let Ok(text) = serde_json::to_string(&all_choices) {
            config.options.insert(COLUMN_CHOICES_OPTION.to_string(), text);
        }

        let _ = self.controller.dispatch(AppCommand::UpdateConnection(config));
        let _ = self
            .storage
            .save_connections(&self.controller.connection_configs());
        self.refresh_active_data_table(pending.tab_id, cx);
        self.show_message("枚举值已保存", AppMessageKind::Success, cx);
        cx.notify();
    }

    fn add_column_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .column_choice_value_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if value.is_empty() {
            self.show_message("请先输入枚举值", AppMessageKind::Warning, cx);
            return;
        }
        let label = self
            .column_choice_label_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(pending) = &mut self.pending_column_choices else {
            return;
        };
        if let Some(choice) = pending.choices.iter_mut().find(|choice| choice.value == value) {
            choice.label = label;
        } else {
            pending.choices.push(ColumnChoice { value, label });
        }
        pending.adding = false;
        self.column_choice_value_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.column_choice_label_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        cx.notify();
    }

    fn start_column_choice_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = &mut self.pending_column_choices {
            pending.adding = true;
        }
        self.column_choice_value_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        self.column_choice_label_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        cx.notify();
    }

    fn remove_column_choice(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some(pending) = &mut self.pending_column_choices {
            pending.choices.retain(|choice| choice.value != value);
            cx.notify();
        }
    }

    fn data_editor_column_for_choices(
        &self,
        tab_id: TabId,
        column_name: &str,
    ) -> Option<(ObjectPath, String)> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => {
                    let column = editor
                        .page
                        .as_ref()?
                        .columns
                        .iter()
                        .find(|column| column.name == column_name)?;
                    Some((
                        editor.object.clone(),
                        column
                            .type_name
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ))
                }
                _ => None,
            })
    }

    fn column_choices_for_tab(&self, tab_id: TabId) -> BTreeMap<String, Vec<ColumnChoice>> {
        let Some((object, columns)) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some((
                    editor.object.clone(),
                    editor
                        .page
                        .as_ref()?
                        .columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect::<Vec<_>>(),
                )),
                _ => None,
            })
        else {
            return BTreeMap::new();
        };

        columns
            .into_iter()
            .filter_map(|column| {
                let choices = self.column_choices_for_object_column(&object, &column);
                (!choices.is_empty()).then_some((column, choices))
            })
            .collect()
    }

    fn column_choices_for_editing_cell(
        &self,
        editing: DataCellEditState,
        column_name: &str,
    ) -> Vec<ColumnChoice> {
        let Some(object) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == editing.tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some(editor.object.clone()),
                _ => None,
            })
        else {
            return Vec::new();
        };
        self.column_choices_for_object_column(&object, column_name)
    }

    fn column_choices_for_object_column(
        &self,
        object: &ObjectPath,
        column_name: &str,
    ) -> Vec<ColumnChoice> {
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == object.connection_id)
            .map(|connection| column_choices_config(&connection.config.options))
            .and_then(|choices| choices.get(&column_choices_key(object, column_name)).cloned())
            .unwrap_or_default()
    }
}

fn column_choices_modal(
    form: PendingColumnChoices,
    value_input: Entity<InputState>,
    label_input: Entity<InputState>,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let PendingColumnChoices {
        column_name,
        type_name,
        choices,
        adding,
        ..
    } = form;

    div()
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
                this.cancel_column_choices_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(420.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("ColumnChoicesModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_column_choices_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(column_choices_header(colors, cx))
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(column_choices_field(column_name, type_name, colors))
                        .child(column_choices_list(
                            choices,
                            adding,
                            value_input,
                            label_input,
                            colors,
                            cx,
                        )),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            Button::new("column-choices-cancel")
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_column_choices_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("column-choices-save")
                                .label("保存")
                                .primary()
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_column_choices_modal(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn column_choices_header(colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .px_5()
        .pt_4()
        .pb_2()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(17.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("字段枚举"),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 14., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_column_choices_modal(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn column_choices_field(column_name: String, type_name: String, colors: UiColors) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .child(
            div()
                .overflow_hidden()
                .text_ellipsis()
                .font_family("Menlo")
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(column_name),
        )
        .child(
            div()
                .px_2()
                .h(px(22.))
                .rounded(colors.radius)
                .bg(colors.panel_alt)
                .text_color(colors.muted)
                .flex()
                .items_center()
                .font_family("Menlo")
                .child(type_name),
        )
}

fn column_choices_input_frame(input: Entity<InputState>, width: Pixels, colors: UiColors) -> impl IntoElement {
    div()
        .w(width)
        .h(px(32.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(px(13.)),
        )
}

fn column_choices_list(
    choices: Vec<ColumnChoice>,
    adding: bool,
    value_input: Entity<InputState>,
    label_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let mut list = div()
        .w_full()
        .max_h(px(220.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .overflow_y_scrollbar()
        .text_size(px(13.));

    if choices.is_empty() && !adding {
        list.child(column_choices_empty_state(colors, cx))
    } else {
        for (index, choice) in choices.into_iter().enumerate() {
            list = list.child(column_choices_row(index, choice, colors, cx));
        }
        if adding {
            list.child(column_choices_add_editor(
                value_input,
                label_input,
                colors,
                cx,
            ))
        } else {
            list.child(column_choices_add_row(colors, cx))
        }
    }
}

fn column_choices_row(
    index: usize,
    choice: ColumnChoice,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let value = choice.value.clone();
    div()
        .w_full()
        .flex_none()
        .h(px(34.))
        .px_4()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(154.))
                .overflow_hidden()
                .text_ellipsis()
                .font_family("Menlo")
                .child(choice.value),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .text_color(colors.muted)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(choice.label),
        )
        .child(
            Button::new(("column-choice-remove", index))
                .ghost()
                .xsmall()
                .h(px(24.))
                .min_w(px(24.))
                .p_0()
                .child(app_icon(AppIcon::Close, 13., colors.muted))
                .tooltip("删除")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.remove_column_choice(&value, cx);
                    cx.stop_propagation();
                })),
        )
}

fn column_choices_add_row(colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .w_full()
        .h(px(34.))
        .px_4()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(rgb(0x1677ff))
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(AppIcon::Plus, 13., rgb(0x1677ff)))
        .child("添加枚举值")
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.start_column_choice_add(window, cx);
                cx.stop_propagation();
            }),
        )
}

fn column_choices_empty_state(colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .w_full()
        .h(px(96.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("尚未添加枚举值"),
        )
        .child(
            div()
                .h(px(30.))
                .px_3()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(colors.text)
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Plus, 13., colors.muted))
                .child("添加枚举值")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.start_column_choice_add(window, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn column_choices_add_editor(
    value_input: Entity<InputState>,
    label_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .w_full()
        .h(px(42.))
        .px_4()
        .flex()
        .items_center()
        .gap_2()
        .child(column_choices_input_frame(value_input, px(152.), colors))
        .child(column_choices_input_frame(label_input, px(152.), colors))
        .child(
            Button::new("column-choices-add-confirm")
                .ghost()
                .xsmall()
                .h(px(28.))
                .min_w(px(28.))
                .p_0()
                .child(app_icon(AppIcon::Check, 14., colors.text))
                .tooltip("添加")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.add_column_choice(window, cx);
                    cx.stop_propagation();
                })),
        )
}

fn column_choices_config(options: &BTreeMap<String, String>) -> ColumnChoicesConfig {
    options
        .get(COLUMN_CHOICES_OPTION)
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default()
}

fn column_choices_key(object: &ObjectPath, column_name: &str) -> String {
    let parts = [
        object.database.as_deref().unwrap_or_default(),
        object.schema.as_deref().unwrap_or_default(),
        object.name.as_str(),
        column_name,
    ];
    serde_json::to_string(&parts).unwrap_or_else(|_| parts.join("/"))
}
