// 「角色」与「权限」页。
//
// 只读当前账号的授权；其它账号一律显示「目录不可读」，绝不画成空授权或直接可撤销。
// `INHERITED = YES` 的行标为继承，不给撤销按钮（docs/oracle.md §8）。

/// 「角色」页：顶部是类型锁死为「角色」的授权编辑器，下面是已授予的角色。
///
/// 新建草稿同样可用编辑器（被授权人取草稿名），只是读不到既有授权。
fn oracle_roles_panel(
    tab_id: TabId,
    state: &OracleUserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(oracle_grant_composer(
            tab_id,
            state,
            OracleGrantKind::Role,
            this,
            window,
            colors,
            cx,
        ));
    if !state.selected_is_self() && state.creating.is_none() {
        body = body.child(oracle_form_hint(tr("app.oracle_security.grants_unreadable"), colors));
        return oracle_panel_shell(body, colors);
    }
    body = body.child(oracle_section_title(
        tr("app.oracle_security.roles_section"),
        colors,
    ));
    // 新建草稿在目录里还不存在，没有可读、可撤销的既有授权：
    // 只提示保存时连同新建一起授予，不画成空授权。
    if state.creating.is_some() {
        body = body.child(user_admin_empty_row(
            tr("app.oracle_security.grants_empty"),
            colors,
        ));
        return oracle_panel_shell(body, colors);
    }
    let Some(snapshot) = state.snapshot.as_ref() else {
        return oracle_panel_shell(
            body.child(inline_loading_row(tr("app.oracle_security.loading"), colors)),
            colors,
        );
    };
    if snapshot.roles.is_empty() {
        body = body.child(user_admin_empty_row(tr("app.oracle_security.grants_empty"), colors));
    }
    for (index, role) in snapshot.roles.iter().enumerate() {
        // 继承行没有撤销命令；直接授予才生成 RevokeOracleRole。
        let revoke = if role.inherited {
            None
        } else {
            Some(AppCommand::RevokeOracleRole {
                tab_id,
                role: role.role.clone(),
            })
        };
        body = body.child(oracle_privilege_row(
            "oracle-security-role",
            index,
            role.role.clone(),
            tr("app.oracle_security.default_role_hint")
                .replace("%{name}", if role.default_role { "YES" } else { "NO" })
                .into(),
            role.inherited,
            role.admin_option,
            revoke,
            colors,
            cx,
        ));
    }
    oracle_panel_shell(body, colors)
}

/// 「系统权限」页：类型锁死为「系统权限」。
fn oracle_system_privileges_panel(
    tab_id: TabId,
    state: &OracleUserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(oracle_grant_composer(
            tab_id,
            state,
            OracleGrantKind::System,
            this,
            window,
            colors,
            cx,
        ));
    if !state.selected_is_self() && state.creating.is_none() {
        body = body.child(oracle_form_hint(tr("app.oracle_security.grants_unreadable"), colors));
        return oracle_panel_shell(body, colors);
    }
    if state.creating.is_some() {
        return oracle_panel_shell(body, colors);
    }
    let Some(snapshot) = state.snapshot.as_ref() else {
        return oracle_panel_shell(
            body.child(inline_loading_row(tr("app.oracle_security.loading"), colors)),
            colors,
        );
    };
    body = body.child(oracle_section_title(
        tr("app.oracle_security.system_privileges"),
        colors,
    ));
    if snapshot.system_privileges.is_empty() {
        body = body.child(user_admin_empty_row(tr("app.oracle_security.grants_empty"), colors));
    }
    for (index, privilege) in snapshot.system_privileges.iter().enumerate() {
        let revoke = if privilege.inherited {
            None
        } else {
            Some(AppCommand::RevokeOracleSystemPrivilege {
                tab_id,
                privilege: privilege.privilege.clone(),
            })
        };
        body = body.child(oracle_privilege_row(
            "oracle-security-system-privilege",
            index,
            privilege.privilege.clone(),
            tr("app.oracle_security.system_detail"),
            privilege.inherited,
            privilege.admin_option,
            revoke,
            colors,
            cx,
        ));
    }
    oracle_panel_shell(body, colors)
}

