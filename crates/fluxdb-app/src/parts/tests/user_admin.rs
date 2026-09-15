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

/// PG：改版工作台的草稿状态默认值（无草稿、无变更、预定义角色判定）。
#[test]
fn user_admin_pg_draft_defaults_and_predefined_roles() {
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::Postgres);
    assert!(admin.pg_draft.is_none(), "默认无草稿");
    assert!(!admin.pg_has_draft_changes(), "无草稿时不应有未保存变更");
    assert!(UserAdminState::pg_is_predefined_role("pg_monitor"));
    assert!(UserAdminState::pg_is_predefined_role("pg_read_all_data"));
    assert!(!UserAdminState::pg_is_predefined_role("postgres"), "postgres 是普通超级用户名，不是预定义角色");
    // 新建草稿：角色名留空待填、默认 LOGIN；存在草稿即视为有变更。
    admin.pg_draft = Some(PgRoleDraft::new_create());
    assert!(admin.pg_draft_name().unwrap().is_empty(), "新建草稿角色名留空，避免自动误建");
    assert!(admin.pg_draft.as_ref().unwrap().can_login, "新建默认 LOGIN");
    assert!(admin.pg_has_draft_changes(), "新建草稿即为待保存变更");
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
    fn sql(scope: &PgObjectGrantScope) -> String {
        pg_grant_object_sql(scope).expect("合法目标应能渲染")
    }
    let table = PgObjectGrantScope::Relation {
        schema: "s".into(),
        name: "t".into(),
        kind: PgRelationKind::Table,
    };
    assert_eq!(sql(&table), "TABLE \"s\".\"t\"");
    let seq = PgObjectGrantScope::Relation {
        schema: "s".into(),
        name: "q".into(),
        kind: PgRelationKind::Sequence,
    };
    assert_eq!(sql(&seq), "SEQUENCE \"s\".\"q\"");
    let routine = PgObjectGrantScope::Routine {
        schema: "s".into(),
        name: "fn".into(),
        signature: "a integer".into(),
    };
    assert_eq!(sql(&routine), "FUNCTION \"s\".\"fn\"(a integer)");
    assert_eq!(
        sql(&PgObjectGrantScope::Schema { schema: "s".into() }),
        "SCHEMA \"s\""
    );
    assert_eq!(
        sql(&PgObjectGrantScope::Database { database: "d".into() }),
        "DATABASE \"d\""
    );
    // 双引号标识符转义。
    let quoted = PgObjectGrantScope::Relation {
        schema: "s".into(),
        name: "a\"b".into(),
        kind: PgRelationKind::Table,
    };
    assert_eq!(sql(&quoted), "TABLE \"s\".\"a\"\"b\"");
}

/// 函数签名是唯一无法靠引号消毒的部分（类型列表不是标识符），必须白名单拒绝注入。
///
/// 回归：早期签名原样拼进 `GRANT ... ON FUNCTION`，而 GRANT 走 simple query protocol，
/// 分号可另起语句——签名里塞 `int) TO postgres; ALTER ROLE x SUPERUSER; --` 即以管理角色
/// 执行任意 DDL。
#[test]
fn pg_grant_object_sql_rejects_injected_routine_signature() {
    for bad in [
        "int) TO postgres; ALTER ROLE attacker SUPERUSER; --",
        "int) TO postgres; DROP TABLE t; --",
        "integer /* 注释 */",
        "integer -- 行注释",
        "a' OR '1'='1",
        "integer\"x",
    ] {
        let scope = PgObjectGrantScope::Routine {
            schema: "s".into(),
            name: "fn".into(),
            signature: bad.into(),
        };
        assert!(
            pg_grant_object_sql(&scope).is_err(),
            "{bad:?} 应被拒绝，不得进入 GRANT 语句"
        );
    }
    // 正常签名（含数组、带长度、schema 限定、参数名）仍放行。
    for ok in [
        "integer, text",
        "character varying(10)",
        "integer[]",
        "pg_catalog.text",
        "IN a integer",
    ] {
        let scope = PgObjectGrantScope::Routine {
            schema: "s".into(),
            name: "fn".into(),
            signature: ok.into(),
        };
        assert!(
            pg_grant_object_sql(&scope).is_ok(),
            "{ok:?} 是合法签名，不应拒绝"
        );
    }
}

