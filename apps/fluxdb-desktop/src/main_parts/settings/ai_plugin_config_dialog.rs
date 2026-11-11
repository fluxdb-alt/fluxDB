// 插件「管理」弹框：宿主按插件自己声明的配置渲染表单（设计 §24）。
//
// 字段、标签、候选与校验全部来自应用层的配置声明（内置能力读代码注册表，外置插件读
// manifest 的 `config_declaration`）。这里不判断插件 id、不自己拼资源列表，因此新增
// 可配置插件不需要改这个文件；弹框只提交「字段键 + 选择」，落库口径由应用层决定。
//
// 视觉：标题区（图标 + 标题 + 一行副标题）/ 表单区（左标签、右控件）/ 底部操作区。
// 候选用浮层下拉（`dropdown_menu_with_anchor`）：展开不改变弹框高度，选项多也不会把
// 弹框撑成一条。


/// 插件管理弹框的数据；`None` 表示未打开，不常驻内存。
#[derive(Clone)]
struct AiPluginConfigDialog {
    instance_id: String,
    display_name: String,
    /// 与插件行一致的图标，避免弹框标题区与来源行对不上。
    icon: AppIcon,
    /// 打开时的字段快照（含宿主解析好的候选项）。
    fields: Vec<fluxdb_app::AiPluginConfigFieldItem>,
    /// 草稿值：字段键 → 文本值（资源字段是资源 ID；`None` 表示清空该字段）。
    drafts: BTreeMap<String, Option<String>>,
    /// 标量字段的输入实体：字段键 → 单行输入。
    ///
    /// 打开弹框时按声明创建，关闭即随弹框一起丢弃。
    inputs: BTreeMap<String, Entity<InputState>>,
    /// 多行标量字段（`text_list`）的输入实体。
    textareas: BTreeMap<String, Entity<gpui_component::input::TextareaState>>,
    /// 运行设置 CAS 期望值（打开时快照）。
    expected_revision: i64,
    /// 内联错误：声明校验失败、CAS 冲突等；有值时保持弹框打开并保留草稿。
    error: Option<String>,
}

