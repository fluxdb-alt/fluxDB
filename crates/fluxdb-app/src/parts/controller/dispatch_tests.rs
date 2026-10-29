#[cfg(test)]
mod controller_dispatch_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn sqlite_draft(name: &str) -> ConnectionDraft {
        ConnectionDraft {
            name: name.into(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: ":memory:".into(),
                read_only: false,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    #[test]
    fn connection_commands_preserve_grouping_and_remove_only_owned_tabs() {
        let mut controller = AppController::new();
        let AppEvent::ConnectionCreated(mut first) =
            controller.dispatch(AppCommand::CreateConnection(sqlite_draft("first")))
        else {
            panic!("first connection must be created");
        };
        let AppEvent::ConnectionCreated(second) =
            controller.dispatch(AppCommand::CreateConnection(sqlite_draft("second")))
        else {
            panic!("second connection must be created");
        };
        assert_ne!(first.id, second.id);
        let AppEvent::ConnectionGroupCreated(group) =
            controller.dispatch(AppCommand::CreateConnectionGroup("local".into()))
        else {
            panic!("connection group must be created");
        };
        controller.dispatch(AppCommand::MoveConnectionToGroup {
            connection_id: first.id,
            group_id: group.id,
        });
        controller.dispatch(AppCommand::RenameConnectionGroup {
            group_id: group.id,
            name: "renamed".into(),
        });
        controller.dispatch(AppCommand::ToggleConnectionGroup(group.id));
        let layout = &controller.state().sidebar_layout;
        assert_eq!(layout.groups[0].name, "renamed");
        assert!(layout.groups[0].collapsed);
        assert!(layout.order.iter().any(|entry| matches!(
            entry,
            fluxdb_core::SidebarOrderEntry::Group { id, connection_ids }
                if *id == group.id && connection_ids == &[first.id]
        )));

        controller.dispatch(AppCommand::OpenQueryEditor(first.id));
        let AppEvent::TabOpened(second_tab) =
            controller.dispatch(AppCommand::OpenQueryEditor(second.id))
        else {
            panic!("second query tab must open");
        };
        first.name = "updated".into();
        assert_eq!(
            controller.dispatch(AppCommand::UpdateConnection(first.clone())),
            AppEvent::ConnectionUpdated(first.clone())
        );
        assert_eq!(controller.state().tabs.len(), 2);
        assert_eq!(controller.connection_config(first.id), Some(&first));
        assert_eq!(
            controller.dispatch(AppCommand::DeleteConnection(first.id)),
            AppEvent::ConnectionDeleted(first.id)
        );
        assert!(controller.connection_config(first.id).is_none());
        assert_eq!(controller.connection_config(second.id), Some(&second));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().tabs[0].id, second_tab);
        assert_eq!(controller.state().active_tab, Some(second_tab));
    }

    #[test]
    fn tab_commands_reuse_views_and_keep_dirty_close_confirmation() {
        let mut controller = AppController::new();
        let config = sqlite_draft("tabs").into_config(ConnectionId(1));
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".into()),
            schema: None,
            name: "items".into(),
            kind: ObjectKind::Table,
        };
        let AppEvent::TabOpened(data_tab) =
            controller.dispatch(AppCommand::OpenDataEditor(object.clone()))
        else {
            panic!("data tab must open");
        };
        assert_eq!(
            controller.dispatch(AppCommand::OpenDataEditor(object.clone())),
            AppEvent::TabActivated(data_tab)
        );
        let AppEvent::TabOpened(backup_tab) =
            controller.dispatch(AppCommand::OpenBackupList(object.clone()))
        else {
            panic!("backup tab must open");
        };
        assert_eq!(
            controller.dispatch(AppCommand::OpenBackupList(object.clone())),
            AppEvent::TabActivated(backup_tab)
        );
        let AppEvent::TabOpened(er_tab) =
            controller.dispatch(AppCommand::OpenErDiagram(object.clone()))
        else {
            panic!("ER tab must open");
        };
        assert_eq!(
            controller.dispatch(AppCommand::OpenErDiagram(object)),
            AppEvent::TabActivated(er_tab)
        );
        assert_eq!(controller.state().tabs.len(), 3);
        controller.dispatch(AppCommand::SetTabDirty {
            tab_id: data_tab,
            dirty: true,
        });
        assert_eq!(
            controller.dispatch(AppCommand::CloseTab(data_tab)),
            AppEvent::TabCloseRequested(data_tab)
        );
        assert_eq!(controller.state().pending_dirty_tab_close, Some(data_tab));
        assert_eq!(controller.state().tabs.len(), 3);
        assert_eq!(
            controller.dispatch(AppCommand::CancelCloseDirtyTab(data_tab)),
            AppEvent::TabCloseCancelled(data_tab)
        );
        assert!(controller.state().pending_dirty_tab_close.is_none());
        assert!(controller.find_tab(data_tab).unwrap().dirty);
        controller.dispatch(AppCommand::CloseTab(data_tab));
        assert_eq!(
            controller.dispatch(AppCommand::ConfirmCloseDirtyTab(data_tab)),
            AppEvent::TabClosed(data_tab)
        );
        assert!(controller.find_tab(data_tab).is_none());
        assert_eq!(controller.state().active_tab, Some(er_tab));
        assert_eq!(
            controller.dispatch(AppCommand::PinTab(backup_tab)),
            AppEvent::TabActivated(backup_tab)
        );
        assert_eq!(controller.state().tabs[0].id, backup_tab);
        assert_eq!(
            controller.dispatch(AppCommand::DeactivateTab),
            AppEvent::TabActivated(TabId(0))
        );
        assert!(controller.state().active_tab.is_none());
        controller.dispatch(AppCommand::ActivateTab(er_tab));
        assert_eq!(
            controller.dispatch(AppCommand::CloseTabs(vec![backup_tab, backup_tab])),
            AppEvent::TabClosed(backup_tab)
        );
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().active_tab, Some(er_tab));
    }

    #[test]
    fn connection_tree_refresh_still_invalidates_completion_before_routing() {
        let mut controller = AppController::new();
        let config = sqlite_draft("completion").into_config(ConnectionId(1));
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        let key = CompletionIndex::db_key(ConnectionId(1), Some("main"), None);
        controller
            .completion_index
            .lock()
            .unwrap()
            .metas
            .insert(key.clone(), empty_completion_meta(DatabaseKind::Sqlite));
        assert!(!controller.completion_index.lock().unwrap().metas[&key].dirty);

        // 连接未展开，不访问数据库；公共前置处理仍须失效已有 scope。
        assert_eq!(
            controller.dispatch(AppCommand::RefreshConnectionTree),
            AppEvent::ObjectsLoaded(None, Vec::new())
        );
        let index = controller.completion_index.lock().unwrap();
        assert!(index.metas[&key].dirty);
        assert!(index.is_database_dirty(ConnectionId(1), Some("main"), None));
    }

    /// 连接串导入命令必须路由到 Mongo 处理器，而不是兜底落到表处理器报内部错误。
    #[test]
    fn discover_mongo_connection_routes_to_mongo_handler() {
        let mut controller = AppController::new();
        let event = controller.dispatch(AppCommand::DiscoverMongoConnection {
            connection_string:
                "mongodb+srv://user:pass@cluster0.h0l2m0b.mongodb.net".to_string(),
        });
        let AppEvent::MongoConnectionDiscovered(draft) = event else {
            panic!("MongoDB 连接串导入应返回发现事件，实际 {event:?}");
        };
        assert_eq!(draft.kind, DatabaseKind::MongoDb);
        let profile = draft.mongo_profile.expect("应带上解析出的 Mongo 档案");
        assert!(profile.srv);
        assert_eq!(profile.basic.username, "user");
    }

    #[test]
    fn commands_on_small_stack_handle_mysql_handshake_failure() {
        const CHILD_PORT: &str = "FLUXDB_DISPATCH_STACK_TEST_PORT";
        const CHILD_MODE: &str = "FLUXDB_DISPATCH_STACK_TEST_MODE";
        const TEST_NAME: &str =
            "controller_dispatch_tests::commands_on_small_stack_handle_mysql_handshake_failure";

        if let Ok(port) = std::env::var(CHILD_PORT) {
            let mut controller = AppController::new();
            let config = ConnectionConfig {
                id: ConnectionId(1),
                name: "query-stack-test".into(),
                kind: DatabaseKind::MySql,
                endpoint: fluxdb_core::Endpoint::Tcp {
                    host: "127.0.0.1".into(),
                    port: port.parse().unwrap(),
                    database: None,
                },
                options: BTreeMap::new(),
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                sqlserver_profile: None,
                mongo_profile: None,
                oracle_profile: None,
                redis_profile: None,
            };
            controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
            controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
            let tab_id = controller.state().active_tab.unwrap();
            let mode = std::env::var(CHILD_MODE).unwrap();
            let is_query = matches!(mode.as_str(), "default" | "explicit");
            if is_query {
                controller.dispatch(AppCommand::StartQueryExecution(tab_id));
            }
            let object = ObjectPath {
                connection_id: ConnectionId(1),
                database: None,
                schema: None,
                name: "stack_source".into(),
                kind: ObjectKind::Table,
            };
            let command = match mode.as_str() {
                "default" => AppCommand::ExecuteQueryText {
                    tab_id,
                    text: "SELECT 1".into(),
                },
                "explicit" => AppCommand::ExecuteQueryTextWithOptions {
                    tab_id,
                    text: "SELECT 1".into(),
                    options: QueryExecutionOptions::default(),
                },
                "copy" => AppCommand::CopyTable {
                    object,
                    new_name: "stack_target".into(),
                    copy_data: false,
                },
                "drop" => AppCommand::DropTable {
                    object,
                    foreign_key_check: ForeignKeyCheckMode::Default,
                },
                _ => panic!("unexpected command mode"),
            };
            // 与 macOS GCD 一样使用小栈；溢出会终止子进程，不能拖垮其他用例。
            std::thread::Builder::new()
                .stack_size(512 * 1024)
                .spawn(move || {
                    let AppEvent::Failed(error) = controller.dispatch(command) else {
                        panic!("incomplete TLS handshake must report a connection error");
                    };
                    assert_eq!(error.title, "连接失败");
                    assert!(error.retryable);
                    assert_eq!(controller.state().last_error.as_ref(), Some(&error));
                    let TabKind::QueryEditor(editor) =
                        &controller.state().active_tab().unwrap().kind
                    else {
                        panic!("query tab must remain open");
                    };
                    assert!(!editor.running);
                    if is_query {
                        assert_eq!(editor.error.as_ref(), Some(&error));
                    } else {
                        assert!(editor.error.is_none());
                    }
                    assert!(editor.results.is_empty());
                    assert!(editor.summaries.is_empty());
                })
                .unwrap()
                .join()
                .unwrap();
            return;
        }

        for mode in ["default", "explicit", "copy", "drop"] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(
                    CHILD_PORT,
                    listener.local_addr().unwrap().port().to_string(),
                )
                .env(CHILD_MODE, mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                            let _ = child.kill();
                            let output = child.wait_with_output().unwrap();
                            panic!("{mode}: no handshake connection: {output:?}");
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{mode}: accept failed: {error}"),
                }
            };
            // macOS 的 accept 会继承 listener 的非阻塞标志，读包前显式恢复。
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();

            // 仅模拟支持 TLS 的 MySQL greeting；不认证，也不接收或执行 SQL。
            let capabilities: u32 = 1 | 4 | 512 | 2048 | 8192 | 32768 | 131072 | 524288;
            let mut greeting = b"\x0a8.0.36\0\x01\0\0\0abcdefgh\0".to_vec();
            greeting.extend_from_slice(&(capabilities as u16).to_le_bytes());
            greeting.extend_from_slice(b"\x2d\x02\0");
            greeting.extend_from_slice(&((capabilities >> 16) as u16).to_le_bytes());
            greeting.push(21);
            greeting.extend_from_slice(&[0; 10]);
            greeting.extend_from_slice(b"ijklmnopqrst\0mysql_native_password\0");
            let mut packet = (greeting.len() as u32).to_le_bytes();
            packet[3] = 0;
            stream.write_all(&packet).unwrap();
            stream.write_all(&greeting).unwrap();
            let mut ssl_request = [0; 36];
            let handshake = stream.read_exact(&mut ssl_request);
            drop(stream);

            let deadline = Instant::now() + Duration::from_secs(30);
            while child.try_wait().unwrap().is_none() {
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("{mode}: query child timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{mode}: {output:?}");
            handshake.unwrap();
            assert_eq!(&ssl_request[..3], &[32, 0, 0]);
            let flags = u32::from_le_bytes(ssl_request[4..8].try_into().unwrap());
            assert_ne!(flags & 2048, 0, "{mode}: client must request TLS");
        }
    }
}
