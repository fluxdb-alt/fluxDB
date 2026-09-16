// PG 用户与角色工作台：权限页。
//
// 对象选择区（数据库/对象种类/schema/对象）+ 权限表格（直接授予/可再授权为草稿，
// 当前有效/来源为已读取的服务端状态）。对象与函数签名来自元数据枚举（list_grant_targets），
// 不要求手填。目标完整后自动异步读取；权限变更只进草稿，保存时统一提交。
//
// 语义注意：
// - effective && !direct：来自 PUBLIC/成员继承/属主等「其他来源」，不可在当前角色直接撤销；
// - ACL NULL 是默认权限语义（属主全权、其余按默认），不等于空授权；
// - GRANT OPTION 与 ADMIN OPTION 分开，取消可再授权生成 REVOKE GRANT OPTION FOR。

fn pg_privileges_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    _window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    // 数据库切换提示：一期同批只允许一个数据库的对象授权。
    let grant_edit_db = admin.pg_grant_database.clone();
    let target_complete = !admin.pg_grant_object.trim().is_empty();
    let current_fingerprint = if target_complete {
        fluxdb_app::pg_grant_target_fingerprint(admin)
    } else {
        String::new()
    };
    let stale = target_complete && admin.pg_loaded_target != current_fingerprint;
    // 失败后保留错误等待手动重试，避免每次渲染重新请求并把错误遮成 loading。
    let should_load = stale && admin.pg_grants_error.is_none();
    if should_load && !admin.loading_pg_grants {
        this.start_pg_object_grants_load(tab_id, cx);
    }

    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .flex()
        .flex_col()
        .gap_3();

    // ===== 对象选择区 =====
    body = body.child(pg_grant_target_selector(tab_id, admin, this, colors, cx));

    // ===== 对象概览（属主/默认权限）=====
    if let Some(grants) = &admin.pg_object_grants
        && !stale
    {
        let summary = if grants.acl_is_null {
            format!(
                "当前目标属主 {}；ACL 为 NULL（默认权限：属主全权、其余按对象类型默认，不等同于空授权）",
                grants.owner
            )
        } else {
            format!("当前目标属主 {}", grants.owner)
        };
        body = body.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(summary),
        );
    }

    // ===== 权限表 =====
    if admin.loading_pg_grants || should_load {
        body = body.child(user_admin_empty_row("正在读取权限…", colors));
    } else if let Some(error) = &admin.pg_grants_error {
        body = body
            .child(user_admin_error_row(&error.message, colors))
            .child(
                user_admin_button("重试", AppIcon::Refresh, false, false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.start_pg_object_grants_load(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
            );
    } else if admin.pg_object_grants.is_none() || !target_complete {
        body = body.child(user_admin_empty_row(
            "选择数据库、对象类型与对象后自动读取权限。",
            colors,
        ));
    } else {
        body = body.child(pg_privilege_table(tab_id, admin, colors, cx));
        body = body.child(div().text_size(px(11.)).text_color(colors.muted).child(
            "说明：「当前有效」包含直接授权、成员继承、PUBLIC、属主与超级用户来源；\
                     撤销直接授权后仍可能因其他来源保持有效。函数 EXECUTE 默认授予 PUBLIC。",
        ));
    }

    // ===== 待保存的授权变更 =====
    if !admin.pg_grant_edits.is_empty() {
        let mut edits = div().flex().flex_col().gap_1();
        for (index, edit) in admin.pg_grant_edits.iter().enumerate() {
            let text = match edit {
                PgRoleChange::GrantObject {
                    privilege,
                    scope,
                    grantee,
                    grant_option: true,
                } => {
                    format!(
                        "+ GRANT {privilege} ON {} TO {grantee} WITH GRANT OPTION",
                        pg_scope_display(scope)
                    )
                }
                PgRoleChange::GrantObject {
                    privilege,
                    scope,
                    grantee,
                    ..
                } => {
                    format!(
                        "+ GRANT {privilege} ON {} TO {grantee}",
                        pg_scope_display(scope)
                    )
                }
                PgRoleChange::RevokeObject {
                    privilege,
                    scope,
                    grantee,
                } => {
                    format!(
                        "- REVOKE {privilege} ON {} FROM {grantee}",
                        pg_scope_display(scope)
                    )
                }
                PgRoleChange::RevokeGrantOption {
                    privilege,
                    scope,
                    grantee,
                } => {
                    format!(
                        "~ REVOKE GRANT OPTION FOR {privilege} ON {} FROM {grantee}",
                        pg_scope_display(scope)
                    )
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
                            .id(("pg-grant-edit-remove", index))
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(rgb(0xff3b45)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::PgRemoveGrantEdit { tab_id, index },
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            )
                            .child("撤销此更改"),
                    ),
            );
        }
        let _ = grant_edit_db;
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
                        .child("待保存的授权变更"),
                )
                .child(edits),
        );
    }
    body.into_any_element()
}

