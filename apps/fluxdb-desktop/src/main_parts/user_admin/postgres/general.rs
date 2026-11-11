// PG 用户与角色工作台：常规页。
//
// 新建、改名、改密、有效期都在本页完成；所有修改只进入草稿，顶部「保存」统一提交。
// 表单规范：标签宽约 120px，输入宽 360px、高 34px，行距约 16px；复用项目表单封装。

fn pg_general_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let Some(draft) = admin.pg_draft.clone() else {
        return div()
            .flex_1()
            .min_h(px(0.))
            .bg(colors.panel_bg)
            .flex()
            .items_center()
            .justify_center()
            .text_size(ui_text_size(13.))
            .text_color(colors.muted)
            .child(tr("app.pg_role.select_or_new"))
            .into_any_element();
    };
    let is_predefined = !draft.create && admin.pg_selected_role.as_deref().is_some_and(UserAdminState::pg_is_predefined_role);
    let password_set = matches!(&draft.password, PgPasswordOp::Set(_));
    let password_mismatch = password_set
        && this.pg_password_input.read(cx).value().as_ref()
            != this.pg_password_confirm_input.read(cx).value().as_ref();
    let custom_valid_until = matches!(draft.valid_until, PgValidUntilOp::At(_));
    // 改名提示：旧 MD5 密码会随改名失效（不读取密码散列，仅提示）。
    let renamed = !draft.create
        && admin
            .pg_baseline_role()
            .is_some_and(|base| base.name != draft.name.trim());

    let mut form = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(pg_text_row(
            tr("app.pg_role.field.name"),
            this.pg_role_name_input.clone(),
            !is_predefined,
            window,
            colors,
            cx,
        ))
        .child(
            // 预定义角色不可改 LOGIN（服务端固定属性）；普通角色可切换。
            pg_switch_row(
                tab_id,
                tr("app.pg_role.field.can_login"),
                draft.can_login,
                !is_predefined,
                colors,
                cx,
            ),
        );
    form = form.child(pg_select_row_dynamic(
        tr("app.pg_role.field.password_op"),
        this.pg_password_op_select.clone(),
        !is_predefined,
        colors,
    ));
    if password_set {
        form = form
            .child(pg_password_row(tr("app.pg_role.field.new_password"), this.pg_password_input.clone(), this, window, colors, cx))
            .child(pg_password_row(
                tr("app.pg_role.field.confirm_password"),
                this.pg_password_confirm_input.clone(),
                this,
                window,
                colors,
                cx,
            ));
        if password_mismatch {
            form = form.child(pg_form_hint(tr("app.pg_role.error.password_mismatch").as_ref(), true, colors));
        }
    }
    form = form.child(pg_select_row_dynamic(
        tr("app.pg_role.field.valid_until"),
        this.pg_valid_until_mode_select.clone(),
        !is_predefined,
        colors,
    ));
    if custom_valid_until {
        form = form.child(pg_text_row(
            tr("app.pg_role.field.until"),
            this.pg_valid_until_input.clone(),
            true,
            window,
            colors,
            cx,
        ));
        form = form.child(pg_form_hint(
            tr("app.pg_role.valid_until.help").as_ref(),
            false,
            colors,
        ));
    }
    if is_predefined {
        form = form.child(pg_form_hint(tr("app.pg_role.hint.predefined").as_ref(), false, colors));
    }
    if renamed {
        form = form.child(pg_form_hint(
            tr("app.pg_role.hint.rename_md5").as_ref(),
            false,
            colors,
        ));
    }
    if !draft.can_login {
        form = form.child(pg_form_hint(
            tr("app.pg_role.hint.nologin").as_ref(),
            false,
            colors,
        ));
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .child(
            div()
                .max_w(px(620.))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(15.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(if draft.create {
                                    draft.name.trim().to_string().is_empty()
                                        .then(|| tr("app.pg_role.draft_title_empty").to_string())
                                        .unwrap_or_else(|| draft.name.trim().to_string())
                                } else {
                                    admin.pg_selected_role.clone().unwrap_or_default()
                                }),
                        )
                        .child(pg_role_chip(if draft.can_login {
                            tr("app.pg_role.identity.can_login")
                        } else {
                            tr("app.pg_role.identity.cannot_login")
                        }, colors))
                        .child(pg_role_chip(tr("app.pg_role.identity.cluster"), colors)),
                )
                .child(div().h(px(4.)))
                .child(form),
        )
        .into_any_element()
}

