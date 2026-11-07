// 「新建用户一次保存」的真实库端到端测试。
//
// 需求口径：一次保存完成 {账号 + 成员关系 + 权限}，不要求先建号再配置；多条 DDL 不做
// 原子保证，失败把原始错误抛给用户。覆盖 MySQL / PostgreSQL / Oracle / 达梦
// （SQL Server 不支持 macOS，不在此列）。
//
// 仅在提供对应环境变量时运行（`#[ignore]`）：
// - `FLUXDB_MYSQL_SMOKE=host:port:user:password`
// - `FLUXDB_PG_SMOKE=host:port:user:password`
// - `FLUXDB_ORACLE_SMOKE=host:port:service:user:password`
// - `FLUXDB_DM_SMOKE=host:port:user:password`
//
// 桌面测试二进制以 `panic=abort` 运行，`catch_unwind` 拦不住断言失败；因此用例体一律返回
// `Result<(), String>`，先清理本次创建的对象，再统一 panic，避免在共享测试库留下残留。

/// 测试体内统一的失败类型：带上下文的错误文本。
type Step<T = ()> = Result<T, String>;

/// 账号名统一小写 + 随机后缀：大小写敏感的库与折叠为大写的库都能用。
fn one_shot_name(prefix: &str) -> String {
    format!("fluxdb_ua_{prefix}_{}", uuid_suffix())
}

/// 8 位 36 进制后缀。
///
/// 各库对账号名长度有硬限制（MySQL 32、Oracle/DM 128），后缀必须短：
/// 纳秒时间戳取低位再做 36 进制，碰撞概率对单次测试足够低。
fn uuid_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() as u64 + elapsed.as_secs() * 1_000_000)
        .unwrap_or(0);
    let mut value = nanos;
    let mut out = String::new();
    for _ in 0..8 {
        let digit = (value % 36) as u32;
        out.push(std::char::from_digit(digit, 36).unwrap_or('0'));
        value /= 36;
    }
    out
}

fn smoke_parts(variable: &str, expected: usize) -> Option<Vec<String>> {
    let raw = std::env::var(variable).ok()?;
    let parts = raw
        .splitn(expected, ':')
        .map(str::to_string)
        .collect::<Vec<_>>();
    (parts.len() == expected).then_some(parts)
}

