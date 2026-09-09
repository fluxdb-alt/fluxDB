    #[test]
    fn apply_data_changes_uses_real_sqlite_connector_for_non_demo_connection() {
        let path = temp_sqlite_path("app-apply");
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "Real SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO products (id, name) VALUES (1, 'Road Bike')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "products".to_string(),
            kind: ObjectKind::Table,
        }));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Touring Bike".to_string()),
        });

        let event = controller.dispatch(AppCommand::ApplyDataChanges(TabId(1)));

        assert!(matches!(event, AppEvent::DataLoaded(TabId(1), _)));
        assert!(!controller.state().active_tab().unwrap().dirty);

        let name: String = runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .connect()
                .await
                .unwrap();
            let row = sqlx::query("SELECT name FROM products WHERE id = 1")
                .fetch_one(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
            row.try_get("name").unwrap()
        });

        assert_eq!(name, "Touring Bike");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn apply_data_changes_reloads_page_after_insert() {
        let path = temp_sqlite_path("app-apply-refresh");
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "Real SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE products (
                    id INTEGER PRIMARY KEY,
                    name TEXT DEFAULT 'friend'
                )",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        });

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "products".to_string(),
            kind: ObjectKind::Table,
        }));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));
        controller.dispatch(AppCommand::InsertDataRow {
            tab_id: TabId(1),
            result_index: None,
            after_row: None,
        });

        let event = controller.dispatch(AppCommand::ApplyDataChanges(TabId(1)));

        let AppEvent::DataLoaded(_, page) = event else {
            panic!("expected refreshed data page");
        };
        assert_eq!(page.rows[0].values[0], CellValue::Text("1".to_string()));
        assert_eq!(
            page.rows[0].values[1],
            CellValue::Text("friend".to_string())
        );
        let editor = active_editor(&controller);
        assert_eq!(
            editor.page.as_ref().unwrap().rows[0].values[1],
            CellValue::Text("friend".to_string())
        );
        assert!(editor.changes.is_none());
        assert!(!controller.state().active_tab().unwrap().dirty);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn apply_data_changes_reloads_page_with_filters() {
        let path = temp_sqlite_path("app-apply-refresh-filter");
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "Real SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO products (id, name) VALUES (1, 'Road Bike'), (2, 'Helmet')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "products".to_string(),
            kind: ObjectKind::Table,
        }));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Touring Bike".to_string()),
        });

        let event = controller.dispatch(AppCommand::ApplyDataChangesWithView {
            tab_id: TabId(1),
            sort: vec![SortSpec {
                field: "name".to_string(),
                direction: SortDirection::Asc,
            }],
            filters: vec![fluxdb_core::FilterSpec {
                field: "name".to_string(),
                op: fluxdb_core::FilterOp::Contains,
                values: vec![CellValue::Text("Hel".to_string())],
                enabled: true,
            }],
        });

        let AppEvent::DataLoaded(_, page) = event else {
            panic!("expected refreshed data page");
        };
        assert_eq!(page.rows.len(), 1);
        assert_eq!(
            page.rows[0].values[1],
            CellValue::Text("Helmet".to_string())
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn apply_data_changes_writes_hex_edited_blob_to_sqlite() {
        let path = temp_sqlite_path("app-blob-apply");
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "Real SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE files (id INTEGER PRIMARY KEY, payload BLOB)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO files (id, payload) VALUES (1, x'ABCD')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "files".to_string(),
            kind: ObjectKind::Table,
        }));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));
        controller.dispatch(AppCommand::OpenCellDetail {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });
        controller.dispatch(AppCommand::StartCellDetailEdit(TabId(1)));
        controller.dispatch(AppCommand::UpdateCellDetailEditValue {
            tab_id: TabId(1),
            value: "DE AD BE EF".to_string(),
        });
        controller.dispatch(AppCommand::SaveCellDetailEdit(TabId(1)));

        let event = controller.dispatch(AppCommand::ApplyDataChanges(TabId(1)));

        assert!(matches!(event, AppEvent::DataLoaded(TabId(1), _)));
        let payload: Vec<u8> = runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .connect()
                .await
                .unwrap();
            let row = sqlx::query("SELECT payload FROM files WHERE id = 1")
                .fetch_one(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
            row.try_get("payload").unwrap()
        });

        assert_eq!(payload, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_cell_binary_reads_full_blob_for_active_data_row() {
        let path = temp_sqlite_path("app-blob-download");
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "Real SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE files (id INTEGER PRIMARY KEY, payload BLOB)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO files (id, payload) VALUES (1, x'000102FEFF')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "files".to_string(),
            kind: ObjectKind::Table,
        }));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));

        let event = controller.dispatch(AppCommand::DownloadBinaryCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });

        assert!(matches!(
            event,
            AppEvent::BinaryCellDownloaded {
                bytes,
                ..
            } if bytes == vec![0x00, 0x01, 0x02, 0xFE, 0xFF]
        ));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dedicated_binary_commands_download_update_file_and_null_cells() {
        let path = temp_sqlite_path("app-binary-commands");
        let replacement_path = temp_sqlite_path("app-binary-replacement");
        let oversized_path = temp_sqlite_path("app-binary-oversized");
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "Real SQLite".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.clone(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE files (id INTEGER PRIMARY KEY, payload BLOB)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO files (id, payload) VALUES (1, x'ABCD')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });
        std::fs::write(&replacement_path, [0xCA, 0xFE]).unwrap();
        let oversized_file = std::fs::File::create(&oversized_path).unwrap();
        oversized_file
            .set_len(BINARY_FILE_UPLOAD_LIMIT + 1)
            .unwrap();

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "files".to_string(),
            kind: ObjectKind::Table,
        }));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));

        let event = controller.dispatch(AppCommand::LoadBinaryPreview {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });
        assert!(matches!(
            event,
            AppEvent::BinaryPreviewLoaded {
                preview,
                ..
            } if preview.byte_length == 2 && preview.preview_hex == "ABCD"
        ));

        let event = controller.dispatch(AppCommand::DownloadBinaryCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });
        assert!(matches!(
            event,
            AppEvent::BinaryCellDownloaded {
                bytes,
                ..
            } if bytes == vec![0xAB, 0xCD]
        ));

        let event = controller.dispatch(AppCommand::UpdateBinaryCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            payload: fluxdb_core::BinaryUpdatePayload::Hex("DE AD".to_string()),
        });
        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        assert_eq!(
            active_editor(&controller).page.as_ref().unwrap().rows[0].values[1],
            CellValue::Bytes(vec![0xDE, 0xAD])
        );

        let event = controller.dispatch(AppCommand::ReplaceBinaryCellFromFile {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            path: replacement_path.clone(),
        });
        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        assert_eq!(
            active_editor(&controller).page.as_ref().unwrap().rows[0].values[1],
            CellValue::Bytes(vec![0xCA, 0xFE])
        );

        let event = controller.dispatch(AppCommand::ReplaceBinaryCellFromFile {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            path: oversized_path.clone(),
        });
        assert!(matches!(event, AppEvent::Failed(_)));
        assert_eq!(
            active_editor(&controller).page.as_ref().unwrap().rows[0].values[1],
            CellValue::Bytes(vec![0xCA, 0xFE])
        );

        let event = controller.dispatch(AppCommand::SetBinaryCellNull {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });
        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        assert_eq!(
            active_editor(&controller).page.as_ref().unwrap().rows[0].values[1],
            CellValue::Null
        );

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(replacement_path);
        let _ = std::fs::remove_file(oversized_path);
    }

    #[test]
    fn binary_file_replacement_rejects_files_larger_than_column_capacity() {
        let path = temp_sqlite_path("app-binary-tinyblob");
        std::fs::write(&path, vec![0xAB; 256]).unwrap();

        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "binary_limit".to_string(),
            kind: ObjectKind::Table,
        }));
        let Some(TabKind::DataEditor(editor)) =
            controller.state.tabs.first_mut().map(|tab| &mut tab.kind)
        else {
            panic!("expected data editor");
        };
        editor.page = Some(DataPage {
            columns: vec![
                Column {
                    name: "id".to_string(),
                    type_name: Some("INTEGER".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                Column {
                    name: "payload".to_string(),
                    type_name: Some("TINYBLOB".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![Row {
                values: vec![
                    CellValue::I64(1),
                    CellValue::BinarySummary(fluxdb_core::BinaryCellSummary {
                        type_name: "TINYBLOB".to_string(),
                        is_null: false,
                        byte_length: 2,
                        preview_hex: Some("ABCD".to_string()),
                    }),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        });
        editor.original_page = editor.page.clone();

        let event = controller.dispatch(AppCommand::ReplaceBinaryCellFromFile {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            path: path.clone(),
        });

        let AppEvent::Failed(error) = event else {
            panic!("expected binary replacement to fail");
        };
        assert!(error.message.contains("最多支持 255 B"));
        assert!(active_editor(&controller).changes.is_none());
        assert!(matches!(
            active_editor(&controller).page.as_ref().unwrap().rows[0].values[1],
            CellValue::BinarySummary(_)
        ));

        let _ = std::fs::remove_file(path);
    }
