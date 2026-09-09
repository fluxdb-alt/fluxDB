fn user_admin_privileges_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> UserAdminSqlPreviewDraft {
    let Some(user) = user_admin_selected_existing_user(admin) else {
        return UserAdminSqlPreviewDraft::default();
    };
    let mut statements = Vec::new();
    let mut danger = false;

    for row in &admin.privilege_rows {
        let base = admin
            .base_privilege_rows
            .iter()
            .find(|base| base.id == row.id);
        let same_database = base.is_some_and(|base| base.database == row.database);
        let base_privileges = base
            .filter(|_| same_database)
            .map(|base| base.privileges.as_slice())
            .unwrap_or(&[]);
        let grant_option_upgraded = row.grant_option
            && !row.privileges.is_empty()
            && base.is_none_or(|base| !same_database || !base.grant_option);
        let grant_privileges = user_admin_privilege_delta(&row.privileges, base_privileges);
        if !grant_privileges.is_empty() && !grant_option_upgraded {
            statements.push(provider.grant_privileges_sql(&fluxdb_core::PrivilegeChangeInput {
                user: user.clone(),
                privileges: grant_privileges,
                database: row.database.clone(),
                table: "*".to_string(),
                grant_option: row.grant_option,
            }));
        }

        if let Some(base) = base {
            let revoke_privileges = if same_database {
                user_admin_privilege_delta(&base.privileges, &row.privileges)
            } else {
                base.privileges.clone()
            };
            if !revoke_privileges.is_empty() {
                danger = true;
                statements.push(provider.revoke_privileges_sql(&fluxdb_core::PrivilegeChangeInput {
                    user: user.clone(),
                    privileges: revoke_privileges,
                    database: base.database.clone(),
                    table: "*".to_string(),
                    grant_option: false,
                }));
            }
            if base.grant_option && (!row.grant_option || !same_database) {
                danger = true;
                statements.push(provider.revoke_privileges_sql(&fluxdb_core::PrivilegeChangeInput {
                    user: user.clone(),
                    privileges: vec!["GRANT OPTION".to_string()],
                    database: base.database.clone(),
                    table: "*".to_string(),
                    grant_option: false,
                }));
            }
        }

        if grant_option_upgraded {
            statements.push(provider.grant_privileges_sql(&fluxdb_core::PrivilegeChangeInput {
                user: user.clone(),
                privileges: row.privileges.clone(),
                database: row.database.clone(),
                table: "*".to_string(),
                grant_option: row.grant_option,
            }));
        }
    }

    UserAdminSqlPreviewDraft {
        statements,
        danger,
    }
}

fn user_admin_privilege_delta(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .filter(|privilege| !right.iter().any(|item| item == *privilege))
        .cloned()
        .collect()
}

fn user_admin_privileges_panel(
    tab_id: TabId,
    state: &AppState,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if admin.creating_user {
        return user_admin_placeholder_panel_with_text("请先保存新用户，再配置权限。", colors);
    }
    let Some(provider) = state
        .connections
        .iter()
        .find(|connection| connection.config.id == admin.connection_id)
        .and_then(|connection| database_user_admin_provider(connection.config.kind))
    else {
        return user_admin_placeholder_panel(colors);
    };
    let database_options = user_admin_privilege_database_options(state, admin);
    let privileges = provider.privileges_for_scope(admin.privilege_scope);
    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_y_scrollbar();
    if admin.privilege_rows.is_empty() {
        body = body.child(user_admin_empty_row("暂无权限配置，点击添加权限开始。", colors));
    } else {
        for row in &admin.privilege_rows {
            body = body.child(user_admin_privilege_row(
                tab_id,
                row.clone(),
                privileges,
                database_options.clone(),
                this.user_admin_privilege_database_menu == Some((tab_id, row.id)),
                colors,
                cx,
            ));
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_3()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.user_admin_privilege_database_menu = None;
                cx.notify();
            }),
        )
        .child(
            div()
                .size_full()
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .flex()
                .flex_col()
                .child(user_admin_privileges_toolbar(
                    tab_id,
                    admin,
                    database_options.clone(),
                    colors,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_x_scrollbar()
                        .child(
                            div()
                                .w(px(user_admin_privileges_table_width(privileges)))
                                .min_h(px(0.))
                                .h_full()
                                .flex()
                                .flex_col()
                                .child(user_admin_privileges_header(privileges, colors))
                                .child(body),
                        ),
                ),
        )
}

