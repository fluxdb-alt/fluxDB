impl NavicatMain {
    fn stage_sqlserver_security_form(&mut self, tab_id: TabId, cx: &mut Context<Self>) -> bool {
        let Some(state) = self
            .user_admin_state_for(tab_id)
            .and_then(|admin| admin.sqlserver)
        else {
            return false;
        };
        if !state.has_form_changes() {
            return true;
        }
        !matches!(
            self.dispatch(AppCommand::SubmitSqlServerSecurityForm(tab_id), cx),
            AppEvent::Failed(_)
        )
    }

    fn prepare_sqlserver_security_preview(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self
            .user_admin_state_for(tab_id)
            .and_then(|admin| admin.sqlserver)
        else {
            return false;
        };
        if state.applying
            || state.loading
            || state.save_status == SqlServerSecuritySaveStatus::NeedsVerify
        {
            return false;
        }
        if !self.stage_sqlserver_security_form(tab_id, cx) {
            return false;
        }
        self.user_admin_state_for(tab_id)
            .and_then(|admin| admin.sqlserver)
            .is_some_and(|state| state.has_pending_changes())
    }

    fn sync_sqlserver_security_selects(
        &mut self,
        state: &SqlServerSecurityState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let logins = state.sql_login_names();
        let database_field = matches!(
            state.form.kind,
            SqlServerSecurityFormKind::CreateLogin | SqlServerSecurityFormKind::EditLogin
        );
        let mut defaults = if database_field {
            state.databases.clone()
        } else {
            state
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.schemas.clone())
                .unwrap_or_default()
        };
        // 保留服务端已有的默认值；读取权限受限时，不因候选缺失而误改它。
        if !state.form.default_value.is_empty() && !defaults.contains(&state.form.default_value) {
            defaults.push(state.form.default_value.clone());
        }
        let options = (logins, defaults);
        if self.sqlserver_security_form_options != options {
            self.sqlserver_security_form_options = options.clone();
            self.sqlserver_security_login_select
                .update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(options.0), window, cx);
                });
            self.sqlserver_security_default_select
                .update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(options.1), window, cx);
                });
        }
        sync_select_value(
            &self.sqlserver_security_login_select,
            &state.form.login,
            window,
            cx,
        );
        sync_select_value(
            &self.sqlserver_security_default_select,
            &state.form.default_value,
            window,
            cx,
        );
    }
}

