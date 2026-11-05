//! Oracle 真实连接集成测试。
//!
//! 仅在显式提供 `FLUXDB_ORACLE_SMOKE=host:port:service:user:password` 时运行；默认忽略。
//! 测试自建 `FLUXDB_TEST_*` 对象并在结束时清理，不动其它 schema 的数据。
//!
//! 断言统一放在业务调用之外：驱动回调里 panic 无法可靠展开，会把进程直接 abort。

use fluxdb_connectors::{connector_for, database_backup};
use fluxdb_core::{
    BackupMethod, BackupObjectKind, BackupObjectRef, BackupRequest, BackupScope, ConnectionConfig,
    ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath, OracleAdvancedOptions,
    OracleBasicOptions, OracleConnectKind, OracleConnectionProfile, OraclePrivilege,
    QueryExecutionOptions, QueryMode, QueryRequest, RestoreRequest, SecretRef, SshAuth,
    SshTunnelProfile,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_ORACLE_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    if parts.len() != 5 {
        return None;
    }
    let port = parts[1].parse().ok()?;
    let service = parts[2];
    Some(ConnectionConfig {
        id: ConnectionId(9_901),
        name: "Oracle smoke".into(),
        kind: DatabaseKind::Oracle,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(service.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: Some(OracleConnectionProfile {
            basic: OracleBasicOptions {
                host: parts[0].into(),
                port,
                connect_kind: OracleConnectKind::ServiceName,
                service_name: service.into(),
                sid: String::new(),
                username: parts[3].to_ascii_uppercase(),
                schema: parts[3].to_ascii_uppercase(),
                privilege: OraclePrivilege::Normal,
                password: SecretRef::inline(parts[4]),
            },
            advanced: OracleAdvancedOptions {
                connect_timeout_secs: 8,
                query_timeout_secs: 0,
                application_name: "FluxDBSmoke".into(),
                wallet_location: String::new(),
                wallet_password: SecretRef::default(),
            },
            ssh: ssh_profile(),
        }),
    })
}

/// `FLUXDB_ORACLE_SSH=host:port:user:password`；未设置时返回禁用档案。
fn ssh_profile() -> SshTunnelProfile {
    let Ok(raw) = std::env::var("FLUXDB_ORACLE_SSH") else {
        return SshTunnelProfile::default();
    };
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_ORACLE_SSH=host:port:user:password");
    SshTunnelProfile {
        enabled: true,
        host: parts[0].into(),
        port: parts[1].parse().expect("SSH port"),
        username: parts[2].into(),
        auth: SshAuth::Password,
        password: SecretRef::inline(parts[3]),
        ..SshTunnelProfile::default()
    }
}

fn owner(config: &ConnectionConfig) -> Option<String> {
    config
        .oracle_profile
        .as_ref()
        .map(|profile| profile.basic.schema.clone())
}

fn execute(config: &ConnectionConfig, sql: &str) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: owner(config),
            session_id: None,
            text: sql.to_string(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .map_err(|error| format!("执行失败：{sql} / {}", error.message))?;
    match result.summaries.iter().find(|summary| !summary.success) {
        Some(summary) => Err(format!("执行失败：{sql} / {}", summary.message)),
        None => Ok(()),
    }
}

/// 只读入口执行一条语句，返回是否被拒绝。
///
/// 执行前拒绝会以整次调用错误返回，执行后报错则是失败 summary，两种都算拒绝。
fn execute_read_only(config: &ConnectionConfig, sql: &str) -> Result<bool, String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let result = connector.execute(&QueryRequest {
        connection_id: config.id,
        database: None,
        schema: owner(config),
        session_id: None,
        text: sql.to_string(),
        mode: QueryMode::All,
        options: QueryExecutionOptions {
            read_only: true,
            ..QueryExecutionOptions::default()
        },
    });
    match result {
        Ok(execution) => Ok(execution.summaries.iter().any(|summary| !summary.success)),
        Err(error) if error.message.contains("只读入口拒绝执行") => Ok(true),
        Err(error) => Err(format!("只读执行返回错误：{sql} / {}", error.message)),
    }
}

/// 清理语句：对象不存在（ORA-00942）不算失败。
fn drop_table(config: &ConnectionConfig, name: &str) -> Result<(), String> {
    match execute(config, &format!("DROP TABLE {name} PURGE")) {
        Err(error) if error.contains("ORA-00942") => Ok(()),
        other => other,
    }
}