impl AiPluginConfigDialog {
    fn new(
        item: &fluxdb_app::AiPluginItem,
        expected_revision: i64,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) -> Self {
        let drafts = item
            .config_fields
            .iter()
            .map(|field| (field.key.clone(), field.selection.clone()))
            .collect();
        let mut inputs = BTreeMap::new();
        let mut textareas = BTreeMap::new();
        for field in &item.config_fields {
            let initial = field.selection.clone().unwrap_or_default();
            let placeholder =
                localize_ai_plugin_text(field.placeholder.as_deref().unwrap_or_default());
            match field.kind {
                fluxdb_app::AiPluginConfigFieldKind::Resource
                | fluxdb_app::AiPluginConfigFieldKind::Switch
                | fluxdb_app::AiPluginConfigFieldKind::Select => {
                    // 资源字段、开关、枚举都用现成控件，不需要文本实体。
                }
                fluxdb_app::AiPluginConfigFieldKind::TextList => {
                    let entity = cx.new(|cx| {
                        gpui_component::input::TextareaState::new(window, cx)
                            .placeholder(placeholder)
                            .default_value(initial)
                            .rows(4)
                    });
                    textareas.insert(field.key.clone(), entity);
                }
                fluxdb_app::AiPluginConfigFieldKind::Text => {
                    let entity = cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder(placeholder)
                            .default_value(initial)
                    });
                    inputs.insert(field.key.clone(), entity);
                }
            }
        }
        Self {
            instance_id: item.id.clone(),
            display_name: item.display_name.clone(),
            icon: ai_builtin_icon(&item.package_key),
            fields: item.config_fields.clone(),
            drafts,
            inputs,
            textareas,
            expected_revision,
            error: None,
        }
    }

    /// 草稿里某个字段的展示文案；选择已不在候选里时如实说明，不假装是选项。
    fn selection_label(&self, field: &fluxdb_app::AiPluginConfigFieldItem) -> String {
        let Some(id) = self.drafts.get(&field.key).and_then(|value| value.as_deref()) else {
            return tr("app.ai.plugins.not_configured").to_string();
        };
        field
            .choices
            .iter()
            .find(|choice| choice.id == id)
            .map(|choice| localize_ai_plugin_text(&choice.label))
            .unwrap_or_else(|| tr("app.ai.plugins.selected_unavailable").to_string())
    }

    /// 该字段当前是否"生效"：标量字段有非空值，资源字段选择了可用资源。
    fn field_is_active(&self, field: &fluxdb_app::AiPluginConfigFieldItem) -> bool {
        match field.kind {
            fluxdb_app::AiPluginConfigFieldKind::Resource => {
                self.drafts
                    .get(&field.key)
                    .and_then(|value| value.as_deref())
                    .is_some_and(|value| !value.is_empty())
            }
            fluxdb_app::AiPluginConfigFieldKind::Switch => self
                .drafts
                .get(&field.key)
                .and_then(|value| value.as_deref())
                .is_some_and(|value| value == "true"),
            _ => self
                .drafts
                .get(&field.key)
                .and_then(|value| value.as_deref())
                .is_some_and(|value| !value.trim().is_empty()),
        }
    }

    /// 是否含标量字段（决定要不要显示"几条规则生效"摘要）。
    fn has_scalar_fields(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.kind != fluxdb_app::AiPluginConfigFieldKind::Resource)
    }

    fn scalar_field_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|field| field.kind != fluxdb_app::AiPluginConfigFieldKind::Resource)
            .count()
    }

    fn active_scalar_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|field| field.kind != fluxdb_app::AiPluginConfigFieldKind::Resource)
            .filter(|field| self.field_is_active(field))
            .count()
    }

    fn group_field_count(&self, group: &str) -> usize {
        self.fields
            .iter()
            .filter(|field| field.group.as_deref() == Some(group))
            .count()
    }

    fn group_active_count(&self, group: &str) -> usize {
        self.fields
            .iter()
            .filter(|field| field.group.as_deref() == Some(group))
            .filter(|field| self.field_is_active(field))
            .count()
    }

    /// 提交给应用层的字段值：资源字段带 `resource_id`，标量字段带 `value`。
    ///
    /// 标量字段在提交时从输入实体现读，避免输入事件与草稿状态不同步；
    /// 空字符串与 `None` 都按"清空该字段"提交。
    fn selections(&self, cx: &App) -> Vec<fluxdb_app::AiPluginFieldSelection> {
        self.fields
            .iter()
            .map(|field| {
                let draft = self.drafts.get(&field.key).and_then(|value| value.clone());
                match field.kind {
                    fluxdb_app::AiPluginConfigFieldKind::Resource => {
                        fluxdb_app::AiPluginFieldSelection {
                            key: field.key.clone(),
                            resource_id: draft,
                            value: None,
                        }
                    }
                    fluxdb_app::AiPluginConfigFieldKind::TextList => {
                        let text = self
                            .textareas
                            .get(&field.key)
                            .map(|input| input.read(cx).value().to_string())
                            .unwrap_or_default();
                        fluxdb_app::AiPluginFieldSelection {
                            key: field.key.clone(),
                            resource_id: None,
                            value: if text.trim().is_empty() {
                                None
                            } else {
                                Some(text)
                            },
                        }
                    }
                    fluxdb_app::AiPluginConfigFieldKind::Text => {
                        let text = self
                            .inputs
                            .get(&field.key)
                            .map(|input| input.read(cx).value().to_string())
                            .unwrap_or_default();
                        fluxdb_app::AiPluginFieldSelection {
                            key: field.key.clone(),
                            resource_id: None,
                            value: if text.trim().is_empty() {
                                None
                            } else {
                                Some(text)
                            },
                        }
                    }
                    _ => fluxdb_app::AiPluginFieldSelection {
                        key: field.key.clone(),
                        resource_id: draft,
                        value: None,
                    },
                }
            })
            .collect()
    }
}

