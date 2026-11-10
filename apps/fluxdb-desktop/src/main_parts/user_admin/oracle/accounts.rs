// 左侧账号列表：ALL_USERS 目录 + 待创建草稿。非当前账号只列名字，不显示授权详情。

fn oracle_user_list(
    tab_id: TabId,
    state: &OracleUserAdminState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = this
        .user_admin_state_for(tab_id)
        .map(|admin| admin.search.to_lowercase())
        .unwrap_or_default();
    let current = state
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.current_user.clone())
        .unwrap_or_default();
    let mut list = div().flex_1().min_h(px(0.)).overflow_y_scrollbar();
    if state.loading {
        list = list.child(inline_loading_row(tr("app.oracle_security.loading"), colors));
    } else if let Some(error) = &state.error {
        list = list.child(user_admin_error_row(&error.message, colors));
    } else if let Some(snapshot) = state.snapshot.as_ref() {
        let mut count = 0usize;
        for user in snapshot
            .users
            .iter()
            .filter(|user| query.is_empty() || user.name.to_lowercase().contains(&query))
        {
            count += 1;
            list = list.child(oracle_user_row(tab_id, state, user.clone(), &current, colors, cx));
        }
        // 待创建草稿：与已存在账号同名时不重复显示，避免误以为可以建两个。
        if let Some(draft) = state.creating.as_ref() {
            let named = !draft.name.trim().is_empty();
            let duplicated = named
                && snapshot
                    .users
                    .iter()
                    .any(|user| user.name.eq_ignore_ascii_case(draft.name.trim()));
            if named && !duplicated {
                list = list.child(
                    oracle_draft_row(
                        tr("app.oracle_security.pending_creation")
                            .replace("%{name}", draft.name.trim()),
                        colors,
                    )
                    .debug_selector(|| "oracle-security-pending-creation".into()),
                );
                count += 1;
            }
        }
        if count == 0 {
            list = list.child(user_admin_empty_row(
                tr("app.oracle_security.empty_users"),
                colors,
            ));
        }
    } else {
        list = list.child(inline_loading_row(tr("app.oracle_security.loading"), colors));
    }

    div()
        .min_w(px(0.))
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(48.))
                .px_2()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .gap_2()
                .child(
                    user_admin_input_box(this.user_admin_search_input.clone(), colors).flex_1(),
                )
                .child(
                    user_admin_add_user_button(state.loading || state.snapshot.is_none(), colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.dispatch(AppCommand::BeginOracleCreateUser(tab_id), cx);
                                cx.stop_propagation();
                            }),
                        ),
                ),
        )
        .child(list)
}

fn oracle_user_row(
    tab_id: TabId,
    state: &OracleUserAdminState,
    user: fluxdb_core::OracleUser,
    current: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let active = state
        .selected
        .as_deref()
        .is_some_and(|selected| selected.eq_ignore_ascii_case(&user.name));
    let subtitle = oracle_user_subtitle(&user, current);
    // 两个 move 闭包各自要一份所有权：selector 文案一份，点击命令一份。
    let selector_name = user.name.clone();
    let name = user.name.clone();
    div()
        .id(SharedString::from(format!("oracle-user-{}", user.name)))
        .h(px(52.))
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .cursor_pointer()
        .bg(if active { colors.tree_selected } else { colors.panel_bg })
        .hover(move |style| {
            style.bg(if active { colors.tree_selected } else { colors.hover })
        })
        .flex()
        .items_center()
        .gap_2()
        .debug_selector(move || format!("oracle-user-{selector_name}"))
        .child(app_icon_box(
            if user.oracle_maintained {
                // 自带账号不可操作，用「禁止」图标，避免画成普通可管理账号。
                AppIcon::CircleSlash
            } else {
                AppIcon::Users
            },
            24.,
            15.,
            colors.muted,
        ))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(user.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(subtitle),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::SelectOracleUser {
                        tab_id,
                        name: name.clone(),
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
}

fn oracle_draft_row(title: String, colors: UiColors) -> Stateful<Div> {
    div()
        .id("oracle-security-draft-row")
        .h(px(56.))
        .mx_2()
        .my_2()
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(rgb(0x2563eb))
        .bg(if colors.is_dark {
            rgb(0x16345d)
        } else {
            rgb(0xeaf2ff)
        })
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon_box(AppIcon::Plus, 24., 15., rgb(0x2563eb)))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(tr("app.oracle_security.draft_subtitle")),
                ),
        )
}