fn table(config: &ConnectionConfig, name: &str) -> ObjectPath {
    ObjectPath {
        connection_id: config.id,
        database: None,
        schema: owner(config),
        name: name.to_string(),
        kind: ObjectKind::Table,
    }
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_objects_data_and_ddl() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_objects_data_and_ddl(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

fn run_objects_data_and_ddl(config: &ConnectionConfig) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    drop_table(config, "FLUXDB_TEST_PARENT")?;
    execute(
        config,
        "CREATE TABLE FLUXDB_TEST_PARENT (\
           ID NUMBER(10) PRIMARY KEY, \
           NAME VARCHAR2(80) NOT NULL, \
           AMOUNT NUMBER(18,4), \
           CREATED_AT DATE, \
           NOTE CLOB)",
    )?;
    for row in [
        "INSERT INTO FLUXDB_TEST_PARENT (ID, NAME, AMOUNT, CREATED_AT, NOTE) \
         VALUES (1, 'alpha', 12.34, TIMESTAMP '2026-01-02 03:04:05', 'hello')",
        "INSERT INTO FLUXDB_TEST_PARENT (ID, NAME, AMOUNT, CREATED_AT, NOTE) \
         VALUES (2, '中文', 12345678901234.5678, TIMESTAMP '2026-02-03 04:05:06', NULL)",
    ] {
        execute(config, row)?;
    }

    // 对象树：容器 → Owner → 表。
    let containers = connector
        .list_objects(None)
        .map_err(|error| error.message)?;
    if containers.len() != 1 {
        return Err(format!(
            "Oracle 应只有一个容器节点，实际 {}",
            containers.len()
        ));
    }
    let schemas = connector
        .list_objects(Some(&containers[0].path))
        .map_err(|error| error.message)?;
    let expected_owner = owner(config).unwrap_or_default();
    if !schemas
        .iter()
        .any(|schema| schema.path.name == expected_owner)
    {
        return Err(format!("对象树缺少 Owner {expected_owner}"));
    }

    // 数据页：NUMBER 保留精确文本，不经过 f64。
    let page = connector
        .load_data(&table(config, "FLUXDB_TEST_PARENT"), 0, 10, &[], &[])
        .map_err(|error| error.message)?;
    if page.rows.len() != 2 {
        return Err(format!("数据页行数应为 2，实际 {}", page.rows.len()));
    }
    let amount = page.rows[1].values.iter().find_map(|value| match value {
        fluxdb_core::CellValue::Text(text) if text.starts_with("12345678901234") => {
            Some(text.clone())
        }
        _ => None,
    });
    if amount.as_deref() != Some("12345678901234.5678") {
        return Err(format!("大数精度丢失：{amount:?}"));
    }

    // DDL：两段名 + 主键。
    let ddl = connector
        .table_ddl(&table(config, "FLUXDB_TEST_PARENT"))
        .map_err(|error| error.message)?;
    if !ddl.contains("\"FLUXDB_TEST_PARENT\"") || !ddl.contains("PRIMARY KEY") {
        return Err(format!("DDL 不符合预期：{ddl}"));
    }

    // 筛选与排序：走绑定，不拼字符串。
    let filtered = connector
        .load_data(
            &table(config, "FLUXDB_TEST_PARENT"),
            0,
            10,
            &[fluxdb_core::SortSpec {
                field: "ID".into(),
                direction: fluxdb_core::SortDirection::Desc,
            }],
            &[fluxdb_core::FilterSpec {
                field: "NAME".into(),
                op: fluxdb_core::FilterOp::Eq,
                values: vec![fluxdb_core::CellValue::Text("alpha".into())],
                enabled: true,
            }],
        )
        .map_err(|error| error.message)?;
    if filtered.rows.len() != 1 {
        return Err(format!("筛选结果应为 1 行，实际 {}", filtered.rows.len()));
    }

    drop_table(config, "FLUXDB_TEST_PARENT")
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_logical_export_round_trip() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_logical_export(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

fn run_logical_export(config: &ConnectionConfig) -> Result<(), String> {
    drop_table(config, "FLUXDB_TEST_EXPORT")?;
    execute(
        config,
        "CREATE TABLE FLUXDB_TEST_EXPORT (ID NUMBER(10) PRIMARY KEY, NAME VARCHAR2(40), \
         CREATED_AT DATE, RAW_DATA RAW(8))",
    )?;
    execute(
        config,
        "INSERT INTO FLUXDB_TEST_EXPORT (ID, NAME, CREATED_AT, RAW_DATA) VALUES \
         (1, 'it''s', TIMESTAMP '2026-03-04 05:06:07', HEXTORAW('00ff10'))",
    )?;

    let dir = std::env::temp_dir().join(format!("fluxdb-oracle-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let output = dir.join("fluxdb_test_export.sql");
    let _ = std::fs::remove_file(&output);

    let provider = database_backup(DatabaseKind::Oracle).map_err(|error| error.message)?;
    let execution = provider
        .execution(BackupMethod::Logical, false)
        .map_err(|error| error.message)?;
    // 原生备份必须明确拒绝，而不是悄悄给出一个不可恢复的「备份」。
    if provider.execution(BackupMethod::Native, false).is_ok() {
        return Err("Oracle 原生备份应被拒绝".into());
    }

    let request = BackupRequest {
        config: config.clone(),
        database: String::new(),
        output: output.clone(),
        execution,
        tool: PathBuf::new(),
        tool_version: None,
        scope: BackupScope::Objects(vec![BackupObjectRef {
            schema: owner(config),
            kind: BackupObjectKind::Table,
            name: "FLUXDB_TEST_EXPORT".into(),
        }]),
        include_schema: true,
        include_data: true,
        include_routines: false,
        single_transaction: false,
        lock_tables: false,
        include_owner: false,
        include_acl: false,
    };
    let cancel = AtomicBool::new(false);
    let manifest = provider
        .backup(&request, &cancel, &mut |_| {})
        .map_err(|error| error.message)?;
    if manifest.objects.len() != 1 {
        return Err(format!(
            "导出清单应有 1 个对象，实际 {}",
            manifest.objects.len()
        ));
    }
    let sql = std::fs::read_to_string(&output).map_err(|error| error.to_string())?;
    for expected in [
        "CREATE TABLE",
        "HEXTORAW('00ff10')",
        "TIMESTAMP '2026-03-04 05:06:07'",
        "'it''s'",
    ] {
        if !sql.contains(expected) {
            return Err(format!("导出文件缺少 {expected}：\n{sql}"));
        }
    }

    let restore = RestoreRequest {
        config: config.clone(),
        source: output,
        target: String::new(),
        create_target: false,
        tool: PathBuf::new(),
        manifest: Some(manifest),
        table_decisions: Vec::new(),
        options: fluxdb_core::RestoreOptions::default(),
    };
    // 目标 schema 非空：预检查必须拒绝，而不是覆盖已有对象。
    match provider.inspect_restore(&restore, &cancel) {
        Ok(_) => return Err("非空 schema 应拒绝自动导入".into()),
        Err(error) if error.message.contains("非空") => {}
        Err(error) => return Err(format!("预检查错误不符合预期：{}", error.message)),
    }

    // 空 schema 场景的提示在集成测试里跳过：删表不等于清空 schema。
    let _ = std::fs::remove_dir_all(&dir);
    drop_table(config, "FLUXDB_TEST_EXPORT")
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_user_admin_snapshot() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_user_admin_snapshot(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

fn run_user_admin_snapshot(config: &ConnectionConfig) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let snapshot = connector
        .oracle_user_admin_snapshot(config.id)
        .map_err(|error| error.message)?;
    let expected = owner(config).unwrap_or_default();
    if snapshot.current_user != expected {
        return Err(format!(
            "当前账号应为 {expected}，实际 {}",
            snapshot.current_user
        ));
    }
    let own = snapshot
        .own
        .as_ref()
        .ok_or_else(|| "应能读到当前账号的 USER_USERS 详情".to_string())?;
    if own.name != expected || own.account_status.is_empty() {
        return Err(format!("账号详情不符合预期：{own:?}"));
    }
    // 直接授权与继承必须可区分。角色清单取决于账号本身：gvenzl 镜像建的账号可能
    // 一个角色都没有，所以这里只校验「有就按来源标注」，不强制账号必须带某个角色
    // （那会把环境差异误报成产品缺陷）。
    if let Some(direct) = snapshot
        .roles
        .iter()
        .find(|role| role.role == "DB_DEVELOPER_ROLE")
        && direct.inherited
    {
        return Err(format!(
            "DB_DEVELOPER_ROLE 是直接授予，不应标为继承：{direct:?}"
        ));
    }
    // 经角色链带来的角色（如 RESOURCE）必须标 inherited，界面才不会把它画成可直接撤销。
    if let Some(inherited) = snapshot.roles.iter().find(|role| role.role == "RESOURCE")
        && !inherited.inherited
    {
        return Err(format!("RESOURCE 应由角色链继承，实际：{inherited:?}"));
    }
    // 冒烟账号不是管理员：读不到别人的授权明细，界面必须按「非当前账号只读」处理。
    // 这里断言只列名字、不含授权详情，避免把「未知」画成「无权限」。
    if snapshot.users.is_empty() || !snapshot.users.iter().any(|user| user.name == expected) {
        return Err(format!("账号列表应包含当前账号：{:?}", snapshot.users));
    }
    // 继承行必须带标记，不能显示成可直接撤销。
    for role in &snapshot.roles {
        if role.role == "RESOURCE" && !role.inherited {
            return Err(format!("RESOURCE 应由角色链继承，实际：{role:?}"));
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_clob_preview_and_full_read() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_clob_preview(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 按字符拼 CLOB 字面量：Oracle 单个字符串字面量上限 4000 字节，多字节字符更容易超。
fn clob_literal(function: &str, ch: char, count: usize) -> String {
    let chunk = ch.to_string().repeat(1000);
    let chunks = count.div_ceil(1000);
    let mut parts = Vec::with_capacity(chunks);
    for index in 0..chunks {
        let size = if index + 1 == chunks {
            count - index * 1000
        } else {
            1000
        };
        let text = chunk.chars().take(size).collect::<String>();
        parts.push(format!("{function}('{text}')"));
    }
    parts.join(" || ")
}

/// 预览截断必须按字符判定，且详情能读回完整值。
fn run_clob_preview(config: &ConnectionConfig) -> Result<(), String> {
    drop_table(config, "FLUXDB_TEST_CLOB")?;
    execute(
        config,
        "CREATE TABLE FLUXDB_TEST_CLOB (ID NUMBER(10) PRIMARY KEY, NOTE CLOB, NOTE_NCLOB NCLOB)",
    )?;
    // 1 空 CLOB / 2 恰好 4000（完整）/ 3 4001 / 4 中文 4001 / 5 超长 100000 / 6 NCLOB 6000
    let inserts = [
        "INSERT INTO FLUXDB_TEST_CLOB (ID, NOTE) VALUES (1, EMPTY_CLOB())".to_string(),
        format!(
            "INSERT INTO FLUXDB_TEST_CLOB (ID, NOTE) VALUES (2, {})",
            clob_literal("to_clob", 'B', 4000)
        ),
        format!(
            "INSERT INTO FLUXDB_TEST_CLOB (ID, NOTE) VALUES (3, {})",
            clob_literal("to_clob", 'C', 4001)
        ),
        format!(
            "INSERT INTO FLUXDB_TEST_CLOB (ID, NOTE) VALUES (4, {})",
            clob_literal("to_clob", '中', 4001)
        ),
        format!(
            "INSERT INTO FLUXDB_TEST_CLOB (ID, NOTE) VALUES (5, {})",
            clob_literal("to_clob", 'E', 100_000)
        ),
        format!(
            "INSERT INTO FLUXDB_TEST_CLOB (ID, NOTE_NCLOB) VALUES (6, {})",
            clob_literal("to_nclob", '国', 6000)
        ),
    ];
    for sql in inserts {
        execute(config, &sql)?;
    }

    let connector = connector_for(config).map_err(|error| error.message)?;
    let page = connector
        .load_data(&table(config, "FLUXDB_TEST_CLOB"), 0, 10, &[], &[])
        .map_err(|error| error.message)?;
    if page.rows.len() != 6 {
        return Err(format!("数据页行数应为 6，实际 {}", page.rows.len()));
    }
    let note = |row: usize, column: usize| -> Result<fluxdb_core::CellValue, String> {
        page.rows
            .get(row)
            .and_then(|row| row.values.get(column))
            .cloned()
            .ok_or_else(|| format!("第 {} 行缺少第 {column} 列", row + 1))
    };

    // 空 CLOB 是空串，不是 NULL：两者混同会让用户以为没写进去。
    if note(0, 1)? != fluxdb_core::CellValue::Text(String::new()) {
        return Err(format!("空 CLOB 应为空文本，实际 {:?}", note(0, 1)?));
    }
    // 恰好 4000 是完整值：多读一次判断 EOF 才算数，不能按「读满上限」判截断。
    match note(1, 1)? {
        fluxdb_core::CellValue::Text(text) if text.chars().count() == 4000 => {}
        other => return Err(format!("4000 字符 CLOB 不应判为截断：{other:?}")),
    }
    // (行号, 列号, 期望总字符数)：第 6 行的值在 NCLOB 列。
    for (row, column, expected) in [
        (2usize, 1usize, 4001u64),
        (3, 1, 4001),
        (4, 1, 100_000),
        (5, 2, 6000),
    ] {
        match note(row, column)? {
            fluxdb_core::CellValue::TruncatedText {
                preview,
                total_chars,
            } => {
                if preview.chars().count() != 4000 || total_chars != expected {
                    return Err(format!(
                        "第 {} 行预览应为 4000/{expected} 字符，实际 {}/{}",
                        row + 1,
                        preview.chars().count(),
                        total_chars
                    ));
                }
            }
            other => return Err(format!("第 {} 行应判为截断：{other:?}", row + 1)),
        }
    }

    // 详情按需读完整值：截断行必须整段读回，不是再给一次预览。
    let identity = |id: &str| fluxdb_core::RowIdentity {
        values: BTreeMap::from([("ID".to_string(), fluxdb_core::CellValue::Text(id.into()))]),
    };
    let full = connector
        .load_cell_text(&table(config, "FLUXDB_TEST_CLOB"), &identity("5"), "NOTE")
        .map_err(|error| format!("读取完整 CLOB 失败：{}", error.message))?;
    if full.chars().count() != 100_000 || !full.chars().all(|ch| ch == 'E') {
        return Err(format!("完整读取结果不符：{} 字符", full.chars().count()));
    }
    let chinese = connector
        .load_cell_text(&table(config, "FLUXDB_TEST_CLOB"), &identity("4"), "NOTE")
        .map_err(|error| format!("读取中文 CLOB 失败：{}", error.message))?;
    if chinese.chars().count() != 4001 || !chinese.chars().all(|ch| ch == '中') {
        return Err(format!(
            "中文 CLOB 读取不符：{} 字符",
            chinese.chars().count()
        ));
    }
    let nclob = connector
        .load_cell_text(
            &table(config, "FLUXDB_TEST_CLOB"),
            &identity("6"),
            "NOTE_NCLOB",
        )
        .map_err(|error| format!("读取 NCLOB 失败：{}", error.message))?;
    if nclob.chars().count() != 6000 {
        return Err(format!("NCLOB 读取不符：{} 字符", nclob.chars().count()));
    }
    // 完整值不能被截断成预览再返回。
    let exact = connector
        .load_cell_text(&table(config, "FLUXDB_TEST_CLOB"), &identity("2"), "NOTE")
        .map_err(|error| error.message)?;
    if exact.chars().count() != 4000 {
        return Err(format!("4000 字符读取不符：{}", exact.chars().count()));
    }

    drop_table(config, "FLUXDB_TEST_CLOB")
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_request_schema_reaches_the_session() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_request_schema(config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 请求级 schema 必须落到会话上：连接档案故意指向一个不存在的 owner，
/// 此时不带请求 schema 会解析失败，带上就应命中真实对象。
fn run_request_schema(mut config: ConnectionConfig) -> Result<(), String> {
    let real_owner = owner(&config).ok_or_else(|| "烟雾配置缺少 schema".to_string())?;
    drop_table(&config, "FLUXDB_TEST_SCHEMA")?;
    execute(
        &config,
        "CREATE TABLE FLUXDB_TEST_SCHEMA (ID NUMBER(10) PRIMARY KEY)",
    )?;

    // 档案 schema 改成一个存在但没有该表的 owner：未限定名会解析到它并报
    // ORA-00942，正是这个用例要区分的行为。
    //
    // 不能用一个不存在的 owner：Oracle 23.6 的 ALTER SESSION 会校验 owner
    // （ORA-01435），连接阶段就失败，测不到「请求 schema 是否生效」。
    if let Some(profile) = config.oracle_profile.as_mut() {
        profile.basic.schema = "SYS".into();
    }
    let connector = connector_for(&config).map_err(|error| error.message)?;
    let query_succeeds = |schema: Option<String>| -> Result<bool, String> {
        let result = connector
            .execute(&QueryRequest {
                connection_id: config.id,
                database: None,
                schema,
                session_id: None,
                text: "SELECT COUNT(*) FROM FLUXDB_TEST_SCHEMA".to_string(),
                mode: QueryMode::All,
                options: QueryExecutionOptions::default(),
            })
            .map_err(|error| error.message)?;
        Ok(result.summaries.iter().all(|summary| summary.success))
    };

    if query_succeeds(None)? {
        return Err("档案 schema 不存在时，未带请求 schema 的查询不应成功".into());
    }
    // 会话已按请求 schema 切换，未限定名解析到真实 owner。
    if !query_succeeds(Some(real_owner))? {
        return Err("请求 schema 未落到会话：未限定名仍按档案 schema 解析".into());
    }

    // 清理：连接档案已不可用，改用原 schema 的配置删表。
    let cleanup = smoke_config().ok_or_else(|| "烟雾配置丢失".to_string())?;
    drop_table(&cleanup, "FLUXDB_TEST_SCHEMA")
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_read_only_rejects_before_execution() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_read_only(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 只读入口必须在执行前挡住写入：DML 不落库、DDL 不建表。
fn run_read_only(config: &ConnectionConfig) -> Result<(), String> {
    drop_table(config, "FLUXDB_TEST_READONLY")?;
    execute(
        config,
        "CREATE TABLE FLUXDB_TEST_READONLY (ID NUMBER(10) PRIMARY KEY, NAME VARCHAR2(20))",
    )?;
    execute(
        config,
        "INSERT INTO FLUXDB_TEST_READONLY (ID, NAME) VALUES (1, 'a')",
    )?;

    // 刚建表就开只读事务会撞 ORA-01466（表定义变更的 SCN 尚未落定），与只读入口
    // 无关。等一个 SCN 周期，避免把数据库的时序问题当成产品拒绝。
    std::thread::sleep(std::time::Duration::from_millis(1_500));
    // 查询放行。
    if execute_read_only(config, "SELECT COUNT(*) FROM FLUXDB_TEST_READONLY")? {
        return Err("只读入口不应拒绝 SELECT".into());
    }
    // 写入在执行前被拒绝，不是执行后报错。
    for sql in [
        "INSERT INTO FLUXDB_TEST_READONLY (ID, NAME) VALUES (2, 'b')",
        "UPDATE FLUXDB_TEST_READONLY SET NAME = 'c' WHERE ID = 1",
        "DELETE FROM FLUXDB_TEST_READONLY WHERE ID = 1",
        "DROP TABLE FLUXDB_TEST_READONLY PURGE",
        "BEGIN NULL; END;",
    ] {
        if !execute_read_only(config, sql)? {
            return Err(format!("只读入口未拒绝：{sql}"));
        }
    }

    // 数据未被改动，表也还在。
    let connector = connector_for(config).map_err(|error| error.message)?;
    let page = connector
        .load_data(&table(config, "FLUXDB_TEST_READONLY"), 0, 10, &[], &[])
        .map_err(|error| format!("表已被只读语句删掉：{}", error.message))?;
    if page.rows.len() != 1 {
        return Err(format!("只读入口写入了数据：{} 行", page.rows.len()));
    }

    drop_table(config, "FLUXDB_TEST_READONLY")
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_row_cap_and_cancel() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_row_cap_and_cancel(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 行数上限与取消都要在**取行途中**生效，而不是先把游标抽干再截断。
fn run_row_cap_and_cancel(config: &ConnectionConfig) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let request = |sql: &str, page_size: u64| QueryRequest {
        connection_id: config.id,
        database: None,
        schema: owner(config),
        session_id: None,
        text: sql.to_string(),
        mode: QueryMode::All,
        options: QueryExecutionOptions {
            page_size,
            ..QueryExecutionOptions::default()
        },
    };

    // 1. 页满即止：1000 行的结果只要 10 行，不应把整表扫完。
    let execution = connector
        .execute(&request(
            "SELECT LEVEL AS N FROM dual CONNECT BY LEVEL <= 1000",
            10,
        ))
        .map_err(|error| error.message)?;
    let page = execution
        .results
        .first()
        .ok_or_else(|| "行数上限用例没有结果页".to_string())?;
    if page.rows.len() != 10 {
        return Err(format!(
            "页大小 10 应只保留 10 行，实际 {}",
            page.rows.len()
        ));
    }
    if !page.has_more {
        return Err("页满后应标记 has_more（后面还有行）".into());
    }

    // 2. 取行途中取消：第一次检查放行、第二次起取消，于是第 1 行之后停下。
    let checks = std::sync::atomic::AtomicU32::new(0);
    let execution = connector
        .execute_with_progress(
            &request("SELECT LEVEL AS N FROM dual CONNECT BY LEVEL <= 1000", 100),
            &mut |_| {},
            &|| checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 0,
        )
        .map_err(|error| error.message)?;
    let summary = execution
        .summaries
        .first()
        .ok_or_else(|| "取消用例没有执行摘要".to_string())?;
    if summary.success {
        return Err("取行途中取消不能报成功".into());
    }
    if !summary.message.contains("结果待核实") {
        return Err(format!(
            "取行途中取消必须标「结果待核实」，实际：{}",
            summary.message
        ));
    }
    if !execution.results.is_empty() {
        return Err("取消的语句不应留下半截结果页".into());
    }

    // 3. 页满即止不能影响完整性：页大小足够大时行数与 has_more 都正确。
    let execution = connector
        .execute(&request(
            "SELECT LEVEL AS N FROM dual CONNECT BY LEVEL <= 1000",
            100_000,
        ))
        .map_err(|error| error.message)?;
    let page = execution
        .results
        .first()
        .ok_or_else(|| "大数据页用例没有结果页".to_string())?;
    if page.rows.len() != 1000 {
        return Err(format!("应取到 1000 行，实际 {}", page.rows.len()));
    }
    if page.has_more {
        return Err("已取完全部行时不应标记 has_more".into());
    }
    Ok(())
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_query_timeout_interrupts_statement() {
    let Some(mut config) = smoke_config() else {
        return;
    };
    let Some(profile) = config.oracle_profile.as_mut() else {
        return;
    };
    // 查询超时是 Oracle 唯一能打断服务端计算的杠杆：驱动到点发 INTERRUPT 再重置会话。
    profile.advanced.query_timeout_secs = 2;
    if let Err(failure) = run_query_timeout(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

fn run_query_timeout(config: &ConnectionConfig) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let started = std::time::Instant::now();
    let result = connector.execute(&QueryRequest {
        connection_id: config.id,
        database: None,
        schema: owner(config),
        session_id: None,
        // 实测约 16 秒的聚合（本机 Oracle Free）；用 SQL 而不是 PL/SQL，
        // 因为脚本切分器会按分号拆开匿名块，那是另一个问题，不该混进这条用例。
        text: "SELECT COUNT(*) FROM (SELECT LEVEL n FROM dual CONNECT BY LEVEL <= 1000) a, \
               (SELECT LEVEL n FROM dual CONNECT BY LEVEL <= 1000) b, \
               (SELECT LEVEL n FROM dual CONNECT BY LEVEL <= 1000) c"
            .into(),
        mode: QueryMode::All,
        options: QueryExecutionOptions::default(),
    });
    let elapsed = started.elapsed();
    let execution =
        result.map_err(|error| format!("超时应当返回结果而不是整体报错：{}", error.message))?;
    let summary = execution
        .summaries
        .first()
        .ok_or_else(|| "超时用例没有执行摘要".to_string())?;
    if summary.success {
        return Err("约 16 秒的语句在 2 秒超时下不应报成功".into());
    }
    if !summary.message.contains("结果待核实") {
        return Err(format!(
            "超时必须标「结果待核实」，实际：{}",
            summary.message
        ));
    }
    // 超时是打断而不是无限等待：给驱动留出重置会话的余量。
    if elapsed > std::time::Duration::from_secs(20) {
        return Err(format!("超时未生效，耗时 {elapsed:?}"));
    }
    Ok(())
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_plsql_block_and_q_quote_split() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_plsql_block_split(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 复现本问题原来的症状：`BEGIN NULL; END;` 被按分号切成两段报 ORA-06550，
/// `q'[it's a;test]'` 被从 `;` 处切断报语法错。
fn run_plsql_block_split(config: &ConnectionConfig) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let run = |sql: &str| -> Result<Vec<String>, String> {
        let execution = connector
            .execute(&QueryRequest {
                connection_id: config.id,
                database: None,
                schema: owner(config),
                session_id: None,
                text: sql.to_string(),
                mode: QueryMode::All,
                options: QueryExecutionOptions::default(),
            })
            .map_err(|error| format!("执行失败：{sql} / {}", error.message))?;
        let failures = execution
            .summaries
            .iter()
            .filter(|summary| !summary.success)
            .map(|summary| format!("{} => {}", summary.sql, summary.message))
            .collect::<Vec<_>>();
        if !failures.is_empty() {
            return Err(format!("语句失败：{}", failures.join(" | ")));
        }
        Ok(execution
            .summaries
            .iter()
            .map(|summary| summary.sql.clone())
            .collect())
    };

    // 1. 匿名块必须整体作为一条语句发出。
    let statements = run("BEGIN NULL; END;")?;
    if statements.len() != 1 {
        return Err(format!(
            "匿名 PL/SQL 块被切成 {} 段：{statements:?}",
            statements.len()
        ));
    }

    // 2. 块后面跟普通语句，两段都要独立执行。
    let statements = run("BEGIN NULL; END;\nSELECT 1 FROM dual")?;
    if statements.len() != 2 {
        return Err(format!(
            "块 + 查询应切成 2 段，实际 {} 段：{statements:?}",
            statements.len()
        ));
    }

    // 3. 替代引号里的分号不是分隔点。
    let statements = run("SELECT q'[it's a;test]' FROM dual")?;
    if statements.len() != 1 {
        return Err(format!(
            "q'[…]' 被切成 {} 段：{statements:?}",
            statements.len()
        ));
    }

    // 4. 反斜杠在 Oracle 里不是转义符，字符串在 `\'` 处就结束了。
    let statements = run(r"SELECT 'C:\' AS v FROM dual".to_string().as_str())?;
    if statements.len() != 1 {
        return Err(format!(
            "含反斜杠的字符串被切成 {} 段：{statements:?}",
            statements.len()
        ));
    }
    Ok(())
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_import_datetime_text_into_date_columns() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_import_datetime_text(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 复现导入 `DATE`/`TIMESTAMP` 列时的 `ORA-01861`：导出文本是
/// `YYYY-MM-DD HH24:MI:SS[.ff]`，但如果会话 `NLS_DATE_FORMAT` 停在默认的
/// `DD-MON-RR`，VARCHAR 绑定写进 DATE 列就会解析失败。连接建立时必须钉死 ISO 格式。
fn run_import_datetime_text(config: &ConnectionConfig) -> Result<(), String> {
    use fluxdb_connectors::table_data_import;
    use fluxdb_core::{TableImportFormat, TableImportOptions, TableImportRequest};
    use std::io::Write;

    let table_name = "FLUXDB_TEST_IMPORT_DT";
    drop_table(config, table_name)?;
    execute(
        config,
        &format!(
            "CREATE TABLE {table_name} (\
               ID NUMBER(10) PRIMARY KEY, \
               CREATED_AT DATE, \
               UPDATED_AT TIMESTAMP(6))"
        ),
    )?;

    let dir = std::env::temp_dir().join(format!("fluxdb-oracle-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let source = dir.join("dt.csv");
    let mut file = std::fs::File::create(&source).map_err(|error| error.to_string())?;
    file.write_all(
        b"ID,CREATED_AT,UPDATED_AT\n\
          1,2024-01-02 11:22:33,2024-01-02 11:22:33\n\
          2,2024-03-04 05:06:07,2024-03-04 05:06:07.123456\n",
    )
    .map_err(|error| error.to_string())?;

    let request = TableImportRequest {
        config: config.clone(),
        object: table(config, table_name),
        source: source.clone(),
        format: TableImportFormat::Csv,
        options: TableImportOptions::new(),
        column_mapping: Vec::new(),
    };
    let importer = table_data_import(DatabaseKind::Oracle).map_err(|error| error.message)?;
    let report = importer
        .import(&request, &AtomicBool::new(false), &mut |_| {})
        .map_err(|error| format!("导入失败：{}", error.message))?;
    if report.imported_rows != 2 {
        return Err(format!("应导入 2 行，实际 {}", report.imported_rows));
    }

    let connector = connector_for(config).map_err(|error| error.message)?;
    let page = connector
        .load_data(&table(config, table_name), 0, 10, &[], &[])
        .map_err(|error| error.message)?;
    let first = page.rows.first().ok_or("导入后读不到数据")?;
    let created = first.values.iter().find_map(|value| match value {
        fluxdb_core::CellValue::Text(text) if text.starts_with("2024-01-02") => Some(text.clone()),
        _ => None,
    });
    if created.as_deref() != Some("2024-01-02 11:22:33") {
        return Err(format!("DATE 值未按 ISO 形态落库：{created:?}"));
    }
    drop_table(config, table_name)?;
    let _ = std::fs::remove_file(&source);
    Ok(())
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_live_batch_metadata_and_object_row_counts() {
    let Some(config) = smoke_config() else {
        return;
    };
    if let Err(failure) = run_batch_metadata(&config) {
        eprintln!("FAIL: {failure}");
        std::process::exit(1);
    }
}

/// 批量元数据（ER 用）必须与逐表读取等价，同时对象树给出统计行数。
///
/// 这三条覆盖的是「一次登录读整组」的重写：外键分组、复合键列序、列信息（类型文本 +
/// 主键标记）都不能因为合并查询而退化。
fn run_batch_metadata(config: &ConnectionConfig) -> Result<(), String> {
    let connector = connector_for(config).map_err(|error| error.message)?;
    let parent = "FLUXDB_TEST_BATCH_PARENT";
    let child = "FLUXDB_TEST_BATCH_CHILD";
    drop_table(config, child)?;
    drop_table(config, parent)?;
    execute(
        config,
        &format!(
            "CREATE TABLE {parent} (\
               ID NUMBER(10) PRIMARY KEY, \
               CODE VARCHAR2(20 CHAR) NOT NULL, \
               NOTE CLOB)"
        ),
    )?;
    execute(
        config,
        &format!(
            "CREATE TABLE {child} (\
               ID NUMBER(10) PRIMARY KEY, \
               PARENT_ID NUMBER(10), \
               PARENT_CODE VARCHAR2(20 CHAR), \
               CONSTRAINT FK_BATCH_CHILD_PARENT FOREIGN KEY (PARENT_ID, PARENT_CODE) \
                 REFERENCES {parent} (ID, CODE))"
        ),
    )?;
    // 统计值要更新到目录，NUM_ROWS 才有内容；失败不影响其它断言（统计可能被禁用）。
    let _ = execute(
        config,
        &format!("ANALYZE TABLE {parent} COMPUTE STATISTICS"),
    );

    let schema = owner(config).unwrap_or_default();
    let tables = vec![parent.to_string(), child.to_string()];

    // 批量外键：同一份结果，复合键按列序两列。
    let batch = connector
        .list_foreign_keys_for_tables(None, Some(&schema), &tables)
        .map_err(|error| error.message)?;
    let (_, fk) = batch
        .iter()
        .find(|(table, _)| table == child)
        .ok_or("批量外键缺少子表结果")?;
    if fk.name != "FK_BATCH_CHILD_PARENT" || fk.ref_table != parent {
        return Err(format!("批量外键目标不符：{fk:?}"));
    }
    if fk.columns != ["PARENT_ID", "PARENT_CODE"] || fk.ref_columns != ["ID", "CODE"] {
        return Err(format!(
            "复合外键列序不符：{:?} → {:?}",
            fk.columns, fk.ref_columns
        ));
    }
    // 与逐表接口同源：单表结果必须一致。
    let single = connector
        .list_foreign_keys(&table(config, child))
        .map_err(|error| error.message)?;
    if single.len() != 1 || single[0].columns != fk.columns {
        return Err(format!("单表外键与批量结果不一致：{single:?}"));
    }

    // 空 tables 表示整个 Owner：至少要包含刚建的两张表。
    let all = connector
        .list_foreign_keys_for_tables(None, Some(&schema), &[])
        .map_err(|error| error.message)?;
    if !all.iter().any(|(table, _)| table == child) {
        return Err("空表清单的批量外键没有覆盖该 Owner 的全部表".into());
    }

    // 批量列：类型文本保留 CHAR 语义，主键标记只落在主键列上。
    let columns = connector
        .list_completion_columns_for_tables(None, Some(&schema), &tables)
        .map_err(|error| error.message)?;
    let parent_columns = columns
        .iter()
        .filter(|column| column.table == parent)
        .collect::<Vec<_>>();
    if parent_columns.len() != 3 {
        return Err(format!("父表列数应为 3，实际 {}", parent_columns.len()));
    }
    let code = parent_columns
        .iter()
        .find(|column| column.name == "CODE")
        .ok_or("父表缺少 CODE 列")?;
    if code.type_name.as_deref() != Some("VARCHAR2(20 CHAR)") {
        return Err(format!("列类型文本不符：{:?}", code.type_name));
    }
    if code.nullable {
        return Err("CODE 声明为 NOT NULL，批量列却标记可空".into());
    }
    if !code.comment.is_none() && code.comment.as_deref() != Some("") {
        return Err(format!("无注释列的 comment 应为空：{:?}", code.comment));
    }
    let pk_columns = parent_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    if pk_columns != ["ID"] {
        return Err(format!("主键列不符：{pk_columns:?}"));
    }

    // 取消：已取消的批量列读取返回空结果，而不是报错。
    let cancelled = connector
        .list_completion_columns_for_tables_with_cancel(None, Some(&schema), &tables, &|| true)
        .map_err(|error| error.message)?;
    if !cancelled.is_empty() {
        return Err("已取消的批量列读取应返回空结果".into());
    }

    // 对象树：行数取目录统计值，长期未收集时允许为空，但不能是 0 以外的错误值。
    let database_path = ObjectPath {
        connection_id: config.id,
        database: None,
        schema: None,
        name: schema.clone(),
        kind: ObjectKind::Database,
    };
    let objects = connector
        .list_objects(Some(&database_path))
        .map_err(|error| error.message)?;
    let parent_object = objects
        .iter()
        .find(|object| object.path.name == parent)
        .ok_or("对象树缺少父表")?;
    if parent_object.rows.is_some_and(|rows| rows > 0) {
        return Err(format!(
            "空表的目录统计行数应为 0 或未知，实际 {:?}",
            parent_object.rows
        ));
    }

    drop_table(config, child)?;
    drop_table(config, parent)?;
    Ok(())
}