fn pg_role_chip(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label: SharedString = label.into();
    div()
        .px_2()
        .h(px(20.))
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(ui_text_size(11.))
        .text_color(colors.muted)
        .child(label)
}

/// 标签 + 文本输入行（enabled=false 时输入禁用，用于预定义角色）。
fn pg_text_row(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(pg_form_label(label.clone(), colors))
        .child(pg_form_input_box(input, enabled, window, colors, cx))
}

/// 项目外框规范输入框（34px 高、主题化边框），带禁用态。
fn pg_form_input_box(
    input: Entity<InputState>,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w(px(360.))
        .h(px(34.))
        .rounded(colors.radius)
        .border_1()
        .border_color(user_admin_input_border_color(focused, colors))
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| {
            style.border_color(user_admin_input_hover_border_color(focused, colors))
        })
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .focus_bordered(false)
                .disabled(!enabled)
                .w_full()
                .h_full()
                .px_2()
                .text_size(ui_text_size(13.))
                .text_color(colors.text),
        )
}

/// 标签 + 密码输入行（始终脱敏显示；显隐切换只影响 MySQL 页输入，
/// PG 密码固定掩码显示，避免误投影到共享输入实体）。
fn pg_password_row(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    _this: &NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(pg_form_label(label, colors))
        .child(user_admin_form_input_box(input, window, colors, cx))
}

/// 标签 + 开关行（Switch 组件自持 on_click；预定义角色禁用）。
fn pg_switch_row(
    tab_id: TabId,
    label: impl Into<SharedString>,
    checked: bool,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(pg_form_label(label.clone(), colors))
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    Switch::new(SharedString::from(format!("pg-switch-{label}")))
                        .checked(checked)
                        .disabled(!enabled)
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            let next = this
                                .user_admin_state_for(tab_id)
                                .and_then(|admin| admin.pg_draft.as_ref().map(|d| !d.can_login))
                                .unwrap_or(false);
                            this.dispatch(
                                AppCommand::SetPgDraftCanLogin { tab_id, can_login: next },
                                cx,
                            );
                        })),
                )
                .when(!enabled, |row| {
                    row.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.pg_role.predefined_readonly")),
                    )
                }),
        )
}

/// 标签 + 下拉行（选项与选中值由 sync_pg_inputs 统一同步；enabled=false 禁用）。
fn pg_select_row_dynamic(
    label: impl Into<SharedString>,
    select: Entity<SelectState<SearchableVec<String>>>,
    enabled: bool,
    colors: UiColors,
) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(pg_form_label(label, colors))
        .child(
            div().w(px(360.)).child(
                Select::new(&select)
                    .placeholder(tr("app.pg_role.placeholder.select"))
                    .w_full()
                    .h(px(34.))
                    .menu_width(px(360.))
                    .disabled(!enabled),
            ),
        )
}

fn pg_form_label(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label: SharedString = label.into();
    div()
        .w(px(120.))
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text)
        .child(label)
}

/// 表单提示/错误行。
fn pg_form_hint(text: &str, is_error: bool, colors: UiColors) -> Div {
    div()
        .ml(px(132.))
        .max_w(px(360.))
        .text_size(ui_text_size(12.))
        .text_color(if is_error { rgb(0xff3b45) } else { colors.muted })
        .child(text.to_string())
}