/// PG 权限页目标必须由用户显式选择；默认不得隐式选中 public。
#[test]
fn pg_privilege_targets_start_without_selection() {
    let admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::Postgres);
    assert_eq!(admin.pg_grant_database, "");
    assert_eq!(admin.pg_grant_schema, "");
    assert_eq!(admin.pg_grant_object, "");
    assert_eq!(admin.pg_grant_signature, "");
    assert_eq!(admin.pg_loaded_target, "");
}

/// 切换角色或刷新后，右侧会话态回到干净初始态：目标、权限读取、草稿变更和预览都清空。
#[test]
fn pg_role_editor_session_reset_clears_targets_and_draft_changes() {
    fn role(name: &str) -> PgRole {
        PgRole {
            name: name.into(),
            can_login: true,
            is_superuser: false,
            can_create_db: false,
            can_create_role: false,
            inherit: true,
            is_replication: false,
            bypass_rls: false,
            connection_limit: -1,
            valid_until: None,
            comment: None,
        }
    }

    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::Postgres);
    admin.pg_roles = vec![role("tenant_a"), role("tenant_b")];
    admin.pg_selected_role = Some("tenant_a".into());
    admin.pg_reset_draft_from_baseline();
    admin.pg_grant_database = "fluxdb_manual".into();
    admin.pg_grant_schema = "tenant_b".into();
    admin.pg_grant_object = "orders".into();
    admin.pg_grant_signature = "integer".into();
    admin.pg_grant_edits.push(PgRoleChange::GrantObject {
        privilege: "SELECT".into(),
        scope: PgObjectGrantScope::Relation {
            schema: "tenant_b".into(),
            name: "orders".into(),
            kind: PgRelationKind::Table,
        },
        grantee: "tenant_a".into(),
        grant_option: false,
    });
    admin.pg_memberships.push(PgRoleMembership {
        grantee: "app_group".into(),
        member: "tenant_a".into(),
        admin_option: false,
        inherit_option: true,
        set_option: true,
    });
    admin.pg_memberships_loaded = true;
    admin.active_detail_tab = UserAdminDetailTab::Privileges;

    admin.pg_reset_role_editor_session();
    admin.pg_reset_draft_from_baseline();

    assert_eq!(admin.pg_grant_database, "");
    assert_eq!(admin.pg_grant_schema, "");
    assert_eq!(admin.pg_grant_object, "");
    assert_eq!(admin.pg_grant_signature, "");
    assert_eq!(admin.pg_grant_edits, Vec::new());
    assert_eq!(admin.pg_membership_edits, Vec::new());
    assert_eq!(admin.pg_memberships, Vec::new());
    assert!(!admin.pg_memberships_loaded);
    assert_eq!(admin.pg_object_grants, None);
    assert_eq!(admin.pg_effective_grants, Vec::new());
    assert_eq!(admin.active_detail_tab, UserAdminDetailTab::General);
    assert!(!admin.pg_has_draft_changes());
}

