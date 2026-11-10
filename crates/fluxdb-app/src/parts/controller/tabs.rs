impl AppController {
    fn next_tab_id(&mut self) -> TabId {
        let tab_id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        tab_id
    }

    fn push_tab(&mut self, tab: TabState) {
        self.state.active_tab = Some(tab.id);
        self.state.tabs.push(tab);
    }

    /// 按终端会话复用 key 查找已打开的 tag 页（统一复用入口，不做 Redis 局部判断）。
    ///
    /// key 的比较统一在此完成；后续 MySQL / SSH 只需实现各自的 `session_key()`，
    /// 无需在 UI 或打开逻辑里新增临时代码。
    fn find_tab_by_session_key(&self, key: &fluxdb_core::terminal::TerminalSessionKey) -> Option<TabId> {
        self.state.tabs.iter().find_map(|tab| match &tab.kind {
            TabKind::RedisCli(cli) if cli.session_key() == *key => Some(tab.id),
            TabKind::MongoCli(cli) if cli.session_key() == *key => Some(tab.id),
            _ => None,
        })
    }

    fn open_query_editor(
        &mut self,
        connection_id: ConnectionId,
        database: Option<String>,
        schema: Option<String>,
    ) -> AppEvent {
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: "无标题 - 查询".to_string(),
            kind: TabKind::QueryEditor(QueryEditorState {
                connection_id,
                database,
                schema,
                text: String::new(),
                origin: None,
                saved_fingerprint: None,
                running: false,
                results: Vec::new(),
                result_editors: BTreeMap::new(),
                active_result_editor: None,
                summaries: Vec::new(),
                error: None,
            }),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn open_redis_workbench(&mut self, connection_id: ConnectionId, database: u32) -> AppEvent {
        // 同一连接 + 同一库已打开 Workbench 时复用已有标签页。
        if let Some(tab) = self.state.tabs.iter().find(|tab| {
            matches!(
                &tab.kind,
                TabKind::RedisWorkbench(workbench)
                    if workbench.connection_id == connection_id && workbench.database == database
            )
        }) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }

        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: format!("无标题 - Redis 命令 DB {database}"),
            kind: TabKind::RedisWorkbench(RedisWorkbenchState {
                connection_id,
                database,
                text: String::new(),
                running: false,
                executions: Vec::new(),
                error: None,
                saved_fingerprint: None,
                next_execution_id: 1,
                collapsed: std::collections::BTreeSet::new(),
                json_views: std::collections::BTreeSet::new(),
            }),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// 打开 Redis CLI 终端标签页。同一连接 + 同一库已打开时复用已有标签页（不新开）。
    /// 与 `open_redis_workbench` 并行：Workbench 走输入框，CLI 走真实终端。
    /// 复用判断统一走 `find_tab_by_session_key`（基于 `RedisCliState::session_key`）。
    fn open_redis_cli(&mut self, connection_id: ConnectionId, database: u32) -> AppEvent {
        let cli = RedisCliState { connection_id, database };
        if let Some(tab_id) = self.find_tab_by_session_key(&cli.session_key()) {
            self.state.active_tab = Some(tab_id);
            return AppEvent::TabActivated(tab_id);
        }

        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: format!("Redis CLI - DB {database}"),
            kind: TabKind::RedisCli(cli),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// 打开 Redis Pub/Sub 会话标签页（订阅指定数据库的实时消息流）。
    /// 同一连接 + 同一库已打开时复用已有标签页（不新开）。
    /// 会话连接本身由桌面端 `PubSubSessionModel` 按 tab 建立 / 销毁，此处仅完成 tab 定位。
    fn open_redis_pubsub(&mut self, connection_id: ConnectionId, database: u32) -> AppEvent {
        // 复用判断：同一连接 + 同一库的 Pub/Sub 标签页已经存在则激活，而非新开。
        if let Some(tab_id) = self.state.tabs.iter().find_map(|tab| match &tab.kind {
            TabKind::RedisPubSub(pubsub)
                if pubsub.connection_id == connection_id && pubsub.database == database =>
            {
                Some(tab.id)
            }
            _ => None,
        }) {
            self.state.active_tab = Some(tab_id);
            return AppEvent::TabActivated(tab_id);
        }

        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: format!("Redis Pub/Sub - DB {database}"),
            kind: TabKind::RedisPubSub(RedisPubSubState {
                connection_id,
                database,
            }),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn open_settings(&mut self) -> AppEvent {
        if let Some(tab_id) = self
            .state
            .tabs
            .iter()
            .find(|tab| matches!(tab.kind, TabKind::Settings(_)))
            .map(|tab| tab.id)
        {
            self.state.active_tab = Some(tab_id);
            return AppEvent::TabActivated(tab_id);
        }

        // 设置内容是全局单例，不归属任何连接或数据库。
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: "设置".to_string(),
            kind: TabKind::Settings(SettingsTabState),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn open_ai_chat_tab(&mut self) -> AppEvent {
        if let Some(tab) = self.state.tabs.iter().find(|tab| matches!(tab.kind, TabKind::AiChat { .. })) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: "询问 AI".into(),
            // AI 是全局一级标签（与设置同级），不归属任何连接/数据库；
            // 当前业务页面由桌面端 `ai_chat.context_tab` 单独记录。
            kind: TabKind::AiChat { workspace: None },
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn open_user_admin(&mut self, connection_id: ConnectionId) -> AppEvent {
        if let Some(tab) = self.state.tabs.iter().find(|tab| {
            matches!(&tab.kind, TabKind::UserAdmin(admin) if admin.connection_id == connection_id)
        }) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }

        let Some(connection) = self
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        // 用户与权限：MySQL/TiDB 走既有 provider；PostgreSQL 走 PG 角色管理（角色经连接器读取）。
        let scope = match connection.config.kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => match database_user_admin_provider(connection.config.kind) {
                Some(provider) => provider.default_scope(),
                None => {
                    return self.fail(Error::new(ErrorKind::Internal, "MySQL 用户与权限 provider 缺失"));
                }
            },
            DatabaseKind::Postgres | DatabaseKind::Kingbase => PrivilegeScope::Postgres,
            DatabaseKind::SqlServer => PrivilegeScope::SqlServer,
            // 达梦账号模型与 Oracle 同源（账号 = schema，角色 + 系统权限 + 对象权限），
            // 复用同一套面板，差异只体现在渲染方言（见 `UserAdminSqlDialect`）。
            DatabaseKind::Oracle | DatabaseKind::Dameng => PrivilegeScope::Oracle,
            _ => {
                return self.fail(Error::new(ErrorKind::Unsupported, "暂不支持该连接的用户与权限管理"));
            }
        };
        // 方言跟着连接类型走：借用 `connection` 之前先取出，避免与后面的可变借用冲突。
        let dialect = if connection.config.kind == DatabaseKind::Dameng {
            UserAdminSqlDialect::Dameng
        } else {
            UserAdminSqlDialect::Oracle
        };
        let database = connection
            .config
            .options
            .get("database")
            .cloned()
            .or_else(|| {
                connection
                    .config
                    .sqlserver_profile
                    .as_ref()
                    .map(|profile| profile.basic.database.clone())
                    .filter(|database| !database.trim().is_empty())
            });
        // 新建账号表单的默认认证插件必须跟着服务端：MariaDB 没有
        // caching_sha2_password，预填它会导致创建账号直接语法/插件错误。
        // 在可变借用 `self` 之前取出，避免与 `connection` 的不可变借用冲突。
        let default_auth_plugin = database_user_admin_provider(connection.config.kind)
            .map(|provider| provider.default_auth_plugin().to_string());
        let tab_id = self.next_tab_id();
        let mut admin = UserAdminState::new(connection_id, database, scope);
        if let Some(default_auth_plugin) = default_auth_plugin {
            admin.auth_plugin = default_auth_plugin;
        }
        if scope == PrivilegeScope::SqlServer {
            admin.sqlserver = Some(SqlServerSecurityState {
                database: if admin.privilege_database == "*" { String::new() } else { admin.privilege_database.clone() },
                ..SqlServerSecurityState::default()
            });
        }
        if scope == PrivilegeScope::Oracle {
            admin.oracle = Some(OracleUserAdminState {
                dialect,
                ..OracleUserAdminState::default()
            });
        }
        // PG：面板以「成员关系」为主视图，默认停在 MemberOf，选角色即自动加载成员（无 MySQL 权限页）。
        if scope == PrivilegeScope::Postgres {
            admin.active_detail_tab = UserAdminDetailTab::Postgres(PgUserAdminTab::MemberOf);
        }
        self.push_tab(TabState {
            id: tab_id,
            title: "用户与权限".to_string(),
            kind: TabKind::UserAdmin(admin),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// 打开连接级服务器面板。`supports_server_panel` 之外的数据库明确报不支持，
    /// 不创建一个空白页签让用户猜。
    fn open_server_panel(&mut self, connection_id: ConnectionId) -> AppEvent {
        if let Some(tab) = self.state.tabs.iter().find(|tab| {
            matches!(&tab.kind, TabKind::ServerPanel(panel) if panel.connection_id == connection_id)
        }) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }
        let Some(connection) = self
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let kind = connection.config.kind;
        if !kind.supports_server_panel() {
            return self.fail(Error::new(
                ErrorKind::Unsupported,
                "当前数据库暂不支持服务器状态面板",
            ));
        }
        let database = Self::connection_default_database(&connection.config);
        let name = connection.config.name.clone();
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: format!("服务器 · {name}"),
            kind: TabKind::ServerPanel(ServerPanelState::new(
                connection_id,
                name.clone(),
                database,
            )),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// 打开连接级 Redis 服务器面板。INFO/CLIENT/SLOWLOG 是实例级命令，
    /// 逻辑库只用于标题上下文；同一连接只保留一个面板。
    fn open_redis_server_panel(&mut self, connection_id: ConnectionId) -> AppEvent {
        if let Some(tab) = self.state.tabs.iter().find(|tab| {
            matches!(
                &tab.kind,
                TabKind::RedisServerPanel(panel) if panel.connection_id == connection_id
            )
        }) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }
        let Some(connection) = self
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        if connection.config.kind != DatabaseKind::Redis {
            return self.fail(Error::new(
                ErrorKind::Unsupported,
                "当前数据库暂不支持 Redis 服务器面板",
            ));
        }
        let database = connection
            .config
            .redis_profile
            .as_ref()
            .and_then(|profile| profile.basic.database.as_deref())
            .or_else(|| connection.config.options.get("database").map(String::as_str))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let name = connection.config.name.clone();
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: format!("服务器 · {name}"),
            kind: TabKind::RedisServerPanel(RedisServerPanelState::new(
                connection_id,
                name,
                database,
            )),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// 面板维护/展示用的默认库：优先结构化档案，其次扁平 options。
    fn connection_default_database(config: &ConnectionConfig) -> String {
        if let Some(profile) = config.postgres_profile.as_ref() {
            return profile.maintenance_database().to_string();
        }
        if let Some(profile) = config.mysql_profile.as_ref() {
            if !profile.basic.database.trim().is_empty() {
                return profile.basic.database.clone();
            }
        }
        config
            .options
            .get("database")
            .cloned()
            .filter(|database| !database.trim().is_empty())
            .or_else(|| match &config.endpoint {
                Endpoint::Tcp { database, .. } => database.clone(),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn find_tab_mut(&mut self, tab_id: TabId) -> Option<&mut TabState> {
        self.state.tabs.iter_mut().find(|tab| tab.id == tab_id)
    }

    fn find_data_editor_mut(&mut self, tab_id: TabId) -> Option<&mut DataEditorState> {
        self.find_tab_mut(tab_id)
            .and_then(|tab| match &mut tab.kind {
                TabKind::DataEditor(editor) => Some(editor),
                _ => None,
            })
    }

    fn find_create_table_mut(&mut self, tab_id: TabId) -> Option<&mut CreateTableState> {
        self.find_tab_mut(tab_id)
            .and_then(|tab| match &mut tab.kind {
                TabKind::CreateTable(create) => Some(create),
                _ => None,
            })
    }

    fn find_editable_data_editor_mut(&mut self, tab_id: TabId) -> Option<&mut DataEditorState> {
        self.find_tab_mut(tab_id)
            .and_then(|tab| editable_data_editor_mut(&mut tab.kind))
    }

    fn find_tab(&self, tab_id: TabId) -> Option<&TabState> {
        self.state.tabs.iter().find(|tab| tab.id == tab_id)
    }

    fn connection_kind(&self, connection_id: ConnectionId) -> Option<DatabaseKind> {
        self.state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| connection.config.kind)
    }

    fn connection_config(&self, connection_id: ConnectionId) -> Option<&ConnectionConfig> {
        self.state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| &connection.config)
    }

}

fn editable_data_editor_mut(kind: &mut TabKind) -> Option<&mut DataEditorState> {
    match kind {
        TabKind::DataEditor(editor) => Some(editor),
        TabKind::QueryEditor(editor) => active_query_result_editor_mut(editor),
        _ => None,
    }
}

fn editable_data_editor(kind: &TabKind) -> Option<&DataEditorState> {
    match kind {
        TabKind::DataEditor(editor) => Some(editor),
        TabKind::QueryEditor(editor) => active_query_result_editor(editor),
        _ => None,
    }
}

fn active_query_result_editor(editor: &QueryEditorState) -> Option<&DataEditorState> {
    editor
        .active_result_editor
        .and_then(|page_index| editor.result_editors.get(&page_index))
}

fn active_query_result_editor_mut(editor: &mut QueryEditorState) -> Option<&mut DataEditorState> {
    let page_index = editor.active_result_editor?;
    editor.result_editors.get_mut(&page_index)
}

fn set_active_query_result_editor(editor: &mut QueryEditorState, page_index: Option<usize>) {
    editor.active_result_editor = page_index.filter(|page_index| editor.result_editors.contains_key(page_index));
}
