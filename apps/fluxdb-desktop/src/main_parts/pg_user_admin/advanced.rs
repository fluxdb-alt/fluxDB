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
        .child(pg_section_title("角色能力", colors))
        .child(pg_attr_switch_row("创建数据库", "CREATEDB", "可以在集群中创建数据库。", draft.can_create_db, is_predefined, attr_toggle(PgDraftAttrField::CanCreateDb), colors, cx))
        .child(pg_attr_switch_row("创建与管理角色", "CREATEROLE", "可以创建与管理角色；实际可管理范围受版本与 ADMIN 规则约束。", draft.can_create_role, is_predefined, attr_toggle(PgDraftAttrField::CanCreateRole), colors, cx))
        .child(pg_attr_switch_row(
            "继承权限",
            "INHERIT",
            match member_options {
                Some(true) => "PG16+：作为该角色新成员授权的 INHERIT 默认值，不是已有成员关系的总开关。",
                Some(false) => "PG15 及以前：控制是否自动继承所属角色的权限。",
                None => "控制权限继承；具体语义随服务端版本（PG15/PG16+）不同。",
            },
            draft.inherit,
            is_predefined,
            attr_toggle(PgDraftAttrField::Inherit),
            colors,
            cx,
        ));

    form = form
        .child(div().h(px(6.)))
        .child(pg_section_title("敏感能力", colors))
        .child(pg_attr_switch_row("超级用户", "SUPERUSER", "绕过除登录外的所有权限检查。请谨慎授予。", draft.is_superuser, is_predefined, attr_toggle(PgDraftAttrField::IsSuperuser), colors, cx))
        .child(pg_attr_switch_row("复制能力", "REPLICATION", "允许流复制连接。", draft.is_replication, is_predefined, attr_toggle(PgDraftAttrField::IsReplication), colors, cx))
        .child(pg_attr_switch_row("绕过行级安全", "BYPASSRLS", "忽略所有行级安全策略。请谨慎授予。", draft.bypass_rls, is_predefined, attr_toggle(PgDraftAttrField::BypassRls), colors, cx));

    form = form
        .child(div().h(px(6.)))
        .child(pg_section_title("连接限制", colors))
        .child(pg_connection_limit_row(
            this.pg_connection_limit_input.clone(),
            !is_predefined,
            window,
            colors,
            cx,
        ))
        .child(pg_advanced_hint(
            "-1 表示不限制。该限制针对普通连接，不是精确限流器；超级用户连接等场景存在例外。",
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
        .text_size(px(13.))
        .text_color(colors.muted)
        .child("请选择角色")
}

fn pg_section_title(title: &'static str, colors: UiColors) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(title)
}

/// 属性开关行：中文名 + SQL 属性 + 一句解释。
#[allow(clippy::too_many_arguments)]
fn pg_attr_switch_row(
    label: &'static str,
    attr: &'static str,
    hint: &'static str,
    checked: bool,
    disabled: bool,
    on_click: impl Fn(&mut NavicatMain, &bool, &mut Window, &mut Context<NavicatMain>) + 'static,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
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
                        .text_size(px(11.))
                        .font_family("monospace")
                        .text_color(colors.muted)
                        .child(attr),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(hint),
        )
}

fn pg_form_label_advanced(label: &'static str, colors: UiColors) -> Div {
    div()
        .w(px(120.))
        .text_size(px(13.))
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
        .child(pg_form_label_advanced("连接数限制:", colors))
        .child(pg_form_input_box(input, enabled, window, colors, cx))
}

fn pg_advanced_hint(text: &'static str, colors: UiColors) -> Div {
    div()
        .ml(px(132.))
        .max_w(px(360.))
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(text)
}
