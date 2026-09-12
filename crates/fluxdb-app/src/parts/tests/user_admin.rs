#[test]
fn user_admin_default_role_toggle_does_not_change_grant() {
    let user = DatabaseUserIdentity {
        user: "mysql.infoschema".to_string(),
        host: "localhost".to_string(),
        plugin: None,
    };
    let role = DatabaseUserIdentity {
        user: "mysql.session".to_string(),
        host: "localhost".to_string(),
        plugin: None,
    };
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::MySql);
    admin.users = vec![user.clone(), role.clone()];
    admin.selected_user = Some(user);

    admin.set_role_membership_default(role.clone(), true);
    let membership = admin
        .effective_role_memberships()
        .into_iter()
        .find(|membership| membership.role == role)
        .unwrap();
    assert!(!membership.granted);
    assert!(membership.default_role);

    admin.set_role_membership_default(role.clone(), false);
    let membership = admin
        .effective_role_memberships()
        .into_iter()
        .find(|membership| membership.role == role)
        .unwrap();
    assert!(!membership.granted);
    assert!(!membership.default_role);
}

#[test]
fn user_admin_grant_toggle_does_not_change_default_role() {
    let user = DatabaseUserIdentity {
        user: "mysql.infoschema".to_string(),
        host: "localhost".to_string(),
        plugin: None,
    };
    let role = DatabaseUserIdentity {
        user: "mysql.session".to_string(),
        host: "localhost".to_string(),
        plugin: None,
    };
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::MySql);
    admin.users = vec![user.clone(), role.clone()];
    admin.selected_user = Some(user);
    admin.set_role_membership_default(role.clone(), true);
    admin.set_role_membership_granted(role.clone(), true);

    admin.set_role_membership_granted(role.clone(), false);
    let membership = admin
        .effective_role_memberships()
        .into_iter()
        .find(|membership| membership.role == role)
        .unwrap();
    assert!(!membership.granted);
    assert!(membership.default_role);
}

#[test]
fn user_admin_grants_loaded_marker_tracks_selected_user() {
    let user = DatabaseUserIdentity {
        user: "app".to_string(),
        host: "%".to_string(),
        plugin: None,
    };
    let other = DatabaseUserIdentity {
        user: "root".to_string(),
        host: "localhost".to_string(),
        plugin: None,
    };
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::MySql);
    admin.selected_user = Some(user.clone());

    assert!(!admin.grants_loaded_for_selected_user());

    admin.grants_loaded_user = Some(other);
    assert!(!admin.grants_loaded_for_selected_user());

    admin.grants_loaded_user = Some(user);
    assert!(admin.grants_loaded_for_selected_user());
}

#[test]
fn user_admin_role_member_toggle_marks_member_grants_dirty() {
    let role = DatabaseUserIdentity {
        user: "reader".to_string(),
        host: "%".to_string(),
        plugin: None,
    };
    let member = DatabaseUserIdentity {
        user: "app".to_string(),
        host: "%".to_string(),
        plugin: None,
    };
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::MySql);
    admin.users = vec![role.clone(), member.clone()];
    admin.selected_user = Some(role.clone());
    admin.member_grants = vec![UserRoleMember {
        member: member.clone(),
        granted: false,
    }];
    admin.member_grants_loaded_role = Some(role);

    assert!(!admin.role_members_dirty());

    admin.set_role_member_granted(member.clone(), true);
    let effective_member = admin
        .effective_role_members()
        .into_iter()
        .find(|item| item.member == member)
        .unwrap();
    assert!(effective_member.granted);
    assert!(admin.role_members_dirty());
}

#[test]
fn user_admin_privilege_row_keeps_database_and_checked_privileges() {
    let mut admin = UserAdminState::new(ConnectionId(1), Some("app".to_string()), PrivilegeScope::MySql);

    assert!(!admin.privileges_dirty());
    admin.add_privilege_row("app".to_string());
    let row_id = admin.privilege_rows[0].id;

    admin.set_privilege_row_database(row_id, "analytics".to_string());
    admin.toggle_privilege_row_privilege(row_id, "ALTER".to_string());
    admin.set_privilege_row_grant_option(row_id, true);

    let row = &admin.privilege_rows[0];
    assert_eq!(row.database, "analytics");
    assert_eq!(row.privileges, vec!["ALTER".to_string()]);
    assert!(row.grant_option);
    assert!(admin.privileges_dirty());

    admin.toggle_privilege_row_privilege(row_id, "ALTER".to_string());
    assert!(!admin.privileges_dirty());
}

#[test]
fn user_admin_loaded_privileges_are_not_dirty_until_changed() {
    let mut admin =
        UserAdminState::new(ConnectionId(1), Some("app".to_string()), PrivilegeScope::MySql);
    admin.set_privilege_grants(vec![DatabasePrivilegeGrant {
        database: "llm".to_string(),
        privileges: vec!["SELECT".to_string()],
        grant_option: false,
    }]);

    assert_eq!(admin.privilege_rows.len(), 1);
    assert!(!admin.privileges_dirty());

    let row_id = admin.privilege_rows[0].id;
    admin.toggle_privilege_row_privilege(row_id, "INSERT".to_string());

    assert!(admin.privileges_dirty());
}