/// 插件管理弹框；未打开时不渲染。
fn ai_plugin_config_dialog(
    dialog: Option<AiPluginConfigDialog>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Option<gpui::AnyElement> {
    let dialog = dialog.as_ref()?;
    let this = cx.entity().downgrade();
    let mask = div()
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
                this.ai_plugin_config_dialog = None;
                cx.notify();
                cx.stop_propagation();
            }),
        );

    // 分组渲染：同一 `group` 的字段归到一块并给组标题；未命名的字段各自成段。
    // 分组名来自插件声明，UI 不判断插件 id、也不预设分组集合。
    let mut body = div().flex().flex_col().gap_2();
    let mut current_group: Option<String> = None;
    for field in &dialog.fields {
        let group = field.group.clone();
        if group != current_group {
            if let Some(name) = group.clone() {
                let active = dialog.group_active_count(&name);
                let total = dialog.group_field_count(&name);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pt_1()
                        .child(
                            div()
                                .w(px(3.))
                                .h(px(13.))
                                .rounded(px(2.))
                                .bg(colors.info),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(localize_ai_plugin_text(&name)),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(
                                    tr("app.ai.plugins.active_count")
                                        .replace("%{active}", &active.to_string())
                                        .replace("%{total}", &total.to_string()),
                                ),
                        ),
                );
            }
            current_group = group;
        }
        body = body.child(ai_plugin_config_field(field, dialog, colors, cx));
    }
    // 顶部摘要：一眼看到"配了几条、生效几条"，避免长表单里看不出整体状态。
    if dialog.has_scalar_fields() {
        let active = dialog.active_scalar_count();
        let total = dialog.scalar_field_count();
        body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(colors.radius)
                    .bg(colors.info.alpha(0.08))
                    .child(
                        div()
                            .text_size(ui_text_size(20.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(colors.text)
                            .child(active.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
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
                                            .text_size(ui_text_size(12.))
                                            .text_color(colors.text)
                                            .child(
                                                tr("app.ai.plugins.rules_active")
                                                    .replace("%{total}", &total.to_string()),
                                            ),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_size(ui_text_size(11.))
                                            .text_color(colors.muted)
                                            .child(tr("app.ai.plugins.rules_help")),
                                    ),
                            )
                            // 生效清单：从"生效几条"变成"哪几条"，不用逐条回看长表单。
                            .child({
                                let mut chips = div().flex().items_center().gap_1().flex_wrap();
                                for field in dialog
                                    .fields
                                    .iter()
                                    .filter(|f| {
                                        f.kind != fluxdb_app::AiPluginConfigFieldKind::Resource
                                    })
                                {
                                    let on = dialog.field_is_active(field);
                                    chips = chips.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py(px(1.))
                                            .rounded(px(4.))
                                            .bg(if on {
                                                colors.success.alpha(0.12)
                                            } else {
                                                colors.panel_alt
                                            })
                                            .child(
                                                div()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(if on { colors.success } else { colors.muted }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(ui_text_size(11.))
                                                    .text_color(if on {
                                                        colors.text
                                                    } else {
                                                        colors.muted
                                                    })
                                                    .child(localize_ai_plugin_text(&field.label)),
                                            ),
                                    );
                                }
                                chips
                            }),
                    ),
            )
            .child(body);
    }

    let header = div()
        .px_5()
        .py_4()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(app_icon_box(dialog.icon, 30., 16., colors.info))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(ui_text_size(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(
                                    tr("app.ai.plugins.manage_title")
                                        .replace("%{name}", &localize_ai_plugin_text(&dialog.display_name)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.ai.plugins.config_help")),
                        ),
                ),
        )
        .child(
            Button::new("ai-plugin-config-close")
                .debug_selector(|| "ai-plugin-config-close".into())
                .small()
                .ghost()
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_plugin_config_dialog = None;
                    cx.notify();
                })),
        );

    let button_row = div()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            Button::new("ai-plugin-config-cancel")
                .debug_selector(|| "ai-plugin-config-cancel".into())
                .small()
                .ghost()
                .child(tr("app.common.cancel"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_plugin_config_dialog = None;
                    cx.notify();
                })),
        )
        .child(
            Button::new("ai-plugin-config-save")
                .debug_selector(|| "ai-plugin-config-save".into())
                .small()
                .primary()
                .child(tr("app.common.save"))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| this.submit_ai_plugin_config(cx));
                }),
        );

    let footer = div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_5()
        .py_3()
        .flex()
        .flex_col()
        .gap_2()
        .when_some(dialog.error.clone(), |this, message| {
            let message = localize_ai_plugin_text(&localize_backend_error_text(&message));
            this.child(
                div()
                    .debug_selector(|| "ai-plugin-config-error".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(app_icon(AppIcon::CircleSlash, 14., colors.danger))
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.danger)
                            .child(message),
                    ),
            )
        })
        .child(button_row);

    let panel = div()
        .w(px(880.))
        .max_h(px(780.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(18.),
            px(48.),
            px(0.),
            hsla(220. / 360., 0.3, 0.1, 0.28),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        // 内部点击不穿透到遮罩。
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            // Esc 等同取消，不写入任何数据。
            this.ai_plugin_config_dialog = None;
            cx.notify();
            cx.stop_propagation();
        }))
        .child(header)
        .child(
            div()
                .flex_1()
                .overflow_y_scrollbar()
                .px_5()
                .pb_5()
                .child(body),
        )
        .child(footer);
    Some(mask.child(panel).into_any_element())
}

