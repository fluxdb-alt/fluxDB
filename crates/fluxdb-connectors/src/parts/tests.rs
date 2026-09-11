#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::{CellUpdate, Endpoint, Error, ErrorKind, QueryMode, RowIdentity};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_connection_accepts_matching_kind() {
        let connector = MockConnector::sqlite();

        assert!(connector.test_connection(&sqlite_config()).is_ok());
    }

    #[test]
    fn completion_metadata_cancel_short_circuits_before_provider_call() {
        let connector = MockConnector::sqlite();
        let cancelled = || true;

        assert!(connector
            .list_completion_tables_with_cancel(None, None, "", 20, &cancelled)
            .unwrap()
            .is_empty());
        assert!(connector
            .list_completion_columns_with_cancel(None, None, "Product", &cancelled)
            .unwrap()
            .is_empty());
        assert!(connector
            .list_completion_routines_with_cancel(None, None, "", 20, &cancelled)
            .unwrap()
            .is_empty());
        assert!(connector
            .list_completion_triggers_with_cancel(None, None, "", 20, &cancelled)
            .unwrap()
            .is_empty());
        let path = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "Product".to_string(),
            kind: ObjectKind::Table,
        };
        assert!(connector
            .list_foreign_keys_with_cancel(&path, &cancelled)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn await_with_cancel_drops_pending_metadata_future() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancelled = || true;
        let started = std::time::Instant::now();
        let result = runtime.block_on(await_with_cancel(
            async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok::<_, ()>(())
            },
            &cancelled,
        ));

        assert_eq!(result, Ok(None));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));

        let immediate = runtime.block_on(await_with_cancel(async { Ok::<_, ()>(42) }, &cancelled));
        assert_eq!(immediate, Ok(None));
    }

    #[test]
    fn mysql_connection_url_includes_credentials_database_and_params() {
        let mut config = mysql_config();
        config
            .options
            .insert("username".to_string(), "root user".to_string());
        config
            .options
            .insert("password".to_string(), "p@ss word".to_string());
        config.options.insert(
            "url_params".to_string(),
            "ssl-mode=DISABLED&timezone=%2B08:00".to_string(),
        );

        let url = mysql_connection_url(&config).unwrap();

        assert_eq!(
            url,
            "mysql://root%20user:p%40ss%20word@127.0.0.1:3306/app%20db?ssl-mode=DISABLED&timezone=%2B08:00"
        );
    }

    #[test]
    fn mysql_connection_url_rejects_non_tcp_endpoint() {
        let mut config = mysql_config();
        config.endpoint = Endpoint::SqliteFile {
            path: "demo.db".into(),
            read_only: false,
        };

        let result = mysql_connection_url(&config);

        assert!(result.is_err());
    }

    #[test]
    fn mysql_protocol_kind_includes_tidb() {
        assert!(is_mysql_protocol_kind(DatabaseKind::TiDb));
    }

    #[test]
    fn redis_preview_truncates_long_values() {
        let preview = redis_preview("a".repeat(205));

        assert_eq!(preview.chars().count(), 203);
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn redis_hash_field_ttl_text_formats_special_values() {
        assert_eq!(redis_hash_field_ttl_text(RedisValue::Int(-1)).unwrap(), "无 TTL");
        assert_eq!(redis_hash_field_ttl_text(RedisValue::Int(-2)).unwrap(), "已过期");
        assert_eq!(redis_hash_field_ttl_text(RedisValue::Int(1500)).unwrap(), "1s");
    }

    #[test]
    fn redis_server_version_parse_handles_common_shapes() {
        let v = RedisServerVersion::parse("7.4.0").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (7, 4, 0));
        assert!(v.at_least(7, 4));
        assert!(v.at_least(7, 3));
        assert!(!v.at_least(8, 0));

        let v = RedisServerVersion::parse("6.2.7").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (6, 2, 7));
        assert!(!v.at_least(7, 4));

        // 缺省位按 0 补齐
        let v = RedisServerVersion::parse("7").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (7, 0, 0));
        assert!(!v.at_least(7, 4));

        let v = RedisServerVersion::parse("7.4").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (7, 4, 0));
        assert!(v.at_least(7, 4));

        // patch 前导数字容错 rc 后缀
        let v = RedisServerVersion::parse("7.4.0-rc1").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (7, 4, 0));
        assert!(v.at_least(7, 4));
    }

    #[test]
    fn redis_server_version_parse_rejects_illegal_input() {
        assert!(RedisServerVersion::parse("").is_none());
        assert!(RedisServerVersion::parse("abc").is_none());
        assert!(RedisServerVersion::parse("7.").is_none());
        assert!(RedisServerVersion::parse("..").is_none());
    }

    #[test]
    fn redis_parse_stream_entries_reads_id_and_field_pairs() {
        let value = RedisValue::Array(vec![RedisValue::Array(vec![
            RedisValue::Bulk(Some(b"1785677482094-0".to_vec())),
            RedisValue::Array(vec![
                RedisValue::Bulk(Some(b"test".to_vec())),
                RedisValue::Bulk(Some(b"3".to_vec())),
                RedisValue::Bulk(Some(b"test1".to_vec())),
                RedisValue::Bulk(Some(b"2".to_vec())),
            ]),
        ])]);

        let entries = redis_parse_stream_entries(value).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "1785677482094-0");
        assert_eq!(
            entries[0].fields,
            vec![
                ("test".to_string(), "3".to_string()),
                ("test1".to_string(), "2".to_string()),
            ]
        );
        assert!(!entries[0].time.is_empty());
    }

    #[test]
    fn redis_stream_entry_fields_pads_trailing_field_without_value() {
        let fields = redis_stream_entry_fields(RedisValue::Array(vec![RedisValue::Bulk(Some(b"only".to_vec()))]));

        assert_eq!(fields, vec![("only".to_string(), String::new())]);
    }

    /// 需要本地 127.0.0.1:6379 的真实 Redis，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_stream`
    #[test]
    #[ignore]
    fn redis_stream_entries_paginate_over_real_server() {
        let config = redis_config();
        let connector = RedisConnector::with_config(config);
        let key = format!(
            "gdb_stream_pagination_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: ConnectionId(3),
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        for index in 0..7 {
            connector
                .add_stream_entry(&object, "*", &[("seq".to_string(), index.to_string())], None)
                .unwrap();
        }

        let first = connector.load_stream_entries(&object, RedisStreamRange::default(), "", 3).unwrap();
        assert_eq!(first.total, 7);
        assert_eq!(first.entries.len(), 3);
        assert_ne!(first.next_cursor, "0");

        let second = connector
            .load_stream_entries(&object, RedisStreamRange::default(), &first.next_cursor, 3)
            .unwrap();
        assert_eq!(second.entries.len(), 3);
        // 分页不重不漏：第二页第一条正是第一页给出的游标。
        assert_eq!(second.entries[0].id, first.next_cursor);

        let third = connector
            .load_stream_entries(&object, RedisStreamRange::default(), &second.next_cursor, 3)
            .unwrap();
        assert_eq!(third.entries.len(), 1);
        assert_eq!(third.next_cursor, "0");

        let ids = first
            .entries
            .iter()
            .chain(second.entries.iter())
            .chain(third.entries.iter())
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 7);
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            7
        );
        for id in ids {
            connector.delete_stream_entry(&object, &id).unwrap();
        }
    }

    /// 需要本地 127.0.0.1:6379（Redis 7.4+，支持字段级 TTL），默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_hash_field_ttl_write`
    #[test]
    #[ignore]
    fn redis_hash_field_ttl_write_semantics_over_real_server() {
        let connector = RedisConnector::with_config(redis_config());
        let key = format!(
            "gdb_hash_ttl_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: ConnectionId(3),
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        let ttl_of = |field: &str| {
            connector
                .load_hash_fields(&object, field, "0", 100)
                .unwrap()
                .fields
                .into_iter()
                .find(|(name, _, _)| name == field)
                .map(|(_, _, ttl)| ttl)
        };

        connector
            .set_hash_field(&object, "f", "v1", RedisHashFieldTtl::Seconds(600))
            .unwrap();
        // HPTTL 返回毫秒，展示时向下取整到秒，599s/600s 都算正确
        let initial = ttl_of("f").unwrap();
        assert!(
            (500..=600).contains(&initial.trim_end_matches('s').parse::<u64>().unwrap()),
            "TTL 应为约 600s，实际为 {initial}"
        );

        // 只改值：TTL 必须原样保留（HSET 本身会清掉它）
        connector
            .set_hash_field(&object, "f", "v2", RedisHashFieldTtl::Keep)
            .unwrap();
        let kept = ttl_of("f").unwrap();
        assert!(
            kept.trim_end_matches('s').parse::<u64>().unwrap() > 500,
            "TTL 应保留，实际为 {kept}"
        );

        connector
            .set_hash_field(&object, "f", "v3", RedisHashFieldTtl::Persist)
            .unwrap();
        assert_eq!(ttl_of("f").as_deref(), Some("无 TTL"));

        connector.delete_hash_field(&object, "f").unwrap();
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_hash_full_value_roundtrip`
    #[test]
    #[ignore]
    fn redis_hash_full_value_roundtrip_over_real_server() {
        let connector = RedisConnector::with_config(redis_config());
        let key = format!(
            "gdb_hash_full_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: ConnectionId(3),
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        // 先 raw 写入 >1MB 值，再验证：
        // 1) 普通读被截断成 marker；2) 完整读返回原始值；3) 截断 marker 回写普通 set 被拒、raw 放行
        let big = "x".repeat(REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES + 1);
        connector
            .set_hash_field_raw(&object, "f", &big, RedisHashFieldTtl::Keep)
            .unwrap();
        let page = connector.load_hash_fields(&object, "", "0", 100).unwrap();
        let (_, shown, _) = page
            .fields
            .iter()
            .find(|(field, _, _)| field == "f")
            .unwrap();
        assert!(redis_is_hash_truncated(shown), "普通读应截断，实际 {shown:?}");
        assert_eq!(
            connector.load_hash_field_full(&object, "f").unwrap().as_deref(),
            Some(big.as_str()),
            "完整读应返回未截断原始值"
        );
        // 截断串（marker）回写：普通 set 拒绝，raw 放行（弹框保存语义）
        assert!(
            connector
                .set_hash_field(&object, "f", shown, RedisHashFieldTtl::Keep)
                .is_err(),
            "普通 set 应拒绝截断 marker 回写"
        );
        assert!(
            connector
                .set_hash_field_raw(&object, "f", shown, RedisHashFieldTtl::Keep)
                .is_ok(),
            "raw set 应放行 marker 回写"
        );
        connector.delete_hash_field(&object, "f").unwrap();
    }

    #[test]
    #[ignore]
    fn redis_hash_key_ttl_update_over_real_server() {
        let config = redis_config();
        let connector = RedisConnector::with_config(config.clone());
        let key = format!(
            "gdb_hash_key_ttl_update_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: ConnectionId(3),
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };

        connector
            .apply_changes(&DataChangeSet {
                object: object.clone(),
                inserts: vec![Row {
                    values: vec![
                        CellValue::Text(key.clone()),
                        CellValue::Text("hash".to_string()),
                        CellValue::Text("f=v".to_string()),
                        CellValue::Null,
                        CellValue::Null,
                    ],
                }],
                updates: Vec::new(),
                deletes: Vec::new(),
            })
            .unwrap();

        connector
            .apply_changes(&DataChangeSet {
                object: object.clone(),
                inserts: Vec::new(),
                updates: vec![RowUpdate {
                    identity: RowIdentity {
                        values: [("键".to_string(), CellValue::Text(key.clone()))].into(),
                    },
                    cells: vec![CellUpdate {
                        column: "TTL".to_string(),
                        value: CellValue::Text("600".to_string()),
                    }],
                }],
                deletes: Vec::new(),
            })
            .unwrap();

        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        assert!(matches!(
            connection.command(&["TTL", &key]).unwrap(),
            RedisValue::Int(ttl) if ttl > 0
        ));
        connection.command(&["DEL", &key]).unwrap();
    }

    /// 编辑 string 值后必须保留既有 TTL（对齐 RedisInsight 语义）：写值命令会清掉过期时间，
    /// redis_set_key_value 应在写入前记录 TTL、写值后补回。
    #[test]
    #[ignore]
    fn redis_string_value_edit_preserves_existing_ttl_over_real_server() {
        let config = redis_config();
        let connector = RedisConnector::with_config(config.clone());
        let key = format!(
            "gdb_string_ttl_preserve_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: ConnectionId(3),
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };

        // 初始化 string key 并设过期 600 秒。
        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        connection.command(&["SET", &key, "before"]).unwrap();
        connection.command(&["EXPIRE", &key, "600"]).unwrap();

        // 仅编辑值（不带 TTL cell）：TTL 必须原样保留。
        connector
            .apply_changes(&DataChangeSet {
                object: object.clone(),
                inserts: Vec::new(),
                updates: vec![RowUpdate {
                    identity: RowIdentity {
                        values: [("键".to_string(), CellValue::Text(key.clone()))].into(),
                    },
                    cells: vec![CellUpdate {
                        column: "值".to_string(),
                        value: CellValue::Text("after".to_string()),
                    }],
                }],
                deletes: Vec::new(),
            })
            .unwrap();

        // 值已更新，且仍有过期时间。
        assert_eq!(
            connection.command(&["GET", &key]).unwrap(),
            RedisValue::Bulk(Some(b"after".to_vec()))
        );
        assert!(matches!(
            connection.command(&["TTL", &key]).unwrap(),
            RedisValue::Int(ttl) if ttl > 0
        ));
        connection.command(&["DEL", &key]).unwrap();
    }

    #[test]
    fn redis_hash_field_query_mode_matches_exact_and_glob_behavior() {
        assert!(matches!(
            redis_hash_field_query_mode(""),
            RedisHashFieldQueryMode::All
        ));
        assert!(matches!(
            redis_hash_field_query_mode("field"),
            RedisHashFieldQueryMode::Exact(value) if value == "field"
        ));
        assert!(matches!(
            redis_hash_field_query_mode("*field*"),
            RedisHashFieldQueryMode::Pattern(value) if value == "*field*"
        ));
    }

    #[test]
    fn redis_text_from_bytes_marks_non_utf8_instead_of_lossy_decode() {
        assert_eq!(redis_text_from_bytes(b"plain".to_vec()), "plain");

        // 0x80 起头的字节序列不是合法 UTF-8：不能 lossy 转换，必须给占位符
        let binary = redis_text_from_bytes(vec![0x80, 0x81, 0xfe]);
        assert!(redis_is_binary_placeholder(&binary));
        assert!(binary.contains("3 字节"));
        assert!(!binary.contains('\u{FFFD}'), "不允许出现 lossy 替换字符");
        assert!(redis_reject_binary_placeholder(&binary).is_err());
        assert!(redis_reject_binary_placeholder("plain").is_ok());
    }

    #[test]
    fn redis_utf8_bounded_text_recovers_cut_multibyte_char() {
        // ASCII 原样返回
        assert_eq!(redis_utf8_bounded_text(b"hello".to_vec()), "hello");

        // "中" 是 3 字节 UTF-8（E4 B8 AD），GETRANGE 若从中间切开，应还原前缀、不留 lossy 替换字符
        let cut = redis_utf8_bounded_text(b"a\xE4\xB8".to_vec());
        assert_eq!(cut, "a");
        assert!(!cut.contains('\u{FFFD}'));

        // 回退 3 字节仍无法解码 → 按二进制占位符处理
        let binary = redis_utf8_bounded_text(vec![0xff, 0xfe, 0x00]);
        assert!(redis_is_binary_placeholder(&binary));
    }

    #[test]
    fn redis_value_illegal_control_char_rejects_garbage_but_keeps_text() {
        // 普通可打印文本（含中文/emoji）不拦截。
        assert_eq!(redis_value_illegal_control_char("hello 你好 😀"), None);
        // 文本中合法的制表符 / 换行 / 回车放行。
        assert_eq!(redis_value_illegal_control_char("a\tb\nc\rd"), None);
        // 空字符串合法。
        assert_eq!(redis_value_illegal_control_char(""), None);

        // NUL / ESC 等 C0 控制字符（除 \t\n\r 外）一律拦截。
        assert_eq!(redis_value_illegal_control_char("a\u{0000}b"), Some('\u{0000}'));
        assert_eq!(redis_value_illegal_control_char("a\u{001b}b"), Some('\u{001b}'));
        // DEL 也拦截。
        assert_eq!(redis_value_illegal_control_char("\u{007f}"), Some('\u{007f}'));
        // C1 控制字符（0x80-0x9F 区）拦截。
        assert_eq!(redis_value_illegal_control_char("\u{009b}"), Some('\u{009b}'));
    }

    #[test]
    fn redis_hash_truncate_keeps_value_within_limit() {
        // 1MB 整是边界：<= 阈值不截断（对齐 RI 的「大于 1MB 才截断」语义）
        let value = "a".repeat(REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES);
        assert_eq!(redis_hash_truncate(value.clone()), value);
    }

    #[test]
    fn redis_hash_truncate_over_limit_prefixes_marker() {
        let value = "b".repeat(REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES + 1);
        let truncated = redis_hash_truncate(value.clone());
        assert!(redis_is_hash_truncated(&truncated));
        let head = value.chars().take(REDIS_HASH_TRUNCATED_CHARS).collect::<String>();
        assert_eq!(
            truncated,
            format!("{REDIS_HASH_TRUNCATED_MARKER} {head}...")
        );
    }

    #[test]
    fn redis_hash_truncate_keeps_utf8_char_boundary() {
        // 多字节字符（中 = 3 字节）超限：前导窗口必须恰 30 个字符，且不切坏 UTF-8
        let value = "中".repeat(REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES);
        let truncated = redis_hash_truncate(value);
        assert!(redis_is_hash_truncated(&truncated));
        assert!(!truncated.contains('\u{FFFD}'), "不允许出现 lossy 替换字符");
        let window = truncated
            .strip_prefix(REDIS_HASH_TRUNCATED_MARKER)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|rest| rest.strip_suffix("..."))
            .unwrap();
        assert_eq!(window.chars().count(), REDIS_HASH_TRUNCATED_CHARS);
    }

    #[test]
    fn redis_reject_truncated_value_is_independent_from_binary_guard() {
        // 截断标记 → Err
        let truncated = redis_hash_truncate("x".repeat(REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES + 1));
        assert!(redis_reject_truncated_value(&truncated).is_err());

        // 普通文本 / 二进制占位符 → Ok（两 guard 相互独立）
        assert!(redis_reject_truncated_value("plain").is_ok());
        let binary = redis_text_from_bytes(vec![0x80, 0x81, 0xfe]);
        assert!(redis_is_binary_placeholder(&binary));
        assert!(redis_reject_truncated_value(&binary).is_ok());
        assert!(redis_reject_binary_placeholder(&truncated).is_ok(), "截断串不应被二进制 guard 拦截");
    }

    #[test]
    fn redis_hash_truncate_is_idempotent_on_marker() {
        let original = redis_hash_truncate("c".repeat(REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES + 1));
        // 截断串自身 < 1MB，回灌原样返回，不会二次加工
        assert_eq!(redis_hash_truncate(original.clone()), original);
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_binary`
    #[test]
    #[ignore]
    fn redis_binary_value_is_not_corrupted_by_round_trip() {
        let connector = RedisConnector::with_config(redis_config());
        let key = format!(
            "gdb_binary_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: ConnectionId(3),
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        // 直接用协议写入一段非 UTF-8 的 hash 字段值
        {
            let mut connection = redis_connect(&redis_config()).unwrap();
            redis_select(&mut connection, 0).unwrap();
            let raw = unsafe { String::from_utf8_unchecked(vec![0xff, 0xfe, 0x00, 0x41]) };
            connection
                .command(&["HSET", key.as_str(), "bin", raw.as_str()])
                .unwrap();
        }

        let page = connector.load_hash_fields(&object, "", "0", 100).unwrap();
        let (_, value, _) = page
            .fields
            .iter()
            .find(|(field, _, _)| field == "bin")
            .unwrap();
        assert!(redis_is_binary_placeholder(value), "实际读到 {value:?}");

        // 把占位符原样回写必须被拒绝，否则原始字节就被毁了
        let error = connector
            .set_hash_field(&object, "bin", value, RedisHashFieldTtl::Keep)
            .unwrap_err();
        assert!(error.to_string().contains("二进制"), "实际错误：{error}");

        connector.delete_hash_field(&object, "bin").unwrap();
    }

    #[test]
    fn redis_scan_filter_maps_key_and_type_filters() {
        let filters = vec![
            FilterSpec {
                field: "键".to_string(),
                op: FilterOp::Contains,
                values: vec![CellValue::Text("user*1".to_string())],
                enabled: true,
            },
            FilterSpec {
                field: "类型".to_string(),
                op: FilterOp::Eq,
                values: vec![CellValue::Text("HASH".to_string())],
                enabled: true,
            },
            // 关闭的过滤条件不下推
            FilterSpec {
                field: "键".to_string(),
                op: FilterOp::Eq,
                values: vec![CellValue::Text("ignored".to_string())],
                enabled: false,
            },
        ];

        let filter = redis_scan_filter_from(&filters);

        // 通配符要转义，否则用户输入的 * 会被当成 glob
        assert_eq!(filter.pattern.as_deref(), Some("*user\\*1*"));
        assert_eq!(filter.type_name.as_deref(), Some("hash"));
    }

    #[test]
    fn redis_scan_filter_ignores_unsupported_columns() {
        let filters = vec![FilterSpec {
            field: "值".to_string(),
            op: FilterOp::Contains,
            values: vec![CellValue::Text("x".to_string())],
            enabled: true,
        }];

        assert_eq!(redis_scan_filter_from(&filters), RedisScanFilter::default());
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_scan_page`
    #[test]
    #[ignore]
    fn redis_scan_page_paginates_and_filters_on_server() {
        let config = redis_config();
        let prefix = format!(
            "gdb_scan_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        for index in 0..25 {
            connection
                .command(&["SET", &format!("{prefix}:s{index}"), "v"])
                .unwrap();
        }
        connection
            .command(&["HSET", &format!("{prefix}:h0"), "f", "v"])
            .unwrap();

        let filter = RedisScanFilter {
            pattern: Some(format!("{}*", redis_glob_escape(&prefix))),
            type_name: None,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut cursor = "0".to_string();
        loop {
            let (keys, next) = redis_scan_keys_page(&mut connection, &filter, &cursor, 10).unwrap();
            assert!(keys.len() >= 10 || next == "0", "未到底就该攒够 need 个");
            for key in keys {
                assert!(seen.insert(key), "同一个 key 被返回了两次");
            }
            cursor = next;
            if cursor == "0" {
                break;
            }
        }
        assert_eq!(seen.len(), 26, "MATCH 应命中 25 个 string + 1 个 hash");

        // TYPE 过滤只留 hash
        let hash_only = RedisScanFilter {
            pattern: Some(format!("{}*", redis_glob_escape(&prefix))),
            type_name: Some("hash".to_string()),
        };
        let (hash_keys, _) = redis_scan_keys_page(&mut connection, &hash_only, "0", 100).unwrap();
        assert_eq!(hash_keys, vec![format!("{prefix}:h0")]);

        for key in seen {
            connection.command(&["DEL", &key]).unwrap();
        }
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_load_data_pages`
    #[test]
    #[ignore]
    fn redis_load_data_pages_do_not_duplicate_or_drop_keys() {
        let config = redis_config();
        let prefix = format!(
            "gdb_page_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            for index in 0..23 {
                connection
                    .command(&["SET", &format!("{prefix}:{index:03}"), "v"])
                    .unwrap();
            }
        }
        let path = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: "0".to_string(),
            kind: ObjectKind::RedisDb,
        };
        let filters = vec![FilterSpec {
            field: "键".to_string(),
            op: FilterOp::Contains,
            values: vec![CellValue::Text(prefix.clone())],
            enabled: true,
        }];

        let mut seen = Vec::new();
        let mut offset = 0_u64;
        loop {
            let page = redis_load_data(&config, &path, offset, 10, &filters).unwrap();
            for row in &page.rows {
                seen.push(row.values[0].display_label());
            }
            offset += page.rows.len() as u64;
            if !page.has_more || page.rows.is_empty() {
                break;
            }
        }

        let unique = seen.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(seen.len(), 23, "翻页结果条数不对：{seen:?}");
        assert_eq!(unique.len(), 23, "翻页出现重复 key");

        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        for key in seen {
            connection.command(&["DEL", &key]).unwrap();
        }
    }

    #[test]
    fn redis_key_preview_commands_cover_each_type() {
        assert_eq!(
            redis_key_preview_commands("k", "string"),
            vec![vec!["GET".to_string(), "k".to_string()]]
        );
        // set / stream 各需要两条命令（总数 + 内容），解析端要按同样顺序取
        assert_eq!(redis_key_preview_commands("k", "set").len(), 2);
        assert_eq!(redis_key_preview_commands("k", "stream").len(), 2);
        // 未知类型不发预览命令
        assert!(redis_key_preview_commands("k", "unknown").is_empty());
    }

    #[test]
    fn redis_key_preview_from_detects_json_string() {
        let (type_name, value) = redis_key_preview_from(
            "string",
            vec![RedisValue::Bulk(Some(b"{\"a\":1}".to_vec()))],
        );

        assert_eq!(type_name, "json");
        assert_eq!(value, "{\"a\":1}");
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_key_rows_pipeline`
    #[test]
    #[ignore]
    fn redis_key_rows_pipeline_matches_per_key_results() {
        let config = redis_config();
        let prefix = format!(
            "gdb_pipeline_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        let keys = vec![
            format!("{prefix}:str"),
            format!("{prefix}:hash"),
            format!("{prefix}:list"),
            format!("{prefix}:set"),
            format!("{prefix}:zset"),
        ];
        connection.command(&["SET", &keys[0], "hello"]).unwrap();
        connection.command(&["HSET", &keys[1], "f", "v"]).unwrap();
        connection.command(&["RPUSH", &keys[2], "a", "b"]).unwrap();
        connection.command(&["SADD", &keys[3], "m"]).unwrap();
        connection.command(&["ZADD", &keys[4], "1", "m"]).unwrap();
        connection.command(&["EXPIRE", &keys[0], "600"]).unwrap();

        let rows = redis_key_rows(&mut connection, &keys).unwrap();

        assert_eq!(rows.len(), keys.len());
        let types = rows
            .iter()
            .map(|row| row.values[1].display_label())
            .collect::<Vec<_>>();
        assert_eq!(types, ["string", "hash", "list", "set", "zset"]);
        assert_eq!(rows[0].values[2].display_label(), "hello");
        // 大小列来自 MEMORY USAGE（已换算为可读大小），TTL 列来自 TTL，都应有值
        let size_label = rows[0].values[3].display_label();
        assert!(
            size_label.ends_with(" B")
                || size_label.ends_with(" KB")
                || size_label.ends_with(" MB")
                || size_label.ends_with(" GB")
                || size_label.ends_with(" TB")
        );
        assert!(rows[0].values[4].display_label().ends_with('s'));
        assert_eq!(rows[1].values[4].display_label(), "(No TTL)");

        for key in &keys {
            connection.command(&["DEL", key]).unwrap();
        }
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_insert_key`
    #[test]
    #[ignore]
    fn redis_insert_key_creates_each_type() {
        let config = redis_config();
        let prefix = format!(
            "gdb_insert_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: "0".to_string(),
            kind: ObjectKind::RedisDb,
        };
        let row = |key: &str, kind: &str, value: &str, ttl: &str| Row {
            values: vec![
                CellValue::Text(key.to_string()),
                CellValue::Text(kind.to_string()),
                CellValue::Text(value.to_string()),
                CellValue::Null,
                CellValue::Text(ttl.to_string()),
            ],
        };
        let cases = vec![
            (format!("{prefix}:str"), "string", "hello", "600", "string"),
            (format!("{prefix}:list"), "list", "a\nb", "", "list"),
            (format!("{prefix}:set"), "set", "m1\nm2", "", "set"),
            (format!("{prefix}:hash"), "hash", "f=v\ng=w", "", "hash"),
            (format!("{prefix}:zset"), "zset", "m=1.5", "", "zset"),
            (format!("{prefix}:stream"), "stream", "f=v", "", "stream"),
        ];
        let connector = RedisConnector::with_config(config.clone());
        for (key, kind, value, ttl, _) in &cases {
            connector
                .apply_changes(&DataChangeSet {
                    object: object.clone(),
                    inserts: vec![row(key, kind, value, ttl)],
                    updates: Vec::new(),
                    deletes: Vec::new(),
                })
                .unwrap();
        }

        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        for (key, _, _, _, expected_type) in &cases {
            let actual = redis_value_text(connection.command(&["TYPE", key]).unwrap());
            assert_eq!(&actual, expected_type, "{key} 类型不对");
        }
        // TTL 列生效
        assert!(matches!(
            connection.command(&["TTL", &cases[0].0]).unwrap(),
            RedisValue::Int(ttl) if ttl > 0
        ));

        // 重名必须被拒绝，避免覆盖已有 Key
        let duplicate = connector.apply_changes(&DataChangeSet {
            object: object.clone(),
            inserts: vec![row(&cases[0].0, "string", "x", "")],
            updates: Vec::new(),
            deletes: Vec::new(),
        });
        assert!(duplicate.is_err());

        // 集合类必须带元素
        let empty_list = connector.apply_changes(&DataChangeSet {
            object: object.clone(),
            inserts: vec![row(&format!("{prefix}:empty"), "list", "", "")],
            updates: Vec::new(),
            deletes: Vec::new(),
        });
        assert!(empty_list.is_err());

        for (key, _, _, _, _) in &cases {
            connection.command(&["DEL", key]).unwrap();
        }
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_list_search`
    #[test]
    #[ignore]
    fn redis_list_search_jumps_by_index_and_paginates() {
        let config = redis_config();
        let key = format!(
            "gdb_list_search_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            // 0..29：偶数项带 "hit"，共 15 个命中
            for index in 0..30 {
                let value = if index % 2 == 0 {
                    format!("hit-{index}")
                } else {
                    format!("miss-{index}")
                };
                connection.command(&["RPUSH", key.as_str(), &value]).unwrap();
            }
        }
        let connector = RedisConnector::with_config(config.clone());

        // 无搜索：正常分页
        let page = connector.load_list_items(&object, "", "", 10).unwrap();
        assert_eq!(page.total, 30);
        assert_eq!(page.items.len(), 10);
        assert_eq!(page.items[0].0, 0);
        assert_eq!(page.next_cursor, "10");

        // 搜索语义对齐 RedisInsight：按「下标跳转」（LINDEX）精确读取单个元素，
        // 不做内容包含过滤。
        let hit = connector.load_list_items(&object, "5", "", 10).unwrap();
        assert_eq!(hit.total, 30);
        assert_eq!(hit.items, vec![(5, "miss-5".to_string())], "按下标 5 应命中 miss-5");
        let hit_even = connector.load_list_items(&object, "6", "", 10).unwrap();
        assert_eq!(hit_even.items, vec![(6, "hit-6".to_string())], "按下标 6 应命中 hit-6");
        assert_eq!(hit_even.next_cursor, "0", "跳转结果不分页");

        // 越界下标：LINDEX 返回空，页为空但总数保留。
        let out_of_range = connector.load_list_items(&object, "999", "", 10).unwrap();
        assert!(out_of_range.items.is_empty());
        assert_eq!(out_of_range.total, 30);

        // 非数字搜索词无法解析成下标，视为未命中。
        let non_numeric = connector.load_list_items(&object, "hit", "", 10).unwrap();
        assert!(non_numeric.items.is_empty());

        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        connection.command(&["DEL", key.as_str()]).unwrap();
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// 验证对一个「非 List」类型的键执行 load_list_items，会命中 WRONGTYPE 分支并返回中文提示。
    #[test]
    #[ignore]
    fn redis_list_load_on_non_list_key_returns_wrongtype_message() {
        let config = redis_config();
        let key = format!(
            "gdb_list_nonlist_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            // 造一个 Hash 类型的键（与 List 面板的预期类型不符）
            connection
                .command(&["HSET", key.as_str(), "f", "v"])
                .unwrap();
        }
        let connector = RedisConnector::with_config(config.clone());
        let err = connector.load_list_items(&object, "", "", 10).unwrap_err();
        assert!(
            err.message.contains("不是 List"),
            "WRONGTYPE 应被翻译成清晰提示，实际: {}",
            err.message
        );
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            connection.command(&["DEL", key.as_str()]).unwrap();
        }
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// 验证对一个「非 Set」类型的键执行 load_set_members，会命中 WRONGTYPE 分支并返回中文提示。
    #[test]
    #[ignore]
    fn redis_set_load_on_non_set_key_returns_wrongtype_message() {
        let config = redis_config();
        let key = format!(
            "gdb_set_nonlist_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            // 造一个 Hash 类型的键（与 Set 面板的预期类型不符）
            connection
                .command(&["HSET", key.as_str(), "f", "v"])
                .unwrap();
        }
        let connector = RedisConnector::with_config(config.clone());
        let err = connector.load_set_members(&object, "", "", 10).unwrap_err();
        assert!(
            err.message.contains("不是 Set"),
            "WRONGTYPE 应被翻译成清晰提示，实际: {}",
            err.message
        );
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            connection.command(&["DEL", key.as_str()]).unwrap();
        }
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// 验证 pop_list_items 按数量从头/尾弹出（LPOP/RPOP），且总数随之减少。
    #[test]
    #[ignore]
    fn redis_list_pop_removes_elements_from_head_and_tail() {
        let config = redis_config();
        let key = format!(
            "gdb_list_pop_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        {
            let mut connection = redis_connect(&config).unwrap();
            redis_select(&mut connection, 0).unwrap();
            for value in ["a", "b", "c", "d", "e"] {
                connection.command(&["RPUSH", key.as_str(), value]).unwrap();
            }
        }
        let connector = RedisConnector::with_config(config.clone());

        // 从头弹出 2 个：LPOP，剩余 [c, d, e]
        connector.pop_list_items(&object, true, 2).unwrap();
        let page = connector.load_list_items(&object, "", "", 10).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(
            page.items.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            vec!["c".to_string(), "d".to_string(), "e".to_string()],
            "从头弹出 2 个后应剩 c/d/e"
        );

        // 从尾弹出 1 个：RPOP，剩余 [c, d]
        connector.pop_list_items(&object, false, 1).unwrap();
        let page = connector.load_list_items(&object, "", "", 10).unwrap();
        assert_eq!(
            page.items.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
            vec!["c".to_string(), "d".to_string()],
            "从尾弹出 1 个后应剩 c/d"
        );

        // 数量为 0 属于非法入参，应报错。
        let err = connector.pop_list_items(&object, true, 0).unwrap_err();
        assert!(err.message.contains("大于 0"), "数量为 0 应报错，实际: {}", err.message);

        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        connection.command(&["DEL", key.as_str()]).unwrap();
    }

    /// 需要本地 127.0.0.1:6379，默认不跑：
    /// `cargo test -p fluxdb-connectors -- --ignored redis_stream_range_maxlen_groups`
    #[test]
    #[ignore]
    fn redis_stream_range_maxlen_groups_work_on_real_server() {
        let config = redis_config();
        let connector = RedisConnector::with_config(config.clone());
        let key = format!(
            "gdb_stream_extra_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some("0".to_string()),
            schema: None,
            name: key.clone(),
            kind: ObjectKind::RedisKey,
        };
        // 用显式 ID 造出三个不同毫秒的条目，方便按时间范围断言
        let mut connection = redis_connect(&config).unwrap();
        redis_select(&mut connection, 0).unwrap();
        for ms in [1_700_000_000_000_u64, 1_700_000_001_000, 1_700_000_002_000] {
            connection
                .command(&["XADD", key.as_str(), &format!("{ms}-0"), "seq", "v"])
                .unwrap();
        }

        // 只要中间那一毫秒
        let page = connector
            .load_stream_entries(
                &object,
                RedisStreamRange {
                    since_ms: Some(1_700_000_001_000),
                    until_ms: Some(1_700_000_001_000),
                },
                "",
                100,
            )
            .unwrap();
        assert_eq!(page.entries.len(), 1, "时间范围过滤结果不对");
        assert_eq!(page.entries[0].id, "1700000001000-0");

        // 只给下界
        let since_only = connector
            .load_stream_entries(
                &object,
                RedisStreamRange {
                    since_ms: Some(1_700_000_001_000),
                    until_ms: None,
                },
                "",
                100,
            )
            .unwrap();
        assert_eq!(since_only.entries.len(), 2);

        // MAXLEN 近似裁剪：~ 语义下不保证精确，但不能超过流原本的长度
        connector
            .add_stream_entry(&object, "*", &[("seq".to_string(), "x".to_string())], Some(1))
            .unwrap();
        let len = match connection.command(&["XLEN", key.as_str()]).unwrap() {
            RedisValue::Int(len) => len,
            other => panic!("XLEN 返回异常：{other:?}"),
        };
        assert!(len <= 4, "MAXLEN 应该没有增长失控，实际 {len}");
        assert!(
            connector
                .add_stream_entry(&object, "*", &[("seq".to_string(), "x".to_string())], Some(0))
                .is_err(),
            "MAXLEN 0 应被拒绝"
        );

        // 消费者组：建组、读一条形成 pending，再看概览
        assert!(connector.load_stream_groups(&object).unwrap().is_empty());
        connection
            .command(&["XGROUP", "CREATE", key.as_str(), "g1", "0"])
            .unwrap();
        connection
            .command(&[
                "XREADGROUP", "GROUP", "g1", "c1", "COUNT", "1", "STREAMS", key.as_str(), ">",
            ])
            .unwrap();

        let groups = connector.load_stream_groups(&object).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "g1");
        assert_eq!(groups[0].consumers, 1);
        assert_eq!(groups[0].pending, 1);
        assert_eq!(groups[0].consumer_detail.len(), 1);
        assert_eq!(groups[0].consumer_detail[0].0, "c1");
        assert_eq!(groups[0].consumer_detail[0].1, 1);

        connection.command(&["DEL", key.as_str()]).unwrap();
    }

    #[test]
    fn redis_parse_redirect_reads_moved_and_ask() {
        let moved = redis_parse_redirect("MOVED 3999 127.0.0.1:6381").unwrap();
        assert!(!moved.ask);
        assert_eq!(moved.host, "127.0.0.1");
        assert_eq!(moved.port, 6381);

        let ask = redis_parse_redirect("ASK 3999 10.0.0.2:7001").unwrap();
        assert!(ask.ask);
        assert_eq!(ask.port, 7001);

        // IPv6 形式
        let v6 = redis_parse_redirect("MOVED 1 [::1]:6379").unwrap();
        assert_eq!(v6.host, "::1");

        // 普通错误不能被当成重定向
        assert!(redis_parse_redirect("WRONGTYPE Operation against a key").is_none());
        assert!(redis_parse_redirect("MOVED 3999").is_none());
    }

    #[test]
    fn redis_is_wrongtype_classifies_type_mismatch_errors() {
        let wrongtype = Error::new(ErrorKind::Query, "WRONGTYPE Operation against a key holding the wrong kind of value");
        assert!(redis_is_wrongtype(&wrongtype));
        // 其他错误类型不被误判
        let not_wrongtype = Error::new(ErrorKind::Query, "NOAUTH Authentication required.");
        assert!(!redis_is_wrongtype(&not_wrongtype));
    }

    /// key 名恰好是纯数字时，若不显式指定库号，绝不能把 key 名当库号——
    /// 那会把 SELECT 打到错误的库，在对同名 key 做 LLEN/LINDEX 时误触发 WRONGTYPE。
    #[test]
    fn redis_path_database_defaults_to_zero_and_never_uses_key_name() {
        let base = |name: &str, database: Option<String>| ObjectPath {
            connection_id: ConnectionId(3),
            database,
            schema: None,
            name: name.to_string(),
            kind: ObjectKind::RedisKey,
        };
        // 未指定库号 → 0（关键回归：key 名是数字也必须是 0）
        assert_eq!(redis_path_database(&base("12345", None)).unwrap(), 0);
        assert_eq!(redis_path_database(&base("my:key", None)).unwrap(), 0);
        // 显式指定库号 → 用之
        assert_eq!(
            redis_path_database(&base("k", Some("5".to_string()))).unwrap(),
            5
        );
        // 非数字库号 → 报错而非 panic
        assert!(redis_path_database(&base("k", Some("abc".to_string()))).is_err());
    }

    #[test]
    fn redis_tls_options_read_switches_and_pool_key_separates_them() {
        let mut config = redis_config();
        assert!(!redis_tls_enabled(&config));
        assert!(!redis_tls_insecure(&config));
        let plain_key = redis_pool_key(&config).unwrap();

        config.options.insert("tls".to_string(), "TRUE".to_string());
        assert!(redis_tls_enabled(&config));
        let tls_key = redis_pool_key(&config).unwrap();
        // 明文连接不能被复用到 TLS 连接上
        assert_ne!(plain_key, tls_key);

        config
            .options
            .insert("tls_insecure".to_string(), "yes".to_string());
        assert!(redis_tls_insecure(&config));

        config
            .options
            .insert("sentinel_master".to_string(), "mymaster".to_string());
        assert_ne!(redis_pool_key(&config).unwrap(), tls_key);
    }

    #[test]
    fn redis_pool_key_separates_host_port_and_credentials() {
        let mut config = redis_config();
        let base = redis_pool_key(&config).unwrap();

        config
            .options
            .insert("username".to_string(), "reader".to_string());
        let with_user = redis_pool_key(&config).unwrap();

        assert_ne!(base, with_user);
        assert_eq!(with_user.username, "reader");
        assert_eq!(redis_pool_key(&config).unwrap(), with_user);
    }

    #[test]
    fn redis_pool_key_rejects_non_tcp_endpoint() {
        let mut config = redis_config();
        config.endpoint = Endpoint::SqliteFile {
            path: "demo.db".into(),
            read_only: false,
        };

        assert!(redis_pool_key(&config).is_err());
    }

    #[test]
    fn redis_key_columns_match_first_screen() {
        let columns = redis_key_columns()
            .into_iter()
            .map(|column| column.name)
            .collect::<Vec<_>>();

        assert_eq!(columns, ["键", "类型", "值", "大小", "TTL"]);
    }

    #[test]
    fn redis_preview_items_formats_collections() {
        let value = RedisValue::Array(vec![RedisValue::Bulk(Some(b"test".to_vec()))]);

        assert_eq!(redis_preview_items(Some(value)), "[test]");
    }

    #[test]
    fn redis_set_preview_formats_members() {
        let members = RedisValue::Array(vec![
            RedisValue::Bulk(Some(b"test".to_vec())),
            RedisValue::Bulk(Some(b"test1".to_vec())),
        ]);

        assert_eq!(
            redis_set_preview(Some(RedisValue::Int(2)), Some(members)),
            "2 成员\ntest\ntest1"
        );
    }

    #[test]
    fn redis_set_members_from_text_skips_blank_rows() {
        assert_eq!(
            redis_set_members_from_text("alpha\n\n beta \n"),
            Ok(vec!["alpha".to_string(), " beta ".to_string()])
        );
    }

    #[test]
    fn redis_stream_preview_includes_entry_count() {
        assert_eq!(redis_stream_preview(Some(RedisValue::Int(2)), None), "2 条目");
    }

    #[test]
    fn redis_stream_preview_formats_entries() {
        let entries = RedisValue::Array(vec![RedisValue::Array(vec![
            RedisValue::Bulk(Some(b"1785677482094-0".to_vec())),
            RedisValue::Array(vec![
                RedisValue::Bulk(Some(b"test".to_vec())),
                RedisValue::Bulk(Some(b"3".to_vec())),
                RedisValue::Bulk(Some(b"test1".to_vec())),
                RedisValue::Bulk(Some(b"2".to_vec())),
            ]),
        ])]);

        let preview = redis_stream_preview(Some(RedisValue::Int(2)), Some(entries));

        assert!(preview.contains("2 条目"));
        assert!(preview.contains("1785677482094-0"));
        assert!(preview.contains("test: 3"));
        assert!(preview.contains("test1: 2"));
    }

    #[test]
    fn mysql_index_metadata_query_uses_stable_non_unique_alias() {
        assert!(mysql_indexes_query().contains("CAST(non_unique AS SIGNED) AS non_unique"));
    }

    #[test]
    fn mysql_table_objects_query_uses_signed_row_count() {
        assert!(mysql_table_objects_query().contains("CAST(table_rows AS SIGNED) AS row_count"));
        assert!(mysql_table_objects_query().contains("CAST(update_time AS CHAR) AS modified_at"));
    }

    #[test]
    fn mysql_show_table_objects_query_quotes_database() {
        assert_eq!(
            mysql_show_table_objects_query("sync-db"),
            "SHOW FULL TABLES FROM `sync-db`"
        );
    }

    #[test]
    fn mysql_ensure_columns_available_rejects_empty_column_list() {
        // 空列列表会拼出 `SELECT  FROM \`db\`.\`t\``，必须提前拦下，而不是丢给服务端报 1064。
        let error = mysql_ensure_columns_available("gaea", "3d_applications", &[])
            .expect_err("空列列表必须报错");

        assert!(
            error.to_string().contains("3d_applications"),
            "实际错误：{error}"
        );
    }

    #[test]
    fn mysql_ensure_columns_available_accepts_non_empty_column_list() {
        let columns = vec![Column {
            name: "id".to_string(),
            type_name: Some("int".to_string()),
            nullable: false,
            primary_key: true,
            comment: None,
        }];

        assert!(mysql_ensure_columns_available("gaea", "3d_applications", &columns).is_ok());
    }

    #[test]
    fn mysql_create_database_sql_quotes_database_and_validates_options() {
        let sql = mysql_create_database_sql(&CreateDatabaseRequest {
            connection_id: ConnectionId(2),
            name: "app-db".to_string(),
            charset: "utf8mb4".to_string(),
            collation: "utf8mb4_unicode_ci".to_string(),
            path: None,
        })
        .unwrap();

        assert_eq!(
            sql,
            "CREATE DATABASE `app-db` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
        );

        let result = mysql_create_database_sql(&CreateDatabaseRequest {
            connection_id: ConnectionId(2),
            name: "app".to_string(),
            charset: "utf8mb4;drop".to_string(),
            collation: "utf8mb4_unicode_ci".to_string(),
            path: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn mysql_delete_database_sql_quotes_database() {
        assert_eq!(
            mysql_delete_database_sql("app-db").unwrap(),
            "DROP DATABASE `app-db`"
        );
        assert!(mysql_delete_database_sql(" ").is_err());
    }

    #[test]
    fn sqlite_create_database_creates_file_and_rejects_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "gdb-sqlite-create-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "source".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: std::env::temp_dir().join("source.db"),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };
        let request = CreateDatabaseRequest {
            connection_id: config.id,
            name: "created".to_string(),
            charset: String::new(),
            collation: String::new(),
            path: Some(path.clone()),
        };

        SqliteConnector::with_config(config)
            .create_database(&request)
            .unwrap();
        assert!(path.exists());
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "source".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: std::env::temp_dir().join("source.db"),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };
        assert!(SqliteConnector::with_config(config)
            .create_database(&request)
            .is_err());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_load_data_reads_attached_database_table() {
        let dir = std::env::temp_dir().join(format!(
            "gdb-sqlite-read-attach-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let main_path = dir.join("main.db");
        let attached_path = dir.join("analytics.db");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&attached_path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO events (id, name) VALUES (1, 'login')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });
        let mut config = ConnectionConfig {
            id: ConnectionId(8),
            name: "source".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: main_path,
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        };
        fluxdb_core::set_sqlite_attached_database(&mut config, "analytics", attached_path);

        let page = SqliteConnector::with_config(config)
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(8),
                    database: Some("analytics".to_string()),
                    schema: None,
                    name: "events".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                10,
                &[],
                &[],
            )
            .unwrap();

        assert_eq!(page.rows[0].values[1], CellValue::Text("login".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mysql_object_row_count_ignores_negative_estimates() {
        assert_eq!(mysql_object_row_count(Some(42)), Some(42));
        assert_eq!(mysql_object_row_count(Some(-1)), None);
        assert_eq!(mysql_object_row_count(None), None);
    }

    #[test]
    fn mysql_query_bytes_cell_value_decodes_small_printable_utf8() {
        assert_eq!(
            mysql_query_bytes_cell_value(b"syncdb".to_vec()),
            CellValue::Text("syncdb".to_string())
        );
    }

    #[test]
    fn mysql_query_bytes_cell_value_keeps_binary_or_large_values() {
        assert_eq!(
            mysql_query_bytes_cell_value(vec![0, b'a']),
            CellValue::Bytes(vec![0, b'a'])
        );
        assert_eq!(
            mysql_query_bytes_cell_value(vec![b'a'; MYSQL_QUERY_TEXT_BYTES_LIMIT + 1]),
            CellValue::Bytes(vec![b'a'; MYSQL_QUERY_TEXT_BYTES_LIMIT + 1])
        );
    }

    #[test]
    fn mysql_optional_display_cell_value_preserves_unsigned_bigint_text() {
        assert_eq!(
            mysql_optional_display_cell_value(Some(391041_u64)),
            CellValue::Text("391041".to_string())
        );
        assert_eq!(mysql_optional_display_cell_value(None::<u64>), CellValue::Null);
    }

    #[test]
    fn mysql_optional_display_cell_value_preserves_decimal_text() {
        let value = "1234567890.0123456789".parse::<BigDecimal>().unwrap();

        assert_eq!(
            mysql_optional_display_cell_value(Some(value)),
            CellValue::Text("1234567890.0123456789".to_string())
        );
    }

    #[test]
    fn sqlite_test_connection_accepts_memory_database() {
        let connector = SqliteConnector::new();
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: ":memory:".into(),
            read_only: false,
        };

        assert!(connector.test_connection(&config).is_ok());
    }

    #[test]
    fn sqlite_test_connection_creates_file_database() {
        let connector = SqliteConnector::new();
        let path = temp_sqlite_path("create-file");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let result = connector.test_connection(&config);

        assert!(result.is_ok());
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_test_connection_rejects_missing_parent_directory() {
        let connector = SqliteConnector::new();
        let path = temp_sqlite_path("missing-parent").join("demo.db");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path,
            read_only: false,
        };

        let result = connector.test_connection(&config);

        assert!(result.is_err());
    }

    #[test]
    fn sqlite_test_connection_rejects_non_sqlite_endpoint() {
        let connector = SqliteConnector::new();
        let mut config = sqlite_config();
        config.endpoint = Endpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: 3306,
            database: None,
        };

        let result = connector.test_connection(&config);

        assert!(result.is_err());
    }

    #[test]
    fn list_objects_returns_mock_database_then_tables() {
        let connector = MockConnector::sqlite();

        let databases = connector.list_objects(None).unwrap();

        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0].path.name, "main");
        assert_eq!(databases[0].path.kind, ObjectKind::Database);

        let objects = connector.list_objects(Some(&databases[0].path)).unwrap();

        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].path.name, "Product");
    }

    #[test]
    fn sqlite_list_objects_reads_real_schema() {
        let path = temp_sqlite_path("schema");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("CREATE VIEW active_customers AS SELECT id, name FROM customers")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let databases = connector.list_objects(None).unwrap();
        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0].path.name, "main");

        let objects = connector.list_objects(Some(&databases[0].path)).unwrap();

        assert!(
            objects
                .iter()
                .any(|object| object.path.name == "customers"
                    && object.path.kind == ObjectKind::Table)
        );
        assert!(
            objects
                .iter()
                .any(|object| object.path.name == "active_customers"
                    && object.path.kind == ObjectKind::View)
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_data_returns_page() {
        let connector = MockConnector::sqlite();
        let database = connector.list_objects(None).unwrap().remove(0).path;
        let object = connector
            .list_objects(Some(&database))
            .unwrap()
            .remove(0)
            .path;

        let page = connector.load_data(&object, 10, 50, &[], &[]).unwrap();

        assert_eq!(page.offset, 10);
        assert_eq!(page.limit, 50);
        assert_eq!(page.rows.len(), 2);
    }

    #[test]
    fn sqlite_load_data_reads_real_rows() {
        let path = temp_sqlite_path("rows");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO customers (name) VALUES ('Alice'), ('Bob')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                100,
                &[],
                &[],
            )
            .unwrap();

        assert_eq!(page.columns[0].name, "id");
        assert_eq!(page.columns[1].name, "name");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[1], CellValue::Text("Alice".to_string()));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_load_data_sorts_in_database() {
        let path = temp_sqlite_path("sort");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO customers (name) VALUES ('Alice'), ('Carol'), ('Bob')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                2,
                &[SortSpec {
                    field: "name".to_string(),
                    direction: SortDirection::Desc,
                }],
                &[],
            )
            .unwrap();

        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[1], CellValue::Text("Carol".to_string()));
        assert_eq!(page.rows[1].values[1], CellValue::Text("Bob".to_string()));
        assert!(page.has_more);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_load_data_filters_in_database() {
        let path = temp_sqlite_path("filter");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO customers (name) VALUES ('Alice'), ('Carol'), ('Bob')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                100,
                &[SortSpec {
                    field: "name".to_string(),
                    direction: SortDirection::Asc,
                }],
                &[fluxdb_core::FilterSpec {
                    field: "name".to_string(),
                    op: fluxdb_core::FilterOp::Contains,
                    values: vec![CellValue::Text("o".to_string())],
                    enabled: true,
                }],
            )
            .unwrap();

        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[1], CellValue::Text("Bob".to_string()));
        assert_eq!(page.rows[1].values[1], CellValue::Text("Carol".to_string()));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_preview_data_export_counts_filtered_rows_and_sql() {
        let path = temp_sqlite_path("export_preview");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO customers (name) VALUES ('Alice'), ('Carol'), ('Bob')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let preview = connector
            .preview_data_export(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                &["id".to_string(), "name".to_string()],
                &[SortSpec {
                    field: "name".to_string(),
                    direction: SortDirection::Asc,
                }],
                &[fluxdb_core::FilterSpec {
                    field: "name".to_string(),
                    op: fluxdb_core::FilterOp::Contains,
                    values: vec![CellValue::Text("o".to_string())],
                    enabled: true,
                }],
            )
            .unwrap();

        assert_eq!(preview.row_count, 2);
        assert!(preview.sql.contains("SELECT \"id\", \"name\""));
        assert!(preview.sql.contains("WHERE \"name\" LIKE '%o%'"));
        assert!(preview.sql.contains("ORDER BY \"name\" ASC"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_apply_changes_writes_inserts_updates_and_deletes() {
        let path = temp_sqlite_path("apply");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE customers (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    active INTEGER NULL,
                    nickname TEXT DEFAULT 'friend'
                )",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO customers (id, name, active)
                 VALUES (1, 'Alice', 1), (2, 'Bob', 1)",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        connector
            .apply_changes(&DataChangeSet {
                object: ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                inserts: vec![Row {
                    values: vec![
                        CellValue::I64(3),
                        CellValue::Text("Carol".to_string()),
                        CellValue::Bool(false),
                        CellValue::Null,
                    ],
                }],
                updates: vec![fluxdb_core::RowUpdate {
                    identity: fluxdb_core::RowIdentity {
                        values: [("id".to_string(), CellValue::I64(1))].into(),
                    },
                    cells: vec![
                        fluxdb_core::CellUpdate {
                            column: "name".to_string(),
                            value: CellValue::Text("Alicia".to_string()),
                        },
                        fluxdb_core::CellUpdate {
                            column: "active".to_string(),
                            value: CellValue::Null,
                        },
                    ],
                }],
                deletes: vec![fluxdb_core::RowIdentity {
                    values: [("id".to_string(), CellValue::I64(2))].into(),
                }],
            })
            .unwrap();

        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                100,
                &[SortSpec {
                    field: "id".to_string(),
                    direction: SortDirection::Asc,
                }],
                &[],
            )
            .unwrap();

        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[0], CellValue::Text("1".to_string()));
        assert_eq!(
            page.rows[0].values[1],
            CellValue::Text("Alicia".to_string())
        );
        assert_eq!(page.rows[0].values[2], CellValue::Null);
        assert_eq!(page.rows[1].values[0], CellValue::Text("3".to_string()));
        assert_eq!(page.rows[1].values[1], CellValue::Text("Carol".to_string()));
        assert_eq!(page.rows[1].values[2], CellValue::Text("0".to_string()));
        assert_eq!(
            page.rows[1].values[3],
            CellValue::Text("friend".to_string())
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_apply_changes_uses_default_values_for_all_null_insert() {
        let path = temp_sqlite_path("apply-defaults");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE customers (
                    id INTEGER PRIMARY KEY,
                    name TEXT DEFAULT 'friend'
                )",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        connector
            .apply_changes(&DataChangeSet {
                object: ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                inserts: vec![Row {
                    values: vec![CellValue::Null, CellValue::Null],
                }],
                updates: Vec::new(),
                deletes: Vec::new(),
            })
            .unwrap();

        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "customers".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                100,
                &[SortSpec {
                    field: "id".to_string(),
                    direction: SortDirection::Asc,
                }],
                &[],
            )
            .unwrap();

        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].values[0], CellValue::Text("1".to_string()));
        assert_eq!(
            page.rows[0].values[1],
            CellValue::Text("friend".to_string())
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_load_cell_binary_reads_full_blob_by_identity() {
        let path = temp_sqlite_path("cell-binary");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE files (id INTEGER PRIMARY KEY, payload BLOB)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO files (id, payload) VALUES (1, x'DEADBEEF')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let bytes = connector
            .load_cell_binary(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "files".to_string(),
                    kind: ObjectKind::Table,
                },
                &fluxdb_core::RowIdentity {
                    values: [("id".to_string(), CellValue::Text("1".to_string()))].into(),
                },
                "payload",
            )
            .unwrap();

        assert_eq!(bytes, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_reads_table_metadata() {
        let path = temp_sqlite_path("metadata");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE users (
                    id INTEGER PRIMARY KEY,
                    team_id INTEGER REFERENCES teams(id),
                    email TEXT NOT NULL
                )",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            sqlx::query("CREATE UNIQUE INDEX users_email_idx ON users(email)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TRIGGER users_ai AFTER INSERT ON users
                 BEGIN
                   SELECT NEW.id;
                 END",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            name: "users".to_string(),
            kind: ObjectKind::Table,
        };

        let indexes = connector.list_indexes(&object).unwrap();
        let foreign_keys = connector.list_foreign_keys(&object).unwrap();
        let triggers = connector.list_triggers(&object).unwrap();
        let completion_triggers = connector
            .list_completion_triggers(Some("main"), None, "users", 20)
            .unwrap();
        let completion_routines = connector
            .list_completion_routines(Some("main"), None, "", 20)
            .unwrap();
        let ddl = connector.table_ddl(&object).unwrap();

        assert!(indexes.iter().any(|index| {
            index.name == "users_email_idx" && index.is_unique && index.columns == ["email"]
        }));
        assert_eq!(foreign_keys[0].column, "team_id");
        assert_eq!(foreign_keys[0].ref_table, "teams");
        assert_eq!(foreign_keys[0].ref_column, "id");
        assert_eq!(triggers[0].name, "users_ai");
        assert_eq!(triggers[0].event, "INSERT");
        assert_eq!(triggers[0].timing, "AFTER");
        assert_eq!(completion_triggers[0].name, "users_ai");
        assert_eq!(completion_triggers[0].table.as_deref(), Some("users"));
        assert!(completion_routines.is_empty());
        assert!(ddl.contains("CREATE TABLE users"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_load_data_summarizes_blob_columns() {
        let path = temp_sqlite_path("blob_summary");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };
        let blob = vec![0xAB; 128];

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE files (id INTEGER PRIMARY KEY, payload BLOB)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO files (payload) VALUES (?)")
                .bind(blob)
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    schema: None,
                    name: "files".to_string(),
                    kind: ObjectKind::Table,
                },
                0,
                100,
                &[],
                &[],
            )
            .unwrap();

        assert_eq!(
            page.rows[0].values[1],
            CellValue::BinarySummary(fluxdb_core::BinaryCellSummary {
                type_name: "BLOB".to_string(),
                is_null: false,
                byte_length: 128,
                preview_hex: Some("AB".repeat(64)),
            })
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn execute_returns_result_page() {
        let connector = MockConnector::sqlite();

        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "select * from Product".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions::default(),
            })
            .unwrap();

        assert_eq!(execution.results.len(), 1);
        assert_eq!(execution.results[0].columns.len(), 2);
        assert_eq!(execution.summaries.len(), 1);
    }

    #[test]
    fn execute_splits_multiple_statements() {
        let connector = MockConnector::sqlite();

        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "select * from Product；\nselect * from Product".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions::default(),
            })
            .unwrap();

        assert_eq!(execution.results.len(), 2);
        assert_eq!(execution.summaries.len(), 2);
    }

    #[test]
    fn execute_records_mock_error_query_as_failed_summary() {
        let connector = MockConnector::sqlite();

        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "select error; select * from Product".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions::default(),
            })
            .unwrap();

        assert_eq!(execution.summaries.len(), 2);
        assert!(!execution.summaries[0].success);
        assert!(execution.summaries[1].success);
        assert_eq!(execution.results.len(), 1);
    }

    #[test]
    fn execute_can_stop_after_statement_error() {
        let connector = MockConnector::sqlite();

        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "select error; select * from Product".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions {
                    continue_on_error: false,
                    split_statements: true,
                    ..fluxdb_core::QueryExecutionOptions::default()
                },
            })
            .unwrap();

        assert_eq!(execution.summaries.len(), 1);
        assert!(!execution.summaries[0].success);
        assert_eq!(execution.results.len(), 0);
    }

    #[test]
    fn execute_can_skip_statement_splitting() {
        let connector = MockConnector::sqlite();

        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "select * from Product; select * from Product".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions {
                    continue_on_error: true,
                    split_statements: false,
                    ..fluxdb_core::QueryExecutionOptions::default()
                },
            })
            .unwrap();

        assert_eq!(execution.summaries.len(), 1);
        assert_eq!(execution.results.len(), 1);
    }

    #[test]
    fn split_sql_keeps_create_trigger_body_together() {
        let statements = split_sql_statements(
            "CREATE TRIGGER users_ai AFTER INSERT ON users
FOR EACH ROW
BEGIN
  INSERT INTO audit_log VALUES (NEW.id);
  UPDATE counters SET value = value + 1;
END;
SELECT 1;",
        );

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("UPDATE counters"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn execute_with_progress_reports_summaries_and_honors_cancel() {
        let connector = MockConnector::sqlite();
        let seen = std::cell::Cell::new(0);
        let mut summaries = Vec::new();

        let execution = connector
            .execute_with_progress(
                &QueryRequest {
                    connection_id: ConnectionId(1),
                    database: Some("main".to_string()),
                    text: "select * from Product; select * from Product".to_string(),
                    mode: QueryMode::All,
                    options: fluxdb_core::QueryExecutionOptions::default(),
                },
                &mut |summary| {
                    seen.set(seen.get() + 1);
                    summaries.push(summary);
                },
                &|| seen.get() >= 1,
            )
            .unwrap();

        assert_eq!(execution.summaries.len(), 1);
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].success);
    }

    #[test]
    fn sqlite_execute_runs_real_select_and_update_with_summary() {
        let path = temp_sqlite_path("execute");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO items (id, name) VALUES (1, 'old')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "SELECT id, name FROM items; UPDATE items SET name = 'new' WHERE id = 1"
                    .to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions::default(),
            })
            .unwrap();

        assert_eq!(execution.results.len(), 1);
        assert_eq!(execution.results[0].rows.len(), 1);
        assert_eq!(execution.summaries.len(), 2);
        assert_eq!(execution.summaries[0].kind, QueryStatementKind::ResultSet);
        assert_eq!(execution.summaries[0].returned_rows, 1);
        assert_eq!(execution.summaries[1].kind, QueryStatementKind::Command);
        assert_eq!(execution.summaries[1].affected_rows, 1);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_execute_split_sql_runs_create_trigger_body() {
        let path = temp_sqlite_path("execute-trigger");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let connector = SqliteConnector::with_config(config);
        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE audit_log (item_id INTEGER, name TEXT);
CREATE TRIGGER items_ai AFTER INSERT ON items
FOR EACH ROW
BEGIN
  INSERT INTO audit_log VALUES (NEW.id, NEW.name);
END;
INSERT INTO items (id, name) VALUES (1, 'created');
SELECT item_id, name FROM audit_log;"
                    .to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions {
                    split_statements: true,
                    ..fluxdb_core::QueryExecutionOptions::default()
                },
            })
            .unwrap();

        assert_eq!(execution.results.len(), 1);
        assert_eq!(execution.results[0].rows[0].values[1], CellValue::Text("created".to_string()));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_execute_continues_after_statement_error() {
        let path = temp_sqlite_path("execute_continue_after_error");
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut connection = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .connect()
                .await
                .unwrap();
            sqlx::query("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query("INSERT INTO items (id, name) VALUES (1, 'old')")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        });

        let connector = SqliteConnector::with_config(config);
        let execution = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "SELECT * FROM missing_table; SELECT id, name FROM items".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions::default(),
            })
            .unwrap();

        assert_eq!(execution.summaries.len(), 2);
        assert!(!execution.summaries[0].success);
        assert!(execution.summaries[1].success);
        assert_eq!(execution.results.len(), 1);
        assert_eq!(execution.results[0].rows.len(), 1);

        let stopped = connector
            .execute(&QueryRequest {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                text: "SELECT * FROM missing_table; SELECT id, name FROM items".to_string(),
                mode: QueryMode::All,
                options: fluxdb_core::QueryExecutionOptions {
                    continue_on_error: false,
                    split_statements: true,
                    ..fluxdb_core::QueryExecutionOptions::default()
                },
            })
            .unwrap();
        assert_eq!(stopped.summaries.len(), 1);
        assert!(!stopped.summaries[0].success);
        assert_eq!(stopped.results.len(), 0);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn completion_table_filter_matches_fuzzy_input() {
        assert!(matches_completion_fuzzy_filter("Product", "pdt"));
        assert!(matches_completion_fuzzy_filter("Product", "duct"));
        assert!(!matches_completion_fuzzy_filter("Product", "zz"));
        assert_eq!(completion_fuzzy_like_filter("p_t"), "%p%\\_%t%");
    }

    #[test]
    fn mysql_datetime_cell_value_keeps_fractional_seconds() {
        let value = NaiveDateTime::parse_from_str("2022-11-24 06:41:21.726040", "%Y-%m-%d %H:%M:%S%.f")
            .unwrap();

        assert_eq!(
            mysql_datetime_cell_value(Some(value)),
            CellValue::Text("2022-11-24 06:41:21.726040".to_string())
        );
    }

    #[test]
    fn apply_changes_accepts_dirty_change_set() {
        let connector = MockConnector::sqlite();
        let object = connector.list_objects(None).unwrap().remove(0).path;

        let result = connector.apply_changes(&DataChangeSet {
            object,
            inserts: Vec::new(),
            updates: vec![fluxdb_core::RowUpdate {
                identity: fluxdb_core::RowIdentity {
                    values: [("id".to_string(), CellValue::I64(1))].into(),
                },
                cells: vec![fluxdb_core::CellUpdate {
                    column: "name".to_string(),
                    value: CellValue::Text("Touring Bike".to_string()),
                }],
            }],
            deletes: Vec::new(),
        });

        assert!(result.is_ok());
    }

    fn sqlite_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(1),
            name: "SQLite Demo".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: "demo.db".into(),
                read_only: false,
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }
    }

    fn mysql_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(2),
            name: "MySQL Local".to_string(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: Some("app db".to_string()),
            },
            credential_ref: Some("gdb.connection.2".to_string()),
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
        }
    }

    fn redis_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(3),
            name: "Redis Local".to_string(),
            kind: DatabaseKind::Redis,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 6379,
                database: Some("0".to_string()),
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: Some(fluxdb_core::RedisConnectionProfile {
                basic: fluxdb_core::RedisBasicOptions {
                    host: "127.0.0.1".to_string(),
                    port: 6379,
                    database: Some("0".to_string()),
                    username: Some("default".to_string()),
                    password: fluxdb_core::SecretRef::inline("secret"),
                },
                ..Default::default()
            }),
            mysql_profile: None,
        }
    }

    fn workbench_redis_request() -> CommandWorkbenchRequest {
        CommandWorkbenchRequest {
            target: CommandExecutionTarget::Redis {
                connection_id: ConnectionId(3),
                database: 0,
            },
            text: "GET a".to_string(),
            run_mode: fluxdb_core::CommandRunMode::Text,
            results_mode: fluxdb_core::CommandResultsMode::Default,
            batch_size: 0,
            continue_on_error: true,
            source: fluxdb_core::CommandExecutionSource::Workbench,
        }
    }

    #[test]
    fn redis_workbench_requires_connection_config_context() {
        // 无配置上下文（RedisConnector::new）：在执行任何命令前即报连接错误。
        let connector = RedisConnector::new();
        let err = connector
            .execute_command_workbench(&workbench_redis_request())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Connection);
    }

    #[test]
    fn redis_workbench_rejects_unsupported_target() {
        // 提供配置但目标为 MySQL：在真正连接 Redis 之前即返回 Unsupported（不依赖本机是否有 Redis）。
        let connector = RedisConnector::with_config(redis_config());
        let mut request = workbench_redis_request();
        request.target = CommandExecutionTarget::MySql {
            connection_id: ConnectionId(3),
            database: None,
            schema: None,
        };
        let err = connector.execute_command_workbench(&request).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
    }

    /// 批量执行（执行单元 = 单条命令）同样遵循连接上下文与目标校验：
    /// 与单条路径共用前置校验，且不依赖本机是否有 Redis 服务。
    #[test]
    fn redis_workbench_commands_requires_connection_config_context() {
        let connector = RedisConnector::new();
        let err = connector
            .execute_command_workbench_commands(&workbench_redis_request())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Connection);
    }

    #[test]
    fn redis_workbench_commands_rejects_unsupported_target() {
        let connector = RedisConnector::with_config(redis_config());
        let mut request = workbench_redis_request();
        request.target = CommandExecutionTarget::MySql {
            connection_id: ConnectionId(3),
            database: None,
            schema: None,
        };
        let err = connector
            .execute_command_workbench_commands(&request)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
    }

    fn temp_sqlite_path(label: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "gdb-sqlite-{label}-{}-{suffix}.db",
            std::process::id()
        ))
    }
}