/// PG：成员关系 → 某角色所在组角色，纯函数过滤逻辑。
#[test]
fn pg_groups_for_member_filters_and_returns_groups() {
    let memberships = vec![
        fluxdb_core::PgRoleMembership {
            grantee: "analyst_group".to_string(),
            member: "alice".to_string(),
            admin_option: true,
            inherit_option: true,
            set_option: true,
        },
        fluxdb_core::PgRoleMembership {
            grantee: "reader".to_string(),
            member: "alice".to_string(),
            admin_option: false,
            inherit_option: true,
            set_option: true,
        },
        fluxdb_core::PgRoleMembership {
            grantee: "reader".to_string(),
            member: "bob".to_string(),
            admin_option: false,
            inherit_option: true,
            set_option: false,
        },
    ];
    assert_eq!(pg_groups_for_member(&memberships, "alice"), vec!["analyst_group", "reader"]);
    assert_eq!(pg_groups_for_member(&memberships, "bob"), vec!["reader"]);
    assert!(pg_groups_for_member(&memberships, "eve").is_empty());
}

/// PG：UserAdminState 的 pg_can_login（新建角色可登录）默认与切换。
#[test]
fn user_admin_pg_can_login_defaults_and_reflects() {
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::Postgres);
    assert!(admin.pg_can_login, "PG 新建角色默认可登录 LOGIN");
    admin.pg_can_login = false;
    assert!(!admin.pg_can_login, "切换为 NOLOGIN 组角色应生效");
}

/// PG：UserAdminState 角色内联编辑模式（改密/重命名）默认关闭，可切换与清空。
#[test]
fn user_admin_pg_edit_mode_defaults_and_clears() {
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::Postgres);
    assert_eq!(admin.pg_edit_mode, PgRoleEditMode::None, "默认无编辑模式");
    admin.pg_edit_mode = PgRoleEditMode::Rename;
    assert_eq!(admin.pg_edit_mode, PgRoleEditMode::Rename);
    admin.pg_edit_mode = PgRoleEditMode::Password;
    assert_eq!(admin.pg_edit_mode, PgRoleEditMode::Password);
    admin.pg_edit_mode = PgRoleEditMode::None;
    assert_eq!(admin.pg_edit_mode, PgRoleEditMode::None, "结束后回到 None");
}

/// PG 权限面板：授权目标 scope 构造（表/视图/序列/函数/schema/数据库 + 空 schema 回退 public）。
#[test]
fn pg_grant_scope_from_state_maps_kinds_and_defaults_schema() {
    assert_eq!(
        pg_grant_scope_from_state(PgGrantObjectKind::Table, "s1", "t", ""),
        PgObjectGrantScope::Relation {
            schema: "s1".into(),
            name: "t".into(),
            kind: PgRelationKind::Table,
        }
    );
    assert_eq!(
        pg_grant_scope_from_state(PgGrantObjectKind::Sequence, "s1", "seq", ""),
        PgObjectGrantScope::Relation {
            schema: "s1".into(),
            name: "seq".into(),
            kind: PgRelationKind::Sequence,
        }
    );
    assert_eq!(
        pg_grant_scope_from_state(PgGrantObjectKind::Routine, "s1", "fn", "a integer"),
        PgObjectGrantScope::Routine {
            schema: "s1".into(),
            name: "fn".into(),
            signature: "a integer".into(),
        }
    );
    assert_eq!(
        pg_grant_scope_from_state(PgGrantObjectKind::Schema, "s1", "", ""),
        PgObjectGrantScope::Schema { schema: "s1".into() }
    );
    assert_eq!(
        pg_grant_scope_from_state(PgGrantObjectKind::Database, "", "appdb", ""),
        PgObjectGrantScope::Database { database: "appdb".into() }
    );
    // 空 schema 回退 public。
    assert_eq!(
        pg_grant_scope_from_state(PgGrantObjectKind::Table, "  ", "t", ""),
        PgObjectGrantScope::Relation {
            schema: "public".into(),
            name: "t".into(),
            kind: PgRelationKind::Table,
        }
    );
}

/// PG 权限面板：GRANT/REVOKE 的 `ON <object>` 片段（双引号 + 关键字 + 函数签名）。
#[test]
fn pg_grant_object_sql_renders_keyword_and_quoting() {
    let table = PgObjectGrantScope::Relation {
        schema: "s".into(),
        name: "t".into(),
        kind: PgRelationKind::Table,
    };
    assert_eq!(pg_grant_object_sql(&table), "TABLE \"s\".\"t\"");
    let seq = PgObjectGrantScope::Relation {
        schema: "s".into(),
        name: "q".into(),
        kind: PgRelationKind::Sequence,
    };
    assert_eq!(pg_grant_object_sql(&seq), "SEQUENCE \"s\".\"q\"");
    let routine = PgObjectGrantScope::Routine {
        schema: "s".into(),
        name: "fn".into(),
        signature: "a integer".into(),
    };
    assert_eq!(pg_grant_object_sql(&routine), "FUNCTION \"s\".\"fn\"(a integer)");
    assert_eq!(
        pg_grant_object_sql(&PgObjectGrantScope::Schema { schema: "s".into() }),
        "SCHEMA \"s\""
    );
    assert_eq!(
        pg_grant_object_sql(&PgObjectGrantScope::Database { database: "d".into() }),
        "DATABASE \"d\""
    );
    // 双引号标识符转义。
    let quoted = PgObjectGrantScope::Relation {
        schema: "s".into(),
        name: "a\"b".into(),
        kind: PgRelationKind::Table,
    };
    assert_eq!(pg_grant_object_sql(&quoted), "TABLE \"s\".\"a\"\"b\"");
}