/// 一个字段：左侧标签（含必填标记），右侧选择器与说明。
fn ai_plugin_config_field(
    field: &fluxdb_app::AiPluginConfigFieldItem,
    dialog: &AiPluginConfigDialog,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    // 控件按字段种类分流；说明块与标签样式由 `ai_plugin_config_field_shell` 统一包裹。
    match field.kind {
        fluxdb_app::AiPluginConfigFieldKind::Resource => {
            ai_plugin_config_resource_field(field, dialog, colors, cx)
        }
        fluxdb_app::AiPluginConfigFieldKind::Switch => {
            ai_plugin_config_switch_field(field, dialog, colors, cx)
        }
        fluxdb_app::AiPluginConfigFieldKind::Select => {
            ai_plugin_config_select_field(field, dialog, colors, cx)
        }
        fluxdb_app::AiPluginConfigFieldKind::Text
        | fluxdb_app::AiPluginConfigFieldKind::TextList => {
            ai_plugin_config_text_field(field, dialog, colors, cx)
        }
    }
}

/// 字段外壳：左标签（含必填标记）+ 右控件 + 说明块（口径/后果/校验原因）。
/// 字段外壳：左标签列 + 右控件列。
///
/// 说明文字压成控件下方的两行小字（而不是整块灰底），8 个规则字段因此能在
/// 一屏内扫读；校验失败的原因仍单独用警示色，和普通说明区分开。
fn ai_plugin_config_field_shell(
    field: &fluxdb_app::AiPluginConfigFieldItem,
    controls: gpui::Div,
    colors: UiColors,
) -> gpui::AnyElement {
    let mut controls = controls;
    if let Some(help) = field.help.as_deref().map(localize_ai_plugin_text) {
        controls = controls.child(
            div()
                .flex()
                .items_start()
                .gap_1()
                .child(app_icon(AppIcon::Info, 11., colors.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(ui_text_size(11.))
                        .line_height(ui_text_size(16.))
                        .text_color(colors.muted)
                        .child(help),
                ),
        );
    }
    if let Some(reason) = field.rejection.clone() {
        controls = controls.child(
            div()
                .flex()
                .items_start()
                .gap_1()
                .child(app_icon(AppIcon::CircleSlash, 11., colors.warning))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(ui_text_size(11.))
                        .text_color(colors.warning)
                        .child(reason),
                ),
        );
    }
    div()
        .flex()
        .items_start()
        .gap_4()
        .py_1()
        .child(
            div()
                .w(px(176.))
                .flex_shrink_0()
                .pt(px(6.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(localize_ai_plugin_text(&field.label)),
                )
                .when(field.required, |this| {
                    this.child(ai_pill(tr("app.ai.plugins.required"), colors.warning, colors))
                }),
        )
        .child(controls)
        .into_any_element()
}

/// 开关字段：`Switch` 直接改草稿，保存时才落库。
fn ai_plugin_config_switch_field(
    field: &fluxdb_app::AiPluginConfigFieldItem,
    dialog: &AiPluginConfigDialog,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let checked = dialog
        .drafts
        .get(&field.key)
        .and_then(|value| value.as_deref())
        .is_some_and(|value| value == "true");
    let field_key = field.key.clone();
    let selector = format!("ai-plugin-config-switch:{}", field.key);
    let controls = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div().child(
                div()
                    .id(SharedString::from(format!("{selector}:wrap")))
                    .debug_selector({
                        let selector = format!("{selector}:wrap");
                        move || selector.clone()
                    })
                    .child(
                        Switch::new(SharedString::from(selector))
                            .checked(checked)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.set_ai_plugin_config_draft(
                                    &field_key,
                                    Some(checked.to_string()),
                                    cx,
                                );
                            })),
                    ),
            ),
        );
    ai_plugin_config_field_shell(field, controls, colors)
}

