#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::{
        CellUpdate, Endpoint, Error, ErrorKind, QueryExecutionOptions, QueryMode, RowIdentity,
    };
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
                insert_intents: None,
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
                insert_intents: None,
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
                insert_intents: None,
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
                    insert_intents: None,
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
            insert_intents: None,
            deletes: Vec::new(),
        });
        assert!(duplicate.is_err());

        // 集合类必须带元素
        let empty_list = connector.apply_changes(&DataChangeSet {
            object: object.clone(),
            inserts: vec![row(&format!("{prefix}:empty"), "list", "", "")],
            updates: Vec::new(),
            insert_intents: None,
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
            postgres_profile: None,
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
            postgres_profile: None,
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
            postgres_profile: None,
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

        // mock 元数据集（T081）当前提供 4 张关联表
        assert_eq!(objects.len(), 4);
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
                insert_intents: None,
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
                insert_intents: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
    fn split_sql_keeps_dollar_quoted_function_body_together() {
        // PostgreSQL 美元引用（$$ 与 $tag$）体内的分号不得被切分。
        let statements = split_sql_statements(
            "DROP TABLE IF EXISTS t CASCADE; CREATE FUNCTION f() RETURNS trigger AS $$ BEGIN RETURN NEW; END; $$ LANGUAGE plpgsql; CREATE TRIGGER t_i AFTER INSERT ON t FOR EACH ROW EXECUTE FUNCTION f(); SELECT 1; SELECT 2;",
        );
        assert_eq!(statements.len(), 5);
        assert_eq!(statements[1], "CREATE FUNCTION f() RETURNS trigger AS $$ BEGIN RETURN NEW; END; $$ LANGUAGE plpgsql");
        // 关键：分号切分不得破坏函数体（体内分号原样保留在单条语句里）。
        assert!(statements[1].contains("RETURN NEW; END; $$"));
        let statements = split_sql_statements("SELECT $1 FROM t WHERE x = $2; DO $$ BEGIN RAISE NOTICE 'x; y'; END $$;");
        assert_eq!(statements.len(), 2);
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
                    schema: None,
                    session_id: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
                schema: None,
                session_id: None,
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
            insert_intents: None,
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
            postgres_profile: None,
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
            postgres_profile: None,
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
            postgres_profile: None,
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

    // —— PostgreSQL（T04）——

    fn postgres_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(4),
            name: "PG Local".to_string(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 5432,
                database: Some("postgres".to_string()),
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(fluxdb_core::PostgresConnectionProfile {
                basic: fluxdb_core::PostgresBasicOptions {
                    host: "127.0.0.1".to_string(),
                    port: 5432,
                    maintenance_database: "postgres".to_string(),
                    username: "postgres".to_string(),
                    password: fluxdb_core::SecretRef::inline("secret"),
                },
                ..Default::default()
            }),
        }
    }

    fn pg_query_request(config: &ConnectionConfig, session_id: Option<QuerySessionId>) -> QueryRequest {
        QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: "SELECT 1".to_string(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id,
        }
    }

    #[test]
    fn pg_config_rejects_missing_profile() {
        let config = sqlite_config(); // postgres_profile = None
        let err = pg_config(&config, "postgres").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Connection);
    }

    #[test]
    fn pg_config_accepts_tls_after_t05() {
        // T05 起 TLS 由 `pg_connect` 按传输层解析，`pg_config` 只填认证/库/超时，不再拒绝。
        let mut config = postgres_config();
        let profile = config.postgres_profile.as_mut().unwrap();
        profile.tls.enabled = true;
        profile.tls.ssl_mode = fluxdb_core::PostgresSslMode::Require;
        let cfg = pg_config(&config, "postgres").unwrap();
        assert!(cfg.get_user().is_some());
    }

    #[test]
    fn pg_session_keys_isolate_by_session_id_and_role() {
        let config = postgres_config();
        // 两个不同 session_id → 不同会话键（连接/事务互不串扰）。
        let a = pg_session_key_for(&pg_query_request(&config, Some(QuerySessionId(1))), "postgres");
        let b = pg_session_key_for(&pg_query_request(&config, Some(QuerySessionId(2))), "postgres");
        assert_ne!(a, b);
        // 同 session_id → 相同会话键（复用同一连接，事务跨查询保持）。
        let a2 = pg_session_key_for(&pg_query_request(&config, Some(QuerySessionId(1))), "postgres");
        assert_eq!(a, a2);
        // 无 session_id → 隔离瞬态键，与显式会话键不同。
        let transient = pg_session_key_for(&pg_query_request(&config, None), "postgres");
        assert_ne!(transient, a);
        assert!(matches!(transient.purpose, PgSessionPurpose::Transient));
    }

    #[test]
    fn pg_execute_empty_query_is_guard_failure() {
        let connector = PostgresConnector::with_config(postgres_config());
        let mut request = pg_query_request(&postgres_config(), None);
        request.text = "   \n  ".to_string(); // 空语句（无服务器也应在切分阶段失败）
        let err = connector.execute(&request).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Query);
    }

    /// 真实 PG 冒烟（T04 验收）：需要外部 PostgreSQL。
    ///
    /// 通过 `FLUXDB_PG_SMOKE=host:port:user:password:db` 启用；未设置时直接跳过。
    /// 覆盖：真实建连 + 认证 + 版本读取；SELECT 返回真实行；两个显式查询会话互不串事务。
    #[test]
    fn pg_live_smoke_connect_and_version() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG 冒烟");
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::new();
        // 真实建连 + 认证 + version() 读取（test_connection 内部执行 SELECT version()）。
        assert!(
            connector.test_connection(&config).is_ok(),
            "真实 PG 建连/认证/版本读取失败"
        );
    }

    #[test]
    fn pg_live_smoke_select_rows() {
        let Some(params) = pg_smoke_params() else {
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());
        let mut request = pg_query_request(&config, None);
        request.text = "SELECT 1 AS one, 'x'::text AS t".to_string();
        let result = connector.execute(&request).expect("SELECT 应成功");
        let summary = &result.summaries[0];
        assert!(summary.success, "SELECT 应报 success：{}", summary.message);
        assert_eq!(summary.returned_rows, 1);
        assert_eq!(result.results.len(), 1);
        let page = &result.results[0];
        assert_eq!(page.rows.len(), 1);
    }

    #[test]
    fn pg_live_smoke_transient_sessions_do_not_leak_transactions() {
        let Some(params) = pg_smoke_params() else {
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());

        // 会话 A 开启事务写一行但未提交。
        let mut setup = pg_query_request(&config, Some(QuerySessionId(100)));
        setup.text = "DROP TABLE IF EXISTS t04_leak; CREATE TABLE t04_leak(id int); \
                      BEGIN; INSERT INTO t04_leak VALUES (1)".to_string();
        connector.execute(&setup).expect("A 建表并开启事务");

        // 会话 B（不同 session_id = 不同连接）：不应看到 A 未提交的行。
        let mut check = pg_query_request(&config, Some(QuerySessionId(200)));
        check.text = "SELECT count(*) AS c FROM t04_leak".to_string();
        let result = connector.execute(&check).expect("B 查询应成功");
        let page = &result.results[0];
        let first = &page.rows[0].values[0];
        assert_eq!(
            *first,
            CellValue::I64(0),
            "不同会话不应看到未提交事务的行（互不串事务）"
        );

        // 收尾：A 提交，确认 B 之后可见；并清理。
        let mut commit = pg_query_request(&config, Some(QuerySessionId(100)));
        commit.text = "COMMIT; DROP TABLE t04_leak".to_string();
        connector.execute(&commit).expect("A 提交并清理");
    }

    /// 读 FLUXDB_PG_SMOKE 环境变量 → (host, port, user, password, db)。
    fn pg_smoke_params() -> Option<(String, u16, String, String, String)> {
        let value = std::env::var("FLUXDB_PG_SMOKE").ok()?;
        let mut parts = value.split(':');
        let host = parts.next()?.to_string();
        let port: u16 = parts.next()?.parse().ok()?;
        let user = parts.next()?.to_string();
        let password = parts.next()?.to_string();
        let db = parts.next()?.to_string();
        Some((host, port, user, password, db))
    }

    fn pg_smoke_config((host, port, user, password, db): (String, u16, String, String, String)) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(9),
            name: "PG Smoke".to_string(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host: host.clone(),
                port,
                database: Some(db.clone()),
            },
            credential_ref: None,
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(fluxdb_core::PostgresConnectionProfile {
                basic: fluxdb_core::PostgresBasicOptions {
                    host,
                    port,
                    maintenance_database: db,
                    username: user,
                    password: fluxdb_core::SecretRef::inline(&password),
                },
                ..Default::default()
            }),
        }
    }

    // ===== T05 传输、安全策略与生命周期（真实冒烟，环境门控）=====

    /// 读 `FLUXDB_PG_SMOKE_TLS=host:port:user:password:db:ca_path:server_name:hostname`、
    ///   `FLUXDB_PG_SMOKE_TLS_BAD_CA`（错误 CA）与 `FLUXDB_PG_SMOKE_TLS_BAD_HOST`（错误主机名）各一个路径。
    /// 仅验证 TLS 握手方向，不依赖环境是否真的开启 TLS 之外的额外能力。
    /// 未配置时跳过。
    ///
    /// 覆盖 T05 验收：
    /// - verify-full 用正确 CA + 正确主机名 → 建连成功；
    /// - verify-full 用错误 CA（不受信）→ 拒绝；
    /// - verify-full 用正确 CA 但错误主机名 → 拒绝（校验 DNS/主机名）。
    #[test]
    fn pg_live_smoke_tls_verify_full() {
        let tls = env("FLUXDB_PG_SMOKE_TLS");
        let bad_ca = env("FLUXDB_PG_SMOKE_TLS_BAD_CA");
        let bad_host = env("FLUXDB_PG_SMOKE_TLS_BAD_HOST");
        let (params, ca_path, server_name, hostname) = match tls {
            Some(v) => split_tls_env(&v),
            None => return,
        };
        let connector = PostgresConnector::new();

        // 正确 CA + 正确主机名。
        let ok = tls_config(&params, ca_path.as_deref(), &server_name);
        assert!(
            connector.test_connection(&ok).is_ok(),
            "verify-full 正确 CA + 主机名应建连成功"
        );

        if let Some(path) = bad_ca {
            let bad = tls_config(&params, Some(&path), &server_name);
            assert!(
                connector.test_connection(&bad).is_err(),
                "verify-full 错误 CA 应被拒绝"
            );
        }

        if let Some(name) = bad_host {
            let bad = tls_config(&params, ca_path.as_deref(), &name);
            assert!(
                connector.test_connection(&bad).is_err(),
                "verify-full 主机名不匹配应被拒绝"
            );
        }
    }

    /// 组装 verify-full 配置（TLS 启用、VerifyFull、显式 server_name）。
    fn tls_config(
        params: &(String, u16, String, String, String),
        ca_path: Option<&str>,
        server_name: &str,
    ) -> ConnectionConfig {
        let mut config = pg_smoke_config(params.clone());
        if let Some(profile) = config.postgres_profile.as_mut() {
            profile.tls.enabled = true;
            profile.tls.ssl_mode = fluxdb_core::PostgresSslMode::VerifyFull;
            profile.tls.server_name = server_name.to_string();
            if let Some(path) = ca_path {
                profile.tls.ca = fluxdb_core::SecretRef::inline(path);
            }
        }
        config
    }

    /// 拆分 TLS 冒烟字段：`host:port:user:password:db|ca_path|server_name|hostname`。
    /// 返回 (pg 参数, ca_path, server_name, hostname)。
    fn split_tls_env(v: &str) -> ((String, u16, String, String, String), Option<String>, String, String) {
        let mut it = v.split('|');
        let params_raw = it.next().unwrap_or("");
        let host = params_raw.split(':').next().unwrap_or("").to_string();
        let port = params_raw
            .split(':')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let user = params_raw.split(':').nth(2).unwrap_or("").to_string();
        let password = params_raw.split(':').nth(3).unwrap_or("").to_string();
        let db = params_raw.split(':').nth(4).unwrap_or("").to_string();
        let ca_path = it.next().filter(|s| !s.is_empty()).map(|s| s.to_string());
        let server_name = it.next().unwrap_or(&host).to_string();
        let hostname = it.next().unwrap_or(&host).to_string();
        ((host, port, user, password, db), ca_path, server_name, hostname)
    }

    // ===== T06 对象树与真实路由（真实冒烟，环境门控）=====

    /// 真实 PG 对象浏览（T06 验收）：数据库 → schema → 表/视图 三层真实 pg_catalog 路由。
    ///
    /// 覆盖：列出数据库（含配置的库）；库下列 schema（过滤系统 schema）；
    /// schema 列表/视图（含同名对象由 schema 区分、物化视图/分区表归入 Table/View）。
    /// 临时创建 t06_table / t06_view 后列出并校验，最后清理。
    #[test]
    fn pg_live_smoke_object_tree() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG 对象树冒烟");
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());
        let db_name = config
            .postgres_profile
            .as_ref()
            .unwrap()
            .basic
            .maintenance_database
            .clone();

        // 1) 根层：数据库列表应包含配置的维护库。
        let databases = connector.list_objects(None).expect("列出数据库应成功");
        assert!(
            databases.iter().any(|o| o.path.name == db_name),
            "数据库列表应包含配置的维护库 {}",
            db_name
        );

        // 2) 数据库层：schema 列表应包含 public，且不包含 information_schema / pg_* 系统 schema。
        let db_path = databases
            .iter()
            .find(|o| o.path.name == db_name)
            .cloned()
            .expect("配置的库应存在")
            .path;
        let schemas = connector.list_objects(Some(&db_path)).expect("列出 schema 应成功");
        assert!(
            schemas.iter().any(|o| o.path.name == "public"),
            "public schema 应在列表内"
        );
        assert!(
            schemas
                .iter()
                .all(|o| o.path.name != "information_schema" && !o.path.name.starts_with("pg_")),
            "系统 schema 不应出现在对象树"
        );

        // 3) 建临时表/视图，在 public 下列出关系并断言 kind 正确。
        let mut setup = pg_query_request(&config, None);
        setup.text = "DROP VIEW IF EXISTS t06_view; DROP TABLE IF EXISTS t06_table; \
                      CREATE TABLE t06_table(id int); CREATE VIEW t06_view AS SELECT 1 AS one"
            .to_string();
        connector.execute(&setup).expect("建临时表/视图应成功");

        let public_path = schemas
            .iter()
            .find(|o| o.path.name == "public")
            .expect("public schema 应存在")
            .path
            .clone();
        let relations = connector
            .list_objects(Some(&public_path))
            .expect("列出关系应成功");
        assert!(
            relations.iter().any(|o| o.path.name == "t06_table"
                && o.path.kind == ObjectKind::Table
                && o.path.schema.as_deref() == Some("public")),
            "t06_table 应以 public 下的 Table 出现"
        );
        assert!(
            relations
                .iter()
                .any(|o| o.path.name == "t06_view" && o.path.kind == ObjectKind::View),
            "t06_view 应以 View 出现"
        );

        // 4) 清理临时对象。
        let mut cleanup = pg_query_request(&config, None);
        cleanup.text = "DROP VIEW t06_view; DROP TABLE t06_table".to_string();
        connector.execute(&cleanup).expect("清理临时对象应成功");
    }

    // ===== T07 建库/删库（单元 + 真实冒烟）=====

    #[test]
    fn pg_create_database_sql_builds_options_and_quotes() {
        let request = CreateDatabaseRequest {
            connection_id: ConnectionId(9),
            name: "app-db".to_string(),
            charset: "UTF8".to_string(),
            collation: "zh_CN.UTF-8".to_string(),
            path: None,
        };
        assert_eq!(
            pg_create_database_sql(&request).unwrap(),
            "CREATE DATABASE \"app-db\" ENCODING 'UTF8' LC_COLLATE 'zh_CN.UTF-8' LC_CTYPE 'zh_CN.UTF-8'"
        );

        // 空名称拒绝；含引号/分号的 locale 拒绝（防注入）。
        let bad_name = CreateDatabaseRequest { name: "  ".into(), ..request.clone() };
        assert!(pg_create_database_sql(&bad_name).is_err());
        let bad_col = CreateDatabaseRequest { collation: "zh_CN'; DROP SCHEMA public; --".into(), ..request };
        assert!(pg_create_database_sql(&bad_col).is_err());
    }

    #[test]
    fn pg_quote_identifier_escapes_double_quotes() {
        assert_eq!(pg_quote_identifier("plain"), "\"plain\"");
        assert_eq!(pg_quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    /// 真实建/删库（T07 验收）：建库（charset/collation）→ 对象树可见 → 删库；维护库保护。
    #[test]
    fn pg_live_smoke_create_delete_database() {
        let Some(params) = pg_smoke_params() else {
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());
        let db_name = config
            .postgres_profile
            .as_ref()
            .unwrap()
            .basic
            .maintenance_database
            .clone();

        // 建库：UTF8 编码（不强制 locale —— 测试容器模板库 collation 固定 en_US.utf8，
        // 传不匹配的 locale 会报 collation 不兼容；locale 映射已由单元测试覆盖）。
        let create = CreateDatabaseRequest {
            connection_id: config.id,
            name: "t07_db".to_string(),
            charset: "UTF8".to_string(),
            collation: String::new(),
            path: None,
        };
        connector.create_database(&create).expect("建库应成功");
        let databases = connector.list_objects(None).expect("列库应成功");
        assert!(
            databases.iter().any(|o| o.path.name == "t07_db"),
            "新建的 t07_db 应出现在对象树"
        );

        // 维护库保护：删除当前维护库应被拒绝。
        let guard = connector.delete_database(config.id, &db_name);
        assert!(guard.is_err(), "删除当前维护库应被拒绝");

        // 删库：对象树不再包含。
        connector.delete_database(config.id, "t07_db").expect("删库应成功");
        let after = connector.list_objects(None).expect("列库应成功");
        assert!(
            !after.iter().any(|o| o.path.name == "t07_db"),
            "删除后 t07_db 不应再出现"
        );
    }

    // ===== T08 结构元数据（单元 + 真实冒烟）=====

    /// 合成一个覆盖 列默认/identity/generated/PK/唯一/CHECK/FK/索引/注释 的完整结构，
    /// 用于校验 DDL 重建的片段与顺序（纯函数，不依赖服务器）。
    fn t08_structure() -> TableStructure {
        TableStructure {
            database: Some("db".to_string()),
            schema: Some("public".to_string()),
            name: "t08_master".to_string(),
            kind: ObjectKind::Table,
            columns: vec![
                ColumnMeta {
                    name: "id".into(),
                    ordinal: 1,
                    data_type: "integer".into(),
                    type_schema: Some("pg_catalog".into()),
                    type_name: Some("int4".into()),
                    nullable: false,
                    default_expr: None,
                    is_identity: true,
                    identity_generation: Some("a".into()),
                    is_generated: false,
                    is_editable: false,
                    primary_key: true,
                    unique_key: false,
                    comment: None,
                },
                ColumnMeta {
                    name: "name".into(),
                    ordinal: 2,
                    data_type: "character varying(50)".into(),
                    type_schema: Some("pg_catalog".into()),
                    type_name: Some("varchar".into()),
                    nullable: false,
                    default_expr: Some("'n/a'::character varying".into()),
                    is_identity: false,
                    identity_generation: None,
                    is_generated: false,
                    is_editable: true,
                    primary_key: false,
                    unique_key: true,
                    comment: Some("名称".into()),
                },
                ColumnMeta {
                    name: "total".into(),
                    ordinal: 3,
                    data_type: "numeric(10,2)".into(),
                    type_schema: Some("pg_catalog".into()),
                    type_name: Some("numeric".into()),
                    nullable: true,
                    default_expr: None,
                    is_identity: false,
                    identity_generation: None,
                    is_generated: false,
                    is_editable: true,
                    primary_key: false,
                    unique_key: false,
                    comment: None,
                },
                ColumnMeta {
                    name: "full_name".into(),
                    ordinal: 4,
                    data_type: "text".into(),
                    type_schema: Some("pg_catalog".into()),
                    type_name: Some("text".into()),
                    nullable: true,
                    default_expr: None,
                    is_identity: false,
                    identity_generation: None,
                    is_generated: true,
                    is_editable: false,
                    primary_key: false,
                    unique_key: false,
                    comment: None,
                },
            ],
            primary_key: vec!["id".into()],
            foreign_keys: vec![ForeignKeyMeta {
                name: "t08_master_parent_fk".into(),
                columns: vec!["parent_id".into()],
                ref_schema: Some("public".into()),
                ref_table: "t08_parent".into(),
                ref_columns: vec!["id".into()],
                on_delete: Some("CASCADE".into()),
                on_update: Some("SET NULL".into()),
                match_type: Some("s".into()),
                deferrable: true,
                initially_deferred: true,
                definition: "FOREIGN KEY (parent_id) REFERENCES public.t08_parent(id)".into(),
            }],
            checks: vec![CheckMeta {
                name: "t08_master_total_check".into(),
                expression: "(total >= 0)".into(),
                definition: "CHECK ((total >= 0))".into(),
            }],
            unique_keys: vec![UniqueKeyMeta {
                name: "t08_master_name_key".into(),
                columns: vec!["name".into()],
                is_constraint: true,
                definition: "UNIQUE (name)".into(),
            }],
            indexes: vec![
                IndexMeta {
                    name: "t08_master_lower_idx".into(),
                    columns: vec![IndexColumnItem {
                        column: None,
                        expression: Some("lower(name)".into()),
                        descending: false,
                        nulls_first: false,
                    }],
                    include_columns: vec!["total".into()],
                    is_unique: false,
                    is_primary: false,
                    index_type: Some("btree".into()),
                    predicate: Some("(total > 0)".into()),
                    valid: true,
                    definition:
                        "CREATE INDEX t08_master_lower_idx ON public.t08_master USING btree (lower(name)) INCLUDE (total) WHERE (total > 0)"
                            .into(),
                },
                IndexMeta {
                    name: "t08_master_pkey".into(),
                    columns: vec![IndexColumnItem {
                        column: Some("id".into()),
                        expression: None,
                        descending: false,
                        nulls_first: false,
                    }],
                    include_columns: vec![],
                    is_unique: true,
                    is_primary: true,
                    index_type: Some("btree".into()),
                    predicate: None,
                    valid: true,
                    definition: "CREATE UNIQUE INDEX t08_master_pkey ON public.t08_master USING btree (id)".into(),
                },
                IndexMeta {
                    name: "t08_master_name_key".into(),
                    columns: vec![IndexColumnItem {
                        column: Some("name".into()),
                        expression: None,
                        descending: false,
                        nulls_first: false,
                    }],
                    include_columns: vec![],
                    is_unique: true,
                    is_primary: false,
                    index_type: Some("btree".into()),
                    predicate: None,
                    valid: true,
                    definition: "CREATE UNIQUE INDEX t08_master_name_key ON public.t08_master USING btree (name)".into(),
                },
            ],
            triggers: vec![TriggerMeta {
                name: "t08_master_audit".into(),
                event: "INSERT OR UPDATE".into(),
                timing: "AFTER".into(),
                level: "ROW".into(),
                function: "public.audit_fn()".into(),
                enabled: true,
                definition: "CREATE TRIGGER t08_master_audit AFTER INSERT OR UPDATE ON public.t08_master FOR EACH ROW EXECUTE FUNCTION public.audit_fn()".into(),
            }],
            comment: Some("主表".into()),
        }
    }

    #[test]
    fn pg_build_table_ddl_round_trips_clauses() {
        let ddl = build_table_ddl(&t08_structure());

        // 列定义：identity、默认、NOT NULL、generated 都在位。
        assert!(ddl.contains("\"id\" integer GENERATED ALWAYS AS IDENTITY NOT NULL"));
        assert!(ddl.contains("\"name\" character varying(50) DEFAULT 'n/a'::character varying NOT NULL"));
        assert!(ddl.contains("\"full_name\" text GENERATED ALWAYS AS (expr) STORED"));
        // 主键 / 唯一约束。
        assert!(ddl.contains("PRIMARY KEY (\"id\")"));
        assert!(ddl.contains("CONSTRAINT \"t08_master_name_key\" UNIQUE (\"name\")"));
        // CHECK 表达式。
        assert!(ddl.contains("CONSTRAINT \"t08_master_total_check\" CHECK ((total >= 0))"));
        // 外键：动作/延迟属性。
        assert!(ddl.contains("CONSTRAINT \"t08_master_parent_fk\" FOREIGN KEY (\"parent_id\") REFERENCES \"public\".\"t08_parent\" (\"id\") ON DELETE CASCADE ON UPDATE SET NULL DEFERRABLE INITIALLY DEFERRED"));
        // 独立表达式索引保留（含 INCLUDE/predicate）；主键与唯一约束背衬索引不应重复出现。
        assert!(ddl.contains("CREATE INDEX t08_master_lower_idx ON public.t08_master USING btree (lower(name)) INCLUDE (total) WHERE (total > 0);"));
        assert!(!ddl.contains("CREATE UNIQUE INDEX t08_master_pkey"));
        assert!(!ddl.contains("CREATE UNIQUE INDEX t08_master_name_key"));
        // 注释。
        assert!(ddl.contains("COMMENT ON TABLE \"public\".\"t08_master\" IS '主表';"));
        assert!(ddl.contains("COMMENT ON COLUMN \"public\".\"t08_master\".\"name\" IS '名称';"));
    }

    #[test]
    fn pg_split_index_keys_respects_nesting_and_quotes() {
        let def = "CREATE INDEX i ON s.t USING btree (lower(name), \"weird col\" DESC NULLS LAST) INCLUDE (x) WHERE (a > 0)";
        let keys = pg_split_index_keys(def);
        assert_eq!(keys, vec!["lower(name)", "\"weird col\" DESC NULLS LAST"]);
        // 表达式键项解析（attnum==0 信号驱动）。
        let expr_item = pg_parse_index_item(&keys[0], true);
        assert!(expr_item.column.is_none());
        assert_eq!(expr_item.expression.as_deref(), Some("lower(name)"));
        // 列 + DESC + NULLS LAST 解析（命名列）。
        let col_item = pg_parse_index_item(&keys[1], false);
        assert_eq!(col_item.column.as_deref(), Some("\"weird col\""));
        assert!(col_item.descending);
        assert!(!col_item.nulls_first);
    }

    #[test]
    fn pg_trigger_bits_decode_event_and_timing() {
        // AFTER INSERT OR UPDATE（ROW）: ROW=1, AFTER 无位, INSERT=4, UPDATE=16 → 1|4|16=21。
        assert_eq!(pg_trigger_event(21), "INSERT OR UPDATE");
        assert_eq!(pg_trigger_timing(21), "AFTER");
        // BEFORE DELETE（ROW）: 1|2|8=11。
        assert_eq!(pg_trigger_event(11), "DELETE");
        assert_eq!(pg_trigger_timing(11), "BEFORE");
        // INSTEAD OF（STATEMENT? 实际行级）: INSTEAD=64|INSERT=4 = 68。
        assert_eq!(pg_trigger_timing(68), "INSTEAD OF");
    }

    #[test]
    fn pg_indexes_from_structure_keeps_expression_item() {
        let structure = t08_structure();
        let indexes = pg_indexes_from_structure(&structure);
        let expr_index = indexes
            .iter()
            .find(|i| i.name == "t08_master_lower_idx")
            .expect("表达式索引应在索引列表");
        assert_eq!(expr_index.columns, vec!["lower(name)"]);
        assert_eq!(expr_index.index_type.as_deref(), Some("btree"));
        assert!(!expr_index.is_unique);
        assert!(!expr_index.is_primary);
    }

    #[test]
    fn pg_foreign_keys_from_structure_joins_composite_columns() {
        let mut structure = t08_structure();
        structure.foreign_keys = vec![ForeignKeyMeta {
            name: "fk_multi".into(),
            columns: vec!["a".into(), "b".into()],
            ref_schema: Some("public".into()),
            ref_table: "t".into(),
            ref_columns: vec!["x".into(), "y".into()],
            on_delete: None,
            on_update: None,
            match_type: Some("s".into()),
            deferrable: false,
            initially_deferred: false,
            definition: String::new(),
        }];
        let fks = pg_foreign_keys_from_structure(&structure);
        assert_eq!(fks.len(), 1);
        // 复合外键按序位折叠展示，不丢序、不出现笛卡尔积排列。
        assert_eq!(fks[0].column, "a, b");
        assert_eq!(fks[0].ref_column, "x, y");
        assert_eq!(fks[0].ref_table, "t");
    }

    #[test]
    fn pg_triggers_from_structure_maps_display() {
        let structure = t08_structure();
        let triggers = pg_triggers_from_structure(&structure);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].name, "t08_master_audit");
        assert_eq!(triggers[0].event, "AFTER INSERT OR UPDATE");
        assert_eq!(triggers[0].timing, "ROW");
        assert!(triggers[0].body.as_deref().unwrap().starts_with("CREATE TRIGGER"));
    }

    /// T08 真实冒烟：在维护库 public 下建一张含 列/PK/唯一/FK/CHECK/表达式索引/触发器/注释
    /// 的表，逐 tab 校验元数据投影与 DDL 重建，随后清理。
    #[test]
    fn pg_live_smoke_table_info() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG 结构元数据冒烟");
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());

        let mut setup = pg_query_request(&config, None);
        setup.text = "\
            DROP TRIGGER IF EXISTS t08_audit ON t08_parent CASCADE; \
            DROP TABLE IF EXISTS t08_child CASCADE; \
            DROP TABLE IF EXISTS t08_master CASCADE; \
            DROP TABLE IF EXISTS t08_parent CASCADE; \
            CREATE TABLE t08_parent(id integer PRIMARY KEY); \
            CREATE TABLE t08_child( \
                id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, \
                name varchar(50) NOT NULL DEFAULT 'x'::varchar, \
                total numeric(10,2) CHECK (total >= 0), \
                parent_id integer \
            ); \
            ALTER TABLE t08_child ADD CONSTRAINT t08_child_parent_fk \
                FOREIGN KEY (parent_id) REFERENCES t08_parent(id) ON DELETE CASCADE; \
            ALTER TABLE t08_child ADD CONSTRAINT t08_child_name_key UNIQUE (name); \
            CREATE INDEX t08_child_lower_idx ON t08_child (lower(name)); \
            COMMENT ON TABLE t08_child IS '子表'; \
            COMMENT ON COLUMN t08_child.name IS '名称'; \
            CREATE OR REPLACE FUNCTION t08_audit_fn() RETURNS trigger AS $$ BEGIN RETURN NEW; END; $$ LANGUAGE plpgsql; \
            CREATE TRIGGER t08_audit AFTER INSERT OR UPDATE ON t08_child \
                FOR EACH ROW EXECUTE FUNCTION t08_audit_fn(); \
        "
        .to_string();
        connector.execute(&setup).expect("建临时结构应成功");

        let child_path = ObjectPath {
            connection_id: config.id,
            database: config
                .postgres_profile
                .as_ref()
                .unwrap()
                .basic
                .maintenance_database
                .clone()
                .into(),
            schema: Some("public".to_string()),
            name: "t08_child".to_string(),
            kind: ObjectKind::Table,
        };

        // 索引：含主键/唯一约束背衬/表达式索引；表达式键项、INCLUDE/predicate 不丢。
        let indexes = connector.list_indexes(&child_path).expect("列索引应成功");
        let pkey = indexes.iter().find(|i| i.name == "t08_child_pkey").expect("主键索引存在");
        assert!(pkey.is_primary && pkey.is_unique);
        let lower = indexes
            .iter()
            .find(|i| i.name == "t08_child_lower_idx")
            .expect("表达式索引存在");
        assert!(
            lower.columns.iter().any(|c| c.contains("lower")),
            "表达式键项应保留，实际 {:?}",
            lower.columns
        );
        let name_key = indexes
            .iter()
            .find(|i| i.name == "t08_child_name_key")
            .expect("唯一约束背衬索引存在");
        assert!(name_key.is_unique);

        // 外键：本表列→被引用表 正确投影（无笛卡尔积）。
        let fks = connector.list_foreign_keys(&child_path).expect("列外键应成功");
        assert!(
            fks.iter().any(|f| f.name == "t08_child_parent_fk"
                && f.column == "parent_id"
                && f.ref_schema.as_deref() == Some("public")
                && f.ref_table == "t08_parent"
                && f.ref_column == "id"),
            "外键投影应与建表一致，实际 {:?}",
            fks
        );

        // 触发器：用户触发器可见，内部约束触发器被过滤。
        let triggers = connector.list_triggers(&child_path).expect("列触发器应成功");
        assert!(
            triggers.iter().any(|t| t.name == "t08_audit" && t.timing == "ROW"),
            "用户触发器应可见，实际 {:?}",
            triggers
        );

        // DDL：重建包含 列/约束/索引/注释。
        let ddl = connector.table_ddl(&child_path).expect("表 DDL 应成功");
        assert!(ddl.contains("\"id\" integer GENERATED BY DEFAULT AS IDENTITY"));
        // pg_get_expr 会把 varchar 规整为 character varying，DDL 重建以此为准。
        assert!(ddl.contains("DEFAULT 'x'::character varying"));
        assert!(ddl.contains("PRIMARY KEY (\"id\")"));
        assert!(ddl.contains("CONSTRAINT \"t08_child_parent_fk\""));
        assert!(ddl.contains("ON DELETE CASCADE"));
        assert!(ddl.contains("CREATE INDEX t08_child_lower_idx"));
        assert!(ddl.contains("COMMENT ON TABLE \"public\".\"t08_child\" IS '子表';"));

        // 视图 DDL 走 pg_get_viewdef。
        let mut view_setup = pg_query_request(&config, None);
        view_setup.text = "CREATE OR REPLACE VIEW t08_view AS SELECT id, name FROM t08_child".to_string();
        connector.execute(&view_setup).expect("建视图应成功");
        let view_path = ObjectPath {
            connection_id: config.id,
            database: child_path.database.clone(),
            schema: Some("public".to_string()),
            name: "t08_view".to_string(),
            kind: ObjectKind::View,
        };
        let view_ddl = connector.table_ddl(&view_path).expect("视图 DDL 应成功");
        assert!(
            view_ddl.contains("CREATE OR REPLACE VIEW") && view_ddl.contains("t08_view"),
            "视图 DDL 应由 viewdef 重建，实际 {view_ddl}"
        );
        connector
            .execute(&{
                let mut c = pg_query_request(&config, None);
                c.text = "DROP VIEW t08_view".to_string();
                c
            })
            .expect("清理视图应成功");

        // 清理。
        let mut cleanup = pg_query_request(&config, None);
        cleanup.text = "DROP TRIGGER IF EXISTS t08_audit ON t08_child; \
                        DROP FUNCTION IF EXISTS t08_audit_fn(); \
                        DROP TABLE IF EXISTS t08_child CASCADE; \
                        DROP TABLE IF EXISTS t08_parent CASCADE"
            .to_string();
        connector.execute(&cleanup).expect("清理临时结构应成功");
    }

    fn env(key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }

    // ---- T09 值转换、参数编码与 bytea ----

    #[test]
    fn pg_type_base_strips_modifiers_and_array_suffix() {
        assert_eq!(pg_type_base("int4"), "int4");
        assert_eq!(pg_type_base("varchar"), "varchar");
        assert_eq!(pg_type_base("varchar(50)"), "varchar");
        assert_eq!(pg_type_base("numeric(10,2)"), "numeric");
        assert_eq!(pg_type_base("integer[]"), "integer");
        assert_eq!(pg_type_base("text[]"), "text");
        assert_eq!(pg_type_base(" double precision "), "double precision");
    }

    /// 逐值生成 `$n` 占位并分别绑定各自的值（回归：曾误用 `values[0].1` 让所有占位绑定同一值）。
    #[test]
    fn pg_insert_sql_binds_each_column_value() {
        let mkcol = |name: &str, ty: &str| Column {
            name: name.to_string(),
            type_name: Some(ty.to_string()),
            nullable: true,
            primary_key: false,
            comment: None,
        };
        let c_id = mkcol("id", "int4");
        let c_name = mkcol("name", "text");
        let c_note = mkcol("note", "text");
        let v_one = CellValue::I64(1);
        let v_alice = CellValue::Text("alice".to_string());
        let v_bob = CellValue::Text("bob".to_string());
        let values = vec![(&c_id, &v_one), (&c_name, &v_alice), (&c_note, &v_bob)];
        let (sql, _params) = pg_insert_sql("\"public\".\"t\"", &values).unwrap();
        assert!(
            sql.contains("(\"id\", \"name\", \"note\")") && sql.contains("VALUES ($1, $2, $3)"),
            "插入 SQL 应带三列三占位，实际 {sql}"
        );
    }

    #[test]
    fn pg_insert_sql_empty_values_falls_back_to_default() {
        let (sql, params) = pg_insert_sql("\"public\".\"t\"", &[]).unwrap();
        assert_eq!(sql, "INSERT INTO \"public\".\"t\" DEFAULT VALUES");
        assert!(params.is_empty());
    }

    /// 三态意图下 `pg_insert_values`：Default 省略、Null 显式 NULL、Value 写值、生成列剔除。
    #[test]
    fn pg_insert_values_respects_three_state_intents() {
        let mkcol = |name: &str, ty: &str| Column {
            name: name.to_string(),
            type_name: Some(ty.to_string()),
            nullable: true,
            primary_key: false,
            comment: None,
        };
        let c_id = mkcol("id", "int4");
        let c_name = mkcol("name", "text");
        let c_gen = mkcol("gen", "int4"); // 模拟服务端生成列
        let columns = vec![c_id.clone(), c_name.clone(), c_gen.clone()];
        let row = Row { values: vec![CellValue::Null, CellValue::Null, CellValue::Null] };

        // Default / Value / 生成列 → 只写 name；Null 列省略由数据库默认值填充。
        let intents = vec![
            WriteValue::Default,
            WriteValue::Value(CellValue::Text("alice".to_string())),
            WriteValue::Null,
        ];
        let generated: std::collections::BTreeSet<String> =
            ["gen".to_string()].into_iter().collect();

        let built = pg_insert_values(&row, &columns, &generated, Some(&intents)).unwrap();
        assert_eq!(built.len(), 1, "仅未生成列且非 Default 的列被写入，实际 {built:?}");
        assert_eq!(built[0].0.name, "name");
        assert_eq!(built[0].1, &CellValue::Text("alice".to_string()));

        // 显式 Null 意图 → 写入 NULL（区别于 Default 省略）。
        let intents_null = vec![
            WriteValue::Null,
            WriteValue::Default,
            WriteValue::Null,
        ];
        let built_null = pg_insert_values(&row, &columns, &generated, Some(&intents_null)).unwrap();
        assert_eq!(built_null.len(), 1);
        assert_eq!(built_null[0].0.name, "id");
        assert_eq!(built_null[0].1, &CellValue::Null);
    }

    #[test]
    fn pg_identity_where_maps_null_value_to_is_null() {
        let id_val = CellValue::I64(7);
        let null_val = CellValue::Null;
        let identity = RowIdentity {
            values: [("id".to_string(), id_val), ("deleted_at".to_string(), null_val)].into(),
        };
        let c_id = Column {
            name: "id".to_string(),
            type_name: Some("int4".to_string()),
            nullable: false,
            primary_key: true,
            comment: None,
        };
        let c_del = Column {
            name: "deleted_at".to_string(),
            type_name: Some("timestamptz".to_string()),
            nullable: true,
            primary_key: false,
            comment: None,
        };
        let columns = vec![c_id, c_del];
        let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
        let where_sql = pg_identity_where(&mut params, &identity, &columns).unwrap();
        // BTreeMap 按键升序迭代：deleted_at 在 id 之前。
        assert_eq!(where_sql, " WHERE \"deleted_at\" IS NULL AND \"id\" = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn pg_identity_where_rejects_empty_identity() {
        let identity = RowIdentity { values: Default::default() };
        assert!(pg_identity_where(&mut Vec::new(), &identity, &[]).is_err());
    }

    #[test]
    fn pg_next_param_binds_null_as_option_none() {
        let mut params: Vec<Box<dyn ToSql + Sync>> = Vec::new();
        let p1 = pg_next_param(&mut params, &CellValue::Null, None);
        let p2 = pg_next_param(&mut params, &CellValue::Text("x".to_string()), None);
        assert_eq!(p1, "$1");
        assert_eq!(p2, "$2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn pg_order_by_clause_appends_primary_key_tiebreaker() {
        let mkcol = |name: &str, pk: bool| Column {
            name: name.to_string(),
            type_name: Some("int4".to_string()),
            nullable: false,
            primary_key: pk,
            comment: None,
        };
        let columns = vec![
            mkcol("id", true),
            mkcol("score", false),
            mkcol("name", false),
        ];
        // 无用户排序：追加主键 ASC 作为稳定 tie breaker。
        assert_eq!(
            pg_order_by_clause(&[], &columns),
            " ORDER BY \"id\"",
            "无用户排序时应只按主键稳定排序"
        );
        // 用户排序与主键不同列：主键追加在末尾，逗号分隔。
        let sort = [SortSpec { field: "score".to_string(), direction: SortDirection::Desc }];
        assert_eq!(
            pg_order_by_clause(&sort, &columns),
            " ORDER BY \"score\" DESC, \"id\"",
            "同值行应按主键稳定排序"
        );
        // 用户已按主键排：不重复追加。
        let sort = [SortSpec { field: "id".to_string(), direction: SortDirection::Asc }];
        assert_eq!(
            pg_order_by_clause(&sort, &columns),
            " ORDER BY \"id\" ASC"
        );
        // 无主键表：保持共享排序原样，不额外 ORDER。
        let no_pk = vec![mkcol("score", false)];
        assert_eq!(pg_order_by_clause(&[], &no_pk), "");
    }

    #[test]
    fn pg_where_params_rejects_unknown_column_and_missing_value() {
        let mkcol = |name: &str, ty: &str| Column {
            name: name.to_string(),
            type_name: Some(ty.to_string()),
            nullable: true,
            primary_key: false,
            comment: None,
        };
        let columns = vec![mkcol("id", "int4"), mkcol("name", "text")];

        // 过滤引用不存在的列：显式报错，不静默忽略。
        let unknown = vec![FilterSpec {
            field: "nope".to_string(),
            op: FilterOp::Eq,
            values: vec![CellValue::Text("x".to_string())],
            enabled: true,
        }];
        let err = pg_where_params(&unknown, &columns).unwrap_err();
        assert!(err.to_string().contains("nope"), "应指出缺失列名: {err}");

        // 空 IN：缺值操作必须显式报错。
        let empty_in = vec![FilterSpec {
            field: "id".to_string(),
            op: FilterOp::InList,
            values: vec![],
            enabled: true,
        }];
        assert!(
            pg_where_params(&empty_in, &columns).is_err(),
            "空 IN 应报错而非丢弃"
        );
    }

    #[test]
    fn pg_where_params_translates_typed_and_pattern_filters() {
        let mkcol = |name: &str, ty: &str| Column {
            name: name.to_string(),
            type_name: Some(ty.to_string()),
            nullable: true,
            primary_key: false,
            comment: None,
        };
        let columns = vec![
            mkcol("id", "int4"),
            mkcol("price", "numeric(8,2)"),
            mkcol("name", "text"),
        ];
        // 文本值写非字符串列 → 双重转换占位（对齐 T09 绑定策略）。
        let between = vec![FilterSpec {
            field: "price".to_string(),
            op: FilterOp::Between,
            values: vec![
                CellValue::Text("1.00".to_string()),
                CellValue::Text("9.99".to_string()),
            ],
            enabled: true,
        }];
        let (where_sql, params) = pg_where_params(&between, &columns).unwrap();
        assert!(
            where_sql.contains("CAST(CAST($1 AS text) AS numeric(8,2))")
                && where_sql.contains("CAST(CAST($2 AS text) AS numeric(8,2))"),
            "numeric 列应有双重转换占位: {where_sql}"
        );
        assert_eq!(params.len(), 2);

        // Contains → LIKE `%值%` 参数化。
        let contains = vec![FilterSpec {
            field: "name".to_string(),
            op: FilterOp::Contains,
            values: vec![CellValue::Text("alice".to_string())],
            enabled: true,
        }];
        let (where_sql, params) = pg_where_params(&contains, &columns).unwrap();
        assert!(
            where_sql.contains("\"name\" LIKE $1"),
            "Contains 应翻译为 LIKE: {where_sql}"
        );
        assert_eq!(params.len(), 1);
    }

    /// T09 冒烟：建含类型化字段与 bytea 的表 → 类型化读取 + 二进制投影 →
    /// 编辑提交(插/改/删) + 单格完整二进制读取。真实 PG 才跑。
    #[test]
    fn pg_live_smoke_typed_read_binary_and_apply_changes() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG T09 冒烟");
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());

        let mut setup = pg_query_request(&config, None);
        setup.text = "\
            DROP TABLE IF EXISTS t09_typed CASCADE; \
            CREATE TABLE t09_typed( \
                id integer PRIMARY KEY, \
                price numeric(8,2), \
                ratio double precision, \
                tags text[], \
                meta jsonb, \
                payload bytea, \
                created_at timestamptz \
            ); \
            INSERT INTO t09_typed \
                (id, price, ratio, tags, meta, payload, created_at) VALUES \
                (1, 12.50, 0.25, ARRAY['a','b'], '{\"k\":1}', \
                 decode('deadbeef','hex'), '2024-01-02 03:04:05+00'); \
        "
        .to_string();
        connector.execute(&setup).expect("建临时结构应成功");

        let path = ObjectPath {
            connection_id: config.id,
            database: config
                .postgres_profile
                .as_ref()
                .unwrap()
                .basic
                .maintenance_database
                .clone()
                .into(),
            schema: Some("public".to_string()),
            name: "t09_typed".to_string(),
            kind: ObjectKind::Table,
        };

        // 有条件的完整读取：类型化值应解码为对应 CellValue（numeric 文本、jsonb Json、bytea 摘要）。
        let page = connector.load_data(&path, 0, 50, &[], &[]).expect("读取数据应成功");
        assert_eq!(page.rows.len(), 1, "应读到 1 行");
        let row = &page.rows[0];
        let index_of = |name: &str| page.columns.iter().position(|c| c.name == name).unwrap();
        let i_price = index_of("price");
        let i_ratio = index_of("ratio");
        let i_tags = index_of("tags");
        let i_meta = index_of("meta");
        let i_payload = index_of("payload");
        assert_eq!(row.values[i_price], CellValue::Text("12.50".to_string()));
        assert_eq!(row.values[i_ratio], CellValue::F64(0.25));
        assert_eq!(row.values[i_tags], CellValue::Text("{a,b}".to_string()));
        assert_eq!(row.values[i_meta], CellValue::Json("{\"k\": 1}".to_string()));
        // bytea 走摘要投影：非空、长度 4（deadbeef）。
        match &row.values[i_payload] {
            CellValue::BinarySummary(summary) => {
                assert!(!summary.is_null, "payload 不应为 NULL");
                assert_eq!(summary.byte_length, 4);
                assert_eq!(summary.preview_hex, Some("deadbeef".to_string()));
            }
            other => panic!("bytea 应以摘要投影，实际 {other:?}"),
        }

        // 单格完整二进制读取：能得到原始 4 字节。
        let identity = RowIdentity {
            values: [("id".to_string(), CellValue::I64(1))].into(),
        };
        let bytes = connector
            .load_cell_binary(&path, &identity, "payload")
            .expect("完整二进制读取应成功");
        assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);

        // 编辑提交：更新价格、改 bytea、插入新行、删除该行 —— 单事务可回滚语义由连接器保证。
        let mut changes = DataChangeSet {
            object: path.clone(),
            inserts: vec![Row {
                values: vec![
                    CellValue::I64(2),
                    CellValue::Text("9.99".to_string()),
                    CellValue::F64(-1.0),
                    CellValue::Text("{}".to_string()),
                    CellValue::Json("null".to_string()),
                    CellValue::Bytes(vec![0x01, 0x02]),
                    CellValue::Null, // created_at
                ],
            }],
            updates: vec![RowUpdate {
                identity: identity.clone(),
                cells: vec![
                    CellUpdate {
                        column: "price".to_string(),
                        value: CellValue::Text("20.00".to_string()),
                    },
                    CellUpdate {
                        column: "payload".to_string(),
                        value: CellValue::Bytes(vec![0xca, 0xfe]),
                    },
                ],
            }],
            deletes: vec![],
            insert_intents: None,
        };
        connector.apply_changes(&changes).expect("插入+更新应成功");

        // 校验更新结果。
        let page2 = connector.load_data(&path, 0, 50, &[], &[]).expect("重新读取应成功");
        let rows = page2.rows.iter().any(|r| {
            matches!(r.values[index_of("price")], CellValue::Text(ref p) if p == "20.00")
        });
        assert!(rows, "更新后的价格应生效");
        let new_bytes = connector
            .load_cell_binary(&path, &identity, "payload")
            .expect("更新后二进制读取应成功");
        assert_eq!(new_bytes, vec![0xca, 0xfe]);

        // 删除新插入的行：仅提交删除，清空其后的插入/更新，避免重复执行。
        changes.inserts.clear();
        changes.updates.clear();
        changes.deletes.push(RowIdentity {
            values: [("id".to_string(), CellValue::I64(2))].into(),
        });
        connector.apply_changes(&changes).expect("删除应成功");
        let page3 = connector.load_data(&path, 0, 50, &[], &[]).expect("删除后读取应成功");
        assert!(
            !page3.rows.iter().any(|r| r.values[index_of("id")] == CellValue::I64(2)),
            "删除后的行不应存在"
        );

        // 清理。
        let mut cleanup = pg_query_request(&config, None);
        cleanup.text = "DROP TABLE IF EXISTS t09_typed CASCADE".to_string();
        connector.execute(&cleanup).expect("清理临时结构应成功");
    }

    /// T10 冒烟：分页 has_more、主键稳定 tie breaker、过滤（Between/Contains/等值）在真实 PG 生效。
    #[test]
    fn pg_live_smoke_pagination_sort_and_filter() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG T10 冒烟");
            return;
        };
        let db_name = params.4.clone();
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());
        let path = ObjectPath {
            connection_id: ConnectionId(1),
            kind: ObjectKind::Table,
            database: Some(db_name),
            schema: Some("public".to_string()),
            name: "t10_page".to_string(),
        };

        let mut setup = pg_query_request(&config, None);
        setup.text = "\
            DROP TABLE IF EXISTS t10_page CASCADE; \
            CREATE TABLE t10_page( \
                id integer PRIMARY KEY, \
                score integer, \
                name text \
            ); \
            INSERT INTO t10_page (id, score, name) VALUES \
                (1, 10, 'alice'), (2, 10, 'bob'), (3, 30, 'carol'), (4, 40, 'dave'), (5, 50, 'erin'); \
        "
        .to_string();
        connector.execute(&setup).expect("建表+数据应成功");

        // 分页：limit=2，应 has_more 且只回两行。
        let page1 = connector
            .load_data(&path, 0, 2, &[], &[])
            .expect("第一页读取应成功");
        assert!(page1.has_more, "limit 小于总数应 has_more");
        assert_eq!(page1.rows.len(), 2);

        // 稳定排序：按 score DESC，同级(10) 由主键 id ASC 作 tie breaker → id 1 先于 id 2。
        let sort = [SortSpec { field: "score".to_string(), direction: SortDirection::Desc }];
        let sorted = connector
            .load_data(&path, 0, 50, &sort, &[])
            .expect("排序读取应成功");
        let ids: Vec<i64> = sorted
            .rows
            .iter()
            .map(|r| match r.values[0] {
                CellValue::I64(v) => v,
                _ => panic!("id 应为整数"),
            })
            .collect();
        assert_eq!(ids, vec![5, 4, 3, 1, 2], "降序 + 主键 tie breaker 应稳定");

        // 过滤：score BETWEEN 10 AND 40 且 name LIKE '%a%' → id 1,3。
        let filters = vec![
            FilterSpec {
                field: "score".to_string(),
                op: FilterOp::Between,
                values: vec![CellValue::I64(10), CellValue::I64(40)],
                enabled: true,
            },
            FilterSpec {
                field: "name".to_string(),
                op: FilterOp::Contains,
                values: vec![CellValue::Text("a".to_string())],
                enabled: true,
            },
        ];
        let filtered = connector
            .load_data(&path, 0, 50, &[], &filters)
            .expect("过滤读取应成功");
        let fids: Vec<i64> = filtered
            .rows
            .iter()
            .map(|r| match r.values[0] {
                CellValue::I64(v) => v,
                _ => panic!("id 应为整数"),
            })
            .collect();
        // score∈[10,40]: id 1/2/3/4；name 含 'a': alice/carol/dave → 交集 1/3/4。
        assert_eq!(fids, vec![1, 3, 4], "BETWEEN + LIKE 过滤应命中 id 1、3、4");
        assert!(!filtered.has_more);

        // 非法过滤（引用不存在的列）→ 明确报错，不静默忽略。
        let bad = vec![FilterSpec {
            field: "missing_col".to_string(),
            op: FilterOp::Eq,
            values: vec![CellValue::I64(1)],
            enabled: true,
        }];
        assert!(
            connector.load_data(&path, 0, 50, &[], &bad).is_err(),
            "引用不存在的过滤列应报错"
        );

        // 清理。
        let mut cleanup = pg_query_request(&config, None);
        cleanup.text = "DROP TABLE IF EXISTS t10_page CASCADE".to_string();
        connector.execute(&cleanup).expect("清理临时结构应成功");
    }

    /// T11 冒烟：生成列保护 + 行数检查整批回滚（真实 PG）。
    #[test]
    fn pg_live_smoke_generated_column_protection_and_rowcount_rollback() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG T11 冒烟");
            return;
        };
        let db_name = params.4.clone();
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());
        let path = ObjectPath {
            connection_id: ConnectionId(1),
            kind: ObjectKind::Table,
            database: Some(db_name.clone()),
            schema: Some("public".to_string()),
            name: "t11_edit".to_string(),
        };

        let mut setup = pg_query_request(&config, None);
        setup.text = "\
            DROP TABLE IF EXISTS t11_edit CASCADE; \
            CREATE TABLE t11_edit( \
                id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, \
                price integer, \
                doubled integer GENERATED ALWAYS AS (price * 2) STORED \
            ); \
        "
        .to_string();
        connector.execute(&setup).expect("建表应成功");

        // 1) 插入只写非生成列 price；id/doubled 由数据库生成，不触碰生成列。
        let insert = DataChangeSet {
            object: path.clone(),
            inserts: vec![Row {
                values: vec![CellValue::I64(1), CellValue::I64(5), CellValue::I64(0)],
            }],
            updates: vec![],
            deletes: vec![],
            insert_intents: None,
        };
        connector.apply_changes(&insert).expect("插入应成功且自动生成 id/doubled");
        let page = connector.load_data(&path, 0, 50, &[], &[]).expect("读取应成功");
        assert_eq!(page.rows.len(), 1, "应插入 1 行");
        // doubled = price * 2 = 10（服务端生成列生效）。
        assert_eq!(page.rows[0].values[2], CellValue::I64(10), "生成列应由服务端计算");

        // 2) 更新生成列 → 拒绝，明确报错。
        let bad_update = DataChangeSet {
            object: path.clone(),
            inserts: vec![],
            updates: vec![RowUpdate {
                identity: RowIdentity {
                    values: [("id".to_string(), CellValue::I64(1))].into(),
                },
                cells: vec![CellUpdate {
                    column: "doubled".to_string(),
                    value: CellValue::I64(99),
                }],
            }],
            deletes: vec![],
            insert_intents: None,
        };
        let err = connector.apply_changes(&bad_update).expect_err("更新生成列应报错");
        assert!(err.to_string().contains("生成列"), "应提示生成列不可改: {err}");

        // 3) 批内一条更新命中不存在行 → 行数检查回滚整批，插入也不生效。
        let rollback = DataChangeSet {
            object: path.clone(),
            inserts: vec![Row {
                values: vec![CellValue::I64(2), CellValue::I64(7), CellValue::I64(0)],
            }],
            updates: vec![RowUpdate {
                identity: RowIdentity {
                    values: [("id".to_string(), CellValue::I64(999))].into(),
                },
                cells: vec![CellUpdate {
                    column: "price".to_string(),
                    value: CellValue::I64(1),
                }],
            }],
            deletes: vec![],
            insert_intents: None,
        };
        assert!(
            connector.apply_changes(&rollback).is_err(),
            "命中不存在行的更新应使整批回滚"
        );
        let page = connector.load_data(&path, 0, 50, &[], &[]).expect("回滚后读取应成功");
        assert_eq!(page.rows.len(), 1, "回滚后新增行不应存在");

        // 4) 三态写入意图：Default 不写列（DB 默认值填充）、Null 显式写 NULL、Value 写具体值。
        let mut setup3 = pg_query_request(&config, None);
        setup3.text = "\
            DROP TABLE IF EXISTS t11_tristate CASCADE; \
            CREATE TABLE t11_tristate( \
                id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, \
                note text DEFAULT 'n/a' \
            ); \
        "
        .to_string();
        connector.execute(&setup3).expect("建三态表应成功");
        let path3 = ObjectPath {
            connection_id: ConnectionId(1),
            kind: ObjectKind::Table,
            database: Some(db_name.clone()),
            schema: Some("public".to_string()),
            name: "t11_tristate".to_string(),
        };
        // 意图按表列序：id(Default 由 DB 生成), note(Default → 'n/a')。
        let tri = DataChangeSet {
            object: path3.clone(),
            inserts: vec![
                Row { values: vec![CellValue::Null, CellValue::Null] },
                Row { values: vec![CellValue::Null, CellValue::Null] },
                Row { values: vec![CellValue::Null, CellValue::Null] },
            ],
            updates: vec![],
            deletes: vec![],
            insert_intents: Some(vec![
                vec![
                    WriteValue::Default,                    // id：不写
                    WriteValue::Default,                    // note：不写 → DB 默认 'n/a'
                ],
                vec![
                    WriteValue::Default,                    // id：不写
                    WriteValue::Null,                       // note：显式写 NULL
                ],
                vec![
                    WriteValue::Default,                    // id：不写
                    WriteValue::Value(CellValue::Text("x".to_string())), // note：写具体值
                ],
            ]),
        };
        connector.apply_changes(&tri).expect("三态插入应成功");
        let page3 = connector.load_data(&path3, 0, 50, &[], &[]).expect("三态读取应成功");
        assert_eq!(page3.rows.len(), 3, "应插入 3 行");
        // 行序按 id 自增：Default→'n/a'，Null→NULL，Value→'x'。
        assert_eq!(page3.rows[0].values[1], CellValue::Text("n/a".to_string()), "Default 应落 DB 默认值");
        assert_eq!(page3.rows[1].values[1], CellValue::Null, "Null 应显式写 NULL");
        assert_eq!(page3.rows[2].values[1], CellValue::Text("x".to_string()), "Value 应写具体值");

        // 清理。
        let mut cleanup = pg_query_request(&config, None);
        cleanup.text = "DROP TABLE IF EXISTS t11_edit CASCADE; DROP TABLE IF EXISTS t11_tristate CASCADE;"
            .to_string();
        connector.execute(&cleanup).expect("清理临时结构应成功");
    }

    #[test]
    fn pg_live_smoke_aborted_transaction_skips_remaining() {
        let Some(params) = pg_smoke_params() else {
            tracing::warn!(target: "fluxdb_connectors", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG T13 冒烟");
            return;
        };
        let config = pg_smoke_config(params);
        let connector = PostgresConnector::with_config(config.clone());

        let mut setup = pg_query_request(&config, None);
        setup.text = "DROP TABLE IF EXISTS t13_abort CASCADE; CREATE TABLE t13_abort(id int PRIMARY KEY);"
            .to_string();
        connector.execute(&setup).expect("建表应成功");

        // continue_on_error 下，显式事务内冲突使会话进入 aborted；
        // 后续语句须逐条跳过（标注「需 ROLLBACK」，不得在 ROLLBACK 前继续）。
        let mut batch = pg_query_request(&config, None);
        batch.text = "\
            BEGIN; \
            INSERT INTO t13_abort VALUES (1); \
            INSERT INTO t13_abort VALUES (1); \
            INSERT INTO t13_abort VALUES (2); \
            INSERT INTO t13_abort VALUES (3); \
        "
        .to_string();
        batch.options.continue_on_error = true;
        let result = connector.execute(&batch).expect("continue_on_error 批次应返回而非抛错");
        // BEGIN 成功、首次插入成功、冲突失败、25P02 失败、随后语句被跳过。
        assert!(result.summaries.len() >= 4, "summaries = {:#?}", result.summaries);
        assert!(result.summaries[0].success, "BEGIN 应成功");
        assert!(result.summaries[1].success, "首次插入应成功");
        assert!(!result.summaries[2].success, "冲突插入应失败");
        // 事务内冲突后，存在一条 25P02 与一条被跳过的记录。
        assert!(
            result.summaries[3..].iter().any(|s| s.message.contains("已跳过")),
            "应存在跳过摘要：{:#?}",
            result.summaries
        );

        // 用户显式 ROLLBACK 恢复会话，随后仍可正常执行。
        let mut recover = pg_query_request(&config, None);
        recover.text = "ROLLBACK; SELECT count(*) AS cnt FROM t13_abort;".to_string();
        let recovered = connector.execute(&recover).expect("ROLLBACK 后应恢复");
        assert!(
            recovered.summaries.iter().all(|s| s.success),
            "ROLLBACK 后应全部成功：{:#?}",
            recovered.summaries
        );

        // 清理。
        let mut cleanup = pg_query_request(&config, None);
        cleanup.text = "DROP TABLE IF EXISTS t13_abort CASCADE;".to_string();
        connector.execute(&cleanup).expect("清理临时结构应成功");
    }

}