fn mysql_one_shot_config(parts: &[String]) -> fluxdb_core::ConnectionConfig {
    use std::collections::BTreeMap;

    let port = parts[1].parse::<u16>().expect("MySQL port");
    fluxdb_core::ConnectionConfig {
        id: fluxdb_core::ConnectionId(9_931),
        name: "MySQL 一次保存".into(),
        kind: fluxdb_core::DatabaseKind::MySql,
        endpoint: fluxdb_core::Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some("mysql".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(fluxdb_core::MysqlConnectionProfile {
            basic: fluxdb_core::MysqlBasicOptions {
                host: parts[0].clone(),
                port,
                database: "mysql".into(),
                username: parts[2].clone(),
                password: fluxdb_core::SecretRef::inline(parts[3].clone()),
                ..fluxdb_core::MysqlBasicOptions::default()

            },
            ..fluxdb_core::MysqlConnectionProfile::default()
        }),
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
        mongo_profile: None,
    }
    .mysql_resolved()
}

fn postgres_one_shot_config(parts: &[String]) -> fluxdb_core::ConnectionConfig {
    use std::collections::BTreeMap;

    let port = parts[1].parse::<u16>().expect("PostgreSQL port");
    fluxdb_core::ConnectionConfig {
        id: fluxdb_core::ConnectionId(9_932),
        name: "PostgreSQL 一次保存".into(),
        kind: fluxdb_core::DatabaseKind::Postgres,
        endpoint: fluxdb_core::Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some("postgres".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(fluxdb_core::PostgresConnectionProfile {
            basic: fluxdb_core::PostgresBasicOptions {
                host: parts[0].clone(),
                port,
                maintenance_database: "postgres".into(),
                username: parts[2].clone(),
                password: fluxdb_core::SecretRef::inline(parts[3].clone()),
            },
            ..fluxdb_core::PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
        mongo_profile: None,
    }
    .postgres_resolved()
}

fn oracle_one_shot_config(parts: &[String]) -> fluxdb_core::ConnectionConfig {
    use std::collections::BTreeMap;

    let port = parts[1].parse::<u16>().expect("Oracle port");
    let user = parts[3].to_ascii_uppercase();
    fluxdb_core::ConnectionConfig {
        id: fluxdb_core::ConnectionId(9_933),
        name: "Oracle 一次保存".into(),
        kind: fluxdb_core::DatabaseKind::Oracle,
        endpoint: fluxdb_core::Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(parts[2].clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        oracle_profile: Some(fluxdb_core::OracleConnectionProfile {
            basic: fluxdb_core::OracleBasicOptions {
                host: parts[0].clone(),
                port,
                connect_kind: fluxdb_core::OracleConnectKind::ServiceName,
                service_name: parts[2].clone(),
                sid: String::new(),
                schema: user.clone(),
                username: user,
                privilege: fluxdb_core::OraclePrivilege::Normal,
                password: fluxdb_core::SecretRef::inline(parts[4].clone()),
            },
            advanced: fluxdb_core::OracleAdvancedOptions::default(),
            ssh: Default::default(),
        }),
        mongo_profile: None,
    }
}

fn dameng_one_shot_config(parts: &[String]) -> fluxdb_core::ConnectionConfig {
    use std::collections::BTreeMap;

    let port = parts[1].parse::<u16>().expect("DM8 port");
    let user = parts[2].to_ascii_uppercase();
    fluxdb_core::ConnectionConfig {
        id: fluxdb_core::ConnectionId(9_934),
        name: "DM8 一次保存".into(),
        kind: fluxdb_core::DatabaseKind::Dameng,
        endpoint: fluxdb_core::Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some("DAMENG".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(fluxdb_core::DamengConnectionProfile {
            basic: fluxdb_core::DamengBasicOptions {
                host: parts[0].clone(),
                port,
                database: "DAMENG".into(),
                schema: user.clone(),
                username: user,
                password: fluxdb_core::SecretRef::inline(parts[3].clone()),
            },
            advanced: fluxdb_core::DamengAdvancedOptions::default(),
            ssh: Default::default(),
        }),
        sqlserver_profile: None,
        oracle_profile: None,
        mongo_profile: None,
    }
}

/// 建一个只装载了目标连接的控制器。
fn one_shot_controller(config: &fluxdb_core::ConnectionConfig) -> fluxdb_app::AppController {
    let mut controller = fluxdb_app::AppController::new();
    controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![config.clone()]));
    controller
}

fn open_one_shot_tab(
    controller: &mut fluxdb_app::AppController,
    connection_id: fluxdb_core::ConnectionId,
) -> Step<fluxdb_app::TabId> {
    match controller.dispatch(fluxdb_app::AppCommand::OpenUserAdmin(connection_id)) {
        fluxdb_app::AppEvent::TabOpened(tab_id) => Ok(tab_id),
        fluxdb_app::AppEvent::Failed(error) => Err(format!("打开用户与权限失败: {}", error.message)),
        event => Err(format!("unexpected open event: {event:?}")),
    }
}

/// 执行一段 SQL（建库/建角色、清理、核对都走生产查询链路）。
fn run_sql(
    controller: &mut fluxdb_app::AppController,
    connection_id: fluxdb_core::ConnectionId,
    text: &str,
) -> Step<fluxdb_core::QueryExecutionResult> {
    let tab_id = match controller.dispatch(fluxdb_app::AppCommand::OpenQueryEditor(connection_id)) {
        fluxdb_app::AppEvent::TabOpened(tab_id) => tab_id,
        event => return Err(format!("unexpected query tab event: {event:?}")),
    };
    match controller.dispatch(fluxdb_app::AppCommand::ExecuteQueryText {
        tab_id,
        text: text.to_string(),
    }) {
        fluxdb_app::AppEvent::QueryFinished(_, execution) => Ok(execution),
        fluxdb_app::AppEvent::Failed(error) => Err(format!("执行失败: {}\n{text}", error.message)),
        event => Err(format!("unexpected execution event: {event:?}")),
    }
}

/// 执行 DDL/SQL，任一条失败即算失败（用于建库、建角色等前置与清理）。
fn run_sql_ok(
    controller: &mut fluxdb_app::AppController,
    connection_id: fluxdb_core::ConnectionId,
    text: &str,
) -> Step {
    let execution = run_sql(controller, connection_id, text)?;
    match execution.summaries.iter().find(|summary| !summary.success) {
        Some(failed) => Err(format!("SQL 失败: {}\n{text}", failed.message)),
        None => Ok(()),
    }
}

fn scalar(
    controller: &mut fluxdb_app::AppController,
    connection_id: fluxdb_core::ConnectionId,
    text: &str,
) -> Step<Option<String>> {
    Ok(run_sql(controller, connection_id, text)?
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first())
        .map(|value| value.display_label()))
}

/// 用例收尾：先跑完清理，再把用例体的失败抛出来（桌面测试 panic 会 abort，清理必须在前）。
fn finish_one_shot(outcome: Step, cleanup: impl FnOnce() -> Step) {
    let cleanup_result = cleanup();
    if let Err(message) = outcome {
        panic!("{message}");
    }
    if let Err(message) = cleanup_result {
        panic!("清理失败: {message}");
    }
}

/// MySQL：一次保存里 CREATE USER + GRANT 成员角色 + GRANT 库权限，同批执行。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password 指向可达的 MySQL 8"]
fn mysql_one_shot_create_carries_membership_and_privilege() {
    use fluxdb_app::{AppCommand, AppEvent};

    const PASSWORD: &str = "Ua_Pw_12345!";
    let parts = smoke_parts("FLUXDB_MYSQL_SMOKE", 4).expect("FLUXDB_MYSQL_SMOKE");
    let config = mysql_one_shot_config(&parts);
    let connection_id = config.id;
    let database = format!("fluxdb_ua_{}", uuid_suffix());
    let role = one_shot_name("role");
    let user = one_shot_name("user");

    // 前置：被授予的使用者角色与目标库。角色先建，建库失败时不会留下孤儿库。
    // 查询连接库固定为 mysql，新建用户在这里执行。
    let mut setup = one_shot_controller(&config);
    run_sql_ok(
        &mut setup,
        connection_id,
        &format!("CREATE USER '{role}'@'%' IDENTIFIED BY 'Pw_role_12345!'"),
    )
    .expect("create role");
    run_sql_ok(
        &mut setup,
        connection_id,
        &format!("CREATE DATABASE `{database}`"),
    )
    .expect("create database");

    let outcome: Step = (|| {
        let mut controller = one_shot_controller(&config);
        let tab_id = open_one_shot_tab(&mut controller, connection_id)?;
        // 真实读取用户列表：成员关系的候选来自服务端用户/角色。
        let users = match controller.dispatch(AppCommand::LoadUserAdminUsers(tab_id)) {
            AppEvent::UserAdminUsersLoaded(_, users) => users,
            AppEvent::Failed(error) => return Err(format!("读取用户列表失败: {}", error.message)),
            event => return Err(format!("unexpected users event: {event:?}")),
        };
        if !users.iter().any(|candidate| candidate.user == role) {
            return Err(format!("服务端用户列表里没有 {role}"));
        }
        controller.dispatch(AppCommand::FinishUserAdminUsersLoad {
            tab_id,
            result: Ok(users),
        });
        controller.dispatch(AppCommand::BeginUserAdminCreateUser(tab_id));
        controller.dispatch(AppCommand::SetUserAdminCreateUser {
            tab_id,
            user: user.clone(),
        });
        controller.dispatch(AppCommand::SetUserAdminCreateHost {
            tab_id,
            host: "%".into(),
        });
        controller.dispatch(AppCommand::SetUserAdminCreatePassword {
            tab_id,
            password: PASSWORD.into(),
        });
        controller.dispatch(AppCommand::SetUserAdminNewPassword {
            tab_id,
            password: PASSWORD.into(),
        });
        // 成员关系候选的完整身份（含认证插件）来自服务端列表，界面勾选传的也是它。
        let role_identity = controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                fluxdb_app::TabKind::UserAdmin(admin) => admin
                    .effective_role_memberships()
                    .into_iter()
                    .map(|membership| membership.role)
                    .find(|candidate| candidate.user == role),
                _ => None,
            })
            .ok_or_else(|| format!("成员关系候选中没有 {role}"))?;
        controller.dispatch(AppCommand::SetUserAdminRoleMembershipGranted {
            tab_id,
            role: role_identity,
            granted: true,
        });
        controller.dispatch(AppCommand::AddUserAdminPrivilegeRow {
            tab_id,
            database: database.clone(),
        });
        let row_id = controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                fluxdb_app::TabKind::UserAdmin(admin) => {
                    admin.privilege_rows.first().map(|row| row.id)
                }
                _ => None,
            })
            .ok_or("权限行没有创建")?;
        controller.dispatch(AppCommand::ToggleUserAdminPrivilegeRowPrivilege {
            tab_id,
            row_id,
            privilege: "SELECT".into(),
        });

        // 一次保存 = 一份 SQL：建号、成员关系、权限同批。
        let admin = controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                fluxdb_app::TabKind::UserAdmin(admin) => Some(admin.clone()),
                _ => None,
            })
            .ok_or("用户与权限状态缺失")?;
        let provider = fluxdb_core::database_user_admin_provider(fluxdb_core::DatabaseKind::MySql)
            .ok_or("MySQL provider 缺失")?;
        let sql = crate::user_admin_all_sql_preview(provider, &admin).sql();
        let statements = sql.lines().collect::<Vec<_>>();
        let expect_all = [
            format!("CREATE USER '{user}'@'%'"),
            format!("GRANT '{role}'@'%' TO '{user}'@'%'"),
            format!("GRANT SELECT ON `{database}`.* TO '{user}'@'%'"),
        ];
        for prefix in &expect_all {
            if !statements.iter().any(|statement| statement.starts_with(prefix)) {
                return Err(format!("一次保存的 SQL 缺少 `{prefix}`:\n{sql}"));
            }
        }
        // 与 PG / Oracle 一致：保存前渲染的 SQL 里口令按草稿明文显示。
        if !sql.contains(PASSWORD) {
            return Err(format!("保存生成的 SQL 应显示明文口令（与 PG / Oracle 一致）:\n{sql}"));
        }

        controller.dispatch(AppCommand::StartUserAdminSqlApply(tab_id));
        match controller.dispatch(AppCommand::ApplyUserAdminSql { tab_id, sql }) {
            AppEvent::UserAdminSqlApplied(_) => {}
            AppEvent::Failed(error) => return Err(format!("一次保存执行失败: {}", error.message)),
            event => return Err(format!("unexpected apply event: {event:?}")),
        }
        controller.dispatch(AppCommand::FinishUserAdminSqlApply {
            tab_id,
            result: Ok(()),
        });
        Ok(())
    })();

    // 真实库核对：账号、成员关系、权限三者都来自同一次保存。
    let mut check = one_shot_controller(&config);
    let observed = (|| -> Step<(Option<String>, Option<String>, Option<String>)> {
        Ok((
            scalar(
                &mut check,
                connection_id,
                &format!("SELECT COUNT(*) FROM mysql.user WHERE user = '{user}'"),
            )?,
            scalar(
                &mut check,
                connection_id,
                &format!(
                    "SELECT COUNT(*) FROM mysql.role_edges \
                     WHERE TO_USER = '{user}' AND FROM_USER = '{role}'"
                ),
            )?,
            scalar(
                &mut check,
                connection_id,
                &format!(
                    "SELECT COUNT(*) FROM information_schema.SCHEMA_PRIVILEGES \
                     WHERE GRANTEE = \"'{user}'@'%'\" AND TABLE_SCHEMA = '{database}' \
                     AND PRIVILEGE_TYPE = 'SELECT'"
                ),
            )?,
        ))
    })();

    let cleanup = || -> Step {
        for statement in [
            format!("DROP USER IF EXISTS '{user}'@'%'"),
            format!("DROP USER IF EXISTS '{role}'@'%'"),
            format!("DROP DATABASE IF EXISTS `{database}`"),
        ] {
            run_sql_ok(&mut one_shot_controller(&config), connection_id, &statement)?;
        }
        Ok(())
    };
    let (account, membership, privilege) = match observed {
        Ok(observed) => observed,
        Err(message) => {
            let _ = cleanup();
            panic!("核对失败: {message}");
        }
    };
    finish_one_shot(outcome, cleanup);
    assert_eq!(account.as_deref(), Some("1"), "账号应由一次保存创建");
    assert_eq!(membership.as_deref(), Some("1"), "成员关系应随同一次保存生效");
    assert_eq!(privilege.as_deref(), Some("1"), "权限应随同一次保存生效");
}