// 右侧「常规」页：目录事实 + 账号操作（改密码 / 锁定 / 新建表单）。
fn oracle_general_panel(
    tab_id: TabId,
    state: &OracleUserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    if state.creating.is_some() {
        return oracle_create_form(tab_id, state, this, window, colors, cx);
    }
    let Some(name) = state.selected.clone() else {
        return oracle_empty_panel(tr("app.oracle_security.select_user"), colors).into_any_element();
    };
    let is_self = state.selected_is_self();
    let detail = if is_self {
        state.snapshot.as_ref().and_then(|snapshot| snapshot.own.as_ref())
    } else {
        None
    };
    let user = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.user(&name))
        .cloned();
    let mut form = div().max_w(px(620.)).flex().flex_col().gap_3().child(
        h_flex()
            .mb_1()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child(name.clone()),
            )
            .child(user_admin_badge(
                user.as_ref()
                    .map(|user| oracle_user_subtitle(user, ""))
                    .unwrap_or_else(|| tr("app.oracle_security.local_user"))
                    .to_string(),
                colors,
            )),
    );
    if let Some(detail) = detail {
        form = form
            .child(oracle_readonly_row(
                tr("app.oracle_security.account_status"),
                detail.account_status.clone(),
                colors,
            ))
            .child(oracle_readonly_row(
                tr("app.oracle_security.default_tablespace"),
                detail.default_tablespace.clone().unwrap_or_default(),
                colors,
            ))
            .child(oracle_readonly_row(
                tr("app.oracle_security.temporary_tablespace"),
                detail.temporary_tablespace.clone().unwrap_or_default(),
                colors,
            ))
            .child(oracle_readonly_row(
                tr("app.oracle_security.created"),
                detail.created.clone().unwrap_or_default(),
                colors,
            ))
            .child(oracle_readonly_row(
                tr("app.oracle_security.expiry_date"),
                detail.expiry_date.clone().unwrap_or_default(),
                colors,
            ));
        if let Some(lock_date) = detail.lock_date.as_ref() {
            form = form.child(oracle_readonly_row(
                tr("app.oracle_security.lock_date"),
                lock_date.clone(),
                colors,
            ));
        }
        if state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.quotas.is_empty())
        {
            let quotas = state
                .snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .quotas
                        .iter()
                        .map(|quota| {
                            let bytes = quota
                                .max_bytes
                                .map(|bytes| format!("{}M", bytes / (1024 * 1024)))
                                .unwrap_or_else(|| tr("app.oracle_security.quota_unlimited").to_string());
                            format!("{}: {}", quota.tablespace, bytes)
                        })
                        .collect::<Vec<_>>()
                        .join("  ")
                })
                .unwrap_or_default();
            form = form.child(oracle_readonly_row(
                tr("app.oracle_security.quotas"),
                quotas,
                colors,
            ));
        }
    } else {
        // 非当前账号：Oracle 目录不暴露别人的授权明细，只能列名字。这里明说，不画成「无权限」。
        form = form.child(oracle_form_hint(
            tr("app.oracle_security.other_user_hint"),
            colors,
        ));
    }
    // 账号操作：改密码与锁定/解锁，仅当前账号；口令输入框永不回显历史值。
    if is_self {
        let editable = !state.applying && !state.loading;
        // 按钮文案与命令都用这一份快照值：闭包带走 bool，不带走对 state 的借用。
        let is_locked = detail.is_some_and(|detail| detail.is_locked());
        // 改密码：明文只进内存草稿，保存时随计划一起执行；确认框只在桌面层做一致性校验。
        let mismatch = !this
            .oracle_security_new_password_input
            .read(cx)
            .value()
            .is_empty()
            && this.oracle_security_new_password_input.read(cx).value()
                != this.oracle_security_new_password_confirm_input.read(cx).value();
        form = form
            .child(oracle_form_input(
                tr("app.oracle_security.new_password"),
                this.oracle_security_new_password_input.clone(),
                "oracle-security-new-password",
                editable,
                window,
                colors,
                cx,
            ))
            .child(oracle_form_input(
                tr("app.oracle_security.confirm_password"),
                this.oracle_security_new_password_confirm_input.clone(),
                "oracle-security-new-password-confirm",
                editable,
                window,
                colors,
                cx,
            ));
        if mismatch {
            form = form.child(oracle_form_error(tr(
                "app.oracle_security.password_mismatch",
            )));
        }
        form = form
            .child(
                oracle_form_row(tr("app.oracle_security.account_locked"), colors).child(
                    Switch::new("oracle-security-toggle-lock")
                        .checked(is_locked)
                        .disabled(!editable)
                        .label(if is_locked {
                            tr("app.oracle_security.unlock")
                        } else {
                            tr("app.oracle_security.lock")
                        })
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            this.dispatch(
                                AppCommand::SetOracleUserLocked { tab_id, locked: *checked },
                                cx,
                            );
                        })),
                ),
            )
            .child(oracle_form_hint(
                tr("app.oracle_security.password_preview_hint"),
                colors,
            ));
    }
    if let Some(error) = &state.apply_error {
        form = form.child(user_admin_error_row(&error.message, colors));
    }
    oracle_panel_shell(form, colors)
}

fn oracle_empty_panel(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(text.into())
}

fn oracle_status_bar(state: &OracleUserAdminState, colors: UiColors) -> Div {
    let users = state
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.users.len())
        .unwrap_or(0);
    let direct = state
        .snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .roles
                .iter()
                .filter(|role| !role.inherited)
                .count()
                + snapshot
                    .system_privileges
                    .iter()
                    .filter(|privilege| !privilege.inherited)
                    .count()
        })
        .unwrap_or(0);
    h_flex()
        .items_center()
        .gap_3()
        .px_3()
        .h(px(28.))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(
            tr("app.oracle_security.status_counts")
                .replace("%{users}", &users.to_string())
                .replace("%{grants}", &direct.to_string()),
        )
        .child(div().flex_1())
        .when(state.has_changes(), |this| {
            this.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .text_color(rgb(0xff9f0a))
                    .child(app_icon(AppIcon::Save, 12., rgb(0xff9f0a)))
                    .child(tr("app.oracle_security.pending_changes")),
            )
        })
}
