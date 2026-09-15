// PG 用户与角色工作台：成员关系页。
//
// 双向展示：「所属角色」（当前角色是哪些组角色的成员，GRANT group TO role）与
// 「此角色的成员」（当前角色授予了哪些成员）。编辑只作用于直接成员关系；
// ADMIN/INHERIT/SET 选项按服务端版本展示（PG15 及以前提示「继承由角色属性控制」）。
// 所有变更进入草稿（pg_membership_edits），保存时统一提交。

fn pg_membership_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    if admin.pg_draft.as_ref().is_some_and(|d| d.create) {
        return pg_membership_placeholder("新角色保存后再配置成员关系（保存时先创建角色，再应用成员变更）。", colors)
            .into_any_element();
    }
    let selected = admin.pg_effective_grantee_name();
    if selected.is_empty() {
        return pg_membership_placeholder("请选择角色", colors).into_any_element();
    }
    let supported = admin.pg_member_options_supported;

    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .flex()
        .flex_col()
        .gap_3();

    // ===== 所属角色 =====
    body = body.child(pg_membership_section(
        tab_id,
        "所属角色",
        format!("「{selected}」直接所属的组角色（GRANT 组角色 TO {selected}）"),
        admin.pg_member_of_roles(),
        &selected,
        supported,
        colors,
        cx,
    ));

    // ===== 此角色的成员 =====
    body = body.child(pg_membership_section(
        tab_id,
        "此角色的成员",
        format!("「{selected}」直接授予的成员（GRANT {selected} TO 成员）"),
        admin.pg_members_of_role(),
        &selected,
        supported,
        colors,
        cx,
    ));

    // ===== 添加成员关系（草稿）=====
    body = body.child(pg_membership_add_row(tab_id, admin, this, &selected, supported, colors, cx));

    // ===== 待保存的成员变更 =====
    if !admin.pg_membership_edits.is_empty() {
        let mut edits = div().flex().flex_col().gap_1();
        for (index, edit) in admin.pg_membership_edits.iter().enumerate() {
            let text = match edit {
                PgRoleChange::GrantMembership { role, member, admin: admin_opt, inherit, set } => {
                    format!("+ GRANT {role} TO {member}（ADMIN {admin_opt} / INHERIT {inherit} / SET {set}）")
                }
                PgRoleChange::RevokeMembership { role, member } => {
                    format!("- REVOKE {role} FROM {member}")
                }
                _ => continue,
            };
            edits = edits.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0xff9f0a))
                            .child(text),
                    )
                    .child(
                        div()
                            .id(("pg-membership-edit-remove", index))
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(rgb(0xff3b45)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::PgMembershipRemoveEdit { tab_id, index },
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            )
                            .child("撤销此更改"),
                    ),
            );
        }
        body = body.child(
            div()
                .border_1()
                .border_color(colors.border)
                .rounded(colors.radius)
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("待保存的成员变更"),
                )
                .child(edits),
        );
    }
    body.into_any_element()
}

/// 单向成员表：标题 + 列头（含版本化选项）+ 行。
fn pg_membership_section(
    tab_id: TabId,
    title: &'static str,
    subtitle: String,
    rows: Vec<&PgRoleMembership>,
    selected: &str,
    supported: Option<bool>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut section = div()
        .border_1()
        .border_color(colors.border)
        .rounded(colors.radius)
        .flex()
        .flex_col()
        .child(
            div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(subtitle),
                ),
        );
    if rows.is_empty() {
        section = section.child(user_admin_empty_row("暂无直接成员关系。", colors));
    } else {
        section = section.child(pg_membership_header(supported, colors));
        for row in rows {
            section = section.child(pg_membership_row(tab_id, row, selected, supported, colors, cx));
        }
    }
    section
}