/// 对象选择区：数据库 / 对象种类 / schema / 对象（对象列表来自元数据）。
fn pg_grant_target_selector(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let targets_loading = admin.pg_loading_targets;
    let kind = admin.pg_grant_kind;

    let mut selector = div()
        .border_1()
        .border_color(colors.border)
        .rounded(colors.radius)
        .p_3()
        .flex()
        .flex_col()
        .gap_2();

    // 对象种类（全部可点击，修复旧版仅选中项可点的问题）。
    let mut kinds = h_flex().items_center().gap_1();
    for candidate in PgGrantObjectKind::all() {
        let selected = candidate == kind;
        kinds = kinds.child(
            div()
                .id(SharedString::from(format!(
                    "pg-grant-kind-{}",
                    candidate.label()
                )))
                .px_2()
                .h(px(24.))
                .rounded(colors.radius)
                .cursor_pointer()
                .border_1()
                .border_color(if selected {
                    rgb(0x2563eb)
                } else {
                    colors.border
                })
                .bg(if selected {
                    colors.tree_selected
                } else {
                    colors.panel_bg
                })
                .hover(move |style| {
                    style.bg(if selected {
                        colors.tree_selected
                    } else {
                        colors.hover
                    })
                })
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(if selected { colors.text } else { colors.muted })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.set_pg_grant_kind(tab_id, candidate, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(candidate.label()),
        );
    }
    selector = selector.child(kinds);

    // 数据库 / schema 选择。
    selector = selector.child(
        h_flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child("数据库"),
            )
            .child(
                div()
                    .w(px(160.))
                    .child(Select::new(&this.pg_grant_db_select).w_full().h(px(30.))),
            )
            .when(kind != PgGrantObjectKind::Database, |row| {
                row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("Schema"),
                )
                .child(
                    div().w(px(160.)).child(
                        Select::new(&this.pg_grant_schema_select)
                            .w_full()
                            .h(px(30.)),
                    ),
                )
            })
            .when(pg_grant_uses_object_selector(kind), |row| {
                row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("对象"),
                )
                .child(
                    div().min_w(px(220.)).flex_1().child(
                        Select::new(&this.pg_grant_object_select)
                            .w_full()
                            .h(px(30.)),
                    ),
                )
            })
            .when(targets_loading, |row| {
                row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("加载目标列表…"),
                )
            }),
    );
    if let Some(error) = &admin.pg_targets_error {
        selector = selector.child(user_admin_error_row(&error.message, colors));
    }
    selector
}

/// 权限表：草稿（直接授予/可再授权）+ 服务端状态（当前有效/来源）。
fn pg_privilege_table(
    tab_id: TabId,
    admin: &UserAdminState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let scope = fluxdb_app::pg_grant_scope_from_state(
        admin.pg_grant_kind,
        &admin.pg_grant_schema,
        &admin.pg_grant_object,
        &admin.pg_grant_signature,
    );
    let mut table = div()
        .border_1()
        .border_color(colors.border)
        .rounded(colors.radius)
        .flex()
        .flex_col()
        .child(
            h_flex()
                .h(px(30.))
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(div().flex_1().min_w(px(0.)).px_3().child("权限"))
                .child(div().w(px(88.)).flex().justify_center().child("直接授予"))
                .child(div().w(px(88.)).flex().justify_center().child("可再授权"))
                .child(div().w(px(88.)).flex().justify_center().child("当前有效"))
                .child(div().w(px(160.)).px_3().child("来源")),
        );
    for grant in &admin.pg_effective_grants {
        let privilege = grant.privilege.clone();
        let edit = admin.pg_grant_edits.iter().find_map(|edit| match edit {
            PgRoleChange::GrantObject {
                privilege: p,
                scope: s,
                grant_option,
                ..
            } if p == &privilege && s == &scope => Some(("grant", *grant_option)),
            PgRoleChange::RevokeObject {
                privilege: p,
                scope: s,
                ..
            } if p == &privilege && s == &scope => Some(("revoke", false)),
            PgRoleChange::RevokeGrantOption {
                privilege: p,
                scope: s,
                ..
            } if p == &privilege && s == &scope => Some(("revoke_option", false)),
            _ => None,
        });
        // 草稿覆盖后的展示值。
        let draft_direct = match edit {
            Some(("grant", _)) => true,
            Some(("revoke", _)) | Some(("revoke_option", _)) => false,
            _ => grant.direct,
        };
        let draft_grantable = match edit {
            Some(("grant", go)) => go,
            Some(("revoke_option", _)) => false,
            _ => grant.direct && grant.grant_option,
        };
        // 来源归因：直接 → 可撤销；effective 但非直接 → 其他来源（PUBLIC/继承/属主），
        // 在补齐来源查询前如实标注「其他来源，待展开核实」。
        let source = if grant.direct {
            "直接授权".to_string()
        } else if grant.effective {
            "其他来源（PUBLIC/继承/属主），待展开核实".to_string()
        } else {
            "—".to_string()
        };
        table = table.child(
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
                        .child(privilege.clone())
                        .when(edit.is_some(), |row| {
                            row.child(
                                div()
                                    .px_1()
                                    .rounded(colors.radius * 0.5)
                                    .text_size(px(10.))
                                    .text_color(rgb(0xff9f0a))
                                    .child("待保存"),
                            )
                        }),
                )
                .child(pg_privilege_checkbox(
                    tab_id,
                    privilege.clone(),
                    scope.clone(),
                    draft_direct,
                    colors,
                    cx,
                ))
                .child(pg_privilege_grantable_checkbox(
                    tab_id,
                    privilege.clone(),
                    scope.clone(),
                    draft_direct,
                    draft_grantable,
                    colors,
                    cx,
                ))
                .child(
                    div()
                        .w(px(88.))
                        .flex()
                        .justify_center()
                        .text_color(if grant.effective {
                            colors.text
                        } else {
                            colors.muted
                        })
                        .child(if grant.effective { "是" } else { "否" }.to_string()),
                )
                .child(
                    div()
                        .w(px(160.))
                        .px_3()
                        .truncate()
                        .text_color(colors.muted)
                        .child(source),
                ),
        );
    }
    table
}