/// 标量文本字段：单行 `Input`、多行 `Textarea`；值在提交时从实体现读。
fn ai_plugin_config_text_field(
    field: &fluxdb_app::AiPluginConfigFieldItem,
    dialog: &AiPluginConfigDialog,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let selector = format!("ai-plugin-config-input:{}", field.key);
    let control = if field.kind == fluxdb_app::AiPluginConfigFieldKind::TextList {
        let Some(input) = dialog.textareas.get(&field.key) else {
            return ai_plugin_config_field_shell(field, div().flex_1().min_w(px(0.)), colors);
        };
        // 解析结果 = 保存时会真正落库的条目：把"存进去到底是什么"提前显示出来，
        // 重复项被去掉、行尾多余符号这类问题填错时立刻可见。
        let parsed = fluxdb_app::text_list_items(&input.read(cx).value());
        let mut col = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector({
                        let selector = selector.clone();
                        move || selector.clone()
                    })
                    .child(gpui_component::input::Textarea::new(input).h(px(88.))),
            );
        if !parsed.is_empty() {
            let mut chips = div().flex().items_center().gap_1().flex_wrap();
            chips = chips.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(
                        tr("app.ai.plugins.parsed_count")
                            .replace("%{count}", &parsed.len().to_string()),
                    ),
            );
            for item in parsed.iter().take(8) {
                chips = chips.child(
                    div()
                        .px_2()
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(colors.panel_alt)
                        .border_1()
                        .border_color(colors.border_soft)
                        .text_size(ui_text_size(11.))
                        .text_color(colors.text)
                        .child(item.clone()),
                );
            }
            if parsed.len() > 8 {
                chips = chips.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.ai.plugins.more_items")
                                .replace("%{count}", &parsed.len().to_string()),
                        ),
                );
            }
            let parsed_selector = format!("ai-plugin-config-parsed:{}", field.key);
            col = col.child(
                div()
                    .id(SharedString::from(parsed_selector.clone()))
                    .debug_selector({
                        let parsed_selector = parsed_selector.clone();
                        move || parsed_selector.clone()
                    })
                    .child(chips),
            );
        }
        col.into_any_element()
    } else {
        let Some(input) = dialog.inputs.get(&field.key) else {
            return ai_plugin_config_field_shell(field, div().flex_1().min_w(px(0.)), colors);
        };
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector({
                let selector = selector.clone();
                move || selector.clone()
            })
            .child(Input::new(input).text_size(ui_text_size(13.)))
            .into_any_element()
    };
    let controls = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(control);
    ai_plugin_config_field_shell(field, controls, colors)
}

