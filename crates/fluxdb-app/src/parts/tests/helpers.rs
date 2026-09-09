    fn controller_with_data_editor() -> AppController {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let object = controller.state().connections[0].objects[1].path.clone();
        controller.dispatch(AppCommand::OpenDataEditor(object));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));
        controller
    }

    fn active_editor(controller: &AppController) -> &DataEditorState {
        let Some(TabKind::DataEditor(editor)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active data editor tab");
        };
        editor
    }

    fn temp_sqlite_path(label: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "fluxdb-app-sqlite-{label}-{}-{suffix}.db",
            std::process::id()
        ))
    }

    /// 真实库测试白名单结果（`parse_dev_target` 的判定枚举）。
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum RealDbTarget {
        /// 显式配置的开发环境 MySQL（10.10.1.158）。
        DevMysql,
    }

    /// 真实库测试安全白名单校验（安全边界，勿放宽）。
    ///
    /// 只允许通过显式测试配置（`GDB_TEST_DEV_DSN` 环境变量）指定的开发环境
    /// `10.10.1.158:3306`；SQLite 仅在仓库内临时/固定测试库场景由 `temp_sqlite_path`
    /// 直接构造连接，不经此函数。明确拒绝 TiDB / 生产库 / 未知主机/端口；未配置或
    /// 非法时返回 `None`，由调用方**跳过**测试，绝不自动扫描或复用任意连接。
    ///
    /// 开发库 DSN 白名单解析（纯函数，便于单测；不触网）。
    fn parse_dev_target(dsn: &str) -> Option<RealDbTarget> {
        let dsn = dsn.trim();
        if dsn.is_empty() {
            return None;
        }
        // 统一小写去协议前缀（mysql:// / mysql+pymysql:// 等），便于白名单判断。
        let lower = dsn.to_ascii_lowercase();
        let lower = lower
            .strip_prefix("mysql+pymysql://")
            .or_else(|| lower.strip_prefix("mysql://"))
            .unwrap_or(&lower);
        // 主机段：取 @ 之后到第一个 : 或 / 之间的部分。
        let host_segment = lower.split('@').last().unwrap_or(lower);
        let host = host_segment
            .split(|c| c == ':' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        // 端口显式非 3306 的一律拒绝（开发环境 MySQL 默认端口；未知端口不可信）。
        let port = host_segment
            .split(':')
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or("3306");
        // TiDB / 生产库 / 未知主机一律拒绝。
        if lower.contains("tidb") || lower.contains("prod") || lower.contains("production") {
            return None;
        }
        if host == "10.10.1.158" && port == "3306" {
            Some(RealDbTarget::DevMysql)
        } else {
            None
        }
    }

    /// 解析后的开发库连接信息（白名单校验通过后才填充，供真实连接测试复用）。
    /// 含密码敏感字段，只用于在测试内构建 `ConnectionConfig`，绝不打印/落库。
    #[derive(Clone, Debug)]
    struct DevDsnParts {
        host: String,
        port: u16,
        database: Option<String>,
        username: String,
        password: String,
    }

    /// 从 `GDB_TEST_DEV_DSN` 解析并通过白名单校验后返回开发库连接信息（纯函数，不触网）。
    /// 与 `parse_dev_target` 同白名单：仅 `10.10.1.158:3306`，拒绝 TiDB/prod/未知主机。
    /// DSN 形如 `mysql://user:password@10.10.1.158:3306/dbname`（password 可省略）。
    fn parse_dev_dsn_parts(dsn: &str) -> Option<DevDsnParts> {
        if parse_dev_target(dsn) != Some(RealDbTarget::DevMysql) {
            return None;
        }
        let dsn = dsn.trim();
        let mid = dsn
            .to_ascii_lowercase()
            .find('@')?;
        // 凭证段.user/pass：位于 @ 之前。
        let cred = dsn[..mid].rsplit("://").next().unwrap_or(&dsn[..mid]);
        let (username, password) = match cred.split_once(':') {
            Some((u, p)) => (u.to_string(), p.to_string()),
            None => (cred.to_string(), String::new()),
        };
        // 主机段：@ 之后到首个 / 前的 host:port；/ 后为 database。
        let rest = &dsn[mid + 1..];
        let (host_port, db) = match rest.split_once('/') {
            Some((hp, db)) => (hp, Some(db.trim().trim_end_matches('/').to_string())),
            None => (rest, None),
        };
        let (host, port) = match host_port.split_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(0)),
            None => (host_port.to_string(), 3306),
        };
        let db = db.filter(|d| !d.is_empty());
        Some(DevDsnParts { host, port, database: db, username, password })
    }

    /// 由解析好的开发库信息构造成真实 MySqlConnector 入口配置（非 demo）。
    fn dev_mysql_config_from_parts(parts: DevDsnParts) -> ConnectionConfig {
        let mut options = BTreeMap::new();
        options.insert("username".to_string(), parts.username);
        if !parts.password.is_empty() {
            options.insert("password".to_string(), parts.password);
        }
        ConnectionConfig {
            id: ConnectionId(701),
            name: "Dev MySQL (10.10.1.158, real)".to_string(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: parts.host,
                port: parts.port,
                database: parts.database.clone(),
            },
            credential_ref: None,
            options, // 非 demo：携带明文字段走真实 MySqlConnector（与 app 一致）
            redis_profile: None,
            mysql_profile: None,
        }
    }

    /// 安全检查通过时返回开发库 `ConnectionConfig`（真实 MySqlConnector 入口），
    /// 否则返回 `None`（未配置/非法，调用方跳过，绝不自动连接）。
    fn guarded_dev_mysql_config() -> Option<ConnectionConfig> {
        let dsn = std::env::var("GDB_TEST_DEV_DSN").ok()?;
        let parts = parse_dev_dsn_parts(&dsn)?;
        Some(dev_mysql_config_from_parts(parts))
    }

