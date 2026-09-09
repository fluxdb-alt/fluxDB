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