/// 「对象权限」页：类型锁死为「对象权限」，编辑器多出 Owner / 对象名两栏。
fn oracle_object_privileges_panel(
    tab_id: TabId,
    state: &OracleUserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(oracle_grant_composer(
            tab_id,
            state,
            OracleGrantKind::Object,
            this,
            window,
            colors,
            cx,
        ));
    if !state.selected_is_self() && state.creating.is_none() {
        body = body.child(oracle_form_hint(tr("app.oracle_security.grants_unreadable"), colors));
        return oracle_panel_shell(body, colors);
    }
    if state.creating.is_some() {
        return oracle_panel_shell(body, colors);
    }
    let Some(snapshot) = state.snapshot.as_ref() else {
        return oracle_panel_shell(
            body.child(inline_loading_row(tr("app.oracle_security.loading"), colors)),
            colors,
        );
    };
    body = body.child(oracle_section_title(
        tr("app.oracle_security.object_privileges"),
        colors,
    ));
    if snapshot.object_privileges.is_empty() {
        body = body.child(user_admin_empty_row(tr("app.oracle_security.grants_empty"), colors));
    }
    for (index, privilege) in snapshot.object_privileges.iter().enumerate() {
        let revoke = if privilege.inherited {
            None
        } else {
            Some(AppCommand::RevokeOracleObjectPrivilege {
                tab_id,
                owner: privilege.owner.clone(),
                object: privilege.object.clone(),
                privilege: privilege.privilege.clone(),
            })
        };
        body = body.child(oracle_privilege_row(
            "oracle-security-object-privilege",
            index,
            format!("\"{}\".\"{}\"", privilege.owner, privilege.object),
            privilege.privilege.clone().into(),
            privilege.inherited,
            privilege.grantable,
            revoke,
            colors,
            cx,
        ));
    }
    oracle_panel_shell(body, colors)
}

/// 授权编辑器：名称（对象权限另有 Owner / 对象名）+ 加入待执行计划。
///
/// 类型由所在页签决定（角色 / 系统权限 / 对象权限各一页），不再让用户在编辑器里选类型。
fn oracle_grant_composer(
    tab_id: TabId,
    state: &OracleUserAdminState,
    kind: OracleGrantKind,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let editable = !state.applying && !state.loading;
    let is_object = kind == OracleGrantKind::Object;
    let mut composer = div()
        .p_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .gap_1()
        .child(oracle_section_title(
            tr("app.oracle_security.grant_composer"),
            colors,
        ))
        .child(oracle_grant_name_row(
            if is_object {
                tr("app.oracle_security.grant_object_privilege")
            } else {
                tr("app.oracle_security.grant_name")
            },
            this.oracle_security_grant_name_input.clone(),
            state.grant_name_candidates(kind).to_vec(),
            tab_id,
            (editable, window, colors),
            cx,
        ));
    if is_object {
        composer = composer
            .child(oracle_form_input(
                tr("app.oracle_security.object_owner"),
                this.oracle_security_grant_owner_input.clone(),
                "oracle-security-grant-owner",
                editable,
                window,
                colors,
                cx,
            ))
            .child(oracle_form_input(
                tr("app.oracle_security.object_name"),
                this.oracle_security_grant_object_input.clone(),
                "oracle-security-grant-object",
                editable,
                window,
                colors,
                cx,
            ));
    }
    composer = composer.child(oracle_form_row(
        tr("app.oracle_security.with_option"),
        colors,
    ).child(
        Switch::new("oracle-security-grant-with-option")
            .checked(state.grant.with_option)
            .disabled(!editable)
            .label(tr("app.oracle_security.with_option_on"))
            .on_click(cx.listener(move |this, checked, _, cx| {
                this.dispatch(
                    AppCommand::SetOracleGrantDraftOption { tab_id, with_option: *checked },
                    cx,
                );
            })),
    ));
    // 一次保存要能带多条授权：编辑器只放一条，先「加入」进待执行列表，
    // 再由工具栏保存统一出 SQL 预览、一次性执行。
    composer = composer.child(
        div().ml(px(132.)).child(
            user_admin_button(
                tr("app.oracle_security.grant_add"),
                AppIcon::Plus,
                false,
                !editable,
                colors,
            )
            .debug_selector(|| "oracle-security-grant-add".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::QueueOracleGrantDraft { tab_id, kind }, cx);
                    cx.stop_propagation();
                }),
            ),
        ),
    );
    // 待执行列表只展示本页类型的条目；删除仍按全局下标，避免跨页删错。
    if state.pending_grants.iter().any(|draft| draft.kind == kind) {
        let mut list = div().ml(px(132.)).flex().flex_col().gap_1();
        for (index, draft) in state.pending_grants.iter().enumerate() {
            if draft.kind != kind {
                continue;
            }
            list = list.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(0xff9f0a))
                            .child(oracle_grant_draft_label(draft)),
                    )
                    .child(
                        div()
                            .id(("oracle-security-grant-remove", index))
                            .debug_selector({
                                let selector = format!("oracle-security-grant-remove-{index}");
                                move || selector.clone()
                            })
                            .text_size(ui_text_size(12.))
                            .text_color(colors.muted)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(rgb(0xff3b45)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::RemoveOraclePendingGrant { tab_id, index },
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            )
                            .child(tr("app.common.delete")),
                    ),
            );
        }
        composer = composer.child(list);
    }
    composer
}