/// PostgreSQL：一次保存里 CREATE ROLE + 成员关系 + 库级 CONNECT，单事务提交。
#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE=host:port:user:password 指向可达的 PostgreSQL"]
fn postgres_one_shot_create_carries_membership_and_privilege() {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::PgObjectGrantScope;

    let parts = smoke_parts("FLUXDB_PG_SMOKE", 4).expect("FLUXDB_PG_SMOKE");
    let config = postgres_one_shot_config(&parts);
    let connection_id = config.id;
    let group = one_shot_name("grp");
    let role = one_shot_name("role");

    let mut setup = one_shot_controller(&config);
    run_sql_ok(&mut setup, connection_id, &format!("CREATE ROLE \"{group}\""))
        .expect("create group role");

    let outcome: Step = (|| {
        let mut controller = one_shot_controller(&config);
        let tab_id = open_one_shot_tab(&mut controller, connection_id)?;
        controller.dispatch(AppCommand::PgBeginCreateRole(tab_id));
        controller.dispatch(AppCommand::SetPgDraftName {
            tab_id,
            name: role.clone(),
        });
        controller.dispatch(AppCommand::SetPgDraftCanLogin {
            tab_id,
            can_login: true,
        });
        controller.dispatch(AppCommand::SetPgDraftPassword {
            tab_id,
            password: "Ua_Pw_12345!".into(),
        });
        // 成员关系：新角色是 group 的成员。
        controller.dispatch(AppCommand::PgMembershipGrant {
            tab_id,
            role: group.clone(),
            member: role.clone(),
            admin: false,
            inherit: true,
            set: true,
        });
        // 权限：库级 CONNECT，与建角色同一份计划。
        controller.dispatch(AppCommand::SetPgGrantDatabase {
            tab_id,
            database: "postgres".into(),
        });
        controller.dispatch(AppCommand::PgToggleGrant {
            tab_id,
            privilege: "CONNECT".into(),
            scope: PgObjectGrantScope::Database {
                database: "postgres".into(),
            },
            op: fluxdb_app::PgGrantEditOp::Grant,
            grant_option: false,
        });

        controller.dispatch(AppCommand::StartPgPlanPreview(tab_id));
        let statements = match controller.dispatch(AppCommand::LoadPgPlanPreview(tab_id)) {
            AppEvent::UserAdminPgPlanPreview(_, statements) => statements,
            AppEvent::Failed(error) => return Err(format!("PG 计划预览失败: {}", error.message)),
            event => return Err(format!("unexpected pg preview event: {event:?}")),
        };
        let expect_all = [
            format!("CREATE ROLE \"{role}\""),
            format!("GRANT \"{group}\" TO \"{role}\""),
            format!("GRANT CONNECT ON DATABASE \"postgres\" TO \"{role}\""),
        ];
        for prefix in &expect_all {
            if !statements.iter().any(|sql| sql.starts_with(prefix)) {
                return Err(format!("PG 计划缺少 `{prefix}`:\n{statements:#?}"));
            }
        }
        controller.dispatch(AppCommand::FinishPgPlanPreview {
            tab_id,
            result: Ok(statements),
        });

        controller.dispatch(AppCommand::StartPgPlanApply(tab_id));
        let (plan, result) = match controller.dispatch(AppCommand::ApplyPgRolePlan(tab_id)) {
            AppEvent::UserAdminPgRolePlanFinished(_, plan, result) => (plan, result),
            event => return Err(format!("unexpected pg apply event: {event:?}")),
        };
        if let Err(error) = &result {
            return Err(format!("PG 一次保存失败: {}", error.message));
        }
        controller.dispatch(AppCommand::FinishPgRolePlanApply {
            tab_id,
            plan,
            result: Ok(Vec::new()),
        });
        Ok(())
    })();

    let mut check = one_shot_controller(&config);
    let observed = (|| -> Step<(Option<String>, Option<String>, Option<String>)> {
        Ok((
            scalar(
                &mut check,
                connection_id,
                &format!("SELECT COUNT(*) FROM pg_roles WHERE rolname = '{role}'"),
            )?,
            scalar(
                &mut check,
                connection_id,
                &format!(
                    "SELECT COUNT(*) FROM pg_auth_members m \
                     JOIN pg_roles granted ON granted.oid = m.roleid \
                     JOIN pg_roles member ON member.oid = m.member \
                     WHERE granted.rolname = '{group}' AND member.rolname = '{role}'"
                ),
            )?,
            scalar(
                &mut check,
                connection_id,
                &format!(
                    "SELECT COUNT(*) FROM pg_database d, aclexplode(d.datacl) acl \
                     JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                     WHERE d.datname = 'postgres' AND grantee.rolname = '{role}' \
                     AND acl.privilege_type = 'CONNECT'"
                ),
            )?,
        ))
    })();

    let cleanup = || -> Step {
        // 角色身上挂着库级 GRANT，直接 DROP 会被拒；先回收它的对象与特权。
        for statement in [
            format!("DROP OWNED BY \"{role}\""),
            format!("DROP ROLE IF EXISTS \"{role}\""),
            format!("DROP ROLE IF EXISTS \"{group}\""),
        ] {
            run_sql_ok(&mut one_shot_controller(&config), connection_id, &statement)?;
        }
        Ok(())
    };
    let (exists, membership, privilege) = match observed {
        Ok(observed) => observed,
        Err(message) => {
            let _ = cleanup();
            panic!("核对失败: {message}");
        }
    };
    finish_one_shot(outcome, cleanup);
    assert_eq!(exists.as_deref(), Some("1"), "角色应由一次保存创建");
    assert_eq!(membership.as_deref(), Some("1"), "成员关系应随同一次保存生效");
    assert_eq!(privilege.as_deref(), Some("1"), "库级权限应随同一次保存生效");
}