fn sqlserver_form_panel(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let kind = state.form.kind;
    let create = matches!(
        kind,
        SqlServerSecurityFormKind::CreateLogin
            | SqlServerSecurityFormKind::CreateUser
            | SqlServerSecurityFormKind::CreateRole
    );
    let editable = !state.applying
        && !state.loading
        && !state.preview_loading
        && state.save_status != SqlServerSecuritySaveStatus::NeedsVerify;
    let title = match kind {
        SqlServerSecurityFormKind::CreateLogin => {
            tr("app.sqlserver_security.create_login").to_string()
        }
        SqlServerSecurityFormKind::CreateUser => {
            tr("app.sqlserver_security.create_user").to_string()
        }
        SqlServerSecurityFormKind::CreateRole => {
            tr("app.sqlserver_security.create_role").to_string()
        }
        SqlServerSecurityFormKind::GrantPermission => {
            tr("app.sqlserver_security.grant_permission").to_string()
        }
        _ => state.form.name.clone(),
    };
    let mut form = div().max_w(px(620.)).flex().flex_col().gap_3().child(
        div()
            .mb_1()
            .text_size(ui_text_size(15.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(colors.text)
            .child(title),
    );
    match kind {
        SqlServerSecurityFormKind::CreateLogin | SqlServerSecurityFormKind::EditLogin => {
            form = form.child(sqlserver_form_input(
                tr("app.sqlserver_security.login_name"),
                this.sqlserver_security_name_input.clone(),
                "sqlserver-security-form-name",
                editable && create,
                window,
                colors,
                cx,
            ));
            if !create && let Some(login) = state.selected_login() {
                let enabled = state.effective_login_enabled(&login.name, !login.is_disabled);
                let can_toggle = editable
                    && state
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.current_login != login.name);
                form = form.child(
                    sqlserver_form_row(tr("app.sqlserver_security.allow_login"), colors).child(
                        Switch::new("sqlserver-security-login-enabled")
                            .checked(enabled)
                            .disabled(!can_toggle)
                            .on_click(cx.listener(move |this, checked, _, cx| {
                                this.dispatch(
                                    AppCommand::SetSqlServerSelectedLoginEnabled {
                                        tab_id,
                                        enabled: *checked,
                                    },
                                    cx,
                                );
                            })),
                    ),
                );
            }
            form = form
                .child(sqlserver_form_input(
                    tr("app.sqlserver_security.password"),
                    this.sqlserver_security_password_input.clone(),
                    "sqlserver-security-form-password",
                    editable,
                    window,
                    colors,
                    cx,
                ))
                .child(sqlserver_form_input(
                    tr("app.sqlserver_security.confirm_password"),
                    this.sqlserver_security_confirm_password_input.clone(),
                    "sqlserver-security-form-confirm-password",
                    editable,
                    window,
                    colors,
                    cx,
                ));
            if kind == SqlServerSecurityFormKind::EditLogin {
                form = form.child(sqlserver_form_select(
                    tr("app.sqlserver_security.default_database"),
                    this.sqlserver_security_default_select.clone(),
                    "sqlserver-security-form-default",
                    editable,
                    colors,
                ));
            }
            form = form.child(sqlserver_form_hint(
                if create {
                    tr("app.sqlserver_security.login_help")
                } else {
                    tr("app.sqlserver_security.password_keep_hint")
                },
                colors,
            ));
        }
        SqlServerSecurityFormKind::CreateUser | SqlServerSecurityFormKind::EditUser => {
            form = form
                .child(sqlserver_form_input(
                    tr("app.sqlserver_security.user_name"),
                    this.sqlserver_security_name_input.clone(),
                    "sqlserver-security-form-name",
                    editable && create,
                    window,
                    colors,
                    cx,
                ))
                .child(sqlserver_form_select(
                    tr("app.sqlserver_security.mapped_login"),
                    this.sqlserver_security_login_select.clone(),
                    "sqlserver-security-form-login",
                    editable,
                    colors,
                ))
                .child(sqlserver_form_select(
                    tr("app.sqlserver_security.default_schema"),
                    this.sqlserver_security_default_select.clone(),
                    "sqlserver-security-form-default",
                    editable,
                    colors,
                ))
                .child(sqlserver_form_hint(
                    tr("app.sqlserver_security.user_help"),
                    colors,
                ));
        }
        SqlServerSecurityFormKind::CreateRole => {
            form = form.child(sqlserver_form_input(
                tr("app.sqlserver_security.role_name"),
                this.sqlserver_security_name_input.clone(),
                "sqlserver-security-form-name",
                editable,
                window,
                colors,
                cx,
            ));
        }
        SqlServerSecurityFormKind::GrantPermission => {
            form = form.child(sqlserver_permission_scope_row(tab_id, state, colors, cx));
            if matches!(state.form.permission_scope.as_str(), "SCHEMA" | "OBJECT") {
                form = form.child(sqlserver_form_input(
                    if state.form.permission_scope == "OBJECT" {
                        tr("app.sqlserver_security.permission_object")
                    } else {
                        tr("app.sqlserver_security.permission_schema")
                    },
                    this.sqlserver_security_permission_target_input.clone(),
                    "sqlserver-security-form-target",
                    editable,
                    window,
                    colors,
                    cx,
                ));
            }
            form = form
                .child(sqlserver_permission_chips(state, colors, cx))
                .child(sqlserver_permission_state_row(tab_id, state, colors, cx));
        }
        SqlServerSecurityFormKind::None => {}
    }
    if !state.form.password.is_empty() && state.form.password != state.form.confirm_password {
        form = form.child(
            div()
                .ml(px(120.))
                .text_size(ui_text_size(12.))
                .text_color(rgb(0xff3b45))
                .child(tr("app.sqlserver_security.password_mismatch")),
        );
    }
    if let Some(error) = &state.apply_error {
        form = form.child(user_admin_error_row(&error.message, colors));
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .child(form)
        .into_any_element()
}

fn sqlserver_form_row(label: SharedString, colors: UiColors) -> Div {
    // 行高固定 38px（与 MySQL / PG / Oracle 一致），避免多行标签把行撑高。
    h_flex().h(px(38.)).gap_3().items_center().child(
        div()
            .w(px(120.))
            .flex_none()
            .text_size(ui_text_size(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(colors.text)
            .child(label),
    )
}

fn sqlserver_form_input(
    label: SharedString,
    input: Entity<InputState>,
    id: &'static str,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    sqlserver_form_row(label, colors)
        .debug_selector(move || format!("{id}-row"))
        .child(
            div()
                .w(px(360.))
                .h(px(34.))
                .rounded(colors.radius)
                .debug_selector(move || id.to_string())
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
                        .id(id)
                        .small()
                        .appearance(false)
                        .focus_bordered(false)
                        .disabled(!enabled)
                        .w_full()
                        .h_full()
                        .px_2()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.text),
                ),
        )
}

fn sqlserver_form_select(
    label: SharedString,
    select: Entity<SelectState<SearchableVec<String>>>,
    id: &'static str,
    enabled: bool,
    colors: UiColors,
) -> Div {
    sqlserver_form_row(label, colors).child(
        div()
            .w(px(360.))
            .debug_selector(move || id.to_string())
            .child(
                Select::new(&select)
                    .placeholder(tr("app.common.choose"))
                    .w_full()
                    .h(px(34.))
                    .menu_width(px(360.))
                    .disabled(!enabled),
            ),
    )
}

fn sqlserver_form_hint(text: SharedString, colors: UiColors) -> Div {
    div()
        .ml(px(132.))
        .max_w(px(360.))
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(text)
}
