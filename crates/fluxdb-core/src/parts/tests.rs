#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_clamps_limit() {
        assert_eq!(Pagination::new(0, 0).limit, 1);
        assert_eq!(
            Pagination::new(10, Pagination::MAX_LIMIT + 1).limit,
            Pagination::MAX_LIMIT
        );
        assert_eq!(
            Pagination::default(),
            Pagination {
                offset: 0,
                limit: 100
            }
        );
    }

    #[test]
    fn connection_draft_becomes_config() {
        let draft = ConnectionDraft {
            name: "local".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: PathBuf::from("test.db"),
                read_only: true,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        };

        let config = draft.into_config(ConnectionId(7));

        assert_eq!(config.id, ConnectionId(7));
        assert_eq!(config.name, "local");
        assert_eq!(config.kind, DatabaseKind::Sqlite);
    }

    #[test]
    fn sidebar_layout_repairs_deleted_duplicate_and_new_connections() {
        let connections = vec![
            sample_connection(1, "one"),
            sample_connection(2, "two"),
            sample_connection(3, "three"),
        ];
        let mut layout = SidebarLayout {
            groups: vec![
                ConnectionGroup {
                    id: ConnectionGroupId(1),
                    name: "开发".to_string(),
                    collapsed: true,
                },
                ConnectionGroup {
                    id: ConnectionGroupId(99),
                    name: "孤儿".to_string(),
                    collapsed: false,
                },
            ],
            order: vec![
                SidebarOrderEntry::Connection {
                    id: ConnectionId(1),
                },
                SidebarOrderEntry::Group {
                    id: ConnectionGroupId(1),
                    connection_ids: vec![ConnectionId(2), ConnectionId(1), ConnectionId(404)],
                },
            ],
            ..Default::default()
        };

        layout.repair(&connections);

        assert_eq!(
            layout.order,
            vec![
                SidebarOrderEntry::Connection {
                    id: ConnectionId(1)
                },
                SidebarOrderEntry::Group {
                    id: ConnectionGroupId(1),
                    connection_ids: vec![ConnectionId(2)]
                },
                SidebarOrderEntry::Connection {
                    id: ConnectionId(3)
                },
            ]
        );
        assert_eq!(layout.groups.len(), 1);
    }

    #[test]
    fn sidebar_layout_moves_connections_between_group_and_top_level_with_order() {
        let mut layout = SidebarLayout {
            groups: vec![ConnectionGroup {
                id: ConnectionGroupId(1),
                name: "开发".to_string(),
                collapsed: false,
            }],
            order: vec![
                SidebarOrderEntry::Connection {
                    id: ConnectionId(1),
                },
                SidebarOrderEntry::Group {
                    id: ConnectionGroupId(1),
                    connection_ids: vec![ConnectionId(2), ConnectionId(3)],
                },
            ],
            ..Default::default()
        };

        layout.move_connection_to_group_after(
            ConnectionId(1),
            ConnectionGroupId(1),
            Some(ConnectionId(2)),
        );

        assert_eq!(
            layout.order,
            vec![SidebarOrderEntry::Group {
                id: ConnectionGroupId(1),
                connection_ids: vec![ConnectionId(2), ConnectionId(1), ConnectionId(3)]
            }]
        );

        layout.move_connection_to_top_level_after(ConnectionId(3), Some(ConnectionId(1)));

        assert_eq!(
            layout.order,
            vec![
                SidebarOrderEntry::Group {
                    id: ConnectionGroupId(1),
                    connection_ids: vec![ConnectionId(2), ConnectionId(1)]
                },
                SidebarOrderEntry::Connection {
                    id: ConnectionId(3)
                },
            ]
        );
    }

    #[test]
    fn data_change_set_counts_insert_rows_and_updated_cells() {
        let changes = DataChangeSet {
            object: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                name: "users".to_string(),
                kind: ObjectKind::Table,
            },
            inserts: vec![Row {
                values: vec![
                    CellValue::I64(1),
                    CellValue::Text("Ada".to_string()),
                    CellValue::Null,
                ],
            }],
            updates: vec![RowUpdate {
                identity: RowIdentity {
                    values: BTreeMap::from([("id".to_string(), CellValue::I64(1))]),
                },
                cells: vec![CellUpdate {
                    column: "name".to_string(),
                    value: CellValue::Text("Grace".to_string()),
                }],
            }],
            deletes: Vec::new(),
        };

        assert!(!changes.is_empty());
        assert_eq!(changes.dirty_cell_count(), 2);
    }

    #[test]
    fn binary_type_names_are_detected() {
        for type_name in [
            "BINARY(16)",
            "VARBINARY(64)",
            "TINYBLOB",
            "BLOB",
            "MEDIUMBLOB",
            "LONGBLOB",
            "BYTEA",
            "IMAGE",
            "RAW(16)",
            "LONG RAW",
        ] {
            assert!(is_binary_type_name(type_name), "{type_name}");
        }

        for type_name in ["VARCHAR(255)", "TEXT", "INTEGER", "JSONB", "TIMESTAMP"] {
            assert!(!is_binary_type_name(type_name), "{type_name}");
        }
    }

    #[test]
    fn binary_type_max_bytes_is_detected() {
        assert_eq!(binary_type_max_bytes("BINARY(16)"), Some(16));
        assert_eq!(binary_type_max_bytes("VARBINARY(64)"), Some(64));
        assert_eq!(binary_type_max_bytes("TINYBLOB"), Some(255));
        assert_eq!(binary_type_max_bytes("BLOB"), Some(65_535));
        assert_eq!(binary_type_max_bytes("MEDIUMBLOB"), Some(16_777_215));
        assert_eq!(binary_type_max_bytes("LONGBLOB"), Some(4_294_967_295));
        assert_eq!(binary_type_max_bytes("BYTEA"), None);
        assert_eq!(binary_type_max_bytes("VARCHAR(255)"), None);
    }

    #[test]
    fn binary_summary_label_never_contains_raw_bytes() {
        let summary = BinaryCellSummary {
            type_name: "BLOB".to_string(),
            is_null: false,
            byte_length: 2_457_600,
            preview_hex: Some("89504E470D0A1A0A".to_string()),
        };

        assert_eq!(
            CellValue::BinarySummary(summary).display_label(),
            "BLOB [2.3 MB]"
        );
    }

    #[test]
    fn format_byte_length_scales_units_and_rounds() {
        // 五档单位：B / KB / MB / GB / TB（1024 进制，一位小数）
        assert_eq!(format_byte_length(0), "0 B");
        assert_eq!(format_byte_length(512), "512 B");
        assert_eq!(format_byte_length(1023), "1023 B");
        assert_eq!(format_byte_length(1024), "1.0 KB");
        // 1.5 MB 由 1024 * 1.5 得 1536 KB 换算而来
        assert_eq!(format_byte_length(1_572_864), "1.5 MB");
        assert_eq!(format_byte_length(1_073_741_824), "1.0 GB");
        // 2.5 TB = 1024^4 * 2.5
        assert_eq!(
            format_byte_length(2_748_779_069_440),
            "2.5 TB"
        );
    }

    #[test]
    fn user_facing_error_keeps_message_and_retry_hint() {
        let error = Error::new(ErrorKind::Connection, "127.0.0.1:3306 refused");
        let user_error = UserFacingError::from(error);

        assert_eq!(user_error.title, "连接失败");
        assert_eq!(user_error.message, "127.0.0.1:3306 refused");
        assert!(user_error.retryable);
    }

    #[test]
    fn mysql_user_admin_can_fall_back_to_current_user() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        assert!(provider.current_user_sql().contains("CURRENT_USER()"));

        let page = DataPage {
            columns: vec![
                Column {
                    name: "user".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                Column {
                    name: "host".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                Column {
                    name: "plugin".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![Row {
                values: vec![
                    CellValue::Text("app".to_string()),
                    CellValue::Text("%".to_string()),
                    CellValue::Text("CURRENT_USER()".to_string()),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
        };

        assert_eq!(
            provider.parse_users(&page),
            vec![DatabaseUserIdentity {
                user: "app".to_string(),
                host: "%".to_string(),
                plugin: Some("CURRENT_USER()".to_string()),
            }]
        );
    }

    #[test]
    fn mysql_create_user_sql_escapes_account_and_password_values() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let sql = provider.create_user_sql(&CreatePrincipalInput {
            user: "app'user".to_string(),
            host: "host'name".to_string(),
            password: "p\\'ass".to_string(),
            auth_plugin: Some("mysql_native_password".to_string()),
        });

        assert_eq!(
            sql,
            "CREATE USER 'app''user'@'host''name' IDENTIFIED WITH `mysql_native_password` BY 'p\\\\''ass';"
        );
    }

    #[test]
    fn mysql_alter_user_options_sql_are_escaped() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let user = DatabaseUserIdentity {
            user: "app'user".to_string(),
            host: "%".to_string(),
            plugin: None,
        };

        assert_eq!(
            provider.alter_auth_plugin_sql(&user, "mysql_native_password", Some("p'ass")),
            "ALTER USER 'app''user'@'%' IDENTIFIED WITH `mysql_native_password` BY 'p''ass';"
        );
        assert_eq!(
            provider.alter_password_expiry_sql(&user, "NEVER").unwrap(),
            "ALTER USER 'app''user'@'%' PASSWORD EXPIRE NEVER;"
        );
        assert_eq!(
            provider
                .alter_resource_limits_sql(
                    &user,
                    &UserResourceLimits {
                        max_queries_per_hour: Some(10),
                        max_updates_per_hour: None,
                        max_connections_per_hour: Some(30),
                        max_user_connections: Some(4),
                    },
                )
                .unwrap(),
            "ALTER USER 'app''user'@'%' WITH MAX_QUERIES_PER_HOUR 10 MAX_CONNECTIONS_PER_HOUR 30 MAX_USER_CONNECTIONS 4;"
        );
        assert_eq!(
            provider
                .alter_ssl_requirement_sql(&user, "SPECIFIED", "TLS", "CA'1", "")
                .unwrap(),
            "ALTER USER 'app''user'@'%' REQUIRE CIPHER 'TLS' AND ISSUER 'CA''1';"
        );
    }

    #[test]
    fn mysql_role_membership_sql_uses_role_to_user_direction() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let role = DatabaseUserIdentity {
            user: "test".to_string(),
            host: "%".to_string(),
            plugin: None,
        };
        let user = DatabaseUserIdentity {
            user: "root".to_string(),
            host: "localhost".to_string(),
            plugin: None,
        };

        assert_eq!(
            provider.grant_role_sql(&role, &user),
            "GRANT 'test'@'%' TO 'root'@'localhost';"
        );
        assert_eq!(
            provider.revoke_role_sql(&role, &user),
            "REVOKE 'test'@'%' FROM 'root'@'localhost';"
        );
        assert_eq!(
            provider.set_default_roles_sql(&user, std::slice::from_ref(&role)),
            "SET DEFAULT ROLE 'test'@'%' TO 'root'@'localhost';"
        );
    }

    #[test]
    fn mysql_role_memberships_parse_granted_and_default_roles() {
        let user = DatabaseUserIdentity {
            user: "root".to_string(),
            host: "localhost".to_string(),
            plugin: Some("caching_sha2_password".to_string()),
        };
        let test_role = DatabaseUserIdentity {
            user: "test".to_string(),
            host: "%".to_string(),
            plugin: Some("caching_sha2_password".to_string()),
        };
        let read_role = DatabaseUserIdentity {
            user: "read_role".to_string(),
            host: "%".to_string(),
            plugin: Some("mysql_native_password".to_string()),
        };
        let grants = vec![
            "GRANT USAGE ON *.* TO `root`@`localhost`".to_string(),
            "GRANT `test`@`%`, `read_role`@`%` TO `root`@`localhost`".to_string(),
            "SET DEFAULT ROLE `test`@`%` TO `root`@`localhost`".to_string(),
        ];

        assert_eq!(
            role_memberships_from_grants(&[test_role.clone(), read_role.clone()], &grants, &user),
            vec![
                UserRoleMembership {
                    role: test_role,
                    granted: true,
                    default_role: true,
                },
                UserRoleMembership {
                    role: read_role,
                    granted: true,
                    default_role: false,
                },
            ]
        );
    }

    #[test]
    fn mysql_privilege_grants_parse_database_privileges() {
        let provider = database_user_admin_provider(DatabaseKind::MySql).unwrap();
        let grants = vec![
            "GRANT USAGE ON *.* TO 'test'@'%'".to_string(),
            "GRANT SELECT, INSERT ON `llm`.* TO 'test'@'%'".to_string(),
            "GRANT UPDATE ON `llm`.`jobs` TO 'test'@'%'".to_string(),
            "GRANT DELETE ON `app`.* TO 'test'@'%' WITH GRANT OPTION".to_string(),
        ];

        assert_eq!(
            provider.privilege_grants_from_grants(&grants),
            vec![
                DatabasePrivilegeGrant {
                    database: "app".to_string(),
                    privileges: vec!["DELETE".to_string()],
                    grant_option: true,
                },
                DatabasePrivilegeGrant {
                    database: "llm".to_string(),
                    privileges: vec!["INSERT".to_string(), "SELECT".to_string()],
                    grant_option: false,
                },
            ]
        );
    }

    fn sample_connection(id: u64, name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(id),
            name: name.to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: PathBuf::from(format!("{name}.db")),
                read_only: false,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
        }
    }

    // ---- terminal 网格 / 解析器 ----

    #[test]
    fn grid_prints_chars_and_wraps() {
        let mut g = terminal::TermGrid::new(4, 2);
        g.print_char('a');
        g.print_char('b');
        assert_eq!(g.content_text(), "ab");
        // 走到行尾后再打印：折行到第二行。
        g.print_char('c');
        g.print_char('d');
        g.print_char('e');
        assert_eq!(g.cursor(), terminal::TermPoint { x: 1, y: 1 });
        assert_eq!(g.content_text(), "abcd\ne");
    }

    #[test]
    fn grid_cursor_and_erase_primitives() {
        let mut g = terminal::TermGrid::new(8, 2);
        g.print_char('a'); // (0,0)
        g.print_char('b'); // (1,0)
        g.carriage_return();
        g.print_char('X'); // 覆盖 a
        assert_eq!(g.content_text(), "Xb");
        // 从光标列擦到行尾。
        g.set_cursor(1, 0);
        g.erase_to_eol();
        let r0 = g.row_view(0);
        assert_eq!(r0.chars, vec!['X', ' ', ' ', ' ', ' ', ' ', ' ', ' ']);
        // 退格：光标回退一列。
        g.set_cursor(2, 1);
        g.backspace();
        assert_eq!(g.cursor(), terminal::TermPoint { x: 1, y: 1 });
    }

    #[test]
    fn parser_clears_ansi_and_styles() {
        // "\x1b[31mred\x1b[0m" → 前 3 个字符红色，后 reset 无色。
        let mut g = terminal::TermGrid::new(20, 2);
        terminal::feed_bytes(&mut g, b"\x1b[31mred\x1b[0mplain");
        assert_eq!(g.content_text(), "redplain");
        let row = g.row_view(0);
        assert_eq!(row.fg[0], Some((205, 0, 0)));
        assert_eq!(row.fg[2], Some((205, 0, 0)));
        assert_eq!(row.fg[3], None); // reset 后无色
    }

    #[test]
    fn parser_handles_newlines_and_cr() {
        // CRLF 回行首；单独的 LF 只下移一格保持列（真实终端语义）。
        // "ef" 应落在 row2 的列 2-3（LF 后 x 仍是 2），而不是列 0-1。
        let mut g = terminal::TermGrid::new(20, 3);
        terminal::feed_bytes(&mut g, b"ab\r\ncd\nef");
        assert_eq!(g.cursor(), terminal::TermPoint { x: 4, y: 2 });
        assert_eq!(&g.row_view(2).chars[2..4], &['e', 'f'][..]);
        // 行首前两个 cell 是空格（未写入），不被 LF 改变。
        assert_eq!(&g.row_view(2).chars[0..2], &[' ', ' '][..]);

        // 穿插 CRLF：回行首后写入新行。
        let mut g2 = terminal::TermGrid::new(20, 2);
        terminal::feed_bytes(&mut g2, b"abc\r\ndef");
        assert_eq!(g2.cursor(), terminal::TermPoint { x: 3, y: 1 });
        assert_eq!(&g2.row_view(1).chars[0..3], &['d', 'e', 'f'][..]);
    }

    #[test]
    fn selection_region_and_text() {
        use terminal::TermPoint;
        let mut g = terminal::TermGrid::new(20, 2);
        terminal::feed_bytes(&mut g, b"hello world\r\nsecond line");
        // 无选区时 selected_text 为空。
        assert!(!g.has_selection());
        assert_eq!(g.selected_text(), "");

        // 选中 row0 的列 0..=5（"hello "），跨行应带上行间 '\n'。
        g.begin_selection(TermPoint { x: 0, y: 0 });
        g.update_selection(TermPoint { x: 5, y: 1 });
        assert!(g.has_selection());
        assert!(g.is_cell_selected(0, 0));
        assert!(g.is_cell_selected(1, 1));
        assert!(!g.is_cell_selected(10, 1));
        // copy_region 语义：首行从 anchor 列到行尾，末行到 end 列。
        assert_eq!(g.selected_text(), "hello world\nsecond");

        // 反向拖动（anchor 在右下）同样被规范化。
        g.begin_selection(TermPoint { x: 5, y: 1 });
        g.update_selection(TermPoint { x: 0, y: 0 });
        assert_eq!(g.selected_text(), "hello world\nsecond");

        // 清除后不再高亮 / 复制。
        g.clear_selection();
        assert!(!g.has_selection());
        assert_eq!(g.selected_text(), "");
    }

    // ---- transcript / mock transport / session key ----

    #[test]
    fn transcript_classify_lines() {
        use terminal::{classify_transcript_line, TerminalTranscriptKind as K};
        let prompt = "127.0.0.1:6379[0]> ";
        // 空行 → Blank。
        assert_eq!(classify_transcript_line("", &prompt), K::Blank);
        assert_eq!(classify_transcript_line("   ", &prompt), K::Blank);
        // 纯提示符行 → Prompt。
        assert_eq!(classify_transcript_line("127.0.0.1:6379[0]> ", &prompt), K::Prompt);
        // 普通输出 → Output。
        assert_eq!(classify_transcript_line("OK", &prompt), K::Output);
        // Redis 错误前缀 → Error。
        assert_eq!(classify_transcript_line("(error) WRONGTYPE wrong kind", &prompt), K::Error);
        assert_eq!(classify_transcript_line("ERR wrong number of arguments", &prompt), K::Error);
        // 空 prompt（未提供）时不误判普通行为 Prompt。
        assert_eq!(classify_transcript_line("x", ""), K::Output);
    }

    #[test]
    fn mock_transport_records_bytes_and_resize() {
        use terminal::{MockTransport, TerminalTransport};
        let mut t = MockTransport::new();
        assert!(!t.closed);
        t.write(b"GET foo\r\n");
        t.write("中文".as_bytes());
        t.resize(120, 32);
        t.resize(80, 24);
        assert_eq!(t.written_text(), "GET foo\r\n中文");
        assert_eq!(t.resizes, vec![(120, 32), (80, 24)]);
        assert!(t.closed == false);
        t.close();
        assert!(t.closed);
    }

    #[test]
    fn session_key_redis_dedup_eq() {
        use terminal::TerminalSessionKey;
        // 同一 connection_id + database 相等 → 可驱动复用。
        assert_eq!(
            TerminalSessionKey::Redis { connection_id: 7, database: 3 },
            TerminalSessionKey::Redis { connection_id: 7, database: 3 }
        );
        // 不同 database 或 connection 不相等。
        assert_ne!(
            TerminalSessionKey::Redis { connection_id: 7, database: 3 },
            TerminalSessionKey::Redis { connection_id: 7, database: 4 }
        );
        assert_ne!(
            TerminalSessionKey::Redis { connection_id: 7, database: 3 },
            TerminalSessionKey::Redis { connection_id: 8, database: 3 }
        );
        // 可用于 HashSet 去重（Hash 派生）。
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TerminalSessionKey::Redis { connection_id: 1, database: 0 });
        set.insert(TerminalSessionKey::Redis { connection_id: 1, database: 0 });
        set.insert(TerminalSessionKey::Redis { connection_id: 1, database: 1 });
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn session_state_machine_basic() {
        use terminal::TerminalSessionState as S;
        // 典型流程：Connecting → PtyRunning → Ready（可与 Busy 往返）。
        let mut state = S::Connecting;
        assert_eq!(state, S::Connecting);
        state = S::PtyRunning;
        assert_eq!(state, S::PtyRunning);
        state = S::Ready;
        state = S::Busy;
        state = S::Ready;
        assert_eq!(state, S::Ready);
        // 退出 / 失败为终止态。
        assert_ne!(S::Exited, S::Failed);
    }

    #[test]
    fn char_cell_width_counts_wide_chars() {
        assert_eq!(terminal::char_cell_width('a'), 1);
        assert_eq!(terminal::char_cell_width(' '), 1);
        assert_eq!(terminal::char_cell_width('中'), 2);
        assert_eq!(terminal::char_cell_width('你'), 2);
        assert_eq!(terminal::char_cell_width('🙂'), 2);
    }

    #[test]
    fn row_trailing_space_visible_in_render_but_stripped_in_content() {
        // Bug 1：redis-cli 输入行末尾的空格是"输入内容"；渲染路径（visible_lines）应保留它，
        // 否则光标被 `input_len` 夹回提示符、显示"空格没出来"。content_text 仍应去掉尾部空白。
        // 注：visible_lines 返回完整 cols 宽的行（含尾部空白填充格），故用 starts_with 断言前缀。
        let mut g = terminal::TermGrid::new(120, 40);
        g.print_char('a');
        g.print_char(' '); // 行尾输入空格
        // 渲染路径：保留"a"后的输入空格（旧行为会被裁剪成只有"a"）。
        assert!(g.visible_lines()[0].starts_with("a "));
        // content_text：复制/输出语义保持去尾部空白（不回归）。
        assert_eq!(g.content_text(), "a");
        // 中间的空格不受影响。
        let mut g2 = terminal::TermGrid::new(120, 40);
        for c in ['a', ' ', 'b'] {
            g2.print_char(c);
        }
        assert!(g2.visible_lines()[0].starts_with("a b"));
        assert_eq!(g2.content_text(), "a b");
    }

    #[test]
    fn redis_space_echo_keeps_cursor_and_space() {
        // 用 redis-cli 实测回显字节验证：输入一个行尾空格后，网格光标正确处于提示符 +1，
        // 且当前输入行（渲染用）保留该空格。
        use terminal::{TermGrid, feed_bytes};
        let mut g = TermGrid::new(120, 40);
        feed_bytes(&mut g, b"127.0.0.1:6379> ");
        feed_bytes(&mut g, b"\r\x1b[0K127.0.0.1:6379>  \r\x1b[17C");
        // "127.0.0.1:6379>" 16 字符 + 输入空格 1 格 = 17。
        assert_eq!(g.cursor(), terminal::TermPoint { x: 17, y: 0 });
        // 渲染路径保留行尾输入空格（提示符 + 原生空格 + 输入空格）。
        assert!(g.visible_lines()[0].starts_with("127.0.0.1:6379>  "));
    }

    #[test]
    fn redis_byte_backspace_over_wide_char_yields_whole_char() {
        // Bug 2 机制验证：redis-cli 的 linenoise 按"字节"退格。桌面端对`中`(3字节)发 3 个 `\x7f`，
        // 等价于 redis 把 `a中` 的末尾 3 字节删掉 → 回显应变回 `a`，且不再残留半个字符的乱码。
        use terminal::{TermGrid, feed_bytes};
        let mut g = TermGrid::new(120, 40);
        // 输入 `a中`：prompt(16 格) + 'a'(1) + 中(2) = 19，redis 回显 `\x1b[19C`。
        feed_bytes(&mut g, b"\r\x1b[0K127.0.0.1:6379> a\xe4\xb8\xad\r\x1b[19C");
        assert_eq!(g.cursor(), terminal::TermPoint { x: 19, y: 0 });
        assert!(g.visible_lines()[0].starts_with("127.0.0.1:6379> a中"));
        // 连发 3 字节 DEL 后，redis 重绘：应只剩 `a`（16+1=17 格），无 `�`。
        feed_bytes(&mut g, b"\r\x1b[0K127.0.0.1:6379> a\r\x1b[17C");
        assert!(g.visible_lines()[0].starts_with("127.0.0.1:6379> a"));
        // 若只删 1 字节（旧行为），会残留半个 `中` 的乱码——此处断言不再出现。
        assert_eq!(g.content_text().find('\u{fffd}'), None);
    }

    #[test]
    fn byte_cursor_snaps_to_cell_boundary_over_wide_char() {
        // redis 光标按"字节"回显，grid 布局按"单元宽"（宽字符=2 格）。验证字节→单元换算：
        // 行 `127.0.0.1:6379> a中` 中，prompt 16 ASCII=16 字节、'a'=1 字节、`中`=3 字节共 20 字节；
        // 单元列 16+a(16)+中(17,18)=19 格。
        use terminal::{TermGrid, byte_cursor_to_cell, row_real_chars, feed_bytes};
        let mut g = TermGrid::new(120, 40);
        feed_bytes(&mut g, b"\r\x1b[0K127.0.0.1:6379> a\xe4\xb8\xad\r\x1b[19C");
        let cells: Vec<char> = g.visible_lines()[0].chars().collect();
        // 光标在 'a' 后（字节 17，即 `中` 起始）：吸附到 `中` 的起始单元列 17。
        assert_eq!(byte_cursor_to_cell(&cells, 17), 17);
        // 光标在末尾（字节 20）：吸附到内容末尾单元列 19。
        assert_eq!(byte_cursor_to_cell(&cells, 20), 19);
        // 光标落 `中` 中间（字节 18）：仍吸附到 `中` 起始列 17，避免"光标在字上"。
        assert_eq!(byte_cursor_to_cell(&cells, 18), 17);
        // 实字符表：`中` 字节长 3、单元宽 2，'a' 均 1。
        let info = row_real_chars(&cells);
        let zhong = info.iter().find(|r| r.c == '中').unwrap();
        assert_eq!(zhong.byte_len, 3);
        assert_eq!(zhong.cell_len, 2);
        let a = info.iter().find(|r| r.c == 'a').unwrap();
        assert_eq!(a.byte_len, 1);
        assert_eq!(a.cell_len, 1);
    }
}