/// Oracle / 达梦共用的账号面板：新建账号 + 角色授权 + 系统权限一次保存。
///
/// 返回实际执行的脱敏语句。语句按顺序逐条提交，失败即把原始错误抛给用户。
fn oracle_like_one_shot_apply(
    controller: &mut fluxdb_app::AppController,
    tab_id: fluxdb_app::TabId,
    user: &str,
    role_grant: &str,
    system_grant: &str,
) -> Step<Vec<String>> {
    use fluxdb_app::{
        AppCommand, AppEvent, OracleCreateDraftField, OracleGrantDraftField, OracleGrantKind,
    };

    controller.dispatch(AppCommand::BeginOracleCreateUser(tab_id));
    for (field, value) in [
        (OracleCreateDraftField::Name, user.to_string()),
        (OracleCreateDraftField::Password, "Ua_Pw_12345".into()),
        (OracleCreateDraftField::ConfirmPassword, "Ua_Pw_12345".into()),
    ] {
        controller.dispatch(AppCommand::SetOracleCreateDraftField { tab_id, field, value });
    }
    // 第一条授权：角色（成员关系），与建号同一次保存。
    controller.dispatch(AppCommand::SetOracleGrantDraftField {
        tab_id,
        field: OracleGrantDraftField::Name,
        value: role_grant.into(),
    });
    controller.dispatch(AppCommand::QueueOracleGrantDraft {
        tab_id,
        kind: OracleGrantKind::Role,
    });
    // 第二条授权：系统权限，同一份计划。
    controller.dispatch(AppCommand::SetOracleGrantDraftField {
        tab_id,
        field: OracleGrantDraftField::Name,
        value: system_grant.into(),
    });
    controller.dispatch(AppCommand::QueueOracleGrantDraft {
        tab_id,
        kind: OracleGrantKind::System,
    });

    // 点保存先出确认框：`PreviewOracleApply` 只渲染并放进 `pending_sql`，不执行。
    if matches!(
        controller.dispatch(AppCommand::PreviewOracleApply(tab_id)),
        AppEvent::Failed(_)
    ) {
        return Err("保存时渲染预览失败".to_string());
    }
    let pending_sql = controller
        .state()
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            fluxdb_app::TabKind::UserAdmin(admin) => admin.pending_sql.clone(),
            _ => None,
        })
        .ok_or_else(|| "保存后没有确认框 SQL".to_string())?;
    if !pending_sql.sql.contains(&format!("CREATE USER \"{user}\"")) {
        return Err(format!("确认框 SQL 缺少建号语句:\n{}", pending_sql.sql));
    }
    // 确认后才执行：`StartOracleApply` 合成计划并置位，`ApplyOraclePlan` 提交。
    if matches!(
        controller.dispatch(AppCommand::StartOracleApply(tab_id)),
        AppEvent::Failed(_)
    ) {
        return Err("确认后合成计划失败".to_string());
    }
    // 计划里必须同时带上建号与两条授权，证明「一次保存」就够了。
    let dialect = controller
        .state()
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            fluxdb_app::TabKind::UserAdmin(admin) => admin.oracle.as_ref(),
            _ => None,
        })
        .map(|state| (state.dialect, state.pending_plan.clone()))
        .ok_or_else(|| "Oracle 用户与权限状态不存在".to_string())?;
    let (dialect, plan) = dialect;
    let plan = plan.ok_or_else(|| "保存后没有待执行计划".to_string())?;
    let rendered = fluxdb_core::render_user_admin_plan(&plan, false, dialect)
        .map_err(|error| format!("渲染待执行语句失败: {}", error.message))?;
    if !rendered
        .first()
        .is_some_and(|sql| sql.contains(&format!("CREATE USER \"{user}\"")))
    {
        return Err(format!("首条不是建号语句:\n{rendered:#?}"));
    }
    for expected in [
        format!("GRANT \"{role_grant}\" TO \"{user}\";"),
        format!("GRANT {system_grant} TO \"{user}\";"),
    ] {
        if !rendered.iter().any(|sql| sql == &expected) {
            return Err(format!("一次保存缺少 `{expected}`:\n{rendered:#?}"));
        }
    }
    match controller.dispatch(AppCommand::ApplyOraclePlan(tab_id)) {
        AppEvent::OraclePlanApplied(_, statements) => Ok(statements),
        AppEvent::Failed(error) => Err(format!(
            "一次保存失败: {}\n待执行语句:\n{rendered:#?}",
            error.message
        )),
        event => Err(format!("unexpected oracle apply event: {event:?}")),
    }
}

