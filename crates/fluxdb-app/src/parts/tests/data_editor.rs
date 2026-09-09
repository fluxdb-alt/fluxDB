    #[test]
    fn open_data_editor_activates_existing_tab_for_same_object() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        let database = controller.state().connections[0].objects[0].path.clone();
        controller.dispatch(AppCommand::LoadObjectChildren(database));
        let object = controller.state().connections[0].objects[1].path.clone();

        let first_event = controller.dispatch(AppCommand::OpenDataEditor(object.clone()));
        let second_event = controller.dispatch(AppCommand::OpenDataEditor(object));

        assert_eq!(first_event, AppEvent::TabOpened(TabId(1)));
        assert_eq!(second_event, AppEvent::TabActivated(TabId(1)));
        assert_eq!(controller.state().tabs.len(), 1);
        assert_eq!(controller.state().active_tab, Some(TabId(1)));
    }

    #[test]
    fn reset_data_page_for_reload_clears_current_page() {
        let mut controller = controller_with_data_editor();

        controller.reset_data_page_for_reload(TabId(1));

        let editor = active_editor(&controller);
        assert!(editor.page.is_none());
        assert!(editor.original_page.is_none());
        assert!(editor.changes.is_none());
        assert!(editor.editing_cell.is_none());
        assert!(editor.loading);
        assert!(editor.error.is_none());
        assert!(!controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn refresh_object_list_updates_open_tab() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenConnection(ConnectionId(1)));
        controller.dispatch(AppCommand::OpenObjectList(None));
        if let TabKind::ObjectList(list) = &mut controller.state.tabs[0].kind {
            list.objects.clear();
        }

        let event = controller.dispatch(AppCommand::RefreshObject(None));

        assert!(matches!(event, AppEvent::ObjectsLoaded(None, _)));
        let Some(TabKind::ObjectList(list)) = controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active object list tab");
        };
        assert_eq!(list.objects.len(), 1);
        assert_eq!(list.objects[0].path.kind, ObjectKind::Database);
        assert_eq!(list.objects[0].path.name, "main");
    }

    #[test]
    fn edit_data_cell_records_dirty_change_set() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Touring Bike".to_string()),
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(
            editor.page.as_ref().unwrap().rows[0].values[1],
            CellValue::Text("Touring Bike".to_string())
        );
        assert_eq!(editor.changes.as_ref().unwrap().dirty_cell_count(), 1);
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn edit_data_cell_with_same_value_does_not_record_dirty_change() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Road Bike".to_string()),
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(editor.changes, None);
        assert!(!controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn replace_redis_key_row_updates_only_matching_key() {
        let mut page = DataPage {
            columns: vec![
                Column {
                    name: "键".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                Column {
                    name: "值".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![
                Row {
                    values: vec![
                        CellValue::Text("a".to_string()),
                        CellValue::Text("1".to_string()),
                    ],
                },
                Row {
                    values: vec![
                        CellValue::Text("b".to_string()),
                        CellValue::Text("2".to_string()),
                    ],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };

        let replaced = replace_redis_key_row(
            &mut page,
            "b",
            Row {
                values: vec![
                    CellValue::Text("b".to_string()),
                    CellValue::Text("3".to_string()),
                ],
            },
        );

        assert!(replaced);
        assert_eq!(page.rows[0].values[1], CellValue::Text("1".to_string()));
        assert_eq!(page.rows[1].values[1], CellValue::Text("3".to_string()));
    }

    #[test]
    fn redis_row_key_uses_named_column() {
        let columns = vec![
            Column {
                name: "值".to_string(),
                type_name: None,
                nullable: true,
                primary_key: false,
                comment: None,
            },
            Column {
                name: "键".to_string(),
                type_name: None,
                nullable: true,
                primary_key: true,
                comment: None,
            },
        ];
        let row = Row {
            values: vec![
                CellValue::Text("value".to_string()),
                CellValue::Text("redis:key".to_string()),
            ],
        };

        assert_eq!(redis_row_key(&columns, &row), Some("redis:key".to_string()));
    }

    #[test]
    fn merge_redis_key_metadata_from_background_controller_updates_live_page() {
        let mut current = controller_with_data_editor();
        let columns = vec![
            Column {
                name: "键".to_string(),
                type_name: None,
                nullable: false,
                primary_key: true,
                comment: None,
            },
            Column {
                name: "类型".to_string(),
                type_name: None,
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ];
        let page = DataPage {
            columns,
            rows: vec![Row {
                values: vec![
                    CellValue::Text("key:1".to_string()),
                    CellValue::Text(String::new()),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        if let TabKind::DataEditor(editor) = &mut current.state.tabs[0].kind {
            editor.page = Some(page);
        }
        let mut loaded = current.clone();
        if let TabKind::DataEditor(editor) = &mut loaded.state.tabs[0].kind {
            editor.page.as_mut().unwrap().rows[0].values[1] =
                CellValue::Text("string".to_string());
        }

        current.merge_redis_key_metadata_from(&loaded, TabId(1), &["key:1".to_string()]);

        assert_eq!(
            active_editor(&current).page.as_ref().unwrap().rows[0].values[1],
            CellValue::Text("string".to_string())
        );
    }

    #[test]
    fn redis_apply_data_changes_uses_real_connector_for_non_demo_connection() {
        let config = ConnectionConfig {
            id: ConnectionId(9),
            name: "Redis".to_string(),
            kind: DatabaseKind::Redis,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 0,
                database: None,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };
        let changes = DataChangeSet {
            object: ObjectPath {
                connection_id: ConnectionId(9),
                database: Some("0".to_string()),
                schema: None,
                name: "0".to_string(),
                kind: ObjectKind::RedisDb,
            },
            inserts: Vec::new(),
            updates: vec![RowUpdate {
                identity: RowIdentity {
                    values: [("键".to_string(), CellValue::Text("key".to_string()))].into(),
                },
                cells: vec![CellUpdate {
                    column: "值".to_string(),
                    value: CellValue::Text("value".to_string()),
                }],
            }],
            deletes: Vec::new(),
        };

        let result = apply_data_changes_for_connection(&config, &changes);

        assert!(result.is_err());
    }

    #[test]
    fn edit_data_cell_back_to_original_removes_dirty_change() {
        let mut controller = controller_with_data_editor();
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Touring Bike".to_string()),
        });

        let event = controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Road Bike".to_string()),
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(editor.changes, None);
        assert!(!controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn insert_data_row_records_dirty_insert() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::InsertDataRow {
            tab_id: TabId(1),
            result_index: None,
            after_row: None,
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        let page = editor.page.as_ref().unwrap();
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.rows[2].values, vec![CellValue::Null, CellValue::Null]);
        assert_eq!(editor.changes.as_ref().unwrap().inserts.len(), 1);
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn insert_data_row_after_selected_row_updates_pending_insert() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::InsertDataRow {
            tab_id: TabId(1),
            result_index: None,
            after_row: Some(0),
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        let page = editor.page.as_ref().unwrap();
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.rows[1].values, vec![CellValue::Null, CellValue::Null]);
        assert_eq!(
            editor.editing_cell,
            Some(CellPosition { row: 1, column: 0 })
        );

        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 1,
            column: 1,
            value: CellValue::Text("Inserted".to_string()),
        });
        let changes = active_editor(&controller).changes.as_ref().unwrap();
        assert_eq!(changes.inserts[0].values[1], CellValue::Text("Inserted".to_string()));
        assert!(changes.updates.is_empty());
    }

    #[test]
    fn clone_data_row_records_dirty_insert_without_primary_key() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::CloneDataRow {
            tab_id: TabId(1),
            result_index: None,
            row: 0,
            after_row: None,
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        let page = editor.page.as_ref().unwrap();
        assert_eq!(page.rows.len(), 3);
        assert_eq!(
            page.rows[2].values,
            vec![CellValue::Null, CellValue::Text("Road Bike".to_string())]
        );
        assert_eq!(editor.changes.as_ref().unwrap().inserts.len(), 1);
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn editing_inserted_row_updates_pending_insert_not_update_set() {
        let mut controller = controller_with_data_editor();

        controller.dispatch(AppCommand::InsertDataRow {
            tab_id: TabId(1),
            result_index: None,
            after_row: None,
        });
        let event = controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 2,
            column: 0,
            value: CellValue::I64(1),
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        let changes = editor.changes.as_ref().unwrap();
        assert_eq!(changes.inserts[0].values[0], CellValue::I64(1));
        assert!(changes.updates.is_empty());
    }

    #[test]
    fn delete_data_row_records_dirty_delete() {
        let mut controller = controller_with_data_editor();
        let original_row_count = active_editor(&controller).page.as_ref().unwrap().rows.len();

        let event = controller.dispatch(AppCommand::DeleteDataRow {
            tab_id: TabId(1),
            result_index: None,
            row: 0,
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(editor.page.as_ref().unwrap().rows.len(), original_row_count);
        assert_eq!(editor.changes.as_ref().unwrap().deletes.len(), 1);
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn open_cell_detail_panel_defaults_to_view_mode_with_current_value() {
        let mut controller = controller_with_data_editor();

        let event = controller.dispatch(AppCommand::OpenCellDetail {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let panel = &active_editor(&controller).cell_detail_panel;
        assert!(panel.open);
        assert_eq!(panel.active_cell, Some(CellPosition { row: 0, column: 1 }));
        assert_eq!(panel.mode, CellDetailMode::View);
        assert_eq!(panel.edit_value, "Road Bike");
    }

    #[test]
    fn saving_cell_detail_edit_updates_cell_and_returns_to_view_mode() {
        let mut controller = controller_with_data_editor();
        controller.dispatch(AppCommand::OpenCellDetail {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });
        controller.dispatch(AppCommand::StartCellDetailEdit(TabId(1)));
        controller.dispatch(AppCommand::UpdateCellDetailEditValue {
            tab_id: TabId(1),
            value: "Touring Bike".to_string(),
        });

        let event = controller.dispatch(AppCommand::SaveCellDetailEdit(TabId(1)));

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(editor.cell_detail_panel.mode, CellDetailMode::View);
        assert_eq!(editor.cell_detail_panel.edit_value, "Touring Bike");
        assert_eq!(
            editor.page.as_ref().unwrap().rows[0].values[1],
            CellValue::Text("Touring Bike".to_string())
        );
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn saving_binary_summary_detail_hex_updates_cell_bytes() {
        let mut controller = controller_with_data_editor();
        let TabKind::DataEditor(editor) = &mut controller.state.tabs[0].kind else {
            panic!("expected data editor");
        };
        let page = editor.page.as_mut().unwrap();
        page.columns.push(Column {
            name: "payload".to_string(),
            type_name: Some("BLOB".to_string()),
            nullable: true,
            primary_key: false,
            comment: None,
        });
        page.rows[0]
            .values
            .push(CellValue::BinarySummary(fluxdb_core::BinaryCellSummary {
                type_name: "BLOB".to_string(),
                is_null: false,
                byte_length: 2,
                preview_hex: Some("ABCD".to_string()),
            }));

        controller.dispatch(AppCommand::OpenCellDetail {
            tab_id: TabId(1),
            row: 0,
            column: 2,
        });
        controller.dispatch(AppCommand::UpdateCellDetailEditValue {
            tab_id: TabId(1),
            value: "DE AD be ef".to_string(),
        });

        let event = controller.dispatch(AppCommand::SaveCellDetailEdit(TabId(1)));

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(editor.cell_detail_panel.mode, CellDetailMode::View);
        assert_eq!(editor.cell_detail_panel.edit_value, "de ad be ef");
        assert_eq!(
            editor.page.as_ref().unwrap().rows[0].values[2],
            CellValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])
        );
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn opening_another_cell_detail_cancels_unsaved_edit_value() {
        let mut controller = controller_with_data_editor();
        controller.dispatch(AppCommand::OpenCellDetail {
            tab_id: TabId(1),
            row: 0,
            column: 1,
        });
        controller.dispatch(AppCommand::StartCellDetailEdit(TabId(1)));
        controller.dispatch(AppCommand::UpdateCellDetailEditValue {
            tab_id: TabId(1),
            value: "Unsaved".to_string(),
        });

        let event = controller.dispatch(AppCommand::OpenCellDetail {
            tab_id: TabId(1),
            row: 1,
            column: 1,
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let panel = &active_editor(&controller).cell_detail_panel;
        assert_eq!(panel.active_cell, Some(CellPosition { row: 1, column: 1 }));
        assert_eq!(panel.mode, CellDetailMode::View);
        assert_eq!(panel.edit_value, "Helmet");
        assert_eq!(
            active_editor(&controller).page.as_ref().unwrap().rows[0].values[1],
            CellValue::Text("Road Bike".to_string())
        );
    }

    #[test]
    fn discard_data_changes_restores_original_page() {
        let mut controller = controller_with_data_editor();
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Touring Bike".to_string()),
        });

        let event = controller.dispatch(AppCommand::DiscardDataChanges(TabId(1)));

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_editor(&controller);
        assert_eq!(
            editor.page.as_ref().unwrap().rows[0].values[1],
            CellValue::Text("Road Bike".to_string())
        );
        assert!(editor.changes.is_none());
        assert!(!controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn apply_failure_keeps_dirty_change_set() {
        let mut controller = AppController::with_mock_data();
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "FailSubmit".to_string(),
            kind: ObjectKind::Table,
        };
        controller.dispatch(AppCommand::OpenDataEditor(object));
        controller.dispatch(AppCommand::LoadDataPage(TabId(1)));
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Will Fail".to_string()),
        });

        let event = controller.dispatch(AppCommand::ApplyDataChanges(TabId(1)));

        let AppEvent::Failed(error) = event else {
            panic!("expected apply changes to fail");
        };
        assert_eq!(error.title, "保存失败");
        let editor = active_editor(&controller);
        assert!(editor.changes.is_some());
        assert!(controller.state().active_tab().unwrap().dirty);
    }

    #[test]
    fn applying_data_changes_records_statement_history_with_rollback_snapshot() {
        let mut controller = controller_with_data_editor();
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Changed".to_string()),
        });
        controller.dispatch(AppCommand::DeleteDataRow {
            tab_id: TabId(1),
            result_index: None,
            row: 1,
        });

        let event = controller.dispatch(AppCommand::ApplyDataChanges(TabId(1)));

        assert!(matches!(event, AppEvent::DataLoaded(TabId(1), _)));
        assert_eq!(controller.state().query_history.len(), 2);
        assert!(
            controller
                .state()
                .query_history
                .iter()
                .all(|entry| entry.rollback_snapshot.is_some())
        );
        assert!(
            controller.state().query_history[0]
                .rollback_sql()
                .unwrap()
                .starts_with("UPDATE ")
        );
        assert!(
            controller.state().query_history[1]
                .rollback_sql()
                .unwrap()
                .starts_with("INSERT INTO ")
        );
    }
