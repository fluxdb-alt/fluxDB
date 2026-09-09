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
            _ => None,
        })
    }

    fn open_query_editor(
        &mut self,
        connection_id: ConnectionId,
        database: Option<String>,
    ) -> AppEvent {
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: "无标题 - 查询".to_string(),
            kind: TabKind::QueryEditor(QueryEditorState {
                connection_id,
                database,
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
        if let Some(tab) = self
            .state
            .tabs
            .iter()
            .find(|tab| matches!(tab.kind, TabKind::Settings))
        {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }

        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: "设置".to_string(),
            kind: TabKind::Settings,
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
        let Some(provider) = database_user_admin_provider(connection.config.kind) else {
            return self.fail(Error::new(ErrorKind::Unsupported, "暂不支持该连接的用户与权限管理"));
        };
        let database = connection.config.options.get("database").cloned();
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: "用户与权限".to_string(),
            kind: TabKind::UserAdmin(UserAdminState::new(
                connection_id,
                database,
                provider.default_scope(),
            )),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
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
