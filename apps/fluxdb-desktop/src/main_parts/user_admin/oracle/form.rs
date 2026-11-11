// 新建账号表单。口令只存在草稿里，提交后由预览渲染成 `IDENTIFIED BY "********"`；
// 界面与日志都不回显口令本身（docs/oracle.md §8）。

fn oracle_create_form(
    tab_id: TabId,
    state: &OracleUserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(draft) = state.creating.as_ref() else {
        return oracle_empty_panel(tr("app.oracle_security.select_user"), colors).into_any_element();
    };
    let editable = !state.applying && !state.loading;
    let mismatch = !draft.password.is_empty() && draft.password != draft.confirm_password;
    let mut form = div().max_w(px(620.)).flex().flex_col().gap_3().child(
        div()
            .mb_1()
            .text_size(ui_text_size(15.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(colors.text)
            .child(tr("app.oracle_security.create_user")),
    );
    form = form
        .child(oracle_form_input(
            tr("app.oracle_security.user_name"),
            this.oracle_security_name_input.clone(),
            "oracle-security-form-name",
            editable,
            window,
            colors,
            cx,
        ))
        .child(oracle_form_input(
            tr("app.oracle_security.password"),
            this.oracle_security_password_input.clone(),
            "oracle-security-form-password",
            editable,
            window,
            colors,
            cx,
        ))
        .child(oracle_form_input(
            tr("app.oracle_security.confirm_password"),
            this.oracle_security_confirm_password_input.clone(),
            "oracle-security-form-confirm-password",
            editable,
            window,
            colors,
            cx,
        ))
        .child(oracle_form_input(
            tr("app.oracle_security.default_tablespace"),
            this.oracle_security_default_ts_input.clone(),
            "oracle-security-form-default-ts",
            editable,
            window,
            colors,
            cx,
        ))
        .child(oracle_form_input(
            tr("app.oracle_security.temporary_tablespace"),
            this.oracle_security_temp_ts_input.clone(),
            "oracle-security-form-temp-ts",
            editable,
            window,
            colors,
            cx,
        ))
        .child(oracle_form_input(
            tr("app.oracle_security.profile"),
            this.oracle_security_profile_input.clone(),
            "oracle-security-form-profile",
            editable,
            window,
            colors,
            cx,
        ));
    // 达梦没有 `QUOTA ... ON ...` 子句，配额输入框画出来只会骗人：直接不渲染。
    if state.dialect == fluxdb_core::UserAdminSqlDialect::Oracle {
        form = form
            .child(oracle_form_input(
                tr("app.oracle_security.quota_tablespace"),
                this.oracle_security_quota_ts_input.clone(),
                "oracle-security-form-quota-ts",
                editable,
                window,
                colors,
                cx,
            ))
            .child(oracle_form_input(
                tr("app.oracle_security.quota_mb"),
                this.oracle_security_quota_bytes_input.clone(),
                "oracle-security-form-quota-mb",
                editable,
                window,
                colors,
                cx,
            ));
    }
    form = form.child(oracle_form_row(
        tr("app.oracle_security.grant_connect_role"),
        colors,
    ).child(
        Switch::new("oracle-security-grant-connect")
            .checked(draft.grant_connect_role)
            .disabled(!editable)
            .label(tr("app.oracle_security.grant_connect_role_hint"))
            .on_click(cx.listener(move |this, checked, _, cx| {
                this.dispatch(
                    AppCommand::SetOracleCreateDraftFlag { tab_id, grant_connect_role: *checked },
                    cx,
                );
            })),
    ));
    form = form.child(oracle_form_hint(tr("app.oracle_security.create_hint"), colors));
    // 与 MySQL / PG 一致：新建就是填表，底部没有独立的「提交」按钮，
    // 由工具栏的保存统一出 SQL 预览、取消放弃草稿。
    if mismatch {
        form = form.child(oracle_form_error(tr(
            "app.oracle_security.password_mismatch",
        )));
    }
    if let Some(error) = &state.apply_error {
        form = form.child(user_admin_error_row(&error.message, colors));
    }
    oracle_panel_shell(form, colors)
}