/// 回读目录：成功后界面必须能看到新账号（证明刷新路径也认同这次保存）。
fn oracle_like_snapshot(
    controller: &mut fluxdb_app::AppController,
    tab_id: fluxdb_app::TabId,
    user: &str,
) -> Step {
    use fluxdb_app::{AppCommand, AppEvent};

    let snapshot = match controller.dispatch(AppCommand::LoadOracleUsers(tab_id)) {
        AppEvent::OracleUsersLoaded(_, snapshot) => snapshot,
        AppEvent::Failed(error) => return Err(format!("回读账号目录失败: {}", error.message)),
        event => return Err(format!("unexpected oracle load event: {event:?}")),
    };
    if !snapshot.contains_user(user) {
        return Err(format!("回读不到新账号 {user}"));
    }
    Ok(())
}

/// Oracle / 达梦共用的一次保存用例体。
fn oracle_like_one_shot_case(
    config: &fluxdb_core::ConnectionConfig,
    user: &str,
    role_grant: &str,
    system_grant: &str,
) -> Step {
    use fluxdb_app::{AppCommand, AppEvent};

    let mut controller = one_shot_controller(config);
    let tab_id = open_one_shot_tab(&mut controller, config.id)?;
    // 先读一次目录：新建草稿要据此判断账号是否已存在，并拿到当前登录身份。
    match controller.dispatch(AppCommand::LoadOracleUsers(tab_id)) {
        AppEvent::OracleUsersLoaded(_, _) => {}
        AppEvent::Failed(error) => return Err(format!("读取账号目录失败: {}", error.message)),
        event => return Err(format!("unexpected oracle load event: {event:?}")),
    }
    let applied = oracle_like_one_shot_apply(&mut controller, tab_id, user, role_grant, system_grant)?;
    if !applied
        .iter()
        .any(|sql| sql == &format!("GRANT {system_grant} TO \"{user}\";"))
    {
        return Err(format!("系统权限未同批执行:\n{applied:#?}"));
    }
    controller.dispatch(AppCommand::FinishOracleApply {
        tab_id,
        result: Ok(applied),
    });
    oracle_like_snapshot(&mut controller, tab_id, user)?;
    // 保存成功后待执行授权必须清空，否则下一次保存会重复 GRANT。
    let pending = controller
        .state()
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            fluxdb_app::TabKind::UserAdmin(admin) => {
                admin.oracle.as_ref().map(|state| state.pending_grants.len())
            }
            _ => None,
        })
        .ok_or("Oracle 状态缺失")?;
    if pending != 0 {
        return Err(format!("保存成功后待执行授权应清空，实际剩 {pending} 条"));
    }
    Ok(())
}