/// 枚举字段：下拉改草稿，候选项由声明给出。
fn ai_plugin_config_select_field(
    field: &fluxdb_app::AiPluginConfigFieldItem,
    dialog: &AiPluginConfigDialog,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let current = dialog
        .drafts
        .get(&field.key)
        .and_then(|value| value.clone());
    let current_label = current
        .as_deref()
        .and_then(|value| {
            field
                .options
                .iter()
                .find(|option| option.value == value)
                .map(|option| localize_ai_plugin_text(&option.label))
        })
        .unwrap_or_else(|| tr("app.ai.plugins.not_configured").to_string());
    let unset = current.is_none();
    let options = field
        .options
        .iter()
        .map(|option| {
            (
                option.value.clone(),
                localize_ai_plugin_text(&option.label),
            )
        })
        .collect::<Vec<_>>();
    let owner = cx.entity().downgrade();
    let field_key = field.key.clone();
    let trigger_id = format!("ai-plugin-config-select:{}", field.key);
    let controls = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            Button::new(SharedString::from(trigger_id.clone()))
                .debug_selector({
                    let trigger_id = trigger_id.clone();
                    move || trigger_id.clone()
                })
                .small()
                .cursor_pointer()
                .w_full()
                .h(px(34.))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .min_w(px(0.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(ui_text_size(12.))
                                .text_color(if unset { colors.muted } else { colors.text })
                                .child(current_label),
                        )
                        .child(app_icon(AppIcon::ChevronDown, 14., colors.muted)),
                )
                .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
                    let mut menu = menu.min_w(px(220.)).max_w(px(320.));
                    for (value, label) in &options {
                        let owner = owner.clone();
                        let field_key = field_key.clone();
                        let value = value.clone();
                        let label = label.clone();
                        let option_id = format!("ai-plugin-config-select-option:{value}");
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| {
                                div()
                                    .id(SharedString::from(option_id.clone()))
                                    .debug_selector({
                                        let option_id = option_id.clone();
                                        move || option_id.clone()
                                    })
                                    .child(label.clone())
                            })
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.set_ai_plugin_config_draft(
                                        &field_key,
                                        Some(value.clone()),
                                        cx,
                                    )
                                });
                            }),
                        );
                    }
                    menu
                }),
        );
    ai_plugin_config_field_shell(field, controls, colors)
}

