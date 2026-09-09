    #[test]
    fn open_data_editor_opens_loading_tab_before_data_load() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let object = controller.state().connections[0].objects[1].path.clone();

        let event = controller.dispatch(AppCommand::OpenDataEditor(object));

        assert_eq!(event, AppEvent::TabOpened(TabId(1)));
        let Some(TabKind::DataEditor(editor)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active data editor tab");
        };
        assert!(editor.page.is_none());
        assert!(editor.loading);
        assert!(editor.error.is_none());
    }

    #[test]
    fn load_data_page_loads_first_page_from_connector() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let object = controller.state().connections[0].objects[1].path.clone();
        controller.dispatch(AppCommand::OpenDataEditor(object));

        let event = controller.dispatch(AppCommand::LoadDataPage(TabId(1)));

        assert!(matches!(event, AppEvent::DataLoaded(TabId(1), _)));
        let Some(TabKind::DataEditor(editor)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active data editor tab");
        };
        let page = editor.page.as_ref().expect("data page should be loaded");
        assert_eq!(page.columns[0].name, "id");
        assert_eq!(page.rows.len(), 2);
        assert!(!editor.loading);
        assert!(editor.error.is_none());
    }

    #[test]
    fn set_data_page_pagination_updates_editor_limit() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let object = controller.state().connections[0].objects[1].path.clone();
        controller.dispatch(AppCommand::OpenDataEditor(object));

        controller.dispatch(AppCommand::SetDataPagePagination {
            tab_id: TabId(1),
            offset: 0,
            limit: 50,
        });

        let Some(TabKind::DataEditor(editor)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active data editor tab");
        };
        assert_eq!(editor.pagination.limit, 50);
    }

    #[test]
    fn table_info_state_tracks_drawer_loading_and_controls() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::ToggleTableInfo {
            tab_id: TabId(1),
            tab: TableInfoTab::Indexes,
        });

        assert_eq!(
            event,
            AppEvent::TableInfoChanged(TabId(1), TableInfoTab::Indexes)
        );
        let editor = active_editor(&controller);
        assert!(editor.table_info.open);
        assert_eq!(editor.table_info.active_tab, TableInfoTab::Indexes);
        assert!(matches!(editor.table_info.indexes, LoadState::Loading));

        let load_event = controller.dispatch(AppCommand::LoadTableInfo {
            tab_id: TabId(1),
            tab: TableInfoTab::Indexes,
        });
        let AppEvent::TableInfoLoaded(_, _, result) = load_event else {
            panic!("expected table info loaded");
        };

        controller.dispatch(AppCommand::FinishTableInfoLoad {
            tab_id: TabId(1),
            tab: TableInfoTab::Indexes,
            result: Ok(result),
        });
        controller.dispatch(AppCommand::SetTableInfoSearch {
            tab_id: TabId(1),
            search: "id".to_string(),
        });
        controller.dispatch(AppCommand::SetTableInfoWidth {
            tab_id: TabId(1),
            width: 999.,
        });
        controller.dispatch(AppCommand::ToggleDdlWrap(TabId(1)));

        let editor = active_editor(&controller);
        assert!(matches!(editor.table_info.indexes, LoadState::Loaded(_)));
        assert_eq!(editor.table_info.search, "id");
        assert_eq!(editor.table_info.width, 800.);
        assert!(editor.table_info.ddl_wrap);
    }

    #[test]
    fn close_table_info_closes_from_any_tab() {
        let mut controller = controller_with_data_editor();

        controller.dispatch(AppCommand::SelectTableInfoTab {
            tab_id: TabId(1),
            tab: TableInfoTab::Ddl,
        });
        controller.dispatch(AppCommand::CloseTableInfo(TabId(1)));

        assert!(!active_editor(&controller).table_info.open);
        assert_eq!(
            active_editor(&controller).table_info.active_tab,
            TableInfoTab::Ddl
        );
    }

    #[test]
    fn table_info_column_click_records_highlighted_column() {
        let mut controller = controller_with_data_editor();

        controller.dispatch(AppCommand::HighlightDataColumn {
            tab_id: TabId(1),
            column: "name".to_string(),
        });

        assert_eq!(
            active_editor(&controller).table_info.highlighted_column,
            Some("name".to_string())
        );
    }

    #[test]
    fn format_sql_text_formats_create_table_ddl() {
        let formatted = format_sql_text_for_dialect(
            "create table users(id integer primary key,name text not null,foreign key(team_id) references teams(id));",
            DatabaseKind::MySql,
        );

        assert_eq!(
            formatted,
            "CREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  name TEXT NOT NULL,\n  FOREIGN KEY (team_id) REFERENCES teams(id)\n);"
        );
    }

    #[test]
    fn format_sql_text_uses_formatter_for_regular_query() {
        let formatted = format_sql_text_for_dialect(
            "select id, name from users where id = 1 order by name",
            DatabaseKind::MySql,
        );

        assert!(formatted.starts_with("SELECT"));
        assert!(formatted.contains('\n'));
    }

    #[test]
    fn format_sql_text_keeps_digit_prefixed_mysql_identifier() {
        let formatted =
            format_sql_text_for_dialect("select * from 3d_dental_order;", DatabaseKind::MySql);

        assert!(formatted.contains("FROM\n  3d_dental_order"), "{formatted}");
        assert!(!formatted.contains("3 d_dental_order"), "{formatted}");
    }

    #[test]
    fn format_sql_text_treats_tidb_like_mysql() {
        let formatted =
            format_sql_text_for_dialect("select * from 3d_dental_order;", DatabaseKind::TiDb);

        assert!(formatted.contains("FROM\n  3d_dental_order"), "{formatted}");
        assert!(!formatted.contains("3 d_dental_order"), "{formatted}");
    }

    #[test]
    fn format_sql_text_keeps_create_table_suffix_readable() {
        let formatted = format_sql_text_for_dialect(
            "create table users(id int primary key) engine=InnoDB default charset=utf8mb4",
            DatabaseKind::MySql,
        );

        assert_eq!(
            formatted,
            "CREATE TABLE users (\n  id INT PRIMARY KEY\n) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4"
        );
    }

    #[test]
    fn format_sql_text_aligns_mysql_create_table_columns() {
        let formatted = format_sql_text_for_dialect(
            "create table `demo` (`id` varchar(64) not null default '' comment '主键', `name` varchar(128) default null comment '名称', `created_at` timestamp(6) not null default current_timestamp(6) comment '创建时间', primary key (`id`) using btree, key `idx_name` (`name`) using btree) engine=InnoDB default charset=utf8mb4 comment='示例表';",
            DatabaseKind::MySql,
        );

        assert!(formatted.contains("`id`          VARCHAR(64)"));
        assert!(formatted.contains("`name`        VARCHAR(128)"));
        assert!(formatted.contains("TIMESTAMP(6)  NOT NULL"));
        assert!(formatted.contains("COMMENT '创建时间'"));
        assert!(
            formatted.contains("  PRIMARY KEY (`id`) USING BTREE,"),
            "{formatted}"
        );
        assert!(formatted.contains("  KEY `idx_name` (`name`) USING BTREE"));
    }

    #[test]
    fn format_sql_text_aligns_each_mysql_create_table_statement() {
        let formatted = format_sql_text_for_dialect(
            "create table `first` (`id` varchar(64) not null default '' comment '主键', `created_at` timestamp(6) not null default current_timestamp(6) comment '创建时间'); create table `second` (`id` varchar(64) not null default '' comment '主键', `created_at` timestamp(6) not null default current_timestamp(6) comment '创建时间');",
            DatabaseKind::MySql,
        );

        let first_id = formatted
            .lines()
            .position(|line| line.contains("CREATE TABLE `first`"))
            .and_then(|index| formatted.lines().nth(index + 1))
            .unwrap();
        let second_id = formatted
            .lines()
            .position(|line| line.contains("CREATE TABLE `second`"))
            .and_then(|index| formatted.lines().nth(index + 1))
            .unwrap();

        assert!(first_id.contains("`id`          VARCHAR(64)"));
        assert!(second_id.contains("`id`          VARCHAR(64)"));
    }

    #[test]
    fn format_sql_text_aligns_mysql_create_table_attrs() {
        let formatted = format_sql_text_for_dialect(
            "create table `demo` (`id` varchar(64) collate utf8mb4_general_ci not null default '' comment '主键', `source` varchar(64) collate utf8mb4_general_ci not null default 'implantCloud' comment '来源', `created_at` datetime(6) not null default current_timestamp(6) comment '创建时间') engine=InnoDB;",
            DatabaseKind::MySql,
        );
        let id = formatted.lines().find(|line| line.contains("`id`")).unwrap();
        let source = formatted
            .lines()
            .find(|line| line.contains("`source`"))
            .unwrap();
        let created_at = formatted
            .lines()
            .find(|line| line.contains("`created_at`"))
            .unwrap();

        assert_eq!(id.find("COLLATE"), source.find("COLLATE"));
        assert_eq!(id.find("NOT NULL"), source.find("NOT NULL"));
        assert_eq!(id.find("NOT NULL"), created_at.find("NOT NULL"));
        assert_eq!(id.find("DEFAULT"), source.find("DEFAULT"));
        assert_eq!(id.find("DEFAULT"), created_at.find("DEFAULT"));
        assert_eq!(id.find("COMMENT"), source.find("COMMENT"));
        assert_eq!(id.find("COMMENT"), created_at.find("COMMENT"));
    }

    #[test]
    fn create_table_sql_preview_includes_mysql_foreign_key() {
        let mut create =
            CreateTableState::new(ConnectionId(1), Some("app".to_string()), DatabaseKind::MySql);
        create.set_field(CreateTableField::TableName, "orders".to_string());
        create.add_column();
        let column_id = create.columns[1].id;
        create.set_column_field(column_id, CreateTableColumnField::Name, "user_id".to_string());
        create.set_column_field(column_id, CreateTableColumnField::DataType, "int".to_string());
        create.add_foreign_key();
        let foreign_key_id = create.foreign_keys[0].id;
        create.set_foreign_key_field(
            foreign_key_id,
            CreateTableForeignKeyField::ReferencedDatabase,
            "auth".to_string(),
        );
        create.set_foreign_key_field(
            foreign_key_id,
            CreateTableForeignKeyField::ReferencedTable,
            "users".to_string(),
        );
        create.add_foreign_key_referenced_column(foreign_key_id);
        create.set_foreign_key_referenced_column(
            foreign_key_id,
            0,
            "id".to_string(),
        );
        create.set_foreign_key_field(
            foreign_key_id,
            CreateTableForeignKeyField::OnDelete,
            "CASCADE".to_string(),
        );

        let sql = create.sql_preview().unwrap();

        assert!(sql.contains(
            "CONSTRAINT `fk_orders_id` FOREIGN KEY (`id`) REFERENCES `auth`.`users` (`id`) ON DELETE CASCADE"
        ));
    }

    #[test]
    fn create_table_sql_preview_includes_mysql_trigger_body() {
        let mut create =
            CreateTableState::new(ConnectionId(1), Some("app".to_string()), DatabaseKind::MySql);
        create.set_field(CreateTableField::TableName, "users".to_string());
        create.add_trigger();
        let trigger_id = create.triggers[0].id;
        create.set_trigger_field(
            trigger_id,
            CreateTableTriggerField::Name,
            "users_bu".to_string(),
        );
        create.set_trigger_field(
            trigger_id,
            CreateTableTriggerField::Timing,
            "BEFORE".to_string(),
        );
        create.set_trigger_event(trigger_id, CreateTableTriggerEvent::Update);
        create.set_trigger_field(
            trigger_id,
            CreateTableTriggerField::Body,
            "BEGIN\n  SET NEW.updated_at = CURRENT_TIMESTAMP;\nEND".to_string(),
        );

        let sql = create.sql_preview().unwrap();

        assert!(sql.contains(
            "CREATE TRIGGER `users_bu`\nBEFORE UPDATE ON `users`\nFOR EACH ROW\nBEGIN\n  SET NEW.updated_at = CURRENT_TIMESTAMP;\nEND;"
        ));
    }

    #[test]
    fn design_table_populates_existing_metadata_without_duplicate_sql() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("app".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let create = CreateTableState::design(
            object,
            DatabaseKind::MySql,
            vec![CompletionColumn {
                table: "users".to_string(),
                name: "email".to_string(),
                type_name: Some("varchar(255)".to_string()),
                nullable: false,
                primary_key: false,
                comment: Some("邮箱".to_string()),
            }, CompletionColumn {
                table: "users".to_string(),
                name: "team_id".to_string(),
                type_name: Some("int".to_string()),
                nullable: true,
                primary_key: false,
                comment: None,
            }],
            vec![IndexInfo {
                name: "users_email_idx".to_string(),
                columns: vec!["email".to_string()],
                is_unique: true,
                is_primary: false,
                index_type: Some("BTREE".to_string()),
                comment: None,
            }],
            vec![ForeignKeyInfo {
                name: "fk_users_team".to_string(),
                column: "team_id".to_string(),
                ref_schema: Some("app".to_string()),
                ref_table: "teams".to_string(),
                ref_column: "id".to_string(),
            }],
            vec![TriggerInfo {
                name: "users_bu".to_string(),
                timing: "BEFORE".to_string(),
                event: "UPDATE".to_string(),
                body: Some("BEGIN\n  SELECT NEW.id;\nEND".to_string()),
            }],
            Some(
                "CREATE TABLE `users` (\n  `email` varchar(255) NOT NULL,\n  CONSTRAINT `chk_email` CHECK (`email` <> '')\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 ROW_FORMAT=DYNAMIC"
                    .to_string(),
            ),
        );

        assert_eq!(create.indexes[0].name, "users_email_idx");
        assert_eq!(create.foreign_keys[0].referenced_table, "teams");
        assert_eq!(create.checks[0].name, "chk_email");
        assert_eq!(create.triggers[0].body, "BEGIN\n  SELECT NEW.id;\nEND");
        assert_eq!(create.engine, "InnoDB");
        assert_eq!(create.charset, "utf8mb4");
        assert_eq!(create.row_format, "DYNAMIC");
        assert_eq!(create.validation_error(), Some("没有需要保存的变更"));
    }

    #[test]
    fn sqlite_design_table_rebuilds_when_existing_column_type_changes() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let mut create = CreateTableState::design(
            object,
            DatabaseKind::Sqlite,
            vec![
                CompletionColumn {
                    table: "users".to_string(),
                    name: "id".to_string(),
                    type_name: Some("integer".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                CompletionColumn {
                    table: "users".to_string(),
                    name: "name".to_string(),
                    type_name: Some("text".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(
                "CREATE TABLE \"users\" (\n  \"id\" INTEGER NOT NULL,\n  \"name\" TEXT,\n  PRIMARY KEY (\"id\")\n)"
                    .to_string(),
            ),
        );

        create.set_column_field(2, CreateTableColumnField::DataType, "integer".to_string());

        let sql = create.sql_preview().unwrap();

        assert!(sql.contains("PRAGMA foreign_keys = OFF;"));
        assert!(sql.contains("ALTER TABLE \"users\" RENAME TO \"__gdb_rebuild_users\";"));
        assert!(sql.contains("\"name\" INTEGER"));
        assert!(sql.contains(
            "INSERT INTO \"users\" (\"id\", \"name\") SELECT \"id\", \"name\" FROM \"__gdb_rebuild_users\";"
        ));
        assert!(sql.contains("DROP TABLE \"__gdb_rebuild_users\";"));
        assert!(!sql.contains("暂不支持"));
    }

    #[test]
    fn create_table_validation_checks_foreign_key_target_field_count() {
        let mut create =
            CreateTableState::new(ConnectionId(1), Some("main".to_string()), DatabaseKind::MySql);
        create.set_field(CreateTableField::TableName, "orders".to_string());
        create.add_foreign_key();
        let foreign_key_id = create.foreign_keys[0].id;
        create.set_foreign_key_field(
            foreign_key_id,
            CreateTableForeignKeyField::ReferencedTable,
            "users".to_string(),
        );
        create.add_foreign_key_referenced_column(foreign_key_id);
        create.set_foreign_key_referenced_column(foreign_key_id, 0, "id".to_string());
        create.add_foreign_key_referenced_column(foreign_key_id);
        create.set_foreign_key_referenced_column(foreign_key_id, 1, "code".to_string());

        assert_eq!(
            create.validation_error(),
            Some("目标字段数量需要与外键字段一致")
        );
    }

    #[test]
    fn rename_table_sql_preview_quotes_for_dialect() {
        assert_eq!(
            rename_table_sql_preview(DatabaseKind::MySql, "orders", "orders_2026").unwrap(),
            "ALTER TABLE `orders` RENAME TO `orders_2026`;"
        );
        assert_eq!(
            rename_table_sql_preview(DatabaseKind::Sqlite, "order log", "order log old").unwrap(),
            "ALTER TABLE \"order log\" RENAME TO \"order log old\";"
        );
    }

    #[test]
    fn copy_table_sql_preview_uses_insert_select_when_copying_data() {
        assert_eq!(
            copy_table_sql_preview(DatabaseKind::MySql, "orders", "orders_copy", false).unwrap(),
            "CREATE TABLE `orders_copy` LIKE `orders`;"
        );
        assert_eq!(
            copy_table_sql_preview(DatabaseKind::MySql, "orders", "orders_copy", true).unwrap(),
            "CREATE TABLE `orders_copy` LIKE `orders`;\nINSERT INTO `orders_copy` SELECT * FROM `orders`;"
        );
        assert_eq!(
            copy_table_sql_preview(DatabaseKind::Sqlite, "event log", "event log copy", true)
                .unwrap(),
            "CREATE TABLE \"event log copy\" AS SELECT * FROM \"event log\" WHERE 0;\nINSERT INTO \"event log copy\" SELECT * FROM \"event log\";"
        );
        assert_eq!(
            copy_table_sql_preview_with_source_ddl(
                DatabaseKind::Sqlite,
                "event log",
                "event log copy",
                true,
                Some(
                    "CREATE TABLE \"event log\" (id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'new')"
                )
            )
            .unwrap(),
            "CREATE TABLE \"event log copy\" (id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'new');\nINSERT INTO \"event log copy\" SELECT * FROM \"event log\";"
        );
    }

    #[test]
    fn danger_table_sql_preview_quotes_for_dialect() {
        assert_eq!(
            drop_table_sql_preview(
                DatabaseKind::MySql,
                "3d_attachment",
                ForeignKeyCheckMode::Default
            )
            .unwrap(),
            "DROP TABLE `3d_attachment`;"
        );
        assert_eq!(
            truncate_table_sql_preview(
                DatabaseKind::MySql,
                "3d_attachment",
                ForeignKeyCheckMode::Default
            )
            .unwrap(),
            "TRUNCATE TABLE `3d_attachment`;"
        );
        assert_eq!(
            truncate_table_sql_preview(
                DatabaseKind::Sqlite,
                "event log",
                ForeignKeyCheckMode::Default
            )
            .unwrap(),
            "DELETE FROM \"event log\";"
        );
        assert_eq!(
            drop_table_sql_preview(
                DatabaseKind::MySql,
                "3d_attachment",
                ForeignKeyCheckMode::Disable
            )
            .unwrap(),
            "SET FOREIGN_KEY_CHECKS = 0;\nDROP TABLE `3d_attachment`;"
        );
    }

    #[test]
    fn create_table_reference_column_load_populates_foreign_key_options() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::AddCreateTableForeignKey(TabId(1)));
        controller.dispatch(AppCommand::SetCreateTableForeignKeyField {
            tab_id: TabId(1),
            foreign_key_id: 1,
            field: CreateTableForeignKeyField::ReferencedTable,
            value: "users".to_string(),
        });
        controller.dispatch(AppCommand::StartCreateTableReferenceColumnsLoad {
            tab_id: TabId(1),
            foreign_key_id: 1,
        });

        let event = controller.dispatch(AppCommand::LoadCreateTableReferenceColumns {
            tab_id: TabId(1),
            foreign_key_id: 1,
        });
        let AppEvent::CreateTableReferenceColumnsLoaded { columns, .. } = event else {
            panic!("expected target columns to load");
        };
        controller.dispatch(AppCommand::FinishCreateTableReferenceColumnsLoad {
            tab_id: TabId(1),
            foreign_key_id: 1,
            result: Ok(columns),
        });

        let Some(TabKind::CreateTable(create)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active create table tab");
        };
        assert!(matches!(
            &create.foreign_keys[0].referenced_column_options,
            LoadState::Loaded(columns) if columns.iter().any(|column| column == "id")
        ));
    }