fn oracle_like_observed(
    config: &fluxdb_core::ConnectionConfig,
    user: &str,
    role_grant: &str,
    system_grant: &str,
) -> Step<(Option<String>, Option<String>, Option<String>)> {
    let mut check = one_shot_controller(config);
    Ok((
        scalar(
            &mut check,
            config.id,
            &format!("SELECT COUNT(*) FROM ALL_USERS WHERE USERNAME = '{user}'"),
        )?,
        scalar(
            &mut check,
            config.id,
            &format!(
                "SELECT COUNT(*) FROM DBA_ROLE_PRIVS \
                 WHERE GRANTEE = '{user}' AND GRANTED_ROLE = '{role_grant}'"
            ),
        )?,
        scalar(
            &mut check,
            config.id,
            &format!(
                "SELECT COUNT(*) FROM DBA_SYS_PRIVS \
                 WHERE GRANTEE = '{user}' AND PRIVILEGE = '{system_grant}'"
            ),
        )?,
    ))
}

fn oracle_like_cleanup(config: &fluxdb_core::ConnectionConfig, user: &str) -> Step {
    run_sql_ok(
        &mut one_shot_controller(config),
        config.id,
        &format!("DROP USER \"{user}\" CASCADE"),
    )
}

/// Oracle：账号 + 角色 + 系统权限一次保存，逐条提交，失败抛原始错误。
#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_one_shot_create_carries_membership_and_privilege() {
    let parts = smoke_parts("FLUXDB_ORACLE_SMOKE", 5).expect("FLUXDB_ORACLE_SMOKE");
    let config = oracle_one_shot_config(&parts);
    let user = one_shot_name("oracle").to_ascii_uppercase();
    // 用连接账号自己有权限转授的项：RESOURCE 角色 + CREATE TABLE 系统权限。
    let outcome = oracle_like_one_shot_case(&config, &user, "RESOURCE", "CREATE TABLE");
    let observed = oracle_like_observed(&config, &user, "RESOURCE", "CREATE TABLE");
    let (None, Ok((exists, role, privilege))) = (None::<()>, observed) else {
        let _ = oracle_like_cleanup(&config, &user);
        panic!("Oracle 核对失败：保存结果 {outcome:?}，核对结果见上一条 panic");
    };
    finish_one_shot(outcome, || oracle_like_cleanup(&config, &user));
    assert_eq!(exists.as_deref(), Some("1"), "账号应由一次保存创建");
    assert_eq!(role.as_deref(), Some("1"), "角色授权应随同一次保存生效");
    assert_eq!(privilege.as_deref(), Some("1"), "系统权限应随同一次保存生效");
}

