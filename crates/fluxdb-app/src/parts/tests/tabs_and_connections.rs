fn create_table(controller: &AppController) -> &CreateTableState {
    let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
    else {
        panic!("expected create table tab");
    };
    create
}

    #[test]
    fn opens_and_activates_query_tab() {
        let mut controller = AppController::with_mock_data();
        let event = controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));

        assert_eq!(event, AppEvent::TabOpened(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().active_tab, Some(TabId(1)));
        assert!(matches!(
            controller.state().active_tab().map(|tab| &tab.kind),
            Some(TabKind::QueryEditor(_))
        ));
    }

    #[test]
    fn opens_query_tab_in_database_context() {
        let mut controller = AppController::with_mock_data();
        let event = controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        assert_eq!(event, AppEvent::TabOpened(TabId(1)));
        let Some(TabKind::QueryEditor(editor)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active query editor");
        };
        assert_eq!(editor.connection_id, ConnectionId(1));
        assert_eq!(editor.database.as_deref(), Some("main"));
    }

    #[test]
    fn opens_redis_pubsub_tab_with_database_context() {
        // 右键 Redis 数据库「打开 Pub/Sub」应携带连接 + 目标库上下文，打开对应标签页。
        let mut controller = AppController::with_mock_data();
        let event = controller.dispatch(AppCommand::OpenRedisPubSub {
            connection_id: ConnectionId(2),
            database: 0,
        });

        assert_eq!(event, AppEvent::TabOpened(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().active_tab, Some(TabId(1)));
        let Some(TabKind::RedisPubSub(pubsub)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active redis pub/sub tab");
        };
        assert_eq!(pubsub.connection_id, ConnectionId(2));
        assert_eq!(pubsub.database, 0);
        assert_eq!(controller.state().tabs[0].title, "Redis Pub/Sub - DB 0");
    }

    #[test]
    fn redis_pubsub_tab_is_reused_for_same_connection_and_database() {
        // 同一连接 + 同一库再次打开应复用标签页（激活），而非新开。
        let mut controller = AppController::with_mock_data();
        let first = controller.dispatch(AppCommand::OpenRedisPubSub {
            connection_id: ConnectionId(2),
            database: 5,
        });
        let second = controller.dispatch(AppCommand::OpenRedisPubSub {
            connection_id: ConnectionId(2),
            database: 5,
        });

        assert_eq!(first, AppEvent::TabOpened(TabId(1)));
        assert_eq!(second, AppEvent::TabActivated(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
    }

    #[test]
    fn redis_pubsub_tab_opens_separately_per_database() {
        // 不同数据库的 Pub/Sub 是独立的订阅会话，应各自新开标签页并传递各自的库上下文。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenRedisPubSub {
            connection_id: ConnectionId(2),
            database: 0,
        });
        controller.dispatch(AppCommand::OpenRedisPubSub {
            connection_id: ConnectionId(2),
            database: 1,
        });

        assert_eq!(controller.state().tabs.len(), 2);
        assert_eq!(controller.state().active_tab, Some(TabId(2)));
        let dbs = controller
            .state()
            .tabs
            .iter()
            .map(|tab| match &tab.kind {
                TabKind::RedisPubSub(pubsub) => (pubsub.connection_id, pubsub.database),
                _ => panic!("expected redis pub/sub tab"),
            })
            .collect::<Vec<_>>();
        assert_eq!(dbs, vec![(ConnectionId(2), 0), (ConnectionId(2), 1)]);
    }

    #[test]
    fn create_table_tab_tracks_draft_and_sql_preview() {
        let mut controller = AppController::with_mock_data();
        let event = controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });

        assert_eq!(event, AppEvent::TabOpened(TabId(1)));
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.columns[0].name, "id");
        assert_eq!(create.columns[0].data_type, "int");
        assert!(create.columns[0].length.is_empty());
        assert!(!create.columns[0].nullable);
        assert!(create.columns[0].primary_key);

        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.database.as_deref(), Some("main"));
        assert_eq!(create.engine, "InnoDB");
        assert_eq!(create.charset, "utf8mb4");
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `orders` (\n  `id` int NOT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );

        controller.dispatch(AppCommand::SelectCreateTableTab {
            tab_id: TabId(1),
            create_tab: CreateTableTab::Ddl,
        });
        assert_eq!(create_table(&controller).active_tab, CreateTableTab::Fields);
    }

    #[test]
    fn design_table_opens_existing_columns_and_generates_alter_preview() {
        let mut controller = AppController::with_mock_data();
        let event = controller.dispatch(AppCommand::OpenDesignTable(ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "Product".to_string(),
            kind: ObjectKind::Table,
        }));

        assert_eq!(event, AppEvent::TabOpened(TabId(1)));
        let create = create_table(&controller);
        assert!(create.is_design());
        assert_eq!(create.table_name, "Product");
        assert!(create.columns.iter().any(|column| column.name == "id"));
        assert_eq!(create.validation_error(), Some("没有需要保存的变更"));

        let column_id = create.next_column_id;
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id,
            field: CreateTableColumnField::Name,
            value: "nickname".to_string(),
        });

        assert!(create_table(&controller)
            .sql_preview()
            .unwrap()
            .contains("ALTER TABLE \"Product\" ADD COLUMN \"nickname\" TEXT;"));
        let ddl = create_table(&controller).ddl_preview().unwrap();
        assert!(ddl.contains("CREATE TABLE \"Product\""));
        assert!(!ddl.contains("nickname"));

        controller.dispatch(AppCommand::SelectCreateTableTab {
            tab_id: TabId(1),
            create_tab: CreateTableTab::Ddl,
        });
        assert_eq!(create_table(&controller).active_tab, CreateTableTab::Ddl);
    }

    #[test]
    fn create_table_mysql_options_are_appended_to_sql_preview() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });
        for (field, value) in [
            (CreateTableOptionField::Engine, "InnoDB"),
            (CreateTableOptionField::Tablespace, "ts_hot"),
            (CreateTableOptionField::Charset, "utf8mb4"),
            (CreateTableOptionField::Collation, "utf8mb4_unicode_ci"),
            (CreateTableOptionField::RowFormat, "DYNAMIC"),
            (CreateTableOptionField::AvgRowLength, "128"),
            (CreateTableOptionField::MaxRows, "10000"),
            (CreateTableOptionField::MinRows, "10"),
            (CreateTableOptionField::KeyBlockSize, "8"),
        ] {
            controller.dispatch(AppCommand::SetCreateTableOptionField {
                tab_id: TabId(1),
                field,
                value: value.to_string(),
            });
        }

        assert_eq!(
            create_table(&controller).sql_preview().unwrap(),
            "CREATE TABLE `orders` (\n  `id` int NOT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB TABLESPACE `ts_hot` DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci ROW_FORMAT=DYNAMIC AVG_ROW_LENGTH=128 MAX_ROWS=10000 MIN_ROWS=10 KEY_BLOCK_SIZE=8;"
        );
    }

    #[test]
    fn create_table_mysql_partition_template_is_appended_to_sql_preview() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTablePartitionField {
            tab_id: TabId(1),
            field: CreateTablePartitionField::Expression,
            value: "TO_DAYS(created_at)".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTablePartitionEnabled(TabId(1)));

        assert!(create_table(&controller).sql_preview().unwrap().contains(
            "PARTITION BY RANGE (TO_DAYS(created_at)) (\n  PARTITION p0 VALUES LESS THAN (MAXVALUE)\n);"
        ));
    }

    #[test]
    fn create_table_column_options_follow_selected_field() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SelectCreateTableColumn {
            tab_id: TabId(1),
            column_id: 2,
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "prices".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "amount".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "decimal".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Length,
            value: "10".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Scale,
            value: "2".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 2,
            flag: CreateTableColumnFlag::Unsigned,
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.selected_column_id, Some(2));
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `prices` (\n  `id` int NOT NULL,\n  `amount` decimal(10,2) UNSIGNED,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_provider_follows_sqlite_connection_kind() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "status".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableIndex(TabId(1)));
        let Some(TabKind::CreateTable(create)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.indexes[0].columns[0].name, "");
        assert_eq!(create.indexes[0].columns[0].sort_order, "");
        assert_eq!(create.indexes[0].index_type, "");
        assert_eq!(create.indexes[0].index_method, "");
        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexType,
            value: "UNIQUE".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableIndexColumnField {
            tab_id: TabId(1),
            index_id: 1,
            column_index: 0,
            field: CreateTableIndexColumnField::Name,
            value: "status".to_string(),
        });

        let create = create_table(&controller);
        assert_eq!(create.database_kind, DatabaseKind::Sqlite);
        assert_eq!(create.columns[0].data_type, "integer");
        assert_eq!(create.columns[1].data_type, "text");
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE \"orders\" (\n  \"id\" INTEGER NOT NULL,\n  \"status\" TEXT,\n  PRIMARY KEY (\"id\")\n);\nCREATE UNIQUE INDEX \"uk_orders_status\" ON \"orders\" (\"status\");"
        );

        controller.dispatch(AppCommand::SetCreateTableIndexColumnField {
            tab_id: TabId(1),
            index_id: 1,
            column_index: 0,
            field: CreateTableIndexColumnField::SortOrder,
            value: "ASC".to_string(),
        });

        assert_eq!(
            create_table(&controller).sql_preview().unwrap(),
            "CREATE TABLE \"orders\" (\n  \"id\" INTEGER NOT NULL,\n  \"status\" TEXT,\n  PRIMARY KEY (\"id\")\n);\nCREATE UNIQUE INDEX \"uk_orders_status\" ON \"orders\" (\"status\" ASC);"
        );
    }

    #[test]
    fn create_table_provider_rejects_unsupported_connection_kind() {
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
            id: ConnectionId(1),
            name: "mongo".to_string(),
            kind: DatabaseKind::MongoDb,
            endpoint: Endpoint::Uri {
                uri: "mongodb://localhost".to_string(),
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }]));
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        let create = create_table(&controller);
        assert_eq!(
            create.validation_error(),
            Some("当前连接类型暂不支持创建表")
        );
        assert_eq!(
            create.sql_preview(),
            Err("当前连接类型暂不支持创建表".to_string())
        );
    }


    #[test]
    fn create_table_column_operations_reorder_and_remove() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));

        controller.dispatch(AppCommand::MoveCreateTableColumnUp {
            tab_id: TabId(1),
            column_id: 3,
        });
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(
            create.columns.iter().map(|column| column.id).collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        assert_eq!(create.selected_column_id, Some(3));

        controller.dispatch(AppCommand::MoveCreateTableColumnDown {
            tab_id: TabId(1),
            column_id: 3,
        });
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(
            create.columns.iter().map(|column| column.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(create.selected_column_id, Some(3));

        controller.dispatch(AppCommand::RemoveCreateTableColumn {
            tab_id: TabId(1),
            column_id: 3,
        });
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(
            create.columns.iter().map(|column| column.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(create.selected_column_id, Some(2));
    }


    #[test]
    fn create_table_clears_text_only_options_when_type_changes() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "events".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Name,
            value: "event_id".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::DataType,
            value: "varchar".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Charset,
            value: "utf8mb4".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Collation,
            value: "utf8mb4_unicode_ci".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 1,
            flag: CreateTableColumnFlag::Binary,
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::DataType,
            value: "int".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert!(!create.columns[0].binary);
        assert!(create.columns[0].charset.is_empty());
        assert!(create.columns[0].collation.is_empty());
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `events` (\n  `event_id` int NOT NULL,\n  PRIMARY KEY (`event_id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_binary_types_support_length() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "files".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Name,
            value: "digest".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::DataType,
            value: "varbinary".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.columns[0].length, "255");
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `files` (\n  `digest` varbinary(255) NOT NULL,\n  PRIMARY KEY (`digest`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_numeric_options_keep_digits_only() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::DataType,
            value: "varchar".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Length,
            value: "2a5;5".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::KeyLength,
            value: "1x0".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.columns[0].length, "255");
        assert_eq!(create.columns[0].key_length, "10");
    }

    #[test]
    fn create_table_primary_key_prefix_length_only_applies_to_supported_key_parts() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "codes".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Name,
            value: "tenant_id".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::KeyLength,
            value: "10".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "code".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 2,
            flag: CreateTableColumnFlag::PrimaryKey,
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::KeyLength,
            value: "20".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert!(create.columns[0].key_length.is_empty());
        assert_eq!(create.columns[1].key_length, "20");
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `codes` (\n  `tenant_id` int NOT NULL,\n  `code` varchar(255) NOT NULL,\n  PRIMARY KEY (`tenant_id`, `code`(20))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_indexes_join_sql_preview() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "status".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "varchar".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 3,
            field: CreateTableColumnField::Name,
            value: "serial_num".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableIndex(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexType,
            value: "UNIQUE".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableIndexColumnField {
            tab_id: TabId(1),
            index_id: 1,
            column_index: 0,
            field: CreateTableIndexColumnField::Name,
            value: "serial_num".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableIndexColumnField {
            tab_id: TabId(1),
            index_id: 1,
            column_index: 0,
            field: CreateTableIndexColumnField::SubPart,
            value: "12x".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.indexes[0].columns[0].sort_order, "");
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `orders` (\n  `id` int NOT NULL,\n  `status` varchar(255),\n  `serial_num` varchar(255),\n  PRIMARY KEY (`id`),\n  UNIQUE KEY `uk_orders_serial_num` (`serial_num`(12))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );

        controller.dispatch(AppCommand::SetCreateTableIndexColumnField {
            tab_id: TabId(1),
            index_id: 1,
            column_index: 0,
            field: CreateTableIndexColumnField::SortOrder,
            value: "desc".to_string(),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.indexes[0].name, "uk_orders_serial_num");
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `orders` (\n  `id` int NOT NULL,\n  `status` varchar(255),\n  `serial_num` varchar(255),\n  PRIMARY KEY (`id`),\n  UNIQUE KEY `uk_orders_serial_num` (`serial_num`(12) DESC)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_checks_join_mysql_sql_preview() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "users".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "age".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "int".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableCheck(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableCheckField {
            tab_id: TabId(1),
            check_id: 1,
            field: CreateTableCheckField::Name,
            value: "chk_users_age".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableCheckField {
            tab_id: TabId(1),
            check_id: 1,
            field: CreateTableCheckField::Expression,
            value: "age >= 0".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableCheckNotEnforced {
            tab_id: TabId(1),
            check_id: 1,
        });

        assert_eq!(
            create_table(&controller).sql_preview().unwrap(),
            "CREATE TABLE `users` (\n  `id` int NOT NULL,\n  `age` int,\n  PRIMARY KEY (`id`),\n  CONSTRAINT `chk_users_age` CHECK (age >= 0) NOT ENFORCED\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_checks_join_sqlite_sql_preview() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "users".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "status".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableCheck(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableCheckField {
            tab_id: TabId(1),
            check_id: 1,
            field: CreateTableCheckField::Expression,
            value: "status IN ('active', 'disabled')".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableCheckNotEnforced {
            tab_id: TabId(1),
            check_id: 1,
        });

        let create = create_table(&controller);
        assert!(!create.checks[0].not_enforced);
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE \"users\" (\n  \"id\" INTEGER NOT NULL,\n  \"status\" TEXT,\n  PRIMARY KEY (\"id\"),\n  CHECK (status IN ('active', 'disabled'))\n);"
        );
    }

    #[test]
    fn create_table_fulltext_and_spatial_indexes_clear_method() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::AddCreateTableIndex(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexType,
            value: "UNIQUE".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexMethod,
            value: "HASH".to_string(),
        });
        assert_eq!(create_table(&controller).indexes[0].index_method, "HASH");

        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexType,
            value: "FULLTEXT".to_string(),
        });
        assert_eq!(create_table(&controller).indexes[0].index_method, "");

        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexMethod,
            value: "BTREE".to_string(),
        });
        assert_eq!(create_table(&controller).indexes[0].index_method, "");

        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexType,
            value: "SPATIAL".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableIndexField {
            tab_id: TabId(1),
            index_id: 1,
            field: CreateTableIndexField::IndexMethod,
            value: "HASH".to_string(),
        });
        assert_eq!(create_table(&controller).indexes[0].index_method, "");
    }

    #[test]
    fn create_table_length_follows_data_type() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.columns[1].data_type, "varchar");
        assert_eq!(create.columns[1].length, "255");

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "int".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Length,
            value: "255".to_string(),
        });
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert!(create.columns[1].length.is_empty());

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "decimal".to_string(),
        });
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert_eq!(create.columns[1].length, "10");

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Scale,
            value: "2".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "timestamp".to_string(),
        });
        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert!(create.columns[1].length.is_empty());
        assert!(create.columns[1].scale.is_empty());
    }

    #[test]
    fn create_table_type_limited_flags_do_not_leak_into_sql() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "events".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Name,
            value: "created_at".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::DataType,
            value: "int".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 1,
            flag: CreateTableColumnFlag::AutoIncrement,
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 1,
            flag: CreateTableColumnFlag::Nullable,
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 1,
            flag: CreateTableColumnFlag::PrimaryKey,
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::DataType,
            value: "timestamp".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Length,
            value: String::new(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 1,
            flag: CreateTableColumnFlag::AutoUpdateTime,
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected create table tab");
        };
        assert!(!create.columns[0].auto_increment);
        assert!(create.columns[0].auto_update_time);
        assert_eq!(
            create.sql_preview().unwrap(),
            "CREATE TABLE `events` (\n  `created_at` timestamp ON UPDATE CURRENT_TIMESTAMP\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
        );
    }

    #[test]
    fn create_table_validation_reports_first_save_blocker() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(2),
            database: Some("main".to_string()),
        });

        let create = create_table(&controller);
        assert_eq!(create.validation_error(), Some("请输入表名"));

        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Name,
            value: String::new(),
        });
        let create = create_table(&controller);
        assert_eq!(create.validation_error(), Some("请至少填写一个字段"));

        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "id".to_string(),
        });
        let create = create_table(&controller);
        assert_eq!(create.validation_error(), Some("主键字段名不能为空"));

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 1,
            field: CreateTableColumnField::Name,
            value: "id".to_string(),
        });
        let create = create_table(&controller);
        assert_eq!(create.validation_error(), Some("字段名不能重复"));

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "amount".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: String::new(),
        });
        let create = create_table(&controller);
        assert_eq!(create.validation_error(), Some("字段类型不能为空"));

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DataType,
            value: "decimal".to_string(),
        });
        controller.dispatch(AppCommand::ToggleCreateTableColumnFlag {
            tab_id: TabId(1),
            column_id: 2,
            flag: CreateTableColumnFlag::AutoIncrement,
        });
        let create = create_table(&controller);
        assert_eq!(create.validation_error(), Some("自增字段必须是整数主键"));
    }

    #[test]
    fn create_table_text_default_value_requires_quotes() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "profiles".to_string(),
        });
        controller.dispatch(AppCommand::AddCreateTableColumn(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::Name,
            value: "nickname".to_string(),
        });
        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DefaultValue,
            value: "guest".to_string(),
        });
        let create = create_table(&controller);
        assert_eq!(
            create.validation_error(),
            Some("字符串默认值需要用引号包裹")
        );

        controller.dispatch(AppCommand::SetCreateTableColumnField {
            tab_id: TabId(1),
            column_id: 2,
            field: CreateTableColumnField::DefaultValue,
            value: "'guest'".to_string(),
        });
        let create = create_table(&controller);
        assert!(create.validation_error().is_none());
        assert!(create.sql_preview().unwrap().contains("DEFAULT 'guest'"));
    }

    #[test]
    fn create_table_apply_tracks_running_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::SetCreateTableField {
            tab_id: TabId(1),
            field: CreateTableField::TableName,
            value: "orders".to_string(),
        });

        let event = controller.dispatch(AppCommand::StartCreateTableApply(TabId(1)));
        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        assert!(create_table(&controller).applying);

        let event = controller.dispatch(AppCommand::ApplyCreateTable(TabId(1)));
        assert_eq!(event, AppEvent::CreateTableApplied(TabId(1)));

        let event = controller.dispatch(AppCommand::FinishCreateTableApply {
            tab_id: TabId(1),
            result: Ok(()),
        });
        assert_eq!(event, AppEvent::CreateTableApplied(TabId(1)));
        let create = create_table(&controller);
        assert!(!create.applying);
        assert!(create.apply_error.is_none());
        assert!(!controller.state().tabs[0].dirty);
    }

    #[test]
    fn opens_settings_tab_once_and_reactivates_it() {
        let mut controller = AppController::with_mock_data();

        let first = controller.dispatch(AppCommand::OpenSettings);
        let second = controller.dispatch(AppCommand::OpenSettings);

        assert_eq!(first, AppEvent::TabOpened(TabId(1)));
        assert_eq!(second, AppEvent::TabActivated(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert!(matches!(controller.state().tabs[0].kind, TabKind::Settings));
    }

    #[test]
    fn closes_active_tab_and_selects_previous() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));

        let event = controller.dispatch(AppCommand::CloseTab(TabId(2)));

        assert_eq!(event, AppEvent::TabClosed(TabId(2)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().active_tab, Some(TabId(1)));
    }

    #[test]
    fn activate_missing_tab_sets_user_error() {
        let mut controller = AppController::new();
        let event = controller.dispatch(AppCommand::ActivateTab(TabId(404)));

        assert!(matches!(event, AppEvent::Failed(_)));
        assert!(controller.state().last_error.is_some());
    }

    #[test]
    fn deactivate_tab_keeps_tabs_and_clears_active() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        assert_eq!(controller.state().active_tab, Some(TabId(2)));

        let event = controller.dispatch(AppCommand::DeactivateTab);

        // 回首页：仅取消聚焦，已打开标签保留在标签栏（非破坏性）。
        assert!(matches!(event, AppEvent::TabActivated(TabId(0))));
        assert_eq!(controller.state().active_tab, None);
        assert_eq!(controller.state().tabs.len(), 2);
    }

    #[test]
    fn dirty_tab_close_is_requested_before_removal() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.state.tabs[0].dirty = true;

        let event = controller.dispatch(AppCommand::CloseTab(TabId(1)));

        assert_eq!(event, AppEvent::TabCloseRequested(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().pending_dirty_tab_close, Some(TabId(1)));
    }

    #[test]
    fn confirms_dirty_tab_close_after_request() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.state.tabs[0].dirty = true;
        controller.dispatch(AppCommand::CloseTab(TabId(1)));

        let event = controller.dispatch(AppCommand::ConfirmCloseDirtyTab(TabId(1)));

        assert_eq!(event, AppEvent::TabClosed(TabId(1)));
        assert!(controller.state().tabs.is_empty());
        assert_eq!(controller.state().active_tab, None);
        assert_eq!(controller.state().pending_dirty_tab_close, None);
    }

    #[test]
    fn cancels_dirty_tab_close_request() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.state.tabs[0].dirty = true;
        controller.dispatch(AppCommand::CloseTab(TabId(1)));

        let event = controller.dispatch(AppCommand::CancelCloseDirtyTab(TabId(1)));

        assert_eq!(event, AppEvent::TabCloseCancelled(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().active_tab, Some(TabId(1)));
        assert_eq!(controller.state().pending_dirty_tab_close, None);
    }

    #[test]
    fn replace_connections_updates_next_connection_id() {
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
            id: ConnectionId(7),
            name: "stored".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: "stored.db".into(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }]));

        let event = controller.dispatch(AppCommand::CreateConnection(ConnectionDraft {
            name: "next".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: "next.db".into(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }));

        assert!(matches!(
            event,
            AppEvent::ConnectionCreated(ConnectionConfig {
                id: ConnectionId(8),
                ..
            })
        ));
    }

    #[test]
    fn update_connection_replaces_existing_config() {
        let mut controller = AppController::with_mock_data();
        let mut config = controller.state().connections[0].config.clone();
        config.name = "renamed".to_string();
        config.endpoint = Endpoint::SqliteFile {
            path: "renamed.db".into(),
            read_only: false,
        };

        let event = controller.dispatch(AppCommand::UpdateConnection(config.clone()));

        assert_eq!(event, AppEvent::ConnectionUpdated(config));
        assert_eq!(controller.state().connections.len(), 2);
        assert_eq!(controller.state().connections[0].config.name, "renamed");
        assert_eq!(controller.state().connections[0].config.id, ConnectionId(1));
    }

    #[test]
    fn create_connections_assign_distinct_credential_refs_for_same_name() {
        let mut controller = AppController::new();
        let draft = |kind, password| ConnectionDraft {
            name: "开发环境".to_string(),
            kind,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: None,
            },
            credential_ref: Some("gdb.connection.pending.开发环境".to_string()),
            options: std::collections::BTreeMap::from([("password".to_string(), password)]),
            redis_profile: None,
            mysql_profile: None,
        };

        let first = controller.dispatch(AppCommand::CreateConnection(draft(
            DatabaseKind::Redis,
            "redis-password".to_string(),
        )));
        let second = controller.dispatch(AppCommand::CreateConnection(draft(
            DatabaseKind::MySql,
            "mysql-password".to_string(),
        )));

        let AppEvent::ConnectionCreated(first) = first else {
            panic!("expected first connection to be created");
        };
        let AppEvent::ConnectionCreated(second) = second else {
            panic!("expected second connection to be created");
        };
        assert_eq!(first.credential_ref.as_deref(), Some("gdb.connection.1"));
        assert_eq!(second.credential_ref.as_deref(), Some("gdb.connection.2"));
    }

    #[test]
    fn sqlite_test_connection_uses_real_connector() {
        let mut controller = AppController::new();

        let event = controller.dispatch(AppCommand::TestConnection(ConnectionConfig {
            id: ConnectionId(3),
            name: "memory".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: ":memory:".into(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }));

        assert!(matches!(
            event,
            AppEvent::ConnectionTested(ConnectionId(3), Ok(()))
        ));
    }

    #[test]
    fn create_database_rejects_unsupported_connection_kind() {
        let mut controller = AppController::with_mock_data();

        let event = controller.dispatch(AppCommand::CreateDatabase(CreateDatabaseRequest {
            connection_id: ConnectionId(1),
            name: "new_db".to_string(),
            charset: "utf8mb4".to_string(),
            collation: "utf8mb4_unicode_ci".to_string(),
            path: None,
        }));

        let AppEvent::Failed(error) = event else {
            panic!("expected unsupported create database to fail");
        };
        assert_eq!(error.message, "暂不支持新建数据库");
    }

    #[test]
    fn delete_database_rejects_unsupported_connection_kind() {
        let mut controller = AppController::with_mock_data();

        let event = controller.dispatch(AppCommand::DeleteDatabase {
            connection_id: ConnectionId(1),
            database: "app".to_string(),
        });

        let AppEvent::Failed(error) = event else {
            panic!("expected unsupported delete database to fail");
        };
        assert_eq!(error.message, "暂不支持删除数据库");
    }

    #[test]
    fn sqlite_create_database_attaches_to_current_connection() {
        let dir = std::env::temp_dir().join(format!(
            "gdb-sqlite-attach-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let main_path = dir.join("main.db");
        let attached_path = dir.join("analytics.db");
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
            id: ConnectionId(9),
            name: "SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: main_path,
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }]));

        let event = controller.dispatch(AppCommand::CreateDatabase(CreateDatabaseRequest {
            connection_id: ConnectionId(9),
            name: "analytics".to_string(),
            charset: String::new(),
            collation: String::new(),
            path: Some(attached_path.clone()),
        }));

        assert!(matches!(
            event,
            AppEvent::ObjectsLoaded(None, objects)
                if objects.iter().any(|object| object.path.name == "analytics")
        ));
        let config = &controller.state().connections[0].config;
        assert_eq!(
            sqlite_attached_database_path(config, "analytics"),
            Some(attached_path.clone())
        );
        assert_eq!(
            sqlite_attached_database_path(config, "Analytics"),
            Some(attached_path.clone())
        );

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&attached_path)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY)")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let parent = ObjectPath {
            connection_id: ConnectionId(9),
            database: Some("analytics".to_string()),
            schema: None,
            name: "analytics".to_string(),
            kind: ObjectKind::Database,
        };
        let event = controller.dispatch(AppCommand::LoadObjectChildren(parent.clone()));
        assert!(matches!(
            event,
            AppEvent::ObjectsLoaded(Some(loaded_parent), objects)
                if loaded_parent == parent
                    && objects.iter().any(|object| object.path.database.as_deref() == Some("analytics")
                        && object.path.name == "events")
        ));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disconnect_connection_clears_session_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));

        let event = controller.dispatch(AppCommand::DisconnectConnection(ConnectionId(1)));

        assert!(matches!(event, AppEvent::ObjectsLoaded(None, objects) if objects.is_empty()));
        let connection = &controller.state().connections[0];
        assert!(!connection.connected);
        assert!(!connection.expanded);
        assert!(connection.objects.is_empty());
    }

    #[test]
    fn merging_slow_connection_load_preserves_newer_tabs_and_other_connections() {
        let mut current = AppController::with_mock_data();
        let mut slow_load = current.clone();
        slow_load.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let object = slow_load.state().connections[0].objects[0].path.clone();

        current.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        current.dispatch(AppCommand::OpenDataEditor(object));
        current.state.connections[1].connected = true;
        current.state.connections[1].expanded = true;

        current.merge_open_connection_from(&slow_load, ConnectionId(1));

        assert_eq!(current.state().tabs.len(), 2);
        assert_eq!(current.state().active_tab, Some(TabId(2)));
        assert!(matches!(
            current.state().active_tab().map(|tab| &tab.kind),
            Some(TabKind::DataEditor(_))
        ));
        assert!(current.state().connections[0].connected);
        assert!(current.state().connections[0].expanded);
        assert!(current.state().connections[1].connected);
        assert!(current.state().connections[1].expanded);
    }


    #[test]
    fn disconnect_database_closes_only_that_database_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let object = controller.state().connections[0]
            .objects
            .iter()
            .find(|object| object.path.kind == ObjectKind::Table)
            .expect("expected table object")
            .path
            .clone();
        controller.dispatch(AppCommand::OpenDataEditor(object));
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("other".to_string()),
        });

        controller.dispatch(AppCommand::DisconnectDatabase {
            connection_id: ConnectionId(1),
            database: "main".to_string(),
        });

        let connection = &controller.state().connections[0];
        assert!(connection.connected);
        assert!(connection.expanded);
        assert_eq!(connection.objects.len(), 1);
        assert_eq!(connection.objects[0].path.kind, ObjectKind::Database);
        assert_eq!(connection.objects[0].path.name, "main");
        assert_eq!(controller.state().tabs.len(), 1);
        assert!(matches!(
            &controller.state().tabs[0].kind,
            TabKind::QueryEditor(QueryEditorState {
                connection_id: ConnectionId(1),
                database: Some(database),
                ..
            }) if database == "other"
        ));
        assert_eq!(controller.state().active_tab, Some(TabId(3)));
    }

    #[test]
    fn delete_connection_removes_connection_and_related_tabs() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(2)));

        let event = controller.dispatch(AppCommand::DeleteConnection(ConnectionId(1)));

        assert_eq!(event, AppEvent::ConnectionDeleted(ConnectionId(1)));
        assert_eq!(controller.state().connections.len(), 1);
        assert_eq!(controller.state().connections[0].config.id, ConnectionId(2));
        assert_eq!(controller.state().tabs.len(), 1);
        assert!(matches!(
            controller.state().tabs[0].kind,
            TabKind::QueryEditor(QueryEditorState {
                connection_id: ConnectionId(2),
                ..
            })
        ));
        assert_eq!(controller.state().active_tab, Some(TabId(2)));
    }

    #[test]
    fn disconnect_connection_closes_related_tabs() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(2)));

        let event = controller.dispatch(AppCommand::DisconnectConnection(ConnectionId(1)));

        assert!(matches!(event, AppEvent::ObjectsLoaded(None, objects) if objects.is_empty()));
        assert_eq!(controller.state().tabs.len(), 1);
        assert!(matches!(
            controller.state().tabs[0].kind,
            TabKind::QueryEditor(QueryEditorState {
                connection_id: ConnectionId(2),
                ..
            })
        ));
        assert_eq!(controller.state().active_tab, Some(TabId(2)));
    }

    #[test]
    fn query_saved_fingerprint_tracks_unsaved_changes() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select 1".to_string(),
        });
        assert!(controller.state().tabs[0].dirty);

        controller.dispatch(AppCommand::MarkQuerySaved {
            tab_id: TabId(1),
            title: "saved".to_string(),
            origin: QueryOrigin::Connection { query_id: 10 },
        });
        assert!(!controller.state().tabs[0].dirty);

        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select 2".to_string(),
        });
        assert!(controller.state().tabs[0].dirty);

        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select 1".to_string(),
        });
        assert!(!controller.state().tabs[0].dirty);
    }

    #[test]
    fn connection_group_moves_and_delete_keeps_connections() {
        let mut controller = AppController::with_mock_data();
        let event = controller.dispatch(AppCommand::CreateConnectionGroup("开发".to_string()));
        let AppEvent::ConnectionGroupCreated(group) = event else {
            panic!("expected group created");
        };

        controller.dispatch(AppCommand::MoveConnectionToGroup {
            connection_id: ConnectionId(1),
            group_id: group.id,
        });

        assert_eq!(
            controller
                .state()
                .sidebar_layout
                .connection_group(ConnectionId(1)),
            Some(group.id)
        );

        controller.dispatch(AppCommand::DeleteConnectionGroup(group.id));

        assert_eq!(controller.state().connections.len(), 2);
        assert!(
            !controller
                .state()
                .sidebar_layout
                .is_connection_grouped(ConnectionId(1))
        );
    }

    #[test]
    fn toggle_connection_expanded_only_changes_connected_connection_tree() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        assert!(controller.state().connections[0].expanded);

        controller.dispatch(AppCommand::ToggleConnectionExpanded(ConnectionId(1)));
        assert!(!controller.state().connections[0].expanded);

        controller.dispatch(AppCommand::ToggleConnectionExpanded(ConnectionId(1)));
        assert!(controller.state().connections[0].expanded);

        controller.dispatch(AppCommand::ToggleConnectionExpanded(ConnectionId(2)));
        assert!(!controller.state().connections[1].expanded);
    }

    #[test]
    fn redis_cloud_discovery_infers_resource_metadata() {
        // Azure 托管连接串：自动发现应记录云元数据（provider + resource），
        // 供弹框回填与后续 Browser/Workbench 复用。
        let mut controller = AppController::new();
        let event = controller.dispatch(AppCommand::DiscoverRedisConnection {
            provider: "azure".to_string(),
            connection_string: "rediss://:pass@mycache.redis.cache.windows.net:6380/0"
                .to_string(),
        });
        let AppEvent::RedisConnectionDiscovered(draft) = event else {
            panic!("期望 Azure 连接串解析成功，实际事件: {event:?}");
        };
        let profile = draft.redis_profile.expect("发现结果应携带 redis_profile");
        assert_eq!(profile.cloud.provider, "azure");
        assert_eq!(profile.cloud.resource, "mycache");
        assert_eq!(draft.name, "mycache.redis.cache.windows.net:6380");
        // 导入来源元数据不得携带明文口令。
        assert_eq!(
            profile.cloud.imported_name,
            "rediss://@mycache.redis.cache.windows.net:6380/0"
        );
    }

    #[test]
    fn redis_cloud_discovery_rejects_invalid_uri() {
        // 非法连接串应走 fail 分支返回 Failed，而不是出现 panic。
        let mut controller = AppController::new();
        let event = controller.dispatch(AppCommand::DiscoverRedisConnection {
            provider: "azure".to_string(),
            connection_string: "不是合法的 URI".to_string(),
        });
        assert!(matches!(event, AppEvent::Failed(_)), "期望 Failed，实际: {event:?}");
    }