/// 「直接授予」草稿勾选：勾=Grant（保留 GRANT OPTION 现状），取消勾=Revoke。
fn pg_privilege_checkbox(
    tab_id: TabId,
    privilege: String,
    scope: PgObjectGrantScope,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!(
            "pg-priv-direct-{privilege}-{checked}"
        )))
        .w(px(88.))
        .h_full()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let op = if checked {
                    PgGrantEditOp::Revoke
                } else {
                    PgGrantEditOp::Grant
                };
                let grant_option = false;
                this.dispatch(
                    AppCommand::PgToggleGrant {
                        tab_id,
                        privilege: privilege.clone(),
                        scope: scope.clone(),
                        op,
                        grant_option,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(user_admin_check_box_visual(checked, colors))
}

/// 「可再授权」草稿勾选：仅在直接授予时可用；切换生成 Grant(grant_option) 或
/// RevokeGrantOption（只取消可再授权，不动基础权限）。
fn pg_privilege_grantable_checkbox(
    tab_id: TabId,
    privilege: String,
    scope: PgObjectGrantScope,
    direct: bool,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!(
            "pg-priv-grantable-{privilege}-{checked}"
        )))
        .w(px(88.))
        .h_full()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if !direct {
                    // 无直接授权时不可再授权不可编辑（需先授予）。
                    cx.stop_propagation();
                    return;
                }
                let op = if checked {
                    PgGrantEditOp::RevokeGrantOption
                } else {
                    PgGrantEditOp::Grant
                };
                this.dispatch(
                    AppCommand::PgToggleGrant {
                        tab_id,
                        privilege: privilege.clone(),
                        scope: scope.clone(),
                        op,
                        grant_option: !checked,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(user_admin_check_box_visual(checked, colors))
        .when(!direct, |cell| cell.opacity(0.45))
}

/// 授权目标的展示名（与 connector 渲染一致，仅用于变更清单）。
fn pg_scope_display(scope: &PgObjectGrantScope) -> String {
    match scope {
        PgObjectGrantScope::Database { database } => format!("DATABASE {database}"),
        PgObjectGrantScope::Schema { schema } => format!("SCHEMA {schema}"),
        PgObjectGrantScope::Relation { schema, name, kind } => match kind {
            PgRelationKind::Sequence => format!("SEQUENCE {schema}.{name}"),
            PgRelationKind::Table | PgRelationKind::View => format!("TABLE {schema}.{name}"),
        },
        PgObjectGrantScope::Routine {
            schema,
            name,
            signature,
        } => {
            format!("FUNCTION {schema}.{name}({signature})")
        }
    }
}

impl NavicatMain {
    /// 读取当前目标的对象权限（后台线程 → 回填 pg_object_grants/pg_effective_grants）。
    fn start_pg_object_grants_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._user_admin_pg_object_grant_tasks.contains_key(&tab_id) {
            return;
        }
        let target_fingerprint = self
            .user_admin_state_for(tab_id)
            .as_ref()
            .map(|admin| fluxdb_app::pg_grant_target_fingerprint(admin))
            .unwrap_or_default();
        self.dispatch(AppCommand::StartUserAdminPgObjectGrantsLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadUserAdminPgObjectGrants(tab_id)) {
                        AppEvent::UserAdminPgObjectGrantsLoaded(_, result) => result,
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "读取对象权限失败".to_string(),
                            message: "对象权限读取没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    // 完成后立即释放任务槽；否则下次切换目标会被旧 Task 挡住，永远停在读取中。
                    this._user_admin_pg_object_grant_tasks.remove(&tab_id);
                    this.dispatch(
                        AppCommand::FinishUserAdminPgObjectGrantsLoad {
                            tab_id,
                            target_fingerprint,
                            result,
                        },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        // GPUI Task 被丢弃会取消异步工作；必须持有到回写完成，否则权限页永远停留 loading。
        self._user_admin_pg_object_grant_tasks.insert(tab_id, task);
    }

    /// 目标切换会作废正在进行的对象权限读取；GPUI Task 移除后旧请求立即取消。
    fn cancel_pg_object_grants_load(&mut self, tab_id: &TabId) {
        self._user_admin_pg_object_grant_tasks.remove(tab_id);
    }

    /// 切换对象种类：清空对象选择，schema 保留（同 schema 内切换常见）。
    fn set_pg_grant_kind(
        &mut self,
        tab_id: TabId,
        kind: PgGrantObjectKind,
        cx: &mut Context<Self>,
    ) {
        self.cancel_pg_object_grants_load(&tab_id);
        let (schema, object, signature) = self
            .user_admin_state_for(tab_id)
            .map(|admin| {
                let object = match kind {
                    // schema/数据库本身是目标；前两个选择器选中后即满足目标完整。
                    PgGrantObjectKind::Schema => admin.pg_grant_schema.clone(),
                    PgGrantObjectKind::Database => admin.pg_grant_database.clone(),
                    _ => String::new(),
                };
                (admin.pg_grant_schema.clone(), object, String::new())
            })
            .unwrap_or_default();
        self.dispatch(
            AppCommand::SetUserAdminPgGrantTarget {
                tab_id,
                kind,
                schema,
                object,
                signature,
            },
            cx,
        );
        cx.notify();
    }

    /// 切换 schema：清空对象选择。
    fn set_pg_grant_schema(&mut self, tab_id: TabId, schema: String, cx: &mut Context<Self>) {
        self.cancel_pg_object_grants_load(&tab_id);
        let kind = self
            .user_admin_state_for(tab_id)
            .map(|admin| admin.pg_grant_kind)
            .unwrap_or_default();
        // Schema 类型没有第三个对象选择器：schema 选择本身要成为授权目标。
        let object = if kind == PgGrantObjectKind::Schema {
            schema.clone()
        } else {
            String::new()
        };
        self.dispatch(
            AppCommand::SetUserAdminPgGrantTarget {
                tab_id,
                kind,
                schema,
                object,
                signature: String::new(),
            },
            cx,
        );
        cx.notify();
    }

    /// 选择对象（`schema.name` / `schema.name(签名)`）：解析并写入目标，自动触发权限读取。
    fn set_pg_grant_object(&mut self, tab_id: TabId, object: String, cx: &mut Context<Self>) {
        self.cancel_pg_object_grants_load(&tab_id);
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let kind = admin.pg_grant_kind;
        // 解析 `schema.name(...)`：函数带签名；其余为 `schema.name`。
        let (schema, name, signature) = match kind {
            PgGrantObjectKind::Routine => {
                let open = object.rfind('(').unwrap_or(object.len());
                let close = object.rfind(')').unwrap_or(object.len());
                let head = &object[..open];
                let (schema, name) = head
                    .rsplit_once('.')
                    .map(|(s, n)| (s.to_string(), n.to_string()))
                    .unwrap_or((String::new(), head.to_string()));
                let signature = if close > open {
                    object[open + 1..close].to_string()
                } else {
                    String::new()
                };
                (schema, name, signature)
            }
            _ => {
                let (schema, name) = object
                    .rsplit_once('.')
                    .map(|(s, n)| (s.to_string(), n.to_string()))
                    .unwrap_or((admin.pg_grant_schema.clone(), object.clone()));
                (schema, name, String::new())
            }
        };
        self.dispatch(
            AppCommand::SetUserAdminPgGrantTarget {
                tab_id,
                kind,
                schema,
                object: name,
                signature,
            },
            cx,
        );
        cx.notify();
    }
}
