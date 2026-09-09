#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_ddl_highlight_query_maps_to_ddl_viewer_colors() {
        let query = mysql_ddl_highlights_query();

        assert_eq!(MYSQL_DDL_HIGHLIGHT_LANGUAGE, "mysql-ddl");
        assert!(query.contains("(identifier) @link_text"));
        assert!(query.contains("(literal) @link_text"));
        assert!(query.contains("(keyword_create)"));
        assert!(query.contains("@variable.special"));
        tree_sitter::Query::new(
            &tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE),
            query,
        )
        .expect("mysql ddl highlight query should compile");
    }

    #[test]
    fn sql_highlight_uses_tree_sitter_sequel_query() {
        let query = mysql_ddl_highlights_query();

        assert_eq!(SQL_HIGHLIGHT_LANGUAGE, "sql");
        assert!(query.contains("(keyword_select)"));
        assert!(query.contains("(keyword_where)"));
        assert!(query.contains("(keyword_limit)"));
        assert!(query.contains("@variable.special"));
        tree_sitter::Query::new(
            &tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE),
            query,
        )
        .expect("sql highlight query should compile");
    }

    #[test]
    fn json_highlight_uses_tree_sitter_json_query() {
        let query = json_highlights_query();

        assert_eq!(JSON_HIGHLIGHT_LANGUAGE, "json");
        assert!(query.contains("(pair"));
        assert!(query.contains("@string"));
        assert!(query.contains("@number"));
        tree_sitter::Query::new(
            &tree_sitter::Language::new(tree_sitter_json::LANGUAGE),
            query,
        )
        .expect("json highlight query should compile");
    }

    #[test]
    fn query_summary_text_collapses_multiline_sql_for_display() {
        assert_eq!(
            single_line_summary_text("SELECT\n  *\r\nFROM users".to_string()),
            "SELECT * FROM users"
        );
        assert_eq!(
            single_line_summary_text("SELECT  * FROM users".to_string()),
            "SELECT  * FROM users"
        );
    }

    #[test]
    fn cmd_enter_adds_row_for_create_table_edit_tabs_only() {
        let tab_id = TabId(7);

        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Fields),
            Some(AppCommand::AddCreateTableColumn(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Indexes),
            Some(AppCommand::AddCreateTableIndex(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::ForeignKeys),
            Some(AppCommand::AddCreateTableForeignKey(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Checks),
            Some(AppCommand::AddCreateTableCheck(tab_id))
        );
        assert_eq!(
            create_table_add_row_command(tab_id, CreateTableTab::Triggers),
            Some(AppCommand::AddCreateTableTrigger(tab_id))
        );

        for tab in [
            CreateTableTab::Options,
            CreateTableTab::Partitions,
            CreateTableTab::SqlPreview,
            CreateTableTab::Ddl,
        ] {
            assert_eq!(create_table_add_row_command(tab_id, tab), None);
        }
    }

    #[test]
    fn query_result_tabs_include_failed_result_set_summaries() {
        let editor = QueryEditorState {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            text: String::new(),
            origin: None,
            saved_fingerprint: None,
            running: false,
            results: vec![DataPage {
                columns: Vec::new(),
                rows: Vec::new(),
                offset: 0,
                limit: 100,
                has_more: false,
            }],
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: vec![
                QueryExecutionSummary {
                    sql: "SELECT * FROM missing".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: false,
                    message: "Table missing doesn't exist".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 3,
                },
                QueryExecutionSummary {
                    sql: "SELECT * FROM users".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 0 行结果表".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 4,
                },
            ],
            error: None,
        };

        assert_eq!(query_result_entry_count(&editor), 2);
        assert_eq!(query_result_page_index(&editor, 0), None);
        assert_eq!(query_result_page_index(&editor, 1), Some(0));
        assert_eq!(
            query_result_sql(&editor, 0),
            Some("SELECT * FROM missing".to_string())
        );
        assert_eq!(
            query_result_sql(&editor, 1),
            Some("SELECT * FROM users".to_string())
        );
        assert_eq!(query_result_summary_index(&editor, 0), Some(0));
        assert_eq!(query_result_summary_index(&editor, 1), Some(1));
    }

    #[test]
    fn query_result_summary_index_skips_command_summaries() {
        let editor = QueryEditorState {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            text: String::new(),
            origin: None,
            saved_fingerprint: None,
            running: false,
            results: vec![
                DataPage {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    offset: 0,
                    limit: 100,
                    has_more: false,
                },
                DataPage {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    offset: 0,
                    limit: 100,
                    has_more: false,
                },
            ],
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: vec![
                QueryExecutionSummary {
                    sql: "UPDATE users SET touched = 1".to_string(),
                    kind: fluxdb_core::QueryStatementKind::Command,
                    success: true,
                    message: "影响 1 行".to_string(),
                    returned_rows: 0,
                    affected_rows: 1,
                    elapsed_ms: 2,
                },
                QueryExecutionSummary {
                    sql: "SELECT * FROM users".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 0 行结果表".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 3,
                },
                QueryExecutionSummary {
                    sql: "SELECT * FROM logs".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 0 行结果表".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 4,
                },
            ],
            error: None,
        };

        assert_eq!(query_result_summary_index(&editor, 0), Some(1));
        assert_eq!(query_result_summary_index(&editor, 1), Some(2));
        assert_eq!(query_result_page_index(&editor, 0), Some(0));
        assert_eq!(query_result_page_index(&editor, 1), Some(1));
    }

    #[test]
    fn sql_file_decoding_strips_bom_and_supports_gbk() {
        assert_eq!(
            decode_sql_file_bytes(encoding_rs::UTF_8, b"\xef\xbb\xbfSELECT 1;"),
            "SELECT 1;"
        );
        assert_eq!(
            decode_sql_file_bytes(encoding_rs::GBK, &[0xb2, 0xe2, 0xca, 0xd4]),
            "测试"
        );
    }

    #[test]
    fn sql_file_task_log_text_contains_summary_and_rows() {
        let started_at = Instant::now();
        let task = SqlFileExecutionTaskState {
            id: 7,
            file_name: "schema.sql".to_string(),
            path: PathBuf::from("/tmp/schema.sql"),
            connection_id: ConnectionId(3),
            database: Some("app".to_string()),
            tab_id: Some(TabId(9)),
            total: 2,
            processed: 2,
            errors: 1,
            started_at,
            finished_at: Some(started_at + Duration::from_millis(12)),
            logs: vec![SqlFileExecutionLogEntry {
                index: 1,
                success: false,
                elapsed_ms: 8,
                message: "语法错误".to_string(),
                sql: "CREATE TABLE broken".to_string(),
            }],
            error: None,
            cancel_requested: false,
            canceled: false,
        };

        let text = sql_file_task_log_text(&task);

        assert!(text.contains("文件: schema.sql"));
        assert!(text.contains("数据库: app"));
        assert!(text.contains("错误: 1"));
        assert!(text.contains("#1 失败 8 ms 语法错误 | CREATE TABLE broken"));
    }

    #[test]
    fn query_parameter_specs_find_named_and_positional_placeholders() {
        let specs = query_parameter_specs(
            "select ':skip', col from t where id = :id and name = :name or parent_id = :id and code = ? and note like ?",
        );

        assert_eq!(
            specs,
            vec![
                QueryParameterSpec {
                    key: ":id".to_string(),
                    label: ":id".to_string(),
                },
                QueryParameterSpec {
                    key: ":name".to_string(),
                    label: ":name".to_string(),
                },
                QueryParameterSpec {
                    key: "?1".to_string(),
                    label: "参数 1".to_string(),
                },
                QueryParameterSpec {
                    key: "?2".to_string(),
                    label: "参数 2".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_query_parameter_array_values_accepts_json_array_in_order() {
        assert_eq!(
            parse_query_parameter_array_values(r#"[42, "Bob's Bike", true, null]"#).unwrap(),
            vec!["42", "Bob's Bike", "true", "null"]
        );
        assert_eq!(
            parse_query_parameter_array_values("42
Bob").unwrap(),
            vec!["42", "Bob"]
        );
    }

    #[test]
    fn bind_query_parameters_replaces_outside_comments_and_strings() {
        let sql = "select ':id', ? from t -- :skip ?
where id = :id and name = :name and flag = ?";
        let mut values = BTreeMap::new();
        values.insert(":id".to_string(), "42".to_string());
        values.insert(":name".to_string(), "Bob's Bike".to_string());
        values.insert("?1".to_string(), "true".to_string());
        values.insert("?2".to_string(), "ignored".to_string());

        assert_eq!(
            bind_query_parameters(sql, &values),
            "select ':id', TRUE from t -- :skip ?
where id = 42 and name = 'Bob''s Bike' and flag = 'ignored'"
        );
    }

    #[test]
    fn query_parameter_specs_ignore_postgres_cast_colons() {
        assert_eq!(
            query_parameter_specs("select value::text from t where id = :id"),
            vec![QueryParameterSpec {
                key: ":id".to_string(),
                label: ":id".to_string(),
            }]
        );
    }

    #[test]
    fn delete_row_labels_reflect_multi_selection() {
        assert_eq!(row_delete_label(1), "删除行");
        assert_eq!(row_delete_label(2), "删除选中行");
        assert_eq!(row_delete_record_label(1), "删除记录");
        assert_eq!(row_delete_record_label(2), "删除选中行");
    }

    #[test]
    fn saved_query_tab_id_finds_open_connection_query() {
        let state = AppState {
            tabs: vec![
                TabState {
                    id: TabId(1),
                    title: "unsaved".to_string(),
                    kind: TabKind::QueryEditor(QueryEditorState {
                        connection_id: ConnectionId(1),
                        database: Some("main".to_string()),
                        text: "select 1".to_string(),
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
                },
                TabState {
                    id: TabId(2),
                    title: "saved".to_string(),
                    kind: TabKind::QueryEditor(QueryEditorState {
                        connection_id: ConnectionId(1),
                        database: Some("main".to_string()),
                        text: "select * from users".to_string(),
                        origin: Some(QueryOrigin::Connection { query_id: 7 }),
                        saved_fingerprint: None,
                        running: false,
                        results: Vec::new(),
                        result_editors: BTreeMap::new(),
                        active_result_editor: None,
                        summaries: Vec::new(),
                        error: None,
                    }),
                    dirty: false,
                },
            ],
            ..AppState::default()
        };

        assert_eq!(saved_query_tab_id(&state, 7), Some(TabId(2)));
        assert_eq!(saved_query_tab_id(&state, 8), None);
    }

    #[test]
    fn row_viewer_snapshot_reads_readonly_query_result_page() {
        let mut state = AppState::default();
        state.tabs.push(TabState {
            id: TabId(1),
            title: "Query".to_string(),
            kind: TabKind::QueryEditor(QueryEditorState {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: String::new(),
                origin: None,
                saved_fingerprint: None,
                running: false,
                results: vec![DataPage {
                    columns: vec![GdbColumn {
                        name: "id".to_string(),
                        type_name: Some("varchar(64)".to_string()),
                        nullable: false,
                        primary_key: false,
                        comment: None,
                    }],
                    rows: vec![fluxdb_core::Row {
                        values: vec![CellValue::Text("a1".to_string())],
                    }],
                    offset: 20,
                    limit: 100,
                    has_more: false,
                }],
                result_editors: BTreeMap::new(),
                active_result_editor: None,
                summaries: Vec::new(),
                error: None,
            }),
            dirty: false,
        });
        let viewer = DataRowViewer {
            tab_id: TabId(1),
            source_row: 0,
            query_result_page_index: Some(0),
        };

        let (name, offset, fields) = data_row_viewer_snapshot(&viewer, &state).unwrap();

        assert_eq!(name, "查询结果 1");
        assert_eq!(offset, 20);
        assert_eq!(fields[0].name, "id");
        assert_eq!(fields[0].value, CellValue::Text("a1".to_string()));
    }

    #[test]
    fn render_state_snapshot_keeps_heavy_content_only_for_active_tab() {
        let page = DataPage {
            columns: Vec::new(),
            rows: vec![fluxdb_core::Row {
                values: vec![CellValue::Text("payload".to_string())],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let mut state = AppState {
            active_tab: Some(TabId(2)),
            tabs: vec![
                TabState {
                    id: TabId(1),
                    title: "users".to_string(),
                    kind: TabKind::DataEditor(DataEditorState {
                        object,
                        page: Some(page.clone()),
                        original_page: Some(page.clone()),
                        pagination: Default::default(),
                        changes: None,
                        editing_cell: None,
                        cell_detail_panel: CellDetailPanelState::default(),
                        table_info: TableInfoState::default(),
                        loading: false,
                        error: None,
                    }),
                    dirty: true,
                },
                TabState {
                    id: TabId(2),
                    title: "Query".to_string(),
                    kind: TabKind::QueryEditor(QueryEditorState {
                        connection_id: ConnectionId(1),
                        database: Some("main".to_string()),
                        text: "select * from users".to_string(),
                        origin: None,
                        saved_fingerprint: None,
                        running: false,
                        results: vec![page],
                        result_editors: BTreeMap::new(),
                        active_result_editor: None,
                        summaries: Vec::new(),
                        error: None,
                    }),
                    dirty: false,
                },
            ],
            ..AppState::default()
        };
        state.query_history.push(fluxdb_app::QueryHistoryEntry {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            text: "select * from users".to_string(),
            tables: vec!["users".to_string()],
            kind: fluxdb_app::QueryHistoryKind::Query,
            success: true,
            summary: QueryExecutionSummary {
                sql: "select * from users".to_string(),
                kind: fluxdb_core::QueryStatementKind::ResultSet,
                success: true,
                message: "返回 1 行结果表".to_string(),
                returned_rows: 1,
                affected_rows: 0,
                elapsed_ms: 3,
            },
            executed_at_unix_secs: 1,
            object: Some("users".to_string()),
            rollback_snapshot: None,
        });

        let snapshot = render_state_snapshot(&state);

        match &snapshot.tabs[0].kind {
            TabKind::DataEditor(editor) => {
                assert!(editor.page.is_none());
                assert!(editor.original_page.is_none());
            }
            _ => panic!("expected data editor tab"),
        }
        match &snapshot.tabs[1].kind {
            TabKind::QueryEditor(editor) => {
                assert_eq!(editor.text, "select * from users");
                assert_eq!(editor.results.len(), 1);
            }
            _ => panic!("expected query editor tab"),
        }
        assert!(snapshot.tabs[0].dirty);
        assert_eq!(snapshot.query_history.len(), 1);
    }

    #[test]
    fn query_result_error_copy_text_contains_sql_and_message() {
        let summary = QueryExecutionSummary {
            sql: "SELECT * FROM missing".to_string(),
            kind: fluxdb_core::QueryStatementKind::ResultSet,
            success: false,
            message: "Table missing doesn't exist".to_string(),
            returned_rows: 0,
            affected_rows: 0,
            elapsed_ms: 3,
        };

        let text = query_result_error_copy_text(&summary);

        assert!(text.contains("SELECT * FROM missing"));
        assert!(text.contains("Table missing doesn't exist"));
    }

    #[test]
    fn delete_connection_modal_uses_theme_colors() {
        let source = include_str!("menus_dialogs/confirmations.rs");
        let start = source.find("fn delete_connection_modal").unwrap();
        let end = source.find("fn delete_data_row_modal").unwrap();
        let body = &source[start..end];

        assert!(body.contains("colors: UiColors"));
        assert!(body.contains(".bg(colors.panel_bg)"));
        assert!(body.contains(".border_color(colors.border)"));
        assert!(body.contains(".text_color(colors.text)"));
        assert!(body.contains(".text_color(colors.muted)"));
        assert!(body.contains(".bg(colors.border_soft)"));
        assert!(!body.contains("rgb(0xffffff)"));
        assert!(!body.contains("rgb(0xd8dde5)"));
        assert!(!body.contains("rgb(0xe4e8ee)"));
        assert!(!body.contains(".label(\"×\")"));
    }

    #[test]
    fn redis_database_menu_has_open_pubsub_entry_guarded_by_redis() {
        // 「打开 Pub/Sub」应作为 Redis 数据库右键菜单的一项（由 is_redis 守卫，仅 Redis 显示），
        // 且动作类型为 DatabaseMenuAction::PubSub，路由到数据库级动作。
        let source = include_str!("menus_dialogs/connection_menu.rs");
        let body = &source[0..source.len()];
        assert!(body.contains("\"打开 Pub/Sub\""));
        assert!(body.contains("DatabaseMenuAction::PubSub"));
        // 与 Redis CLI 同受 is_redis 守卫：非 Redis 连接不显示。
        let redis_cli_pos = body.find("\"Redis CLI\"").unwrap();
        let pubsub_pos = body.find("\"打开 Pub/Sub\"").unwrap();
        assert!(pubsub_pos > redis_cli_pos);
        assert!(body.contains(".when(is_redis, |this|"));

        // 分发端：PubSub 动作解析菜单携带的数据库编号，构造 OpenRedisPubSub 命令传给 controller。
        let dispatch = include_str!("navicat_main/connection_groups.rs");
        let start = dispatch.find("DatabaseMenuAction::PubSub =>").unwrap();
        let end = dispatch.find("DatabaseMenuAction::RunSqlFile =>").unwrap();
        let d = &dispatch[start..end];
        assert!(d.contains("menu.database.parse::<u32>()"));
        assert!(d.contains("AppCommand::OpenRedisPubSub {"));
        assert!(d.contains("connection_id: menu.connection_id"));
        assert!(d.contains("database,"));
    }

    #[test]
    fn query_save_modal_tracks_focus_for_escape() {
        let source = include_str!("menus_dialogs/query_save.rs");
        let start = source.find("fn query_save_modal_panel").unwrap();
        let body = &source[start..];

        assert!(body.contains(".track_focus(&focus_handle)"));
        assert!(body.contains(".key_context(\"QuerySaveModal\")"));
        assert!(body.contains("this.cancel_query_save_modal(cx)"));
    }

    #[test]
    fn redis_hash_full_value_is_inline_panel_not_dialog() {
        let source = include_str!("redis_detail/hash_panel.rs");
        let start = source.find("fn redis_hash_full_value_inline_panel").unwrap();
        let end = source.find("fn redis_hash_field_add_drawer").unwrap();
        let body = &source[start..end];

        // 内嵌面板：标题、字节数与按钮齐全
        assert!(body.contains("fn redis_hash_full_value_inline_panel("));
        assert!(body.contains("完整值 · "));
        assert!(body.contains("format!(\"{} 字节\", bytes)"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-close\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-edit\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-cancel-edit\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-save\")"));
        assert!(body.contains("Button::new(\"redis-hash-full-value-retry\")"));
        // 内容区支持横向+纵向滚动
        assert!(body.contains(".overflow_x_scroll()"));
        assert!(body.contains(".overflow_y_scrollbar()"));
        // 复用共享多行输入进入编辑态
        assert!(body.contains("Input::new(&input)"));
        // 不再使用任何弹框机制
        assert!(!body.contains("gpui_component::dialog::Dialog"));
        assert!(!body.contains(".overlay_closable(true)"));
        assert!(!body.contains("window.open_dialog"));
    }

    #[test]
    fn user_admin_rebuild_uses_inline_create_entry_and_general_panel() {
        let source = concat!(
            include_str!("user_admin.rs"),
            include_str!("user_admin_privileges.rs")
        );

        assert!(!source.contains("fn user_admin_create_modal("));
        assert!(!source.contains("fn user_admin_static_select_row("));
        assert!(source.contains("fn user_admin_add_user_button("));
        assert!(source.contains("fn user_admin_draft_user_row("));
        assert!(source.contains("AppCommand::BeginUserAdminCreateUser(tab_id)"));
        assert!(source.contains("fn preview_user_admin_all_sql("));
        assert!(source.contains("this.preview_user_admin_all_sql(tab_id, cx)"));
        assert!(source.contains("fn user_admin_text_input_row("));
        assert!(source.contains("Input::new(&input)"));
        assert!(source.contains(".w(px(360.))"));
        assert!(source.contains(".h(px(34.))"));
        assert!(source.contains("focus_handle(cx).is_focused(window)"));
        assert!(source.contains("fn user_admin_input_border_color("));
        assert!(source.contains("fn user_admin_input_hover_border_color("));
        assert!(source.contains("user_admin_input_border_color(true, colors)"));
        assert!(source.contains("rgb(0x111111)"));
        assert!(source.contains("fn user_admin_input_shadow("));
        assert!(!source.contains(".disabled(disabled)"));
        assert!(source.contains("fn user_admin_tab_strip("));
        assert!(source.contains("(UserAdminDetailTab::General, \"常规\")"));
        assert!(source.contains("(UserAdminDetailTab::Advanced, \"高级\")"));
        assert!(source.contains("(UserAdminDetailTab::MemberOf, \"成员关系\")"));
        assert!(!source.contains("(UserAdminDetailTab::Members, \"成员\")"));
        assert!(source.contains("fn user_admin_member_relationships_panel("));
        assert!(source.contains("fn user_admin_select_row("));
        assert!(source.contains("Select::new(&select)"));
        assert!(source.contains("fn user_admin_advanced_panel("));
        assert!(!source.contains("使用 OLD_PASSWORD 加密"));
        assert!(source.contains("fn user_admin_ssl_type_options("));
        assert!(source.contains("fn user_admin_password_eye_button("));
        assert!(source.contains("this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx)"));
        assert!(source.contains("fn user_admin_privileges_panel("));
        assert!(source.contains("添加权限"));
        assert!(source.contains("AppCommand::AddUserAdminPrivilegeRow"));
        assert!(source.contains("fn user_admin_privileges_sql_preview("));
        assert!(source.contains("fn user_admin_sql_preview_panel("));
        assert!(source.contains(".code_editor(SQL_HIGHLIGHT_LANGUAGE)"));
        assert!(source.contains(".disabled(true)"));
        assert!(source.contains("fn start_user_admin_database_options_load("));
        assert!(source.contains("this.start_user_admin_database_options_load(tab_id, cx)"));
        assert!(source.contains("AppCommand::ToggleUserAdminPrivilegeRowPrivilege"));
        assert!(source.contains("AppCommand::SetUserAdminPrivilegeRowDatabase"));
        assert!(source.contains("Grant Option"));
    }

    #[test]
    fn explain_sql_text_wraps_only_explainable_sql() {
        assert_eq!(sql_editor_adapter::explain_sql_text("SELECT * FROM Product;"), Some("EXPLAIN SELECT * FROM Product;".to_string()));
        assert_eq!(sql_editor_adapter::explain_sql_text("with q as (select 1) select * from q"), Some("EXPLAIN with q as (select 1) select * from q".to_string()));
        assert_eq!(sql_editor_adapter::explain_sql_text("EXPLAIN SELECT 1"), Some("EXPLAIN SELECT 1".to_string()));
        assert_eq!(sql_editor_adapter::explain_sql_text("UPDATE Product SET name = 'x'"), None);
    }

    fn completion_item(label: &str) -> fluxdb_core::QueryCompletionItem {
        fluxdb_core::QueryCompletionItem {
            label: label.into(),
            insert_text: label.into(),
            kind: fluxdb_core::QueryCompletionKind::Table,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
}
    }

    fn highlighted_text(text: &str, ranges: &[Range<usize>]) -> String {
        ranges
            .iter()
            .filter_map(|range| text.get(range.clone()))
            .collect()
    }

    #[test]
    fn query_editor_text_sync_replaces_buffer_and_resets_selection() {
        // 查询页面把模型 query.text 静默同步进编辑器（sync_text_silent）时，底层 buffer
        // 应整体替换为新文本，并把光标/选区复位到起点，避免旧文本残留或误触发编辑事件。
        // 本测试只在 buffer 层验证（无需 GPUI Window），是查询编辑器文本同步的回归保护。
        use fluxdb_editor_core::{EditorBuffer, Selection};

        let mut buffer = EditorBuffer::new_from("SELECT * FROM t;");
        assert_eq!(buffer.to_string(), "SELECT * FROM t;");
        assert_eq!(buffer.len(), "SELECT * FROM t;".len());
        assert!(!buffer.is_empty());

        // 同步 = 用新文本整体重建 buffer（sync_text_silent 的等价语义）。
        buffer = EditorBuffer::new_from("UPDATE t SET a = 1 WHERE id = 2;");
        assert_eq!(buffer.to_string(), "UPDATE t SET a = 1 WHERE id = 2;");
        assert!(!buffer.is_empty());

        // 同步后选区复位为起点（光标 = 锚点 = 0）。
        let sel = Selection::point(0);
        assert!(sel.is_empty());
        let range = sel.range();
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 0);

        // 空文本同步后 buffer 为空，选区仍在起点。
        buffer = EditorBuffer::new_from("");
        assert!(buffer.is_empty());
        assert!(Selection::point(0).is_empty());
    }

    #[test]
    fn header_sort_appends_updates_and_removes_one_field() {
        let rules = data_sort_rules_after_header_sort(
            &[],
            "id".to_string(),
            Some(DataTableSortDirection::Ascending),
        );
        let rules = data_sort_rules_after_header_sort(
            &rules,
            "name".to_string(),
            Some(DataTableSortDirection::Descending),
        );

        assert_eq!(rules.len(), 2);
        assert_eq!(data_sort_rules_text(&rules), "`id` ASC, `name` DESC");

        let rules = data_sort_rules_after_header_sort(
            &rules,
            "id".to_string(),
            Some(DataTableSortDirection::Descending),
        );
        assert_eq!(data_sort_rules_text(&rules), "`id` DESC, `name` DESC");

        let rules = data_sort_rules_after_header_sort(&rules, "id".to_string(), None);
        assert_eq!(data_sort_rules_text(&rules), "`name` DESC");
    }

    #[test]
    fn query_result_header_sort_is_scoped_to_result_index() {
        let tab_id = TabId(9);
        let first = QueryResultSortKey {
            tab_id,
            result_index: 0,
        };
        let second = QueryResultSortKey {
            tab_id,
            result_index: 1,
        };
        let mut rules = BTreeMap::new();

        apply_query_result_header_sort(
            &mut rules,
            first,
            "id".to_string(),
            Some(DataTableSortDirection::Ascending),
        );
        apply_query_result_header_sort(
            &mut rules,
            second,
            "id".to_string(),
            Some(DataTableSortDirection::Descending),
        );

        assert_eq!(data_sort_rules_text(&rules[&first]), "`id` ASC");
        assert_eq!(data_sort_rules_text(&rules[&second]), "`id` DESC");

        apply_query_result_header_sort(&mut rules, first, "id".to_string(), None);

        assert!(!rules.contains_key(&first));
        assert_eq!(data_sort_rules_text(&rules[&second]), "`id` DESC");
    }

    #[test]
    fn query_output_layout_toggle_changes_icon() {
        assert_eq!(
            query_output_layout_toggle_icon(QueryOutputPlacement::Bottom),
            AppIcon::PanelBottom
        );
        assert_eq!(
            query_output_layout_toggle_icon(QueryOutputPlacement::Right),
            AppIcon::PanelRight
        );
        assert_eq!(
            QueryOutputPlacement::Bottom.toggled(),
            QueryOutputPlacement::Right
        );
    }

    #[test]
    fn redis_workbench_editor_height_clamps_to_min_and_max_ratio() {
        // 结果区 ∈ [split/6, 0.80·split] ⇒ editor ∈ [0.20·split, 5/6·split]。
        let split = 860.;
        let min_editor = split / 5.; // 结果区最大(0.8·split) ⇒ 编辑器最小 = 0.20·split
        let max_editor = split * 5. / 6.; // 结果区最小 ⇒ 编辑器最大 = 5·split/6
        // 默认占比 63%（结果区默认 37.5% 分栏）落在范围内，直接按比例取值。
        let height = redis_workbench_editor_height(0.63, split);
        assert!((height - split * 0.63).abs() < 1e-3);
        // 占比低于下限（结果区拉满）被夹到最小编辑器高度。
        let lo = redis_workbench_editor_height(0.01, split);
        assert!((lo - min_editor).abs() < 1e-3);
        // 占比高于上限（结果区压到最小）被夹到最大编辑器高度。
        let hi = redis_workbench_editor_height(0.99, split);
        assert!((hi - max_editor).abs() < 1e-3);
        // 可用高度过小时不应 panic。
        let tiny = redis_workbench_editor_height(1.0, 10.);
        assert!(tiny > 0.);
    }

    #[test]
    fn query_result_table_refreshes_when_rows_or_sorts_change() {
        let rows = vec![
            vec![SharedString::from("1")],
            vec![SharedString::from("2")],
        ];
        let reversed = vec![
            vec![SharedString::from("2")],
            vec![SharedString::from("1")],
        ];
        let sorts = vec![DataTableSort {
            col_ix: 1,
            direction: DataTableSortDirection::Ascending,
        }];

        assert!(data_table_rows_or_sorts_changed(
            &rows,
            &[],
            &reversed,
            &[]
        ));
        assert!(data_table_rows_or_sorts_changed(
            &rows,
            &[],
            &rows,
            &sorts
        ));
        assert!(!data_table_rows_or_sorts_changed(
            &rows,
            &sorts,
            &rows,
            &sorts
        ));
    }

    #[test]
    fn data_table_column_widths_apply_by_column_key() {
        let mut columns = vec![
            TableColumn::new("__row_index", "#").width(px(54.)),
            TableColumn::new("id", "id").width(px(170.)),
            TableColumn::new("name", "name").width(px(170.)),
        ];
        let widths = BTreeMap::from([
            ("name".to_string(), px(260.)),
            ("id".to_string(), px(90.)),
        ]);

        apply_data_table_column_widths(&mut columns, &widths);

        assert_eq!(columns[0].width, px(54.));
        assert_eq!(columns[1].width, px(90.));
        assert_eq!(columns[2].width, px(260.));
    }

    #[test]
    fn data_table_column_widths_apply_from_resize_event_order() {
        let mut columns = vec![
            TableColumn::new("__row_index", "#").width(px(54.)),
            TableColumn::new("id", "id").width(px(170.)),
            TableColumn::new("name", "name").width(px(170.)),
        ];

        apply_data_table_column_widths_from_list(&mut columns, &[px(54.), px(90.), px(260.)]);

        assert_eq!(columns[0].width, px(54.));
        assert_eq!(columns[1].width, px(90.));
        assert_eq!(columns[2].width, px(260.));
    }

    #[test]
    fn data_table_columns_match_detects_width_changes() {
        let left = vec![TableColumn::new("id", "id").width(px(170.))];
        let right = vec![TableColumn::new("id", "id").width(px(260.))];

        assert!(!data_table_columns_match(&left, &right));
    }

    #[test]
    fn query_result_local_sort_applies_to_result_pages() {
        let page = DataPage {
            columns: vec![GdbColumn {
                name: "id".to_string(),
                type_name: Some("int".to_string()),
                nullable: false,
                primary_key: false,
                comment: None,
            }],
            rows: vec![
                fluxdb_core::Row {
                    values: vec![CellValue::I64(2)],
                },
                fluxdb_core::Row {
                    values: vec![CellValue::I64(1)],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let rules = vec![DataSortRule {
            enabled: true,
            field: "id".to_string(),
            ascending: true,
        }];

        let sorted = sorted_query_result_page(&page, &rules);

        assert_eq!(sorted.page.rows[0].values[0], CellValue::I64(1));
        assert_eq!(sorted.page.rows[1].values[0], CellValue::I64(2));
        assert_eq!(sorted.source_row_indexes, vec![1, 0]);
    }

    #[test]
    fn sorted_query_result_keeps_inline_editing_when_editable() {
        let rules = vec![DataSortRule {
            enabled: true,
            field: "id".to_string(),
            ascending: true,
        }];

        assert!(query_result_cells_editable(true, &[]));
        assert!(query_result_cells_editable(true, &rules));
        assert!(!query_result_cells_editable(false, &[]));
    }

    #[test]
    fn temporal_cell_editor_detects_date_time_types() {
        assert_eq!(
            data_cell_temporal_kind("timestamp(6)"),
            Some(DataCellTemporalKind::DateTime)
        );
        assert_eq!(
            data_cell_temporal_kind("datetime"),
            Some(DataCellTemporalKind::DateTime)
        );
        assert_eq!(
            data_cell_temporal_kind("date"),
            Some(DataCellTemporalKind::Date)
        );
        assert_eq!(
            data_cell_temporal_kind("time"),
            Some(DataCellTemporalKind::Time)
        );
        assert_eq!(data_cell_temporal_kind("varchar(255)"), None);
    }

    #[test]
    fn data_cell_editor_kind_detects_bool_enum_and_set_types() {
        assert_eq!(
            data_cell_editor_kind("tinyint(1) unsigned"),
            DataCellEditorKind::Boolean
        );
        assert_eq!(data_cell_editor_kind("bool"), DataCellEditorKind::Boolean);
        assert_eq!(
            data_cell_editor_kind("enum('draft','published')"),
            DataCellEditorKind::Enum
        );
        assert_eq!(
            data_cell_editor_kind("set('read','write')"),
            DataCellEditorKind::Set
        );
        assert_eq!(
            data_cell_editor_kind("varchar(255)"),
            DataCellEditorKind::Text
        );
    }

    #[test]
    fn binary_data_types_are_read_only_for_cell_editing() {
        for type_name in [
            "BINARY(16)",
            "VARBINARY(255)",
            "TINYBLOB",
            "BLOB",
            "MEDIUMBLOB",
            "LONGBLOB",
        ] {
            assert!(
                data_type_is_binary(type_name),
                "{type_name} should be detected as binary"
            );
        }
        assert!(!data_type_is_binary("varchar(255)"));
        assert!(!data_type_is_binary("datetime"));
    }

    #[test]
    fn dirty_cell_keeps_dirty_background_when_selected() {
        assert!(!data_cell_selection_should_fill_background(true, false));
        assert!(!data_cell_selection_should_fill_background(false, true));
        assert!(data_cell_selection_should_fill_background(false, false));
    }

    #[test]
    fn deleted_row_content_is_dimmed() {
        assert!(data_cell_content_opacity(true) < data_cell_content_opacity(false));
    }

    #[test]
    fn data_cell_edit_commits_on_enter_and_blur() {
        assert!(data_cell_edit_event_should_commit(
            &InputEvent::PressEnter {
                secondary: false,
                shift: false,
            }
        ));
        assert!(data_cell_edit_event_should_commit(&InputEvent::Blur));
        assert!(!data_cell_edit_event_should_commit(&InputEvent::Change));
    }

    #[test]
    fn data_cell_edit_commits_before_switching_to_another_cell() {
        let current = DataCellEditState {
            tab_id: TabId(1),
            query_result_page_index: None,
            visible_row: 0,
            source_row: 0,
            col_ix: 1,
            source_col: 0,
            temporal_kind: None,
        };
        let same = current;
        let other = DataCellEditState {
            visible_row: 1,
            source_row: 1,
            ..current
        };

        assert!(!data_cell_edit_should_commit_before_cell_change(
            Some(current),
            same
        ));
        assert!(data_cell_edit_should_commit_before_cell_change(
            Some(current),
            other
        ));
        assert!(!data_cell_edit_should_commit_before_cell_change(
            None, other
        ));
    }

    #[test]
    fn data_cell_edit_commits_before_query_output_tab_change() {
        let current = DataCellEditState {
            tab_id: TabId(1),
            query_result_page_index: Some(0),
            visible_row: 0,
            source_row: 0,
            col_ix: 1,
            source_col: 0,
            temporal_kind: None,
        };

        assert!(data_cell_edit_should_commit_before_query_output_tab_change(
            Some(current),
            TabId(1),
            false,
        ));
        assert!(!data_cell_edit_should_commit_before_query_output_tab_change(
            Some(current),
            TabId(1),
            true,
        ));
        assert!(!data_cell_edit_should_commit_before_query_output_tab_change(
            Some(current),
            TabId(2),
            false,
        ));
        assert!(!data_cell_edit_should_commit_before_query_output_tab_change(
            None,
            TabId(1),
            false,
        ));
    }

    #[test]
    fn query_result_edit_state_is_scoped_to_result_page() {
        let editing = DataCellEditState {
            tab_id: TabId(1),
            query_result_page_index: Some(0),
            visible_row: 0,
            source_row: 0,
            col_ix: 1,
            source_col: 0,
            temporal_kind: None,
        };

        assert!(data_cell_edit_matches_table(
            &editing,
            TabId(1),
            Some(0),
            0,
            1
        ));
        assert!(!data_cell_edit_matches_table(
            &editing,
            TabId(1),
            Some(1),
            0,
            1
        ));
        assert!(!data_cell_edit_matches_table(
            &editing,
            TabId(2),
            Some(0),
            0,
            1
        ));
    }

    #[test]
    fn data_cell_edit_text_unchanged_uses_display_text() {
        assert!(data_cell_edit_text_unchanged(
            &CellValue::Text("123456".to_string()),
            "123456"
        ));
        assert!(data_cell_edit_text_unchanged(
            &CellValue::I64(123456),
            "123456"
        ));
        assert!(!data_cell_edit_text_unchanged(
            &CellValue::Text("123456".to_string()),
            "123456111"
        ));
    }

    #[test]
    fn null_cell_edit_text_is_empty_but_text_null_is_literal() {
        assert_eq!(data_cell_edit_text(&CellValue::Null), "");
        assert_eq!(
            data_cell_edit_text(&CellValue::Text("NULL".to_string())),
            "NULL"
        );
        assert!(data_cell_edit_text_unchanged(&CellValue::Null, ""));
    }

    #[test]
    fn enum_and_set_options_are_parsed_from_mysql_type_declarations() {
        assert_eq!(
            data_cell_enum_set_options("enum('draft','it\\'s ok','published')"),
            vec![
                "draft".to_string(),
                "it's ok".to_string(),
                "published".to_string()
            ]
        );
        assert_eq!(
            data_cell_enum_set_options("set('read','write')"),
            vec!["read".to_string(), "write".to_string()]
        );
    }

    #[test]
    fn pasted_cell_values_are_validated_by_type_and_nullability() {
        let int_meta = DataTableColumnMeta {
            name: "age".to_string(),
            type_name: "int".to_string(),
            nullable: false,
            primary_key: false,
            choices: Vec::new(),
        };
        assert_eq!(
            data_cell_value_from_text(&int_meta, "42").unwrap(),
            CellValue::I64(42)
        );
        assert!(data_cell_value_from_text(&int_meta, "abc").is_err());
        assert!(data_cell_value_from_text(&int_meta, "NULL").is_err());

        let nullable_bool = DataTableColumnMeta {
            name: "enabled".to_string(),
            type_name: "tinyint(1)".to_string(),
            nullable: true,
            primary_key: false,
            choices: Vec::new(),
        };
        assert_eq!(
            data_cell_value_from_text(&nullable_bool, "true").unwrap(),
            CellValue::Bool(true)
        );
        assert_eq!(
            data_cell_value_from_text(&nullable_bool, "NULL").unwrap(),
            CellValue::Null
        );

        let enum_meta = DataTableColumnMeta {
            name: "status".to_string(),
            type_name: "enum('draft','published')".to_string(),
            nullable: false,
            primary_key: false,
            choices: Vec::new(),
        };
        assert_eq!(
            data_cell_value_from_text(&enum_meta, "draft").unwrap(),
            CellValue::Text("draft".to_string())
        );
        assert!(data_cell_value_from_text(&enum_meta, "archived").is_err());

        let blob_meta = DataTableColumnMeta {
            name: "payload".to_string(),
            type_name: "LONGBLOB".to_string(),
            nullable: true,
            primary_key: false,
            choices: Vec::new(),
        };
        assert!(data_cell_value_from_text(&blob_meta, "hello").is_err());
        assert!(data_cell_value_from_text(&blob_meta, "NULL").is_err());
    }

    #[test]
    fn column_choices_config_reads_saved_choices() {
        let mut saved = BTreeMap::new();
        saved.insert(
            "state".to_string(),
            vec![ColumnChoice {
                value: "success".to_string(),
                label: "成功".to_string(),
            }],
        );
        let mut options = BTreeMap::new();
        options.insert(
            COLUMN_CHOICES_OPTION.to_string(),
            serde_json::to_string(&saved).unwrap(),
        );
        assert_eq!(
            column_choices_config(&options).get("state"),
            Some(&saved["state"])
        );
    }

    #[test]
    fn temporal_cell_editor_replaces_date_and_time_parts() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();

        assert_eq!(
            replace_temporal_date_part(
                "2021-01-25 23:45:37.000000",
                Some(DataCellTemporalKind::DateTime),
                date,
            ),
            "2026-07-15 23:45:37.000000"
        );
        assert_eq!(
            replace_temporal_time_part(
                "2021-01-25 23:45:37.000000",
                Some(DataCellTemporalKind::DateTime),
                "06:00:00",
            ),
            "2021-01-25 06:00:00"
        );
        assert_eq!(
            replace_temporal_time_part("23:45:37", Some(DataCellTemporalKind::Time), "06:00:00"),
            "06:00:00"
        );
    }

    #[test]
    fn temporal_part_input_updates_one_part_and_clamps_ranges() {
        let edit = |part| TemporalPartEditState {
            target: TemporalEditTarget::CellDetail(TabId(1)),
            part,
            kind: DataCellTemporalKind::DateTime,
        };

        assert_eq!(
            temporal_value_after_part_input("2021-02-02 19:21:30", edit(TemporalPart::Month), "13")
                .unwrap(),
            "2021-12-02 19:21:30"
        );
        assert_eq!(
            temporal_value_after_part_input("2021-02-02 19:21:30", edit(TemporalPart::Day), "31")
                .unwrap(),
            "2021-02-28 19:21:30"
        );
        assert_eq!(
            temporal_value_after_part_input("2021-02-02 19:21:30", edit(TemporalPart::Second), "88")
                .unwrap(),
            "2021-02-02 19:21:59"
        );
    }

    #[test]
    fn temporal_time_parts_reads_time_from_time_and_datetime_text() {
        assert_eq!(temporal_time_parts("23:45:37"), (23, 45, 37));
        assert_eq!(temporal_time_parts("2026-07-15 06:08:09.000000"), (6, 8, 9));
        assert_eq!(temporal_time_parts("not-a-time"), (0, 0, 0));
    }

    #[test]
    fn temporal_shift_month_clamps_to_valid_day() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();

        assert_eq!(
            temporal_shift_month(date, -1),
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        );
        assert_eq!(
            temporal_shift_month(date, 1),
            NaiveDate::from_ymd_opt(2026, 4, 30).unwrap()
        );
    }

    #[test]
    fn app_message_replaces_previous_message() {
        let first = next_app_message(None, "已复制", AppMessageKind::Success);
        let second = next_app_message(Some(&first), "保存成功", AppMessageKind::Success);

        assert_eq!(second.id, first.id + 1);
        assert_eq!(second.text, "保存成功");
    }

    #[test]
    fn statusbar_summary_ignores_last_error() {
        let mut state = AppState::default();
        state.last_error = Some(fluxdb_core::UserFacingError {
            title: "保存失败".to_string(),
            message: "字段太长".to_string(),
            detail: None,
            retryable: false,
        });

        assert_eq!(statusbar_summary(&state), "0 个对象");
    }

    #[test]
    fn app_message_layout_uses_bottom_center() {
        let layout = app_message_layout_for_width(736.);

        assert_eq!(layout.bottom, 40.);
        assert_eq!(layout.max_width, 640.);
    }

    #[test]
    fn search_does_not_force_collapsed_connection_open() {
        assert!(!connection_should_show_children(false, true));
        assert!(connection_should_show_children(true, true));
    }

    #[test]
    fn search_tree_nodes_default_open_but_respect_collapse() {
        assert!(tree_expanded_for_search(None, true));
        assert!(!tree_expanded_for_search(Some(false), true));
        assert!(tree_expanded_for_search(Some(true), true));
    }

    #[test]
    fn empty_visible_database_filter_shows_databases() {
        let options = BTreeMap::from([(VISIBLE_DATABASES_OPTION.to_string(), String::new())]);

        assert_eq!(configured_visible_databases(&options), None);
    }

    #[test]
    fn data_page_number_uses_limit_not_visible_row_count() {
        assert_eq!(data_page_number(100, 100), 2);
        assert_eq!(data_page_number(100, 5), 21);
    }

    #[test]
    fn data_page_offset_uses_one_based_page_number() {
        assert_eq!(data_page_offset_for_page(1, 100), 0);
        assert_eq!(data_page_offset_for_page(3, 100), 200);
        assert_eq!(data_page_offset_for_page(0, 100), 0);
    }

    #[test]
    fn data_page_offset_caps_at_supported_max_page() {
        assert_eq!(data_page_offset_for_supported_page(101, 100), 9900);
        assert_eq!(data_page_offset_for_supported_page(999, 50), 4950);
    }

    #[test]
    fn row_copy_formats_json_and_tsv() {
        let fields = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice".to_string()),
            },
        ];

        assert_eq!(
            row_json_text(fields.as_slice()),
            "{\n  \"id\": 7,\n  \"name\": \"Alice\"\n}"
        );
        assert_eq!(row_tsv_text(fields.as_slice()), "id\tname\n7\tAlice");
    }

    #[test]
    fn row_copy_formats_multiple_rows_json_and_tsv() {
        let first = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice".to_string()),
            },
        ];
        let second = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(8),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Bob".to_string()),
            },
        ];
        let rows = vec![first.as_slice(), second.as_slice()];

        assert_eq!(
            row_json_array_text(rows.as_slice()),
            "[\n  {\n    \"id\": 7,\n    \"name\": \"Alice\"\n  },\n  {\n    \"id\": 8,\n    \"name\": \"Bob\"\n  }\n]"
        );
        assert_eq!(
            row_tsv_rows_text(rows.as_slice()),
            "id\tname\n7\tAlice\n8\tBob"
        );
    }

    #[test]
    fn data_row_copy_submenu_width_expands_for_long_labels() {
        let width = data_row_copy_submenu_width([
            "复制选中 8 行 (JSON)",
            "复制选中 8 行为 INSERT 语句",
            "复制选中 8 行为 INSERT 语句（不含主键）",
            "复制选中 8 行为 UPDATE 语句",
            "复制选中 8 行 (TSV)",
        ]);

        assert!(width > 278.);
        assert!(width <= 440.);
    }

    #[test]
    fn data_row_export_writes_csv_with_header_and_escaping() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let rows = vec![vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice, \"A\"".to_string()),
            },
            RowFieldSnapshot {
                index: 3,
                name: "note".to_string(),
                type_name: "text".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("line\nbreak".to_string()),
            },
        ]];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::Csv,
            Some(&object),
            rows.as_slice(),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "id,name,note\n7,\"Alice, \"\"A\"\"\",\"line\nbreak\"\n"
        );
    }

    #[test]
    fn data_row_export_without_object_supports_plain_formats_only() {
        let rows = vec![vec![RowFieldSnapshot {
            index: 1,
            name: "id".to_string(),
            type_name: "int".to_string(),
            primary_key: false,
            comment: None,
            value: CellValue::I64(7),
        }]];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::Csv,
            None,
            rows.as_slice(),
        )
        .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "id\n7\n");

        let mut output = Vec::new();
        assert!(write_data_row_export(
            &mut output,
            DataRowExportFormat::SqlInsert,
            None,
            rows.as_slice(),
        )
        .is_err());
    }

    #[test]
    fn data_row_export_writes_markdown_with_escaped_cells() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let rows = vec![vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name|title".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice|Admin\nLead".to_string()),
            },
        ]];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::Markdown,
            Some(&object),
            rows.as_slice(),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "| id | name\\|title |\n| --- | --- |\n| 7 | Alice\\|Admin<br>Lead |\n"
        );
    }

    #[test]
    fn data_row_export_writes_insert_sql_per_row() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let rows = vec![
            vec![RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            }],
            vec![RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(8),
            }],
        ];
        let mut output = Vec::new();

        write_data_row_export(
            &mut output,
            DataRowExportFormat::SqlInsert,
            Some(&object),
            rows.as_slice(),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "INSERT INTO `shop`.`users` (`id`) VALUES (7);\nINSERT INTO `shop`.`users` (`id`) VALUES (8);\n"
        );
    }

    #[test]
    fn table_data_export_writer_filters_fields_and_escapes_xml() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "name".to_string(),
                    type_name: Some("varchar(20)".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![fluxdb_core::Row {
                values: vec![CellValue::I64(7), CellValue::Text("Alice & Bob".to_string())],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let base = std::env::temp_dir().join(format!(
            "gdb-table-export-test-{}",
            std::process::id()
        ));
        let csv_path = base.with_extension("csv");
        let xml_path = base.with_extension("xml");

        let mut csv = TableDataExportWriter::create(
            &csv_path,
            TableDataExportFormat::Csv,
            object.clone(),
            vec!["name".to_string()],
        )
        .unwrap();
        assert_eq!(csv.write_page(&page).unwrap(), 1);
        csv.finish().unwrap();

        let mut xml = TableDataExportWriter::create(
            &xml_path,
            TableDataExportFormat::Xml,
            object,
            vec!["name".to_string()],
        )
        .unwrap();
        assert_eq!(xml.write_page(&page).unwrap(), 1);
        xml.finish().unwrap();

        assert_eq!(fs::read_to_string(&csv_path).unwrap(), "name\nAlice & Bob\n");
        assert!(fs::read_to_string(&xml_path)
            .unwrap()
            .contains("<field name=\"name\">Alice &amp; Bob</field>"));
        let _ = fs::remove_file(csv_path);
        let _ = fs::remove_file(xml_path);
    }

    #[test]
    fn data_table_rows_tsv_uses_visible_row_order() {
        let rows = vec![
            vec![SharedString::from("1"), SharedString::from("Alice")],
            vec![SharedString::from("2"), SharedString::from("Bob")],
            vec![SharedString::from("3"), SharedString::from("Chen")],
        ];

        assert_eq!(
            data_table_rows_tsv(rows.as_slice(), &BTreeSet::from([0, 2])),
            "1\tAlice\n3\tChen"
        );
    }

    #[test]
    fn data_table_cells_tsv_keeps_sparse_shape_and_sanitizes_cells() {
        let rows = vec![
            vec![
                SharedString::from("A1"),
                SharedString::from("A\t2"),
                SharedString::from("A3"),
            ],
            vec![
                SharedString::from("B1"),
                SharedString::from("B2"),
                SharedString::from("B\n3"),
            ],
        ];

        assert_eq!(
            data_table_cells_tsv(rows.as_slice(), &BTreeSet::from([(0, 1), (1, 3)])),
            "A1\t\n\tB 3"
        );
    }

    #[test]
    fn data_table_index_range_selects_inclusive_rows_in_either_direction() {
        assert_eq!(data_table_index_range(2, 5), BTreeSet::from([2, 3, 4, 5]));
        assert_eq!(data_table_index_range(5, 2), BTreeSet::from([2, 3, 4, 5]));
    }

    #[test]
    fn data_table_cell_range_selects_rectangular_region() {
        assert_eq!(
            data_table_cell_range(1, 2, 3, 4),
            BTreeSet::from([
                (1, 2),
                (1, 3),
                (1, 4),
                (2, 2),
                (2, 3),
                (2, 4),
                (3, 2),
                (3, 3),
                (3, 4),
            ])
        );
    }

    #[test]
    fn row_copy_insert_can_skip_primary_keys() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };
        let fields = vec![
            RowFieldSnapshot {
                index: 1,
                name: "id".to_string(),
                type_name: "int".to_string(),
                primary_key: true,
                comment: None,
                value: CellValue::I64(7),
            },
            RowFieldSnapshot {
                index: 2,
                name: "name".to_string(),
                type_name: "varchar(20)".to_string(),
                primary_key: false,
                comment: None,
                value: CellValue::Text("Alice".to_string()),
            },
        ];

        assert_eq!(
            row_insert_sql(&object, fields.as_slice(), false),
            "INSERT INTO `shop`.`users` (`id`, `name`) VALUES (7, 'Alice');"
        );
        assert_eq!(
            row_insert_sql(&object, fields.as_slice(), true),
            "INSERT INTO `shop`.`users` (`name`) VALUES ('Alice');"
        );
    }

    #[test]
    fn failed_app_event_becomes_error_message() {
        let event = AppEvent::Failed(fluxdb_core::UserFacingError {
            title: "连接失败".to_string(),
            message: "Connection refused".to_string(),
            detail: None,
            retryable: true,
        });

        assert_eq!(
            app_event_message(&event),
            Some((
                "连接失败：Connection refused".to_string(),
                AppMessageKind::Error
            ))
        );
    }

    #[test]
    fn local_table_filter_value_toggle_supports_multiselect() {
        let filters = BTreeMap::new();
        let filters = local_table_filters_after_value_toggle(filters, "status", "active");
        let filters = local_table_filters_after_value_toggle(filters, "status", "pending");

        assert_eq!(
            filters.get("status"),
            Some(&BTreeSet::from([
                "active".to_string(),
                "pending".to_string()
            ]))
        );

        let filters = local_table_filters_after_value_toggle(filters, "status", "active");
        assert_eq!(
            filters.get("status"),
            Some(&BTreeSet::from(["pending".to_string()]))
        );
    }

    #[test]
    fn data_filter_rules_convert_to_filter_specs() {
        let specs = data_filter_specs_from_rules(&[DataFilterRule {
            enabled: true,
            field: Some("name".to_string()),
            operator: DataFilterOperator::Contains,
            values: BTreeSet::from(["bike".to_string(), "helmet".to_string()]),
            grouped: false,
        }]);

        assert_eq!(
            specs,
            vec![FilterSpec {
                field: "name".to_string(),
                op: FilterOp::Contains,
                values: vec![
                    CellValue::Text("bike".to_string()),
                    CellValue::Text("helmet".to_string())
                ],
                enabled: true,
            }]
        );
    }

    #[test]
    fn data_filter_values_strip_outer_quotes_when_used() {
        let specs = data_filter_specs_from_rules(&[DataFilterRule {
            enabled: true,
            field: Some("id".to_string()),
            operator: DataFilterOperator::Eq,
            values: BTreeSet::from([
                "\"abc\"".to_string(),
                "'def'".to_string(),
                "“ghi”".to_string(),
                "「jkl」".to_string(),
            ]),
            grouped: false,
        }]);

        assert_eq!(
            specs[0].values,
            vec![
                CellValue::Text("abc".to_string()),
                CellValue::Text("def".to_string()),
                CellValue::Text("ghi".to_string()),
                CellValue::Text("jkl".to_string())
            ]
        );
    }

    #[test]
    fn local_filter_manager_hides_current_editing_field_from_condition_list() {
        let mut filters = BTreeMap::new();
        filters.insert(
            "status".to_string(),
            BTreeSet::from(["active".to_string(), "pending".to_string()]),
        );
        filters.insert("name".to_string(), BTreeSet::from(["tom".to_string()]));

        let entries = local_filter_manager_condition_entries(&filters, Some("status"));

        assert_eq!(
            entries,
            vec![("name".to_string(), BTreeSet::from(["tom".to_string()]))]
        );
    }

    #[test]
    fn data_search_matches_current_page_cells_case_insensitively() {
        let rows = vec![
            vec![SharedString::from("Alpha"), SharedString::from("beta")],
            vec![SharedString::from("草稿-复制"), SharedString::from("other")],
            vec![SharedString::from("copy"), SharedString::from("ALPHA")],
        ];

        assert_eq!(
            data_search_matches(rows.as_slice(), "alpha"),
            vec![
                DataSearchMatch {
                    row_ix: 0,
                    col_ix: 1
                },
                DataSearchMatch {
                    row_ix: 2,
                    col_ix: 2
                },
            ]
        );
        assert_eq!(
            data_search_matches(rows.as_slice(), "草稿"),
            vec![DataSearchMatch {
                row_ix: 1,
                col_ix: 1
            }]
        );
    }

    #[test]
    fn next_data_search_match_wraps_after_current_match() {
        let matches = vec![
            DataSearchMatch {
                row_ix: 0,
                col_ix: 1,
            },
            DataSearchMatch {
                row_ix: 2,
                col_ix: 3,
            },
        ];

        assert_eq!(
            next_data_search_match(matches.as_slice(), None),
            Some(matches[0])
        );
        assert_eq!(
            next_data_search_match(matches.as_slice(), Some(matches[0])),
            Some(matches[1])
        );
        assert_eq!(
            next_data_search_match(matches.as_slice(), Some(matches[1])),
            Some(matches[0])
        );
    }

    #[test]
    fn data_search_match_label_uses_current_position_and_total() {
        let matches = vec![
            DataSearchMatch {
                row_ix: 0,
                col_ix: 1,
            },
            DataSearchMatch {
                row_ix: 2,
                col_ix: 3,
            },
        ];

        assert_eq!(
            data_search_match_label(matches.as_slice(), Some(matches[1])),
            "2/2 匹配"
        );
        assert_eq!(
            data_search_match_label(matches.as_slice(), None),
            "1/2 匹配"
        );
        assert_eq!(data_search_match_label(&[], None), "0/0 匹配");
    }

    #[test]
    fn data_editor_sql_preview_includes_current_page_by_default() {
        let sql = data_editor_sql_preview(
            &ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                name: "users".to_string(),
                kind: ObjectKind::Table,
            },
            &[],
            &[],
            DataFilterMode::Builder,
            "",
            "",
            100,
            100,
        );

        assert_eq!(sql, "SELECT * FROM `main`.`users` LIMIT 100 OFFSET 100");
    }

    #[test]
    fn parses_editable_data_sql_into_filter_and_sort_text() {
        let parsed = parse_data_editor_sql_text(
            "SELECT * FROM `main`.`users` WHERE `name` LIKE '%tom%' AND `age` >= '18' ORDER BY `id` DESC, `name` ASC LIMIT 100 OFFSET 0",
        )
        .expect("simple data SQL should be parsed");

        assert_eq!(parsed.filter_text, "`name` LIKE '%tom%' AND `age` >= '18'");
        assert_eq!(parsed.sort_text, "`id` DESC, `name` ASC");
        assert_eq!(parsed.limit, Some(100));
    }

    #[test]
    fn parses_editable_data_sql_limit() {
        let parsed = parse_data_editor_sql_text("SELECT * FROM `users` LIMIT 250")
            .expect("limit should be parsed");

        assert_eq!(parsed.limit, Some(250));
    }

    #[test]
    fn sql_selection_offset_returns_valid_byte_boundary() {
        let bounds = Bounds::new(point(px(10.), px(0.)), size(px(200.), px(20.)));
        let text = "SELECT * FROM `测试`";

        let offset = sql_text_selection_offset(text, Some(&bounds), point(px(95.), px(4.)));

        assert!(text.is_char_boundary(offset));
    }

    #[test]
    fn byte_index_for_char_index_clamps_to_text_end() {
        assert_eq!(byte_index_for_char_index("测a", 99), "测a".len());
    }

    #[test]
    fn sql_prefix_char_count_clamps_to_text_end() {
        assert_eq!(sql_prefix_char_count("中文 SQL", 999), 6);
    }

    #[test]
    fn tab_context_menu_close_targets_stay_in_current_workspace_scope() {
        let current = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "main".to_string(),
        };
        let other_database = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "audit".to_string(),
        };
        let other_connection = WorkspaceScope {
            connection_id: ConnectionId(2),
            database: "main".to_string(),
        };
        let scopes = vec![
            (TabId(1), Some(current.clone())),
            (TabId(2), Some(current.clone())),
            (TabId(3), Some(other_database)),
            (TabId(4), Some(other_connection)),
            (TabId(5), None),
        ];

        assert_eq!(
            tab_context_menu_close_targets(scopes.iter().cloned(), TabId(1), false),
            vec![TabId(2)]
        );
        assert_eq!(
            tab_context_menu_close_targets(scopes.iter().cloned(), TabId(1), true),
            vec![TabId(1), TabId(2)]
        );
    }

    #[test]
    fn tab_row_overflow_uses_fixed_tab_width() {
        assert!(!tab_row_overflows(3, 260., 900.));
        assert!(tab_row_overflows(5, 260., 900.));
    }

    #[test]
    fn tab_switcher_popup_top_tracks_tab_layout() {
        assert_eq!(
            tab_switcher_popup_top(TabSwitcherKind::Databases, true),
            32.
        );
        assert_eq!(tab_switcher_popup_top(TabSwitcherKind::Tables, true), 66.);
        assert_eq!(tab_switcher_popup_top(TabSwitcherKind::Tables, false), 30.);
    }

    #[test]
    fn pinning_tabs_appends_to_pinned_group_in_order() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let mut pinned = BTreeSet::from([TabId(2)]);
        let order = tab_order_after_pin(&ids, &ids, &pinned, TabId(4));
        pinned.insert(TabId(4));

        assert_eq!(
            tab_display_order(&ids, &order, &pinned),
            vec![TabId(2), TabId(4), TabId(1), TabId(3),]
        );
    }

    #[test]
    fn unpinning_tab_moves_it_after_last_remaining_pinned_tab() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::from([TabId(2), TabId(4)]);

        assert_eq!(
            tab_order_after_unpin(&ids, &ids, &pinned, TabId(2)),
            vec![TabId(4), TabId(2), TabId(1), TabId(3),]
        );

        let mut remaining_pinned = pinned;
        remaining_pinned.remove(&TabId(2));
        assert_eq!(
            tab_display_order(
                &ids,
                &tab_order_after_unpin(&ids, &ids, &BTreeSet::from([TabId(2), TabId(4)]), TabId(2)),
                &remaining_pinned
            ),
            vec![TabId(4), TabId(2), TabId(1), TabId(3)]
        );
    }

    #[test]
    fn unpinning_last_pinned_tab_keeps_visual_position() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::from([TabId(2), TabId(4)]);
        let order = tab_order_after_unpin(&ids, &ids, &pinned, TabId(4));
        let mut remaining_pinned = pinned;
        remaining_pinned.remove(&TabId(4));

        assert_eq!(
            tab_display_order(&ids, &order, &remaining_pinned),
            vec![TabId(2), TabId(4), TabId(1), TabId(3)]
        );
    }

    #[test]
    fn dragging_within_same_tab_group_inserts_by_direction() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::new();

        let forward = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(4), TabId(2));
        assert_eq!(forward.pinned, None);
        assert_eq!(forward.order, vec![TabId(1), TabId(4), TabId(2), TabId(3)]);

        let backward = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(1), TabId(3));
        assert_eq!(backward.pinned, None);
        assert_eq!(backward.order, vec![TabId(2), TabId(3), TabId(1), TabId(4)]);
    }

    #[test]
    fn dragging_across_pinned_boundary_clamps_to_expected_group() {
        let ids = vec![TabId(1), TabId(2), TabId(3), TabId(4)];
        let pinned = BTreeSet::from([TabId(1), TabId(2)]);

        let pinned_to_normal = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(1), TabId(3));
        assert_eq!(pinned_to_normal.pinned, Some(false));
        assert_eq!(
            pinned_to_normal.order,
            vec![TabId(2), TabId(3), TabId(4), TabId(1),]
        );

        let normal_to_pinned = tab_order_after_tab_drop(&ids, &ids, &pinned, TabId(4), TabId(1));
        assert_eq!(normal_to_pinned.pinned, None);
        assert_eq!(
            normal_to_pinned.order,
            vec![TabId(1), TabId(2), TabId(4), TabId(3),]
        );
    }

    #[test]
    fn workspace_tabs_are_reordered_by_drag_direction() {
        let a = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "a".to_string(),
        };
        let b = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "b".to_string(),
        };
        let c = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "c".to_string(),
        };
        let scopes = vec![a.clone(), b.clone(), c.clone()];

        assert_eq!(
            workspace_tab_order_after_drop(&scopes, &scopes, &c, &b),
            vec![a.clone(), c.clone(), b.clone()]
        );
        assert_eq!(
            workspace_tab_order_after_drop(&scopes, &scopes, &a, &b),
            vec![b.clone(), a.clone(), c.clone()]
        );
    }

    #[test]
    fn tab_switcher_entries_filter_by_search_text() {
        let mut state = AppState::default();
        state.tabs = vec![
            TabState {
                id: TabId(1),
                title: "shining_agent_chat_know_rel".to_string(),
                kind: TabKind::QueryEditor(QueryEditorState {
                    connection_id: ConnectionId(1),
                    database: Some("data_centre_cloud".to_string()),
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
            },
            TabState {
                id: TabId(2),
                title: "3d_device_model".to_string(),
                kind: TabKind::QueryEditor(QueryEditorState {
                    connection_id: ConnectionId(1),
                    database: Some("data_centre_cloud".to_string()),
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
            },
        ];
        state.active_tab = Some(TabId(1));
        let scope = WorkspaceScope {
            connection_id: ConnectionId(1),
            database: "data_centre_cloud".to_string(),
        };

        let entries = table_tab_entries(&state, Some(&scope), "device", &[], &BTreeSet::new());

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, TabId(2));
    }

    #[test]
    fn app_icons_use_lucide_svg_assets() {
        assert_eq!(app_icon_path(AppIcon::Check), "icons/check.svg");
        assert_eq!(app_icon_path(AppIcon::Copy), "icons/copy-plus.svg");
        assert_eq!(app_icon_path(AppIcon::Close), "icons/x.svg");
        assert_eq!(
            app_icon_path(AppIcon::ChevronDown),
            "icons/chevron-down.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronLeft),
            "icons/chevron-left.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronRight),
            "icons/chevron-right.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronsLeft),
            "icons/chevrons-left.svg"
        );
        assert_eq!(
            app_icon_path(AppIcon::ChevronsRight),
            "icons/chevrons-right.svg"
        );
        assert_eq!(app_icon_path(AppIcon::Pin), "icons/pin.svg");
        assert_eq!(app_icon_path(AppIcon::Table), "icons/table-2.svg");
        assert_eq!(app_icon_path(AppIcon::Database), "icons/database.svg");
        assert_eq!(app_icon_path(AppIcon::Eye), "icons/eye.svg");
        assert_eq!(app_icon_path(AppIcon::EyeOff), "icons/eye-off.svg");
        assert_eq!(app_icon_path(AppIcon::List), "icons/list.svg");
        assert_eq!(app_icon_path(AppIcon::Minus), "icons/minus.svg");
        assert_eq!(app_icon_path(AppIcon::PanelBottom), "icons/panel-bottom.svg");
        assert_eq!(app_icon_path(AppIcon::PanelRight), "icons/panel-right.svg");
        assert_eq!(app_icon_path(AppIcon::Redo), "icons/redo-2.svg");
        assert_eq!(app_icon_path(AppIcon::Square), "icons/square.svg");
        assert_eq!(app_icon_path(AppIcon::Undo), "icons/undo-2.svg");
        assert_eq!(app_icon_path(AppIcon::AlignLeft), "icons/align-left.svg");
        assert_eq!(app_icon_path(AppIcon::ArrowUpDown), "icons/arrow-up-down.svg");
        assert_eq!(app_icon_path(AppIcon::FileSearch), "icons/file-search.svg");
        assert_eq!(app_icon_path(AppIcon::Workflow), "icons/workflow.svg");
        assert_eq!(app_icon_path(AppIcon::Bot), "icons/bot.svg");
        assert_eq!(app_icon_path(AppIcon::Select), "icons/text-select.svg");
        assert_eq!(app_icon_path(AppIcon::Text), "icons/text.svg");
        assert_eq!(app_icon_path(AppIcon::WrapText), "icons/wrap-text.svg");
    }

    #[test]
    fn sql_text_selection_returns_selected_text() {
        let selection = SqlTextSelection {
            text: "SELECT 123".to_string(),
            anchor: 0,
            cursor: 6,
            selecting: false,
            bounds: None,
        };

        assert_eq!(selection.selected_text().as_deref(), Some("SELECT"));
    }

    #[test]
    fn data_change_sql_preview_formats_update_insert_and_delete() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop-db".to_string()),
            schema: None,
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "status".to_string(),
                    type_name: Some("varchar(20)".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "note".to_string(),
                    type_name: Some("varchar(255)".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: Vec::new(),
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let changes = DataChangeSet {
            object,
            inserts: vec![fluxdb_core::Row {
                values: vec![
                    CellValue::I64(2),
                    CellValue::Text("new".to_string()),
                    CellValue::Null,
                ],
            }],
            updates: vec![fluxdb_core::RowUpdate {
                identity: RowIdentity {
                    values: BTreeMap::from([("id".to_string(), CellValue::I64(1))]),
                },
                cells: vec![fluxdb_core::CellUpdate {
                    column: "status".to_string(),
                    value: CellValue::Text("Bob's order".to_string()),
                }],
            }],
            deletes: vec![RowIdentity {
                values: BTreeMap::from([("id".to_string(), CellValue::I64(3))]),
            }],
        };

        let preview = data_change_sql_preview(&page, &changes);

        assert_eq!(data_change_statement_count(&changes), 3);
        assert!(
            preview.contains(
                "UPDATE `shop-db`.`orders` SET `status` = 'Bob''s order' WHERE `id` = 1;"
            )
        );
        assert!(
            preview.contains("INSERT INTO `shop-db`.`orders` (`id`, `status`) VALUES (2, 'new');")
        );
        assert!(!preview.contains("`note`"));
        assert!(preview.contains("DELETE FROM `shop-db`.`orders` WHERE `id` = 3;"));
    }

    #[test]
    fn data_change_sql_preview_uses_default_values_for_all_null_insert() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop-db".to_string()),
            schema: None,
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        };
        let page = DataPage {
            columns: vec![GdbColumn {
                name: "note".to_string(),
                type_name: Some("varchar(255)".to_string()),
                nullable: true,
                primary_key: false,
                comment: None,
            }],
            rows: Vec::new(),
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let changes = DataChangeSet {
            object,
            inserts: vec![fluxdb_core::Row {
                values: vec![CellValue::Null],
            }],
            updates: Vec::new(),
            deletes: Vec::new(),
        };

        let preview = data_change_sql_preview(&page, &changes);

        assert_eq!(preview, "INSERT INTO `shop-db`.`orders` DEFAULT VALUES;");
    }

    #[test]
    fn data_change_deleted_rows_maps_delete_identities_to_row_indexes() {
        let page = DataPage {
            columns: vec![GdbColumn {
                name: "id".to_string(),
                type_name: Some("int".to_string()),
                nullable: false,
                primary_key: true,
                comment: None,
            }],
            rows: vec![
                fluxdb_core::Row {
                    values: vec![CellValue::I64(1)],
                },
                fluxdb_core::Row {
                    values: vec![CellValue::I64(2)],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let changes = DataChangeSet {
            object: ObjectPath {
                connection_id: ConnectionId(1),
                database: None,
                schema: None,
                name: "orders".to_string(),
                kind: ObjectKind::Table,
            },
            inserts: Vec::new(),
            updates: Vec::new(),
            deletes: vec![RowIdentity {
                values: BTreeMap::from([("id".to_string(), CellValue::I64(2))]),
            }],
        };

        assert_eq!(
            data_change_deleted_rows(&page, Some(&changes)),
            BTreeSet::from([1])
        );
    }

    #[test]
    fn new_query_scope_auto_database_only_for_single_context_kinds() {
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::Sqlite, None)),
            Some("main".to_string())
        );
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::Redis, Some("3"))),
            Some("3".to_string())
        );
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::MySql, Some("app"))),
            None
        );
        assert_eq!(
            automatic_query_database(&query_scope_config(DatabaseKind::MongoDb, Some("app"))),
            None
        );
    }

    #[test]
    fn user_admin_role_tabs_do_not_reload_loaded_grants() {
        let user = DatabaseUserIdentity {
            user: "app".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let mut admin = UserAdminState::new(ConnectionId(1), None, fluxdb_core::PrivilegeScope::MySql);
        admin.selected_user = Some(user.clone());

        assert_eq!(
            user_admin_detail_tab_grants_load_user(UserAdminDetailTab::Advanced, &admin),
            None
        );
        assert_eq!(
            user_admin_detail_tab_grants_load_user(UserAdminDetailTab::MemberOf, &admin),
            Some(user.clone())
        );

        admin.grants_loaded_user = Some(user.clone());
        assert_eq!(
            user_admin_detail_tab_grants_load_user(UserAdminDetailTab::MemberOf, &admin),
            None
        );

        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::Advanced, &admin),
            None
        );
        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::MemberOf, &admin),
            Some(user.clone())
        );
        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::Members, &admin),
            Some(user.clone())
        );

        admin.member_grants_loaded_role = Some(user);
        assert_eq!(
            user_admin_detail_tab_members_load_role(UserAdminDetailTab::Members, &admin),
            None
        );
    }

    #[test]
    fn user_admin_sql_preview_collects_multiple_statements_on_separate_lines() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let user = DatabaseUserIdentity {
            user: "app".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let role = DatabaseUserIdentity {
            user: "reader".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let mut admin = UserAdminState::new(ConnectionId(1), Some("app".to_string()), fluxdb_core::PrivilegeScope::MySql);
        admin.users = vec![user.clone(), role.clone()];
        admin.selected_user = Some(user.clone());
        admin.set_role_membership_granted(role.clone(), true);
        admin.set_role_membership_default(role, true);
        admin.add_privilege_row("app".to_string());
        let row_id = admin.privilege_rows[0].id;
        admin.toggle_privilege_row_privilege(row_id, "SELECT".to_string());

        let sql = user_admin_all_sql_preview(provider, &admin).sql();

        assert!(sql.contains('\n'));
        assert!(sql.contains("GRANT 'reader'@'%' TO 'app'@'%';"));
        assert!(sql.contains("SET DEFAULT ROLE 'reader'@'%' TO 'app'@'%';"));
        assert!(sql.contains("GRANT SELECT ON `app`.* TO 'app'@'%';"));
    }

    #[test]
    fn user_admin_sql_preview_includes_general_and_advanced_changes() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let user = DatabaseUserIdentity {
            user: "app".to_string(),
            host: "%".to_string(),
            plugin: Some("caching_sha2_password".to_string()),
        };
        let mut admin = UserAdminState::new(
            ConnectionId(1),
            Some("app".to_string()),
            fluxdb_core::PrivilegeScope::MySql,
        );
        admin.selected_user = Some(user);
        admin.auth_plugin = "mysql_native_password".to_string();
        admin.password_expiry_policy = "NEVER".to_string();
        admin.max_queries_per_hour = "10".to_string();
        admin.max_user_connections = "3".to_string();
        admin.ssl_type = "ANY".to_string();

        let sql = user_admin_all_sql_preview(provider, &admin).sql();

        assert!(sql.contains(
            "ALTER USER 'app'@'%' IDENTIFIED WITH `mysql_native_password`;"
        ));
        assert!(sql.contains("ALTER USER 'app'@'%' PASSWORD EXPIRE NEVER;"));
        assert!(sql.contains(
            "ALTER USER 'app'@'%' WITH MAX_QUERIES_PER_HOUR 10 MAX_USER_CONNECTIONS 3;"
        ));
        assert!(sql.contains("ALTER USER 'app'@'%' REQUIRE SSL;"));
        assert!(user_admin_can_save_all(provider, &admin));
    }

    #[test]
    fn connection_tree_does_not_invent_main_database_for_empty_objects() {
        let mut connection = ConnectionState {
            config: query_scope_config(DatabaseKind::MySql, Some("main")),
            connected: true,
            expanded: true,
            objects: Vec::new(),
            redis_overview: RedisConnectionOverview::default(),
        };

        assert!(connection_databases(&connection).is_empty());

        connection.objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                name: "orders".to_string(),
                kind: ObjectKind::Table,
            },
            rows: None,
            comment: None,
            modified_at: None,
        });

        assert_eq!(connection_databases(&connection).len(), 1);
    }

    #[test]
    fn next_table_folder_name_avoids_existing_names() {
        let existing = vec![
            "新建组".to_string(),
            "新建组 1".to_string(),
            "业务分组".to_string(),
        ];

        assert_eq!(next_table_folder_name(&existing), "新建组 2");
    }

    #[test]
    fn table_folders_keep_custom_order_above_pinned_tables() {
        let connection_id = ConnectionId(1);
        let mut folders = BTreeMap::new();
        assert_eq!(
            table_folder_parent_key(connection_id, "main"),
            object_group_tree_key(connection_id, "main", ObjectGroup::Tables)
        );
        folders.insert(
            table_folder_parent_key(connection_id, "main"),
            vec!["z-folder".to_string(), "a-folder".to_string()],
        );
        let folder_names = sorted_table_folders(&folders, connection_id, "main")
            .into_iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(folder_names, vec!["z-folder", "a-folder"]);

        let connection = ConnectionState {
            config: query_scope_config(DatabaseKind::MySql, Some("main")),
            connected: true,
            expanded: true,
            objects: vec![
                ObjectSummary {
                    path: ObjectPath {
                        connection_id,
                        database: Some("main".to_string()),
                        schema: None,
                        name: "orders".to_string(),
                        kind: ObjectKind::Table,
                    },
                    rows: None,
                    comment: None,
                    modified_at: None,
                },
                ObjectSummary {
                    path: ObjectPath {
                        connection_id,
                        database: Some("main".to_string()),
                        schema: None,
                        name: "users".to_string(),
                        kind: ObjectKind::Table,
                    },
                    rows: None,
                    comment: None,
                    modified_at: None,
                },
            ],
            redis_overview: RedisConnectionOverview::default(),
        };
        let mut pinned = BTreeSet::new();
        pinned.insert(table_tree_key(&connection.objects[1].path));
        let table_names = sorted_group_objects(&connection, "main", ObjectGroup::Tables, &pinned)
            .into_iter()
            .map(|object| object.path.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(table_names, vec!["users", "orders"]);

        let mut assignments = BTreeMap::new();
        assignments.insert(
            table_tree_key(&connection.objects[1].path),
            (
                table_folder_parent_key(connection_id, "main"),
                "a-folder".to_string(),
            ),
        );
        let folder_tables = sorted_folder_table_objects(
            &connection,
            "main",
            &table_folder_parent_key(connection_id, "main"),
            "a-folder",
            &pinned,
            &assignments,
        )
        .into_iter()
        .map(|object| object.path.name.as_str())
        .collect::<Vec<_>>();
        let unassigned_tables = sorted_unassigned_group_objects(
            &connection,
            "main",
            ObjectGroup::Tables,
            &pinned,
            &assignments,
        )
        .into_iter()
        .map(|object| object.path.name.as_str())
        .collect::<Vec<_>>();

        assert_eq!(folder_tables, vec!["users"]);
        assert_eq!(unassigned_tables, vec!["orders"]);
    }

    #[test]
    fn move_table_folder_name_swaps_with_neighbor() {
        let mut folders = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        move_table_folder_name(&mut folders, "b", -1);
        assert_eq!(folders, vec!["b", "a", "c"]);

        move_table_folder_name(&mut folders, "b", -1);
        assert_eq!(folders, vec!["b", "a", "c"]);

        move_table_folder_name(&mut folders, "b", 1);
        assert_eq!(folders, vec!["a", "b", "c"]);

        move_table_folder_name(&mut folders, "missing", 1);
        assert_eq!(folders, vec!["a", "b", "c"]);
    }

    #[test]
    fn redis_filter_page_filters_by_type_and_key_mode() {
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "键".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "类型".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![
                Row {
                    values: vec![
                        CellValue::Text("user:test:1".to_string()),
                        CellValue::Text("string".to_string()),
                    ],
                },
                Row {
                    values: vec![
                        CellValue::Text("orders:test".to_string()),
                        CellValue::Text("hash".to_string()),
                    ],
                },
            ],
            offset: 0,
            limit: 100,
            has_more: false,
        };

        // 无显式模式，统一按 Redis SCAN MATCH 通配符语义匹配（对齐 RedisInsight 的 Pattern 检索）。
        // `*test*` 包含匹配两个键；`*test` 后缀匹配；`user:*` 前缀匹配；`orders:test` 精确匹配。
        let contains = redis_filter_page(&page, "所有", "*test*");
        let suffix_hash = redis_filter_page(&page, "hash", "*test");
        let prefix = redis_filter_page(&page, "所有", "user:*");
        let exact = redis_filter_page(&page, "所有", "orders:test");

        assert_eq!(contains.rows.len(), 2);
        assert_eq!(suffix_hash.rows.len(), 1);
        assert_eq!(
            suffix_hash.rows[0].values[0],
            CellValue::Text("orders:test".to_string())
        );
        assert_eq!(prefix.rows.len(), 1);
        assert_eq!(
            prefix.rows[0].values[0],
            CellValue::Text("user:test:1".to_string())
        );
        assert_eq!(exact.rows.len(), 1);
        assert_eq!(
            exact.rows[0].values[0],
            CellValue::Text("orders:test".to_string())
        );
    }

    #[test]
    #[test]
    fn redis_glob_matches_follows_scan_match_wildcards() {
        // `*` 匹配零或多个任意字符，`?` 匹配单个字符，其余按字面匹配（对齐 Redis SCAN MATCH）。
        assert!(redis_glob_matches("user:test:1", "user:*"));
        assert!(redis_glob_matches("user:test:1", "*test*"));
        assert!(redis_glob_matches("orders:test", "*test"));
        assert!(redis_glob_matches("a1c", "a?c"));
        assert!(redis_glob_matches("abc", "abc"));
        assert!(redis_glob_matches("abc", "*"));
        assert!(!redis_glob_matches("user:test:1", "orders:*"));
        assert!(!redis_glob_matches("abc", "abd"));
        // `?` 只能匹配单个字符，不能跨多个字符。
        assert!(!redis_glob_matches("abcd", "a?d"));
    }

    fn redis_key_detail_uses_named_columns() {
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "类型".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "TTL".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "键".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "值".to_string(),
                    type_name: Some("redis".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![Row {
                values: vec![
                    CellValue::Text("string".to_string()),
                    CellValue::Text("无 TTL".to_string()),
                    CellValue::Text("test".to_string()),
                    CellValue::Text("13".to_string()),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };

        assert_eq!(
            redis_key_detail_for_row(&page, 0),
            Some(RedisKeyDetail {
                key: "test".to_string(),
                kind: "string".to_string(),
                value: "13".to_string(),
                ttl: "无 TTL".to_string(),
                size: String::new(),
            })
        );
    }

    #[test]
    fn redis_key_detail_refresh_kind_maps_known_types() {
        assert_eq!(
            RedisKeyDetail {
                key: String::new(),
                kind: "hash".to_string(),
                value: String::new(),
                ttl: String::new(),
                size: String::new(),
            }
            .refresh_kind(),
            RedisKeyDetailRefreshKind::Hash
        );
        assert_eq!(
            RedisKeyDetail {
                key: String::new(),
                kind: "stream".to_string(),
                value: String::new(),
                ttl: String::new(),
                size: String::new(),
            }
            .refresh_kind(),
            RedisKeyDetailRefreshKind::Stream
        );
        assert_eq!(
            RedisKeyDetail {
                key: String::new(),
                kind: "string".to_string(),
                value: String::new(),
                ttl: String::new(),
                size: String::new(),
            }
            .refresh_kind(),
            RedisKeyDetailRefreshKind::Key
        );
    }

    #[test]
    fn redis_table_column_widths_follow_requested_ratios() {
        let table_width = px(950.);
        let assert_width = |actual: Pixels, expected: f32| {
            assert!((f32::from(actual) - expected).abs() < 0.01);
        };

        assert_width(redis_table_column_width("键", table_width), 120.);
        assert_width(redis_table_column_width("类型", table_width), 100.);
        assert_width(redis_table_column_width("值", table_width), 450.);
        assert_width(redis_table_column_width("大小", table_width), 80.);
        assert_width(redis_table_column_width("TTL", table_width), 80.);
        assert_width(redis_table_column_width("__redis_actions", table_width), 120.);
    }

    #[test]
    fn redis_table_fit_width_keeps_columns_inside_bounds() {
        let bounds_width = px(1000.);
        let table_width = redis_table_fit_width(bounds_width);
        let total_width: f32 = ["键", "类型", "值", "大小", "TTL", "__redis_actions"]
            .iter()
            .map(|column| f32::from(redis_table_column_width(column, table_width)))
            .sum();

        assert!(total_width <= f32::from(bounds_width));
    }

    #[test]
    fn redis_hash_field_ttl_command_maps_input_to_write_semantics() {
        // 没改 TTL 这一格 → 必须保留原 TTL，否则 HSET 会把它清掉
        assert_eq!(
            redis_hash_field_ttl_command("120", false),
            Ok(RedisHashFieldTtl::Keep)
        );
        // 清空 → 永不过期
        assert_eq!(
            redis_hash_field_ttl_command("  ", true),
            Ok(RedisHashFieldTtl::Persist)
        );
        assert_eq!(
            redis_hash_field_ttl_command("120", true),
            Ok(RedisHashFieldTtl::Seconds(120))
        );
        assert!(redis_hash_field_ttl_command("0", true).is_err());
        assert!(redis_hash_field_ttl_command("abc", true).is_err());
    }

    #[test]
    fn redis_hash_field_ttl_display_value_strips_seconds_suffix_for_table_view() {
        assert_eq!(redis_hash_field_ttl_display_value("51182034s"), "51182034");
        assert_eq!(redis_hash_field_ttl_display_value("1s"), "1");
        assert_eq!(redis_hash_field_ttl_display_value("无 TTL"), "无 TTL");
        assert_eq!(redis_hash_field_ttl_display_value("已过期"), "已过期");
    }

    #[test]
    fn redis_option_pairs_only_allow_known_keys() {
        let pairs = redis_option_pairs(
            "tls=true&sentinel_master=mymaster&evil=1&tls_server_name=redis.example.com&tls=",
        );

        assert_eq!(
            pairs,
            vec![
                ("tls".to_string(), "true".to_string()),
                ("sentinel_master".to_string(), "mymaster".to_string()),
                ("tls_server_name".to_string(), "redis.example.com".to_string()),
            ]
        );
    }

    #[test]
    fn redis_stream_time_input_round_trips() {
        let millis = redis_stream_time_from_input("2026-08-09 12:34:56")
            .unwrap()
            .unwrap();

        // 回填输入框的文本必须能被原样解析回同一个时间戳
        let text = redis_stream_time_to_input(millis);
        assert_eq!(text, "2026-08-09 12:34:56");
        assert_eq!(redis_stream_time_from_input(&text), Ok(Some(millis)));
    }

    #[test]
    fn redis_stream_maxlen_from_input_requires_positive_integer() {
        assert_eq!(redis_stream_maxlen_from_input("  "), Ok(None));
        assert_eq!(redis_stream_maxlen_from_input("1000"), Ok(Some(1000)));
        assert!(redis_stream_maxlen_from_input("0").is_err());
        assert!(redis_stream_maxlen_from_input("-5").is_err());
        assert!(redis_stream_maxlen_from_input("abc").is_err());
    }

    #[test]
    fn redis_stream_time_from_input_parses_local_time_forms() {
        assert_eq!(redis_stream_time_from_input(""), Ok(None));

        let full = redis_stream_time_from_input("2026-08-09 12:00:00").unwrap();
        let minute = redis_stream_time_from_input("2026-08-09 12:00").unwrap();
        let date_only = redis_stream_time_from_input("2026-08-09").unwrap();

        assert_eq!(full, minute);
        // 只写日期按当天 00:00:00 处理，因此比 12:00 早 12 小时
        assert_eq!(full.unwrap() - date_only.unwrap(), 12 * 3600 * 1000);
        assert!(redis_stream_time_from_input("09/08/2026").is_err());
    }

    #[test]
    fn redis_stream_entry_columns_collect_dynamic_fields() {
        let entries = vec![
            RedisStreamEntryRow {
                id: "1785677482094-0".to_string(),
                time: "2026-08-02 21:31:22".to_string(),
                fields: [("test".to_string(), "3".to_string())]
                    .into_iter()
                    .collect(),
            },
            RedisStreamEntryRow {
                id: "1785677482090-0".to_string(),
                time: "2026-08-02 21:31:22".to_string(),
                fields: [
                    ("test".to_string(), "11".to_string()),
                    ("test1".to_string(), "22".to_string()),
                ]
                .into_iter()
                .collect(),
            },
        ];

        let columns = redis_stream_entry_columns(&entries);

        assert_eq!(columns, vec!["test".to_string(), "test1".to_string()]);
        assert_eq!(entries[0].fields.get("test"), Some(&"3".to_string()));
        assert_eq!(entries[1].fields.get("test1"), Some(&"22".to_string()));
    }

    #[test]
    fn redis_stream_entry_field_pair_validates_field_name() {
        let field = redis_stream_entry_field_pair("name".to_string(), "alice".to_string()).unwrap();

        assert_eq!(field, ("name".to_string(), "alice".to_string()));
        assert!(redis_stream_entry_field_pair(" ".to_string(), "alice".to_string()).is_err());
    }

    #[test]
    fn redis_stream_entry_id_validation_accepts_star_or_timestamp_sequence() {
        assert_eq!(redis_stream_entry_id_validation_error("*"), None);
        assert_eq!(redis_stream_entry_id_validation_error("1700000000000-0"), None);
        assert!(redis_stream_entry_id_validation_error("1700000000000").is_some());
        assert!(redis_stream_entry_id_validation_error("abc-0").is_some());
    }

    #[test]
    fn redis_stream_entry_field_pairs_from_snapshot_collects_multiple_rows() {
        let fields = redis_stream_entry_field_pairs_from_snapshot(&[
            ("name".to_string(), "alice".to_string()),
            ("city".to_string(), "shanghai".to_string()),
        ])
        .unwrap();

        assert_eq!(
            fields,
            vec![
                ("name".to_string(), "alice".to_string()),
                ("city".to_string(), "shanghai".to_string())
            ]
        );
    }

    #[test]
    fn redis_set_preview_members_parse_rows() {
        let (summary, members) = redis_set_preview_members("2 成员\ntest\ntest1");

        assert_eq!(summary, "2 成员");
        assert_eq!(members, vec!["test".to_string(), "test1".to_string()]);
    }

    #[test]
    fn redis_pretty_json_formats_only_valid_json() {
        assert_eq!(
            redis_pretty_json(r#"{"name":"test","items":[1,2]}"#).unwrap(),
            "{\n  \"name\": \"test\",\n  \"items\": [\n    1,\n    2\n  ]\n}"
        );
        assert!(redis_pretty_json("not json").is_err());
    }

    #[test]
    fn redis_ttl_input_value_keeps_only_seconds() {
        assert_eq!(redis_ttl_input_value("(No TTL)"), "");
        assert_eq!(redis_ttl_input_value("120s"), "120");
    }

    #[test]
    fn redis_key_meta_actions_stay_visible_while_editing() {
        assert!(redis_key_meta_actions_visible(
            false,
            false,
            Some(RedisKeyMetaField::Ttl)
        ));
        assert!(redis_key_meta_actions_enabled(
            false,
            false,
            Some(RedisKeyMetaField::Ttl)
        ));
        assert!(!redis_key_meta_actions_enabled(
            true,
            true,
            Some(RedisKeyMetaField::Ttl)
        ));
        assert!(!redis_key_meta_actions_visible(false, false, None));
    }

    fn query_scope_config(kind: DatabaseKind, database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(1),
            name: "test".to_string(),
            kind,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: database.map(ToString::to_string),
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        }
    }

    #[test]
    fn redis_hash_value_is_truncated_detects_prefix_only() {
        // 带截断标记前缀 → true
        assert!(redis_hash_value_is_truncated(
            "[Truncated due to length] the rest..."
        ));
        // 普通值 → false
        assert!(!redis_hash_value_is_truncated("plain value"));
        // 前缀在串中段 → false（只认前缀）
        assert!(!redis_hash_value_is_truncated(
            "not [Truncated due to length] at start"
        ));
        // 仅前缀本身（无后续字符）→ true（starts_with 语义）
        assert!(redis_hash_value_is_truncated(REDIS_HASH_TRUNCATED_MARKER));
    }

    #[test]
    fn dangerous_redis_command_detects_destructive_commands() {
        // 破坏性/高危命令：独立成行或跟在参数后面均需识别
        assert!(is_dangerous_redis_command("FLUSHDB"));
        assert!(is_dangerous_redis_command("flushall async"));
        assert!(is_dangerous_redis_command("SHUTDOWN NOSAVE"));
        assert!(is_dangerous_redis_command("DEBUG SEGFAULT"));
        assert!(is_dangerous_redis_command("SLAVEOF 1.2.3.4 6379"));
        assert!(is_dangerous_redis_command("REPLICAOF no one"));
        assert!(is_dangerous_redis_command("CLUSTER RESET HARD"));
        assert!(is_dangerous_redis_command("MIGRATE 1.2.3.4 6379 key 0 5000"));
        assert!(is_dangerous_redis_command("CLIENT PAUSE 30000"));
    }

    #[test]
    fn dangerous_redis_command_ignores_safe_commands() {
        assert!(!is_dangerous_redis_command(""));
        assert!(!is_dangerous_redis_command("GET foo"));
        assert!(!is_dangerous_redis_command("SET foo bar"));
        assert!(!is_dangerous_redis_command("DEL foo bar"));
        assert!(!is_dangerous_redis_command("LPUSH mylist a"));
        // 大小写与空白不干扰安全判断
        assert!(!is_dangerous_redis_command("  set foo bar  "));
    }

    #[test]
    fn redis_database_row_click_does_not_expand() {
        // 回归：Redis 数据库节点整行点击不得触发展开/加载 key，展开只由箭头负责。
        assert!(!database_row_click_expands(ObjectKind::RedisDb));
    }

    #[test]
    fn other_database_row_click_still_expands() {
        // MySQL/SQLite 等数据库类型保持整行点击展开/加载子节点。
        assert!(database_row_click_expands(ObjectKind::Database));
        assert!(database_row_click_expands(ObjectKind::Schema));
        assert!(database_row_click_expands(ObjectKind::Collection));
    }

    fn data_filter_rule(
        field: &str,
        operator: DataFilterOperator,
        values: &[&str],
        grouped: bool,
    ) -> DataFilterRule {
        DataFilterRule {
            enabled: true,
            field: Some(field.to_string()),
            operator,
            values: values.iter().map(|value| (*value).to_string()).collect(),
            grouped,
        }
    }

    fn assert_data_filter_round_trip(rules: Vec<DataFilterRule>) {
        let sql = data_filter_rules_sql_pretty(&rules);
        let parsed = parse_data_filter_rules_text(&sql).expect("should parse");
        assert_eq!(parsed, rules);
    }

    #[test]
    fn data_filter_round_trips_all_operators() {
        for case in [
            data_filter_rule("id", DataFilterOperator::Eq, &["1"], false),
            data_filter_rule("id", DataFilterOperator::Eq, &["1", "2"], false),
            data_filter_rule("id", DataFilterOperator::Ne, &["1", "2"], false),
            data_filter_rule("name", DataFilterOperator::Contains, &["a", "b"], false),
            data_filter_rule("name", DataFilterOperator::NotContains, &["a", "b"], false),
            data_filter_rule("name", DataFilterOperator::StartsWith, &["ab"], false),
            data_filter_rule("name", DataFilterOperator::NotStartsWith, &["ab"], false),
            data_filter_rule("name", DataFilterOperator::EndsWith, &["xy"], false),
            data_filter_rule("name", DataFilterOperator::NotEndsWith, &["xy"], false),
            data_filter_rule("age", DataFilterOperator::Gt, &["18"], false),
            data_filter_rule("age", DataFilterOperator::Ge, &["18"], false),
            data_filter_rule("age", DataFilterOperator::Lt, &["65"], false),
            data_filter_rule("age", DataFilterOperator::Le, &["65"], false),
            data_filter_rule("age", DataFilterOperator::Between, &["18", "65"], false),
            data_filter_rule("age", DataFilterOperator::NotBetween, &["18", "65"], false),
            data_filter_rule("tag", DataFilterOperator::InList, &["a", "b"], false),
            data_filter_rule("tag", DataFilterOperator::NotInList, &["a", "b"], false),
            data_filter_rule("deleted_at", DataFilterOperator::IsNull, &[], false),
            data_filter_rule("deleted_at", DataFilterOperator::IsNotNull, &[], false),
            data_filter_rule("title", DataFilterOperator::IsEmpty, &[], false),
            data_filter_rule("title", DataFilterOperator::IsNotEmpty, &[], false),
        ] {
            assert_data_filter_round_trip(vec![case]);
        }
    }

    #[test]
    fn data_filter_round_trips_groups_and_between() {
        assert_data_filter_round_trip(vec![
            data_filter_rule("age", DataFilterOperator::Between, &["18", "65"], false),
            data_filter_rule("status", DataFilterOperator::Eq, &["active", "pending"], false),
        ]);

        assert_data_filter_round_trip(vec![
            data_filter_rule("a", DataFilterOperator::Eq, &["1"], true),
            data_filter_rule("b", DataFilterOperator::Eq, &["2"], true),
            data_filter_rule("c", DataFilterOperator::IsNull, &[], false),
        ]);
    }

    #[test]
    fn data_filter_null_like_operators_do_not_require_values() {
        assert!(!DataFilterOperator::IsNull.requires_values());
        assert!(!DataFilterOperator::IsNotNull.requires_values());
        assert!(!DataFilterOperator::IsEmpty.requires_values());
        assert!(!DataFilterOperator::IsNotEmpty.requires_values());
        assert!(DataFilterOperator::Between.requires_values());
    }

    #[test]
    fn loaded_schema_context_extracts_tables_and_columns() {
        // 一个连接：main 库有 orders(表) / users(表) / v_orders(视图) / other_db.t(表)；
        // 另打开一个属于该连接的 DataEditor 标签（users 表已加载 id / name 两列）。
        let connection_id = ConnectionId(1);
        let object = |name: &str, database: &str, kind: ObjectKind| ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some(database.to_string()),
                schema: None,
                name: name.to_string(),
                kind,
            },
            rows: None,
            comment: None,
            modified_at: None,
        };
        let connection = ConnectionState {
            config: query_scope_config(DatabaseKind::MySql, Some("main")),
            connected: true,
            expanded: true,
            objects: vec![
                object("orders", "main", ObjectKind::Table),
                object("users", "main", ObjectKind::Table),
                object("v_orders", "main", ObjectKind::View),
                object("t", "other_db", ObjectKind::Table),
            ],
            redis_overview: RedisConnectionOverview::default(),
        };
        let page = DataPage {
            columns: vec![
                fluxdb_core::Column {
                    name: "id".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                fluxdb_core::Column {
                    name: "name".to_string(),
                    type_name: None,
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: Vec::new(),
            offset: 0,
            limit: 100,
            has_more: false,
        };
        let state = AppState {
            connections: vec![connection],
            tabs: vec![TabState {
                id: TabId(1),
                title: "users".to_string(),
                kind: TabKind::DataEditor(DataEditorState {
                    object: ObjectPath {
                        connection_id,
                        database: Some("main".to_string()),
                        schema: None,
                        name: "users".to_string(),
                        kind: ObjectKind::Table,
                    },
                    page: Some(page),
                    original_page: None,
                    pagination: Default::default(),
                    changes: None,
                    editing_cell: None,
                    cell_detail_panel: CellDetailPanelState::default(),
                    table_info: TableInfoState::default(),
                    loading: false,
                    error: None,
                }),
                dirty: false,
            }],
            ..AppState::default()
        };

        let ctx = loaded_schema_context(&state, connection_id, Some("main"));
        // 表 / 视图候选来自已加载 objects，且过滤掉其他库的 t。
        assert_eq!(ctx.tables, vec!["orders", "users", "v_orders"]);
        // 列候选来自已打开的 DataEditor 标签（users 表两列）。
        assert_eq!(
            ctx.columns,
            vec![
                ("users".to_string(), "id".to_string()),
                ("users".to_string(), "name".to_string()),
            ]
        );

        // 库不匹配时返回空。
        let empty = loaded_schema_context(&state, connection_id, Some("nope"));
        assert!(empty.tables.is_empty());
        assert!(empty.columns.is_empty());
    }

}
