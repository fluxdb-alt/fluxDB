// PG 用户与角色工作台：高级页。
//
// 「角色能力」（CREATEDB/CREATEROLE/INHERIT）+「敏感能力」（SUPERUSER/REPLICATION/
// BYPASSRLS）+「连接限制」。预定义角色只读；INHERIT 的版本语义：PG≤15 控制自动继承，
// PG16+ 是新成员授权的 INHERIT 默认值（页内提示说明，避免误解为已有成员关系的总开关）。

fn pg_advanced_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let Some(draft) = admin.pg_draft.clone() else {
        return pg_advanced_empty(colors).into_any_element();
    };
    let is_predefined = !draft.create
        && admin
            .pg_selected_role
            .as_deref()
            .is_some_and(UserAdminState::pg_is_predefined_role);
    let member_options = admin.pg_member_options_supported;

    // Switch on_click 的事件载荷是切换后的选中值（&bool）。
    let attr_toggle = |field: PgDraftAttrField| {
        move |this: &mut NavicatMain, _checked: &bool, _window: &mut Window, cx: &mut Context<NavicatMain>| {
            let next = this
                .user_admin_state_for(tab_id)
                .and_then(|admin| admin.pg_draft.as_ref().map(|draft| !draft.attr(field)))
                .unwrap_or(false);
            this.dispatch(AppCommand::SetPgDraftAttr { tab_id, field, value: next }, cx);
        }
    };

    let mut form = div().flex().flex_col().gap_2();
    form = form
        .child(pg_section_title(tr("app.pg_role.advanced.capabilities"), colors))
        .child(pg_attr_switch_row(
            tr("app.pg_role.attr.createdb.label"),
            "CREATEDB",
            tr("app.pg_role.attr.createdb.hint"),
            draft.can_create_db,
            is_predefined,
            attr_toggle(PgDraftAttrField::CanCreateDb),
            colors,
            cx,
        ))
        .child(pg_attr_switch_row(
            tr("app.pg_role.attr.createrole.label"),
            "CREATEROLE",
            tr("app.pg_role.attr.createrole.hint"),
            draft.can_create_role,
            is_predefined,
            attr_toggle(PgDraftAttrField::CanCreateRole),
            colors,
            cx,
        ))
        .child(pg_attr_switch_row(
            tr("app.pg_role.attr.inherit.label"),
            "INHERIT",
            match member_options {
                Some(true) => tr("app.pg_role.attr.inherit.pg16"),
                Some(false) => tr("app.pg_role.attr.inherit.pg15"),
                None => tr("app.pg_role.attr.inherit.unknown"),
            },
            draft.inherit,
            is_predefined,
            attr_toggle(PgDraftAttrField::Inherit),
            colors,
            cx,
        ));

    form = form
        .child(div().h(px(6.)))
        .child(pg_section_title(tr("app.pg_role.advanced.sensitive"), colors))
        .child(pg_attr_switch_row(tr("app.pg_role.attr.superuser.label"), "SUPERUSER", tr("app.pg_role.attr.superuser.hint"), draft.is_superuser, is_predefined, attr_toggle(PgDraftAttrField::IsSuperuser), colors, cx))
        .child(pg_attr_switch_row(tr("app.pg_role.attr.replication.label"), "REPLICATION", tr("app.pg_role.attr.replication.hint"), draft.is_replication, is_predefined, attr_toggle(PgDraftAttrField::IsReplication), colors, cx))
        .child(pg_attr_switch_row(tr("app.pg_role.attr.bypassrls.label"), "BYPASSRLS", tr("app.pg_role.attr.bypassrls.hint"), draft.bypass_rls, is_predefined, attr_toggle(PgDraftAttrField::BypassRls), colors, cx));

    form = form
        .child(div().h(px(6.)))
        .child(pg_section_title(tr("app.pg_role.advanced.connection_limit"), colors))
        .child(pg_connection_limit_row(
            this.pg_connection_limit_input.clone(),
            !is_predefined,
            window,
            colors,
            cx,
        ))
        .child(pg_advanced_hint(
            tr("app.pg_role.advanced.connection_limit_hint"),
            colors,
        ));

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .child(div().max_w(px(680.)).flex().flex_col().gap_2().child(form))
        .into_any_element()
}

fn pg_advanced_empty(colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_text_size(13.))
        .text_color(colors.muted)
        .child(tr("app.pg_role.select_role"))
}

fn pg_section_title(title: impl Into<SharedString>, colors: UiColors) -> Div {
    let title: SharedString = title.into();
    div()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(title)
}

/// 属性开关行：中文名 + SQL 属性 + 一句解释。
#[allow(clippy::too_many_arguments)]
fn pg_attr_switch_row(
    label: impl Into<SharedString>,
    attr: &'static str,
    hint: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
    on_click: impl Fn(&mut NavicatMain, &bool, &mut Window, &mut Context<NavicatMain>) + 'static,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let hint: SharedString = hint.into();
    let label: SharedString = label.into();
    div()
        .h(px(48.))
        .flex()
        .items_center()
        .gap_3()
        .child(pg_form_label_advanced(label, colors))
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    Switch::new(SharedString::from(format!("pg-attr-switch-{attr}")))
                        .checked(checked)
                        .disabled(disabled)
                        .on_click(cx.listener(on_click)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .font_family("monospace")
                        .text_color(colors.muted)
                        .child(attr),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(hint),
        )
}

fn pg_form_label_advanced(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label: SharedString = label.into();
    div()
        .w(px(120.))
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text)
        .child(label)
}

fn pg_connection_limit_row(
    input: Entity<InputState>,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(pg_form_label_advanced(tr("app.pg_role.field.connection_limit"), colors))
        .child(pg_form_input_box(input, enabled, window, colors, cx))
}

fn pg_advanced_hint(text: impl Into<SharedString>, colors: UiColors) -> Div {
    let text: SharedString = text.into();
    div()
        .ml(px(132.))
        .max_w(px(360.))
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(text)
}