/// 达梦：与 Oracle 同一面板，方言为 Dameng（登录权限是 CREATE SESSION，无 CONNECT 角色）。
#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_one_shot_create_carries_membership_and_privilege() {
    use fluxdb_app::{AppCommand, TabKind};

    let parts = smoke_parts("FLUXDB_DM_SMOKE", 4).expect("FLUXDB_DM_SMOKE");
    let config = dameng_one_shot_config(&parts);
    let user = one_shot_name("dm").to_ascii_uppercase();

    let outcome: Step = (|| {
        let mut controller = one_shot_controller(&config);
        let tab_id = open_one_shot_tab(&mut controller, config.id)?;
        // 达梦复用 Oracle 账号面板，方言必须是 Dameng。
        let dialect = controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::UserAdmin(admin) => admin.oracle.as_ref().map(|state| state.dialect),
                _ => None,
            });
        if dialect != Some(fluxdb_core::UserAdminSqlDialect::Dameng) {
            return Err(format!("达梦方言应为 Dameng，实际 {dialect:?}"));
        }
        // 打开过一次标签页后关闭，交给共用用例体重开并走完整流程。
        controller.dispatch(AppCommand::CloseTab(tab_id));
        oracle_like_one_shot_case(&config, &user, "RESOURCE", "CREATE TABLE")
    })();
    let observed = oracle_like_observed(&config, &user, "RESOURCE", "CREATE TABLE");
    let cleanup = || oracle_like_cleanup(&config, &user);
    let (exists, role, privilege) = match observed {
        Ok(observed) => observed,
        Err(message) => {
            let _ = cleanup();
            panic!("达梦核对失败: {message}");
        }
    };
    finish_one_shot(outcome, cleanup);
    assert_eq!(exists.as_deref(), Some("1"), "账号应由一次保存创建");
    assert_eq!(role.as_deref(), Some("1"), "角色授权应随同一次保存生效");
    assert_eq!(privilege.as_deref(), Some("1"), "系统权限应随同一次保存生效");
}