/// 授权名行：可编辑输入框 + 候选下拉。
///
/// 候选来自当前账号可转授的角色/系统权限（`ADMIN_OPTION` 且非继承），只作提示：
/// 普通账号看不到完整目录，列表为空时仍可手填任意名称，不报错。
fn oracle_grant_name_row(
    label: SharedString,
    input: Entity<InputState>,
    candidates: Vec<String>,
    tab_id: TabId,
    (enabled, window, colors): (bool, &mut Window, UiColors),
    cx: &mut Context<NavicatMain>,
) -> Div {
    let view = cx.entity().downgrade();
    let selectable = enabled && !candidates.is_empty();
    oracle_form_row(label, colors)
        .debug_selector(|| "oracle-security-grant-name-row".into())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(oracle_form_input_box(
                    input,
                    "oracle-security-grant-name",
                    enabled,
                    window,
                    colors,
                    cx,
                ))
                .when(selectable, |row| {
                    row.child(
                        Popover::new(("oracle-security-grant-name-popover", tab_id.0))
                            .appearance(false)
                            .anchor(Anchor::TopRight)
                            // `Popover` 的 trigger 需要 `Selectable`，用 Ghost 按钮承载图标。
                            .trigger(
                                Button::new("oracle-security-grant-name-candidates")
                                    .ghost()
                                    .xsmall()
                                    .p_0()
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                "oracle-security-grant-name-candidates".into()
                                            })
                                            .size(px(26.))
                                            .rounded(colors.radius)
                                            .border_1()
                                            .border_color(colors.border)
                                            .bg(colors.input_bg)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(app_icon(
                                                AppIcon::ChevronDown,
                                                13.,
                                                colors.muted,
                                            )),
                                    ),
                            )
                            .content(move |_, _window, cx| {
                                let popover = cx.entity();
                                let mut menu = div()
                                    .w(px(220.))
                                    .max_h(px(280.))
                                    .overflow_y_scrollbar()
                                    .rounded(colors.radius)
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.panel_bg)
                                    .shadow_md()
                                    .overflow_hidden();
                                for (index, candidate) in candidates.clone().into_iter().enumerate() {
                                    let view = view.clone();
                                    let popover = popover.clone();
                                    let value = candidate.clone();
                                    menu = menu.child(
                                        div()
                                            .id(("oracle-security-grant-name-candidate", index))
                                            .debug_selector({
                                                let selector = format!(
                                                    "oracle-security-grant-name-candidate-{index}"
                                                );
                                                move || selector.clone()
                                            })
                                            .h(px(26.))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .text_size(ui_text_size(12.))
                                            .text_color(colors.text)
                                            .bg(colors.panel_bg)
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(colors.hover))
                                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                                let value = value.clone();
                                                let _ = view.update(cx, |this, cx| {
                                                    this.dispatch(
                                                        AppCommand::SetOracleGrantDraftField {
                                                            tab_id,
                                                            field: fluxdb_app::OracleGrantDraftField::Name,
                                                            value,
                                                        },
                                                        cx,
                                                    );
                                                });
                                                popover.update(cx, |state, cx| {
                                                    state.dismiss(window, cx);
                                                });
                                                cx.stop_propagation();
                                            })
                                            .child(candidate),
                                    );
                                }
                                menu
                            }),
                    )
                }),
        )
}

/// 待执行授权的单行描述（不含口令；只写类型、名称与 WITH 选项）。
fn oracle_grant_draft_label(draft: &fluxdb_app::OracleGrantDraft) -> String {
    let option = if draft.with_option { " WITH OPTION" } else { "" };
    match draft.kind {
        fluxdb_app::OracleGrantKind::Role => format!("+ GRANT {}{option}", draft.name),
        fluxdb_app::OracleGrantKind::System => format!("+ GRANT {}{option}", draft.name),
        fluxdb_app::OracleGrantKind::Object => format!(
            "+ GRANT {} ON {}.{}{option}",
            draft.name, draft.owner, draft.object
        ),
    }
}