fn user_admin_privileges_toolbar(
    tab_id: TabId,
    admin: &UserAdminState,
    database_options: Vec<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let default_database = user_admin_default_privilege_database(admin, &database_options);
    div()
        .h(px(42.))
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("权限列表"),
        )
        .child(div().flex_1())
        .child(
            user_admin_button("添加权限", AppIcon::Plus, false, admin.applying, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if let Some(admin) = this.user_admin_state_for(tab_id)
                            && !admin.applying
                        {
                            this.user_admin_privilege_database_menu = None;
                            this.dispatch(
                                AppCommand::AddUserAdminPrivilegeRow {
                                    tab_id,
                                    database: default_database.clone(),
                                },
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn user_admin_privileges_header(privileges: &[&'static str], colors: UiColors) -> Div {
    let mut header = div()
        .h(px(32.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(
            div()
                .w(px(160.))
                .h_full()
                .px_3()
                .border_r_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .child("数据库"),
        );
    for privilege in privileges {
        if *privilege == "INDEX" {
            header = header.child(user_admin_privilege_header_cell("Grant Option", colors));
        }
        header = header.child(user_admin_privilege_header_cell(
            &user_admin_privilege_label(privilege),
            colors,
        ));
    }
    header
}

fn user_admin_privilege_header_cell(label: &str, colors: UiColors) -> Div {
    div()
        .w(px(user_admin_privilege_column_width(label)))
        .h_full()
        .px_2()
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .child(label.to_string())
}

fn user_admin_privilege_row(
    tab_id: TabId,
    row: fluxdb_app::UserAdminPrivilegeRow,
    privileges: &[&'static str],
    database_options: Vec<String>,
    menu_open: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let row_id = row.id;
    let mut cells = div()
        .h(px(34.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .child(user_admin_privilege_database_cell(
            tab_id,
            row_id,
            row.database.clone(),
            menu_open,
            colors,
            cx,
        ));
    for privilege in privileges {
        if *privilege == "INDEX" {
            let grant_option_enabled = row.grant_option;
            cells = cells.child(user_admin_privilege_checkbox_cell(
                ("user-admin-privilege-grant-option", row_id),
                user_admin_privilege_column_width("Grant Option"),
                grant_option_enabled,
                colors,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        AppCommand::SetUserAdminPrivilegeRowGrantOption {
                            tab_id,
                            row_id,
                            enabled: !grant_option_enabled,
                        },
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ));
        }
        let privilege_value = (*privilege).to_string();
        let checked = row.privileges.iter().any(|item| item == privilege);
        cells = cells.child(user_admin_privilege_checkbox_cell(
            (
                "user-admin-privilege",
                user_admin_privilege_cell_id(row_id, privilege),
            ),
            user_admin_privilege_column_width(&user_admin_privilege_label(privilege)),
            checked,
            colors,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::ToggleUserAdminPrivilegeRowPrivilege {
                        tab_id,
                        row_id,
                        privilege: privilege_value.clone(),
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        ));
    }

    div()
        .flex()
        .flex_col()
        .child(cells)
        .when(menu_open, |this| {
            this.child(user_admin_privilege_database_menu(
                tab_id,
                row_id,
                row.database,
                database_options,
                colors,
                cx,
            ))
        })
}

fn user_admin_privilege_database_cell(
    tab_id: TabId,
    row_id: u64,
    database: String,
    menu_open: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    div()
        .id(("user-admin-privilege-database", row_id))
        .w(px(160.))
        .h_full()
        .px_2()
        .border_r_1()
        .border_color(colors.border)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.start_user_admin_database_options_load(tab_id, cx);
                this.user_admin_privilege_database_menu =
                    (!menu_open).then_some((tab_id, row_id));
                cx.notify();
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .text_color(colors.text)
                .child(database),
        )
        .child(app_icon(AppIcon::ChevronDown, 13., colors.muted))
}

fn user_admin_privilege_database_menu(
    tab_id: TabId,
    row_id: u64,
    selected: String,
    database_options: Vec<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let mut menu = div()
        .ml(px(6.))
        .mb(px(6.))
        .w(px(180.))
        .max_h(px(220.))
        .overflow_y_scrollbar()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(8.),
            px(18.),
            px(0.),
            hsla(0., 0., 0., 0.18),
        )])
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    for database in database_options {
        let active = database == selected;
        menu = menu.child(
            div()
                .h(px(30.))
                .px_2()
                .cursor_pointer()
                .bg(if active {
                    colors.tree_selected
                } else {
                    colors.panel_bg
                })
                .hover(move |style| {
                    style.bg(if active {
                        colors.tree_selected
                    } else {
                        colors.hover
                    })
                })
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(13.))
                .text_color(if active { colors.text } else { colors.muted })
                .child(div().w(px(16.)).when(active, |this| {
                    this.child(app_icon(AppIcon::Check, 13., rgb(0x2563eb)))
                }))
                .child(div().min_w(px(0.)).flex_1().truncate().child(database.clone()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::SetUserAdminPrivilegeRowDatabase {
                                tab_id,
                                row_id,
                                database: database.clone(),
                            },
                            cx,
                        );
                        this.user_admin_privilege_database_menu = None;
                        cx.stop_propagation();
                    }),
                ),
        );
    }
    menu
}

fn user_admin_privilege_checkbox_cell(
    id: impl Into<gpui::ElementId>,
    width: f32,
    checked: bool,
    colors: UiColors,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(width))
        .h_full()
        .border_r_1()
        .border_color(colors.border)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, on_click)
        .flex()
        .items_center()
        .justify_center()
        .child(user_admin_check_box_visual(checked, colors))
}

fn user_admin_privilege_cell_id(row_id: u64, privilege: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    row_id.hash(&mut hasher);
    privilege.hash(&mut hasher);
    hasher.finish()
}

fn user_admin_privilege_database_options(state: &AppState, admin: &UserAdminState) -> Vec<String> {
    let mut databases = BTreeSet::new();
    databases.insert("*".to_string());
    if !admin.privilege_database.trim().is_empty() {
        databases.insert(admin.privilege_database.clone());
    }
    if let Some(connection) = state
        .connections
        .iter()
        .find(|connection| connection.config.id == admin.connection_id)
    {
        if let Some(database) = connection.config.options.get("database")
            && !database.trim().is_empty()
        {
            databases.insert(database.clone());
        }
        for object in &connection.objects {
            match object.path.kind {
                ObjectKind::Database | ObjectKind::Schema => {
                    let name = object
                        .path
                        .database
                        .clone()
                        .unwrap_or_else(|| object.path.name.clone());
                    if !name.trim().is_empty() {
                        databases.insert(name);
                    }
                }
                ObjectKind::Table | ObjectKind::View => {
                    if let Some(name) = &object.path.database
                        && !name.trim().is_empty()
                    {
                        databases.insert(name.clone());
                    }
                }
                ObjectKind::Column
                | ObjectKind::Index
                | ObjectKind::Collection
                | ObjectKind::RedisDb
                | ObjectKind::RedisKey => {}
            }
        }
    }
    databases.into_iter().collect()
}

fn user_admin_default_privilege_database(
    admin: &UserAdminState,
    database_options: &[String],
) -> String {
    if database_options
        .iter()
        .any(|database| database == &admin.privilege_database)
    {
        return admin.privilege_database.clone();
    }
    database_options
        .iter()
        .find(|database| database.as_str() != "*")
        .cloned()
        .unwrap_or_else(|| "*".to_string())
}

fn user_admin_privileges_table_width(privileges: &[&'static str]) -> f32 {
    let privilege_width = privileges
        .iter()
        .map(|privilege| user_admin_privilege_column_width(&user_admin_privilege_label(privilege)))
        .sum::<f32>();
    160. + privilege_width + user_admin_privilege_column_width("Grant Option")
}

fn user_admin_privilege_column_width(label: &str) -> f32 {
    match label {
        "Create Temporary Tables" => 172.,
        "Alter Routine" | "Create Routine" => 124.,
        "Grant Option" => 112.,
        "References" => 104.,
        "Create View" | "Show View" => 104.,
        _ => 86.,
    }
}

fn user_admin_privilege_label(privilege: &str) -> String {
    privilege
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            format!("{}{}", first, chars.as_str().to_ascii_lowercase())
        })
        .collect::<Vec<_>>()
        .join(" ")
}
