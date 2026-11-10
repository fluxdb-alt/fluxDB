impl AppController {
    // 标签创建、激活与关闭操作独立成帧，避免无关命令长期占用公共分派器的小栈空间。
    fn dispatch_tab_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::OpenObjectList(parent) => {
                let tab_id = self.next_tab_id();
                let objects = mock_child_objects(parent.as_ref());
                self.push_tab(TabState {
                    id: tab_id,
                    title: parent
                        .as_ref()
                        .map(|path| path.name.clone())
                        .unwrap_or_else(|| "对象".to_string()),
                    kind: TabKind::ObjectList(ObjectListState {
                        parent,
                        objects,
                        loading: false,
                        error: None,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenDataEditor(object) => {
                if let Some(existing_tab_id) = self.state.tabs.iter().find_map(|tab| {
                    if let TabKind::DataEditor(editor) = &tab.kind
                        && editor.object == object
                    {
                        return Some(tab.id);
                    }
                    None
                }) {
                    self.state.active_tab = Some(existing_tab_id);
                    return AppEvent::TabActivated(existing_tab_id);
                }

                let tab_id = self.next_tab_id();
                let title = object.name.clone();
                // 数据表默认每页行数来自「设置-数据」，与 SQL 查询结果的 page_size 无关。
                // 经 Pagination::new 收敛到 1..=MAX_LIMIT，避免脏配置把页大小置成 0。
                let pagination = Pagination::new(0, self.state.settings.data_table_page_size);

                self.push_tab(TabState {
                    id: tab_id,
                    title,
                    kind: TabKind::DataEditor(DataEditorState {
                        object,
                        page: None,
                        original_page: None,
                        pagination,
                        load_generation: 0,
                        changes: None,
                        editing_cell: None,
                        cell_detail_panel: CellDetailPanelState::default(),
                        table_info: TableInfoState::default(),
                        loading: true,
                        submitting: false,
                        error: None,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenBackupList(database_path) => {
                // 按库去重：同一连接+库已有备份 tab 则激活既有 tab。
                let database = database_path
                    .database
                    .clone()
                    .unwrap_or_else(|| database_path.name.clone());
                let connection_id = database_path.connection_id;
                if let Some(existing_tab_id) = self.state.tabs.iter().find_map(|tab| {
                    if let TabKind::BackupList(list) = &tab.kind
                        && list.connection_id == connection_id
                        && list.database == database
                    {
                        return Some(tab.id);
                    }
                    None
                }) {
                    self.state.active_tab = Some(existing_tab_id);
                    return AppEvent::TabActivated(existing_tab_id);
                }

                let tab_id = self.next_tab_id();
                self.push_tab(TabState {
                    id: tab_id,
                    title: format!("备份-{database}"),
                    kind: TabKind::BackupList(BackupListState {
                        connection_id,
                        database,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenErDiagram(path) => {
                // 表级 path → 当前表关联 ER（以该表为中心 1 跳）；库/其它 → 整库 ER。
                let database = path.database.clone().unwrap_or_else(|| path.name.clone());
                let schema = path.schema.clone();
                // 中心表用结构化身份（schema + 裸名）：PG 跨 schema 与含点标识符都安全；
                // 展示名由身份统一生成，匹配关系边不靠字符串拆解。
                let center_table =
                    (path.kind == ObjectKind::Table).then(|| fluxdb_core::ErTableRef {
                        database: database.clone(),
                        schema: schema.clone(),
                        name: path.name.clone(),
                    });
                let connection_id = path.connection_id;
                // 按「连接+库+schema+中心表」去重：整库与当前表关联视图各自唯一。
                if let Some(existing_tab_id) = self.state.tabs.iter().find_map(|tab| {
                    if let TabKind::ErDiagram(er) = &tab.kind
                        && er.connection_id == connection_id
                        && er.database == database
                        && er.schema == schema
                        && er.center_table == center_table
                    {
                        return Some(tab.id);
                    }
                    None
                }) {
                    self.state.active_tab = Some(existing_tab_id);
                    return AppEvent::TabActivated(existing_tab_id);
                }

                let tab_id = self.next_tab_id();
                let title = match &center_table {
                    Some(table) => format!("{} · 关联 ER", table.display()),
                    None => format!("{database} · ER"),
                };
                self.push_tab(TabState {
                    id: tab_id,
                    title,
                    kind: TabKind::ErDiagram(ErDiagramState {
                        connection_id,
                        database,
                        schema,
                        center_table,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenQueryEditor(connection_id) => {
                self.open_query_editor(connection_id, None, None)
            }
            AppCommand::OpenUserAdmin(connection_id) => self.open_user_admin(connection_id),
            AppCommand::OpenServerPanel(connection_id) => self.open_server_panel(connection_id),
            AppCommand::OpenRedisServerPanel(connection_id) => {
                self.open_redis_server_panel(connection_id)
            }
            AppCommand::OpenMongoServerPanel(connection_id) => {
                self.open_mongo_server_panel(connection_id)
            }
            AppCommand::OpenSettings => self.open_settings(),
            AppCommand::OpenAiChatTab => self.open_ai_chat_tab(),
            AppCommand::OpenQueryEditorInDatabase {
                connection_id,
                database,
                schema,
            } => self.open_query_editor(connection_id, database, schema),
            AppCommand::OpenRedisWorkbench {
                connection_id,
                database,
            } => self.open_redis_workbench(connection_id, database),
            AppCommand::OpenRedisCli {
                connection_id,
                database,
            } => self.open_redis_cli(connection_id, database),
            AppCommand::OpenRedisPubSub {
                connection_id,
                database,
            } => self.open_redis_pubsub(connection_id, database),
            AppCommand::CloseTab(tab_id) => {
                if self
                    .state
                    .tabs
                    .iter()
                    .any(|tab| tab.id == tab_id && tab.dirty)
                {
                    self.state.pending_dirty_tab_close = Some(tab_id);
                    return AppEvent::TabCloseRequested(tab_id);
                }

                self.release_tab_query_sessions(&[tab_id]);
                self.state.tabs.retain(|tab| tab.id != tab_id);
                if self.state.pending_dirty_tab_close == Some(tab_id) {
                    self.state.pending_dirty_tab_close = None;
                }
                if self.state.active_tab == Some(tab_id) {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                AppEvent::TabClosed(tab_id)
            }
            AppCommand::CloseTabs(tab_ids) => {
                let tab_ids = tab_ids.into_iter().collect::<BTreeSet<_>>();
                let closed = self
                    .state
                    .tabs
                    .iter()
                    .find(|tab| tab_ids.contains(&tab.id))
                    .map(|tab| tab.id);
                let closed_ids = tab_ids.iter().copied().collect::<Vec<_>>();
                self.release_tab_query_sessions(&closed_ids);
                self.state.tabs.retain(|tab| !tab_ids.contains(&tab.id));
                if self
                    .state
                    .active_tab
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                if self
                    .state
                    .pending_dirty_tab_close
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.pending_dirty_tab_close = None;
                }
                closed
                    .map(AppEvent::TabClosed)
                    .unwrap_or(AppEvent::TabActivated(TabId(0)))
            }
            AppCommand::ConfirmCloseDirtyTab(tab_id) => {
                if self.state.pending_dirty_tab_close != Some(tab_id) {
                    return self.fail(Error::new(ErrorKind::Internal, "没有待关闭的标签页"));
                }

                self.release_tab_query_sessions(&[tab_id]);
                self.state.tabs.retain(|tab| tab.id != tab_id);
                self.state.pending_dirty_tab_close = None;
                if self.state.active_tab == Some(tab_id) {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                AppEvent::TabClosed(tab_id)
            }
            AppCommand::CancelCloseDirtyTab(tab_id) => {
                if self.state.pending_dirty_tab_close == Some(tab_id) {
                    self.state.pending_dirty_tab_close = None;
                }
                AppEvent::TabCloseCancelled(tab_id)
            }
            AppCommand::ActivateTab(tab_id) => {
                if self.state.tabs.iter().any(|tab| tab.id == tab_id) {
                    self.state.active_tab = Some(tab_id);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "标签页不存在"))
                }
            }
            // 回首页：仅取消激活当前标签（active_tab 置空），已打开的标签保留在标签栏不关闭。
            // 事件用 TabActivated(TabId(0)) 空态约定（与 CloseTabs 一致），供桌面清理局部状态。
            AppCommand::DeactivateTab => {
                self.state.active_tab = None;
                AppEvent::TabActivated(TabId(0))
            }
            AppCommand::PinTab(tab_id) => {
                if let Some(index) = self.state.tabs.iter().position(|tab| tab.id == tab_id) {
                    let tab = self.state.tabs.remove(index);
                    self.state.tabs.insert(0, tab);
                    self.state.active_tab = Some(tab_id);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "标签页不存在"))
                }
            }
            AppCommand::SetTabDirty { tab_id, dirty } => {
                if let Some(tab) = self.state.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.dirty = dirty;
                }
                AppEvent::SettingsSaved
            }
            // 私有入口仅由 dispatch 的对应变体分组调用。
            _ => unreachable!("仅标签创建、激活与关闭操作命令进入此分发器"),
        }
    }
}