fn pg_membership_header(supported: Option<bool>, colors: UiColors) -> Div {
    let mut header = h_flex()
        .h(px(30.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(div().flex_1().min_w(px(0.)).px_3().child("角色"))
        .child(div().w(px(72.)).flex().justify_center().child("ADMIN"))
        .child(if supported == Some(true) {
            div()
                .w(px(72.))
                .flex()
                .justify_center()
                .child("INHERIT")
                .border_l_1()
                .border_color(colors.border)
        } else {
            div().w(px(0.))
        })
        .child(if supported == Some(true) {
            div()
                .w(px(72.))
                .flex()
                .justify_center()
                .child("SET")
                .border_l_1()
                .border_color(colors.border)
        } else {
            div().w(px(0.))
        });
    header = header.child(div().w(px(76.)).flex().justify_center().child("操作"));
    header
}

/// 成员行：显示 ADMIN/INHERIT/SET（PG16+；PG15 隐藏 INHERIT/SET，继承由角色属性控制）。
fn pg_membership_row(
    tab_id: TabId,
    row: &PgRoleMembership,
    selected: &str,
    supported: Option<bool>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 方向：row.member == selected → 行显示组角色；否则行显示成员名。
    let display = if row.member == selected {
        row.grantee.clone()
    } else {
        row.member.clone()
    };
    let (revoke_role, revoke_member) = if row.member == selected {
        (row.grantee.clone(), selected.to_string())
    } else {
        (selected.to_string(), row.member.clone())
    };
    h_flex()
        .h(px(34.))
        .border_b_1()
        .border_color(colors.border_soft)
        .hover(move |style| style.bg(colors.hover))
        .text_size(px(12.))
        .text_color(colors.text)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Users, 14., colors.muted))
                .child(div().min_w(px(0.)).truncate().child(display)),
        )
        .child(
            div()
                .w(px(72.))
                .flex()
                .justify_center()
                .child(if row.admin_option { "是" } else { "否" }.to_string()),
        )
        .child(if supported == Some(true) {
            div()
                .w(px(72.))
                .flex()
                .justify_center()
                .border_l_1()
                .border_color(colors.border)
                .child(if row.inherit_option { "是" } else { "否" }.to_string())
        } else {
            div().w(px(0.))
        })
        .child(if supported == Some(true) {
            div()
                .w(px(72.))
                .flex()
                .justify_center()
                .border_l_1()
                .border_color(colors.border)
                .child(if row.set_option { "是" } else { "否" }.to_string())
        } else {
            div().w(px(0.))
        })
        .child(
            div().w(px(76.)).flex().justify_center().child(
                div()
                    .id(SharedString::from(format!(
                        "pg-membership-revoke-{revoke_role}-{revoke_member}"
                    )))
                    .text_size(px(12.))
                    .text_color(rgb(0xff3b45))
                    .cursor_pointer()
                    .hover(move |style| style.opacity(0.7))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.dispatch(
                                AppCommand::PgMembershipRevoke {
                                    tab_id,
                                    role: revoke_role.clone(),
                                    member: revoke_member.clone(),
                                },
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    )
                    .child("撤销"),
            ),
        )
}

/// 添加成员关系（草稿）：组角色下拉 + 添加按钮；选项默认 ADMIN 关、INHERIT/SET 随版本默认。
fn pg_membership_add_row(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    selected: &str,
    supported: Option<bool>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let member_options = admin.pg_member_options_supported;
    let mut row = h_flex()
        .items_center()
        .gap_2()
        .flex_wrap()
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.text)
                .child("添加成员关系："),
        )
        .child(
            div()
                .w(px(200.))
                .child(Select::new(&this.pg_member_of_role_select).w_full().h(px(30.))),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("把所选组角色授予当前角色"),
        );
    if member_options == Some(true) {
        row = row.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("（新授权默认 INHERIT/SET 开启，保存前可用 SQL 预览核对）"),
        );
    }
    let selected_owned = selected.to_string();
    row = row.child(
        user_admin_button("授予成员关系", AppIcon::Plus, false, false, colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                // 语义：把所选组角色授予当前角色（GRANT 所选 TO 当前角色）。
                let Some(choice) = this
                    .pg_member_of_role_select
                    .read(cx)
                    .selected_value()
                    .map(|v| v.to_string())
                else {
                    this.show_message("请先选择要授予的组角色", AppMessageKind::Warning, cx);
                    return;
                };
                if choice == selected_owned {
                    this.show_message("不能与自身建立成员关系", AppMessageKind::Warning, cx);
                    return;
                }
                let inherit_default = member_options != Some(false);
                this.dispatch(
                    AppCommand::PgMembershipGrant {
                        tab_id,
                        role: choice,
                        member: selected_owned.clone(),
                        admin: this.pg_member_new_admin,
                        inherit: inherit_default,
                        set: inherit_default,
                    },
                    cx,
                );
                cx.notify();
            }),
        ),
    );
    if supported == Some(false) {
        row = row.child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("PG15 及以前：成员继承由角色 INHERIT 属性控制，无成员级 INHERIT/SET 选项。"),
        );
    }
    row
}

fn pg_membership_placeholder(text: &str, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .justify_center()
        .px_6()
        .text_align(gpui::TextAlign::Center)
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(text.to_string())
}