/// 资源字段：下拉选择模型（原实现）。
fn ai_plugin_config_resource_field(
    field: &fluxdb_app::AiPluginConfigFieldItem,
    dialog: &AiPluginConfigDialog,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let unset = dialog
        .drafts
        .get(&field.key)
        .and_then(|value| value.as_deref())
        .is_none();
    let label = dialog.selection_label(field);
    let choices = field
        .choices
        .iter()
        .map(|choice| {
            (
                choice.id.clone(),
                localize_ai_plugin_text(&choice.label),
                localize_ai_plugin_text(&choice.detail),
            )
        })
        .collect::<Vec<_>>();
    let key = field.key.clone();
    let current_key = field.key.clone();
    let current = dialog
        .drafts
        .get(&field.key)
        .and_then(|value| value.clone());
    let owner = cx.entity().downgrade();
    let trigger_id = format!("ai-plugin-config-field:{}", field.key);
    let trigger = Button::new(SharedString::from(trigger_id.clone()))
        .debug_selector({
            let id = trigger_id.clone();
            move || id.clone()
        })
        .small()
        .cursor_pointer()
        .w_full()
        .h(px(34.))
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .min_w(px(0.))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(ui_text_size(12.))
                        .text_color(if unset { colors.muted } else { colors.text })
                        .child(label),
                )
                .child(app_icon(AppIcon::ChevronDown, 14., colors.muted)),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.min_w(px(280.)).max_w(px(340.));
            let selected = current.clone();
            for (id, name, detail) in &choices {
                let owner = owner.clone();
                let field_key = current_key.clone();
                let value = id.clone();
                let name = name.clone();
                let detail = detail.clone();
                let checked = selected.as_deref() == Some(id.as_str());
                let option_id = format!("ai-plugin-config-option:{}:{}", key, id);
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(SharedString::from(option_id.clone()))
                            .debug_selector({
                                let id = option_id.clone();
                                move || id.clone()
                            })
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(name.clone())
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(colors.muted)
                                    .child(detail.clone()),
                            )
                    })
                    .checked(checked)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.set_ai_plugin_config_draft(&field_key, Some(value.clone()), cx)
                        });
                    }),
                );
            }
            let owner = owner.clone();
            let field_key = current_key.clone();
            let clear_id = format!("ai-plugin-config-clear:{}", key);
            menu.item(
                PopupMenuItem::element(move |_, _| {
                    div()
                        .id(SharedString::from(clear_id.clone()))
                        .debug_selector({
                            let id = clear_id.clone();
                            move || id.clone()
                        })
                        .text_color(colors.muted)
                        .child(tr("app.ai.plugins.not_configured_unused"))
                })
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.set_ai_plugin_config_draft(&field_key, None, cx)
                    });
                }),
            )
        });

    let controls = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(trigger)
        // 候选口径由宿主按声明生成：插件只声明需求，不自己解释过滤规则。
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(field.hint.clone()),
        );
    ai_plugin_config_field_shell(field, controls, colors)
}

impl NavicatMain {
    /// 打开插件管理弹框；字段与候选取当前列表快照（弹框内不再查库）。
    ///
    /// 无配置字段或声明解析失败的插件没有管理入口，这里也拒绝打开，避免出现空表单。
    fn open_ai_plugin_config_dialog(
        &mut self,
        instance_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let expected_revision = self.controller.state().ai_runtime_revision;
        let item = self
            .controller
            .state()
            .ai_plugin_instances
            .iter()
            .find(|plugin| plugin.id == instance_id)
            .cloned();
        let Some(item) = item else { return };
        if item.config_fields.is_empty() || item.config_error.is_some() {
            return;
        }
        self.ai_plugin_config_dialog =
            Some(AiPluginConfigDialog::new(&item, expected_revision, window, cx));
        cx.notify();
    }

    /// 改一个字段的草稿选择（不落库，保存时才提交）。
    fn set_ai_plugin_config_draft(
        &mut self,
        key: &str,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.ai_plugin_config_dialog.as_mut() {
            dialog.drafts.insert(key.to_string(), value);
            dialog.error = None;
        }
        cx.notify();
    }

    /// 提交弹框：一次写入所有字段，只有保存成功才关框，失败保留草稿并内联提示。
    fn submit_ai_plugin_config(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.ai_plugin_config_dialog.clone() else {
            return;
        };
        let event = self.controller.dispatch(AppCommand::SaveAiPluginConfig {
            instance_id: dialog.instance_id.clone(),
            selections: dialog.selections(cx),
            expected_revision: dialog.expected_revision,
        });
        match event {
            AppEvent::AiPluginConfigSaved { .. } => self.ai_plugin_config_dialog = None,
            AppEvent::AiSettingsError(message) => {
                if let Some(dialog) = self.ai_plugin_config_dialog.as_mut() {
                    dialog.error = Some(message);
                }
            }
            _ => {}
        }
        cx.notify();
    }
}
