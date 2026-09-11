    /// 侧边栏「刷新连接树」：只用服务端最新结果替换第一层对象，不碰连接态与展开态。
    #[test]
    fn refresh_connection_tree_keeps_connection_and_expansion_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        assert!(controller.state().connections[0].expanded);

        let event = controller.dispatch(AppCommand::RefreshConnectionTree);

        assert!(matches!(event, AppEvent::ObjectsLoaded(None, _)));
        let connection = &controller.state().connections[0];
        assert!(connection.connected, "刷新不应改变连接态");
        assert!(connection.expanded, "刷新不应改变展开态");
        assert_eq!(connection.objects.len(), 1);
        assert_eq!(connection.objects[0].path.kind, ObjectKind::Database);
        assert_eq!(connection.objects[0].path.name, "main");
    }

    /// 折叠连接在树上不可见，刷新不应为它发请求，也不应覆盖它的缓存。
    #[test]
    fn refresh_connection_tree_skips_collapsed_connections() {
        let mut controller = AppController::with_mock_data();
        // 2 号连接也切到 demo 数据源，避免单测触发真实网络。
        controller.state.connections[1]
            .config
            .options
            .insert("demo".to_string(), "true".to_string());
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(2)));

        // 2 号被用户收起并清空缓存作为哨兵：若刷新错误地覆盖折叠连接，哨兵会被重新填上。
        controller.state.connections[1].expanded = false;
        controller.state.connections[1].objects.clear();

        controller.dispatch(AppCommand::RefreshConnectionTree);

        assert!(
            controller.state().connections[1].objects.is_empty(),
            "折叠连接不应被刷新"
        );
        assert_eq!(controller.state().connections[0].objects.len(), 1);
    }

    /// 已展开数据库下已加载的表行必须保留，否则刷新会把整棵子树清空。
    #[test]
    fn refresh_connection_tree_keeps_loaded_table_rows() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        assert_eq!(controller.state().connections[0].objects.len(), 5); // 1 库 + 4 表

        controller.dispatch(AppCommand::RefreshConnectionTree);

        let objects = &controller.state().connections[0].objects;
        assert_eq!(
            objects
                .iter()
                .filter(|object| is_connection_level0(object.path.kind))
                .count(),
            1,
            "第一层应只保留最新的一份"
        );
        for name in ["Product", "ProductCategory", "Order", "Customer"] {
            assert!(
                objects
                    .iter()
                    .any(|object| object.path.name == name && object.path.kind == ObjectKind::Table),
                "表 {name} 的行应在刷新后保留"
            );
        }
    }

    /// 库已被服务端删除时，其名下的表行应一并丢弃，不留孤儿行。
    #[test]
    fn replace_connection_level0_drops_rows_of_removed_database() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let mut connection = controller.state().connections[0].clone();

        replace_connection_level0(
            &mut connection,
            vec![ObjectSummary {
                path: ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("legacy".to_string()),
                    schema: None,
                    name: "legacy".to_string(),
                    kind: ObjectKind::Database,
                },
                rows: None,
                modified_at: None,
                comment: None,
            }],
        );

        assert_eq!(connection.objects.len(), 1, "旧库及其表行都应被丢弃");
        assert_eq!(connection.objects[0].path.name, "legacy");
    }

    /// 单连接失败不得中断整轮刷新：其余连接照常更新，错误照常上报。
    #[test]
    fn refresh_connection_tree_keeps_going_when_one_connection_fails() {
        let mut controller = AppController::with_mock_data();
        controller.state.connections[1]
            .config
            .options
            .insert("demo".to_string(), "true".to_string());
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(2)));

        // 把 2 号换成端点不合法的 MySQL（非 TCP 端点 → 零 I/O 立即失败），模拟服务端不可达。
        let broken = &mut controller.state.connections[1];
        broken.config.options.remove("demo");
        broken.config.kind = DatabaseKind::MySql;
        broken.config.endpoint = Endpoint::SqliteFile {
            path: "demo.db".into(),
            read_only: false,
        };
        broken.objects.clear();

        let event = controller.dispatch(AppCommand::RefreshConnectionTree);

        assert!(matches!(event, AppEvent::Failed(_)), "期望 Failed，实际: {event:?}");
        assert!(controller.state().last_error.is_some());
        assert_eq!(
            controller.state().connections[0].objects.len(),
            1,
            "失败连接不应阻断其它连接的刷新"
        );
    }

    /// 后台刷新只回搬 `objects`：期间用户折叠连接，展开态不得被后台结果覆盖回去。
    #[test]
    fn merge_refreshed_tree_from_keeps_live_expansion_state() {
        let mut current = AppController::with_mock_data();
        current.dispatch(AppCommand::OpenConnection(ConnectionId(1)));

        let mut refreshed = current.clone();
        refreshed.dispatch(AppCommand::RefreshConnectionTree);

        // 后台任务执行期间用户把连接收起。
        current.state.connections[0].expanded = false;
        current.merge_refreshed_tree_from(&refreshed);

        assert!(
            !current.state().connections[0].expanded,
            "后台结果不应覆盖用户刚做的折叠"
        );
        assert_eq!(current.state().connections[0].objects.len(), 1);
    }