/// 只读集成回归：维护库与目标库不同，逐个读取多个表和序列的权限。
#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE 指定包含多个表和序列的测试库"]
fn pg_object_grants_multiple_targets_in_selected_database() {
    let params = completion_smoke_params().expect("需要 FLUXDB_PG_SMOKE");
    let mut config = completion_smoke_config(&params);
    config.postgres_profile.as_mut().unwrap().basic.maintenance_database = "postgres".into();
    let connection_id = config.id;
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
    let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenUserAdmin(connection_id)) else {
        panic!("应打开权限页");
    };
    let AppEvent::UserAdminPgGrantTargetsLoaded(_, targets) = controller.dispatch(
        AppCommand::LoadPgGrantTargets { tab_id, database: params.4.clone() }
    ) else { panic!("应读取目标库对象"); };
    assert!(targets.tables.len() > 1, "测试库需要多个表");
    assert!(targets.sequences.len() > 1, "测试库需要多个序列");
    controller.dispatch(AppCommand::SetPgGrantDatabase { tab_id, database: params.4 });
    controller.user_admin_state_mut(tab_id).unwrap().pg_selected_role = Some(params.2);
    for (kind, objects) in [(PgGrantObjectKind::Table, targets.tables), (PgGrantObjectKind::Sequence, targets.sequences)] {
        for object in objects {
            let (schema, name) = object.split_once('.').unwrap();
            controller.dispatch(AppCommand::SetUserAdminPgGrantTarget {
                tab_id, kind, schema: schema.into(), object: name.into(), signature: String::new(),
            });
            controller.dispatch(AppCommand::StartUserAdminPgObjectGrantsLoad(tab_id));
            let fingerprint = pg_grant_target_fingerprint(controller.user_admin_state(tab_id).unwrap());
            let AppEvent::UserAdminPgObjectGrantsLoaded(_, result) = controller.dispatch(AppCommand::LoadUserAdminPgObjectGrants(tab_id)) else {
                panic!("应返回权限读取结果");
            };
            assert!(result.is_ok(), "{object}: {result:?}");
            assert!(!result.as_ref().unwrap().1.is_empty());
            controller.dispatch(AppCommand::FinishUserAdminPgObjectGrantsLoad { tab_id, target_fingerprint: fingerprint.clone(), result });
            let admin = controller.user_admin_state(tab_id).unwrap();
            assert!(!admin.loading_pg_grants);
            assert_eq!(admin.pg_loaded_target, fingerprint);
        }
    }
    assert_eq!(controller.connection_config(connection_id).unwrap().postgres_profile.as_ref().unwrap().basic.maintenance_database, "postgres");
}

/// 多对象快速切换时，旧请求回调不得结束新对象的 loading 或覆盖其基线。
#[test]
fn pg_stale_object_grants_result_does_not_finish_current_load() {
    let tab_id = TabId(1);
    let mut admin = UserAdminState::new(ConnectionId(1), None, PrivilegeScope::Postgres);
    admin.pg_grant_kind = PgGrantObjectKind::Table;
    admin.pg_grant_schema = "tenant_a".into();
    admin.pg_grant_object = "orders".into();
    let stale_fingerprint = pg_grant_target_fingerprint(&admin);

    admin.pg_grant_object = "customers".into();
    admin.loading_pg_grants = true;
    admin.pg_object_grants = Some(PgObjectGrants {
        owner: "current_owner".into(),
        acl_is_null: true,
        entries: Vec::new(),
    });

    let mut controller = AppController::new();
    controller.state.tabs.push(TabState {
        id: tab_id,
        title: "用户与权限".into(),
        kind: TabKind::UserAdmin(admin),
        dirty: false,
    });
    controller.dispatch(AppCommand::FinishUserAdminPgObjectGrantsLoad {
        tab_id,
        target_fingerprint: stale_fingerprint,
        result: Ok((
            PgObjectGrants {
                owner: "stale_owner".into(),
                acl_is_null: false,
                entries: Vec::new(),
            },
            Vec::new(),
        )),
    });

    let TabKind::UserAdmin(admin) = &controller.state.tabs[0].kind else {
        panic!("应为用户与权限标签页");
    };
    assert!(admin.loading_pg_grants);
    assert_eq!(
        admin.pg_object_grants.as_ref().map(|grants| grants.owner.as_str()),
        Some("current_owner")
    );
}
