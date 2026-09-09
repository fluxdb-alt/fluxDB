    #[test]
    fn execute_query_returns_mock_results_and_records_history() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product".to_string(),
        });

        let event = controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(matches!(event, AppEvent::QueryFinished(TabId(1), _)));
        let editor = active_query_editor(&controller);
        assert_eq!(editor.results.len(), 1);
        assert_eq!(editor.summaries.len(), 1);
        assert_eq!(editor.summaries[0].sql, "select * from Product LIMIT 100");
        assert!(editor.error.is_none());
        assert_eq!(controller.state().query_history.len(), 1);
        assert_eq!(
            controller.state().query_history[0].text,
            "select * from Product LIMIT 100"
        );
        assert_eq!(
            controller.state().query_history[0].tables,
            vec!["Product".to_string()]
        );
        assert_eq!(
            controller.state().query_history[0].kind,
            QueryHistoryKind::Query
        );
        assert!(controller.state().query_history[0].success);
    }

    #[test]
    fn mysql_completion_context_uses_ast_table_aliases() {
        let sql = "select * from sales.orders as o join customers c on c.id = o.customer_id where c.id = 1";
        let cursor = sql.find("c.id = 1").unwrap() + 2;

        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert_eq!(
            context.referenced_tables,
            vec![
                ReferencedTable {
                    database: Some("sales".to_string()),
                    name: "orders".to_string(),
                    alias: Some("o".to_string()),
                },
                ReferencedTable {
                    database: None,
                    name: "customers".to_string(),
                    alias: Some("c".to_string()),
                },
            ]
        );
        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "customers".to_string(),
                alias: Some("c".to_string()),
            }]
        );
    }

    #[test]
    fn mysql_completion_context_suggests_schemas_in_table_context_without_qualifier() {
        // FROM/JOIN 后、无 qualifier 时给出 schema 候选。
        let sql = "select * from ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(context.suggest_tables);
        assert!(context.suggest_schemas);

        // 带 qualifier（`from sales.`）视为已限定 schema，不再重复建议 schema。
        let qualified = "select * from sales.";
        let context = sql_completion_context(qualified, qualified.len(), DatabaseKind::MySql);
        assert!(context.qualifier.is_some());
        assert!(!context.suggest_schemas);

        // 无表上下文的普通位置不触发 schema。
        let plain = "select ";
        let context = sql_completion_context(plain, plain.len(), DatabaseKind::MySql);
        assert!(!context.suggest_schemas);
    }

    #[test]
    fn mysql_completion_context_keeps_full_qualifier_path() {
        let alias = "select u. from sales.orders u";
        let alias_context = sql_completion_context(alias, "select u.".len(), DatabaseKind::MySql);
        assert_eq!(alias_context.qualifier, Some("u".to_string()));
        assert_eq!(alias_context.qualifier_path, vec!["u".to_string()]);

        let qualified = "select * from sales.public.ord";
        let qualified_context = sql_completion_context(qualified, qualified.len(), DatabaseKind::MySql);
        assert_eq!(qualified_context.prefix, "ord");
        assert_eq!(
            qualified_context.qualifier_path,
            vec!["sales".to_string(), "public".to_string()]
        );
        assert_eq!(qualified_context.qualifier, Some("public".to_string()));

        let quoted = "select * from `sales`.`public`.ord";
        let quoted_context = sql_completion_context(quoted, quoted.len(), DatabaseKind::MySql);
        assert_eq!(quoted_context.prefix, "ord");
        assert_eq!(
            quoted_context.qualifier_path,
            vec!["sales".to_string(), "public".to_string()]
        );
        assert_eq!(quoted_context.qualifier, Some("public".to_string()));

        let escaped = r#"select * from `sales``west`.`public`.ord"#;
        let escaped_context = sql_completion_context(escaped, escaped.len(), DatabaseKind::MySql);
        assert_eq!(
            escaped_context.qualifier_path,
            vec!["sales`west".to_string(), "public".to_string()]
        );
    }

    #[test]
    fn completion_context_limits_large_document_to_current_statement() {
        let mut sql = "select 1; ".repeat(20_000);
        sql.push_str("select * from users wh");

        let context = sql_completion_context(&sql, sql.len(), DatabaseKind::MySql);

        assert_eq!(context.prefix, "wh");
        assert!(context.referenced_tables.iter().any(|table| table.name == "users"));
        assert!(completion_context_start(&sql, sql.len()) >= sql.len() - COMPLETION_CONTEXT_WINDOW_BYTES);
    }

    #[test]
    fn completion_namespace_scope_maps_database_and_schema() {
        let default_sql = "select * from ";
        let default = sql_completion_context(default_sql, default_sql.len(), DatabaseKind::MySql);
        assert_eq!(
            completion_namespace_scope(&default, Some("main")),
            (Some("main".to_string()), None)
        );

        let qualified_sql = "select * from sales.public.";
        let qualified = sql_completion_context(qualified_sql, qualified_sql.len(), DatabaseKind::MySql);
        assert_eq!(
            completion_namespace_scope(&qualified, Some("main")),
            (Some("sales".to_string()), Some("public".to_string()))
        );
    }

    #[test]
    fn schema_completion_items_apply_schema_dot() {
        let schemas = vec![
            (Some("sales".to_string()), Some("public".to_string())),
            (Some("hr".to_string()), None),
        ];
        let items = schema_completion_items(schemas, "p");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "public");
        assert_eq!(items[0].insert_text, "public.");
        assert_eq!(items[0].kind, QueryCompletionKind::Schema);

        // 全部候选时取 schema 名优先于 database 名。
        let schemas = vec![
            (Some("sales".to_string()), Some("public".to_string())),
            (Some("hr".to_string()), None),
        ];
        let items = schema_completion_items(schemas, "");
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| item.label == "public"));
        assert!(items.iter().any(|item| item.label == "hr"));
    }

    #[test]
    fn quote_identifier_mysql_uses_backtick_and_escapes() {
        // 保留字触发反引号。
        assert_eq!(
            quote_identifier("select", DatabaseKind::MySql, |w| w == "select"),
            "`select`"
        );
        assert_eq!(
            quote_identifier("order", DatabaseKind::MySql, |w| w == "order"),
            "`order`"
        );
        // 空格/特殊字符触发引号。
        assert_eq!(
            quote_identifier("my col", DatabaseKind::MySql, |_| false),
            "`my col`"
        );
        assert_eq!(
            quote_identifier("a-b", DatabaseKind::MySql, |_| false),
            "`a-b`"
        );
        // 反引号转义为双反引号。
        assert_eq!(
            quote_identifier("a`b", DatabaseKind::MySql, |_| false),
            "`a``b`"
        );
        // 普通标识符原样返回。
        assert_eq!(
            quote_identifier("user_name", DatabaseKind::MySql, |_| false),
            "user_name"
        );
        // 以数字开头触发引号。
        assert_eq!(
            quote_identifier("1st", DatabaseKind::MySql, |_| false),
            "`1st`"
        );
    }

    #[test]
    fn quote_identifier_standard_dialects_use_double_quote() {
        assert_eq!(
            quote_identifier("select", DatabaseKind::Sqlite, |w| w == "select"),
            "\"select\""
        );
        assert_eq!(
            quote_identifier("a\"b", DatabaseKind::Sqlite, |_| false),
            "\"a\"\"b\""
        );
        assert_eq!(quote_identifier("orders", DatabaseKind::Sqlite, |_| false), "orders");
        // 非 MySQL 方言（含 SQL Server 契约）同样用双引号。
        assert_eq!(
            quote_identifier("group", DatabaseKind::Sqlite, |w| w == "group"),
            "\"group\""
        );
    }

    #[test]
    fn completion_quote_preserves_function_and_qualified_identifiers() {
        let reserved = |word: &str| word.eq_ignore_ascii_case("select");
        let mut function = QueryCompletionItem {
            label: "COUNT".into(),
            insert_text: "COUNT()".into(),
            kind: QueryCompletionKind::Function,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        quote_completion_insert_text(&mut function, DatabaseKind::MySql, &reserved, false);
        assert_eq!(function.insert_text, "COUNT()");

        let mut column = QueryCompletionItem {
            label: "o.select".into(),
            insert_text: "o.select".into(),
            kind: QueryCompletionKind::Column,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        quote_completion_insert_text(&mut column, DatabaseKind::MySql, &reserved, false);
        assert_eq!(column.insert_text, "o.`select`");

        let mut explicitly_quoted = QueryCompletionItem {
            label: "orders".into(),
            insert_text: "orders".into(),
            kind: QueryCompletionKind::Table,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        quote_completion_insert_text(
            &mut explicitly_quoted,
            DatabaseKind::MySql,
            &reserved,
            true,
        );
        assert_eq!(explicitly_quoted.insert_text, "`orders`");
    }

    #[test]
    fn completion_dedupe_keeps_same_name_from_different_databases() {
        let item = |detail: &str| QueryCompletionItem {
            label: "users".into(),
            insert_text: "users".into(),
            kind: QueryCompletionKind::Table,
            detail: Some(detail.into()),
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        let items = dedupe_completion_items(vec![item("community_test"), item("archive")]);
        assert_eq!(items.len(), 2);
        let items = dedupe_completion_items(vec![item("community_test"), item("community_test")]);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn identifier_needs_quote_flags_reserved_digit_and_special() {
        assert!(identifier_needs_quote("select", |w| w == "select"));
        assert!(identifier_needs_quote("1abc", |_| false));
        assert!(identifier_needs_quote("a-b", |_| false));
        assert!(!identifier_needs_quote("user_name", |_| false));
        assert!(!identifier_needs_quote("", |_| false));
    }

    #[test]
    fn routine_completion_items_function_and_procedure() {
        let routines = vec![
            CompletionRoutine {
                schema: None,
                name: "calc_total".into(),
                kind: CompletionRoutineKind::Function,
            },
            CompletionRoutine {
                schema: None,
                name: "do_thing".into(),
                kind: CompletionRoutineKind::Procedure,
            },
        ];
        // 函数无参数 metadata 时插入 name()，kind 为 Function。
        let items = routine_completion_items(routines.clone(), CompletionRoutineKind::Function, "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "calc_total");
        assert_eq!(items[0].insert_text, "calc_total()");
        assert_eq!(items[0].kind, QueryCompletionKind::Function);
        // procedure 插入裸名，kind 为 Procedure。
        let items = routine_completion_items(routines, CompletionRoutineKind::Procedure, "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert_text, "do_thing");
        assert_eq!(items[0].kind, QueryCompletionKind::Procedure);
    }

    #[test]
    fn snippet_completion_items_prefix_triggers_and_plain_insert() {
        // 前缀 `sel` 触发 SELECT 片段，kind 为 Snippet，insert_text 为模板、无占位符导航。
        let items = snippet_completion_items("sel");
        assert!(!items.is_empty());
        let select = items
            .iter()
            .find(|item| item.kind == QueryCompletionKind::Snippet && item.insert_text.starts_with("SELECT"));
        assert!(select.is_some());
        assert_eq!(select.unwrap().filter_text.as_deref(), Some("select"));
        assert_eq!(select.unwrap().insert_text, "SELECT * FROM ");

        // 不匹配的前缀不返回任何片段（如 `zz`）。
        assert!(snippet_completion_items("zz").is_empty());

        // 空前缀不触发，避免在原有输入后无差别弹出片段干扰。
        assert!(snippet_completion_items("").is_empty());
    }

    #[test]
    fn disambiguate_duplicate_columns_qualifies_cross_table_duplicates() {
        let mk = |label: &str, table: &str, qualifier: &str| RankedCompletionItem {
            item: QueryCompletionItem {
                label: label.into(),
                insert_text: label.into(),
                kind: QueryCompletionKind::Column,
                detail: None,
                documentation: None,
                filter_text: None,
                sort_text: None,
                            ..Default::default()
},
            score: 0,
            rank: (0, 0, 0, 0),
            source_table: table.into(),
            source_qualifier: qualifier.into(),
        };
        // orders.id 与 users.id 跨表重复 → 加限定前缀（优先用别名）；name 唯一 → 保持裸列名。
        let items = vec![
            mk("id", "orders", "o"),
            mk("id", "users", "u"),
            mk("name", "users", "u"),
        ];
        let out = disambiguate_duplicate_columns(items);
        assert!(out.iter().any(|i| i.item.label == "o.id" && i.item.insert_text == "o.id"));
        assert!(out.iter().any(|i| i.item.label == "u.id" && i.item.insert_text == "u.id"));
        // 唯一列不因重复列消歧而改变。
        assert!(out.iter().any(|i| i.item.label == "name"));
    }

    #[test]
    fn disambiguate_duplicate_columns_keeps_bare_columns_when_unique() {
        let mk = |label: &str, table: &str, qualifier: &str| RankedCompletionItem {
            item: QueryCompletionItem {
                label: label.into(),
                insert_text: label.into(),
                kind: QueryCompletionKind::Column,
                detail: None,
                documentation: None,
                filter_text: None,
                sort_text: None,
                            ..Default::default()
},
            score: 0,
            rank: (0, 0, 0, 0),
            source_table: table.into(),
            source_qualifier: qualifier.into(),
        };
        // 同一列只来自一张表（即使多次引用同一表）→ 不加前缀。
        let items = vec![
            mk("id", "orders", "o"),
            mk("amount", "orders", "o"),
        ];
        let out = disambiguate_duplicate_columns(items);
        assert!(out.iter().any(|i| i.item.label == "id"));
        assert!(out.iter().any(|i| i.item.label == "amount"));
    }

    #[test]
    fn mysql_completion_context_keeps_legacy_fallback_for_incomplete_sql() {
        let sql = "select u. from users u";
        let cursor = "select u.".len();

        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "users".to_string(),
                alias: Some("u".to_string()),
            }]
        );
    }

    #[test]
    fn mysql_completion_context_supports_unclosed_backtick_table_prefix() {
        let sql = "select * from `3d_dental_efo";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);

        assert!(context.suggest_tables);
        assert_eq!(context.prefix, "3d_dental_efo");
        assert_eq!(context.replace_start, "select * from ".len());
        assert_eq!(context.replace_end, sql.len());
        assert!(context.quoted_identifier);
    }

    #[test]
    fn mysql_completion_context_replaces_auto_closed_backtick_pair() {
        let sql = "select * from `3d_dental_efo`";
        let cursor = sql.len() - 1;
        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert!(context.suggest_tables);
        assert_eq!(context.prefix, "3d_dental_efo");
        assert_eq!(context.replace_start, "select * from ".len());
        assert_eq!(context.replace_end, sql.len());
        assert!(context.quoted_identifier);
    }

    #[test]
    fn sqlite_completion_context_uses_sqlite_dialect_rules() {
        let call_sql = "call refresh";
        let call_context = sql_completion_context(call_sql, call_sql.len(), DatabaseKind::Sqlite);
        assert!(!call_context.suggest_procedures);
        assert!(call_context.suggest_keywords);

        let trigger_sql = "drop trigger users";
        let trigger_context =
            sql_completion_context(trigger_sql, trigger_sql.len(), DatabaseKind::Sqlite);
        assert!(trigger_context.suggest_triggers);
    }

    #[test]
    fn mysql_completion_context_only_uses_current_statement_tables() {
        let sql = "select * from users u; select * from orders o where id = 1";
        let cursor = sql.find("id = 1").unwrap();

        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "orders".to_string(),
                alias: Some("o".to_string()),
            }]
        );
    }

    #[test]
    fn mysql_completion_context_maps_simple_cte_alias_to_source_table() {
        let sql = "with recent as (select * from sales.orders) select * from recent r where r.id = 1";
        let cursor = sql.find("r.id = 1").unwrap() + 2;

        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: Some("sales".to_string()),
                table: "orders".to_string(),
                alias: Some("r".to_string()),
            }]
        );
    }

    #[test]
    fn mysql_completion_context_exposes_cte_projection_aliases() {
        let sql = "WITH recent(id, user_name) AS (SELECT id, name FROM users) SELECT recent.";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.cte_columns.get("recent"), Some(&vec!["id".to_string(), "user_name".to_string()]));
        let labels = cte_column_completion_items(&context, "", Some("recent"))
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["id", "user_name"]);
    }

    #[test]
    fn mysql_completion_context_infers_cte_select_aliases() {
        let sql = "WITH recent AS (SELECT id AS user_id, name FROM users) SELECT recent.";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.cte_columns.get("recent"), Some(&vec!["user_id".to_string(), "name".to_string()]));
    }

    #[test]
    fn cte_fallback_handles_recursive_quoted_names_and_keyword_boundaries() {
        let sql = "WITH RECURSIVE `recent``rows` (`row``id`) AS (SELECT base_id FROM base_table) SELECT `recent``rows`.";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::Sqlite);
        assert_eq!(
            context.cte_columns.get("recent`rows"),
            Some(&vec!["row`id".to_string()])
        );

        let false_as = "WITH recent AS (SELECT base FROM base_table) SELECT recent.";
        let context = sql_completion_context(false_as, false_as.len(), DatabaseKind::Sqlite);
        assert_eq!(context.cte_columns.get("recent"), Some(&vec!["base".to_string()]));
    }

    #[test]
    fn sql_scope_symbols_uses_ast_for_functions_casts_and_qualified_columns() {
        let symbols = sql_scope_symbols(
            "SELECT COUNT(u.id) AS total FROM users u WHERE u.missing = 1 AND missing = 2 AND EXISTS (SELECT 1 FROM orders o WHERE o.id = u.id) AND CAST(u.id AS custom_type) IS NOT NULL",
            DatabaseKind::MySql,
        );
        assert!(symbols.ast_parsed);
        assert!(symbols.function_names.contains(&"count".to_string()));
        assert!(symbols.cast_types.contains(&"custom_type".to_string()));
        assert!(symbols
            .qualified_columns
            .contains(&("u".to_string(), "id".to_string())));
        assert!(symbols.unqualified_columns.contains(&"missing".to_string()));
        assert!(symbols.relation_names.contains(&"orders".to_string()));
    }

    #[test]
    fn mysql_completion_context_inherits_outer_cte_in_nested_scope() {
        let sql = "WITH recent(id, name) AS (SELECT id, name FROM users) SELECT * FROM orders o WHERE EXISTS (SELECT 1 FROM recent r WHERE r.";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);

        assert_eq!(
            context.cte_columns.get("recent"),
            Some(&vec!["id".to_string(), "name".to_string()])
        );
        assert!(context.qualifier.as_deref().is_some_and(|name| name.eq_ignore_ascii_case("r")));
    }

    #[test]
    fn sql_scope_prefers_inner_cte_definition() {
        let sql = "WITH recent(id) AS (SELECT id FROM users) SELECT * FROM (WITH recent(name) AS (SELECT name FROM users) SELECT recent.);";
        let cursor = sql.find("SELECT recent.").unwrap() + "SELECT recent.".len();
        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert_eq!(context.cte_columns.get("recent"), Some(&vec!["name".to_string()]));
    }

    #[test]
    fn mysql_completion_context_uses_innermost_subquery_scope() {
        let sql = "select * from users u where exists (select 1 from orders u where u.";
        let cursor = sql.len();

        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "orders".to_string(),
                alias: Some("u".to_string()),
            }]
        );
    }

    #[test]
    fn mysql_completion_context_suggests_insert_target_columns() {
        let sql = "insert into Product (";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);

        assert!(context.suggest_columns);
        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "Product".to_string(),
                alias: None,
            }]
        );
    }

    #[test]
    fn mysql_completion_context_suggests_update_set_columns() {
        let sql = "update Product set na";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);

        assert!(context.suggest_columns);
        assert_eq!(context.prefix, "na");
        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "Product".to_string(),
                alias: None,
            }]
        );
    }

    #[test]
    fn intent_derives_insert_column_and_update_assignment_dml() {
        // T044 正例：INSERT 列列表 / UPDATE SET 赋值列上下文。
        let cases: Vec<(&str, NextAction)> = vec![
            // INSERT 列列表：尚未 VALUES，光标在列括号内。
            ("INSERT INTO users (id, ", NextAction::InsertColumn),
            ("insert into users (id, name, ", NextAction::InsertColumn),
            // 已进入 VALUES -> 取值，不是列列表。
            ("INSERT INTO users (id, name) VALUES (", NextAction::InsertValue),
            // UPDATE SET / SET 列表逗号后。
            ("UPDATE users SET ", NextAction::UpdateAssignment),
            ("update users set name = 'x', age = ", NextAction::PredicateValue),
        ];
        for (sql, expected) in cases {
            let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
            assert_eq!(
                context.intent.action, expected,
                "intent mismatch for: {sql:?}"
            );
        }
    }

    #[test]
    fn intent_dml_negative_contexts_stay_conservative() {
        // T044 反例：无 DML 前缀 / 已出 SET 值阶段 / 表名未定时不误判。
        let cases: Vec<(&str, NextAction)> = vec![
            // 不是 INSERT（SELECT 投影）。
            ("SELECT id, name ", NextAction::Unknown),
            // INSERT 无左括号（表名后：alias / 后续子句，非列列表）。
            ("INSERT INTO users ", NextAction::RelationAlias),
            // UPDATE 无 SET（表名后：alias/子句）。
            ("UPDATE users ", NextAction::RelationAlias),
            // SET 已到 WHERE 关键字：not assignment（WHERE 是关键字，predicate_operator 也排除）。
            ("UPDATE users SET name = 'x' WHERE ", NextAction::Unknown),
        ];
        for (sql, expected) in cases {
            let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
            assert_eq!(
                context.intent.action, expected,
                "intent mismatch for: {sql:?}"
            );
        }
    }

    #[test]
    fn create_table_context_only_inside_definition_parens() {
        // T044：CREATE TABLE 类型/约束关键字上下文只在定义括号内启用。
        let inside = "CREATE TABLE t (id INT PRIMARY KEY, name ";
        assert!(is_create_table_context(&inside.to_ascii_lowercase()));
        // 表名阶段或已闭合括号不启用。
        assert!(!is_create_table_context("create table t ".to_ascii_lowercase().as_str()));
        assert!(!is_create_table_context("CREATE TABLE t (id INT) ".to_ascii_lowercase().as_str()));
    }

    #[test]
    fn phase3_acceptance_dml_table_contexts_suggest_tables() {
        // T045 验收：DELETE/INSERT/UPDATE/ALTER 表格上下文均进入表候选方向。
        let cases: Vec<(&str, &str)> = vec![
            ("DELETE FROM ", "table"),
            ("INSERT INTO ", "table"),
            ("UPDATE ", "table"),
            ("ALTER TABLE ", "table"),
        ];
        for (sql, expect) in cases {
            let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
            let label = match expect {
                "table" => CompletionExpectation::Table,
                _ => CompletionExpectation::Keyword,
            };
            assert_eq!(context.expectation, label, "expectation mismatch for: {sql:?}");
        }
    }

    #[test]
    fn phase3_acceptance_delete_after_clause_suggests_columns_from_target() {
        // T045 验收：DELETE FROM t WHERE | 进入列候选（目标表已引用）。
        let sql = "DELETE FROM orders WHERE ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(context.suggest_columns);
        // 目标表已解析（orders）。
        assert_eq!(completion_column_tables(&context).len(), 1);
    }

    #[test]
    fn phase3_acceptance_multi_statement_isolates_second_statement() {
        // T045 验收：多 statement 中光标位于第二条语句，只取当前语句作用域。
        // 第一条的 CTE 不得泄漏进第二条。
        let sql = "WITH recent AS (SELECT id FROM users) SELECT id FROM recent;\nSELECT * FROM ";
        let cursor = sql.len();
        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);
        // 第二条 SELECT FROM 后：表候选，且为当前作用域；「已闭合的首条语句 CTE」不进入列作用域。
        assert_eq!(context.expectation, CompletionExpectation::Table);
        assert!(context.cte_columns.is_empty());
    }

    #[test]
    fn mysql_completion_context_resolves_digit_prefixed_table_alias_before_from() {
        let sql = "select u. from 3d_user u";
        let cursor = "select u.".len();

        let context = sql_completion_context(sql, cursor, DatabaseKind::MySql);

        assert!(context.suggest_columns);
        assert!(!context.suggest_keywords);
        assert!(!context.suggest_functions);
        assert_eq!(
            completion_column_tables(&context),
            vec![CompletionColumnTarget {
                database: None,
                table: "3d_user".to_string(),
                alias: Some("u".to_string()),
            }]
        );
    }

    #[test]
    fn extract_select_aliases_collects_as_and_bare_aliases() {
        // `AS` 别名与 MySQL 尾随裸别名均被提取，单纯列引用不视为别名。
        let scope = "select name, count(*) as cnt, sum(amount) total from orders o";
        assert_eq!(
            extract_select_aliases(scope),
            vec!["cnt".to_string(), "total".to_string()]
        );
    }

    #[test]
    fn extract_select_aliases_skips_plain_column_references() {
        // 单纯列引用（裸列 / 限定列）不作为别名；无 SELECT 时返回空。
        let scope = "select name, o.customer_id from orders o where id = 1";
        assert!(extract_select_aliases(scope).is_empty());

        assert!(extract_select_aliases("update orders set name = 'x'").is_empty());
    }

    #[test]
    fn perf_baseline_context_and_rank_over_four_scenarios() {
        // T011 性能基线（纯计算热路径，不含 DB connector）：
        // 空前缀 / 缓存列 / 未命中 metadata / 大 schema —— 每次运行采样，输出中位数到 gdb_sql_perf。
        use std::time::Instant;
        // 大 schema：多表多列文本，模拟高文本字节。
        let large_schema = {
            let mut s = String::with_capacity(4096);
            for i in 0..40 {
                s.push_str(&format!(
                    "select * from table_{i} t{i} where t{i}.col_{i} = t{i}.id and t{i}.flag = 1 order by t{i}.cnt "
                ));
            }
            s
        };
        let scenarios: Vec<(&str, &str)> = vec![
            // 空前缀：`FROM |`。
            ("select * from ", "empty-prefix"),
            // 缓存列：`WHERE t.`（限定列候选）。
            ("select * from orders t where t.", "qualified-column"),
            // 未命中 metadata：未关联表/对象上下文。
            ("select ", "unknown-object"),
            // 大 schema 长文本。
            (large_schema.as_str(), "large-schema"),
        ];
        for (sql, name) in scenarios {
            let mut samples = Vec::new();
            for _ in 0..32 {
                let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
                let expected = expected_token_completion_items(&context.intent, &context.prefix);
                // 合成列候选（跨表同名列制造去重负担）。
                let generic = (0..48)
                    .map(|i| QueryCompletionItem {
                        label: format!("col_{i}"),
                        insert_text: format!("col_{i}"),
                        kind: QueryCompletionKind::Column,
                        detail: Some(format!("table_{}", i % 4)),
                        documentation: None,
                        filter_text: None,
                        sort_text: None,
                        insert_text_format: InsertTextFormat::PlainText,
                    })
                    .collect::<Vec<_>>();
                let started = Instant::now();
                let _ranked = globally_rank_completion_items(
                    generic,
                    expected,
                    &context.intent,
                    &context.prefix,
                    &|_| false,
                    &|_| 0,
                );
                let _deduped = dedupe_completion_items(_ranked);
                samples.push(started.elapsed().as_micros() as u64);
            }
            samples.sort_unstable();
            let p50 = samples[samples.len() / 2];
            let p95 = samples[(samples.len() as f64 * 0.95) as usize];
            // 基线数字可见：`--nocapture` 下输出，供 T011 记录与历史对比。
            println!("[perf-baseline] {name}: text={}B rank p50={p50}us p95={p95}us", sql.len());
            tracing::debug!(
                target: "gdb_sql_perf",
                op = "baseline_test",
                scenario = name,
                text_bytes = sql.len(),
                p50_us = p50,
                p95_us = p95,
            );
            // 宽松上界：本地排序/去重不应显著退化；主要约束是存在性而非绝对值。
            assert!(p50 < 2000, "scenario {name} rank p50 {:?}us over bound", p50);
        }
    }

    #[test]
    fn order_group_context_boosts_select_aliases() {
        // ORDER BY 后提升别名候选（P2.11）。
        let sql = "select sum(amount) total_sum from orders o order by ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(context.suggest_select_aliases);
        assert_eq!(context.select_aliases, vec!["total_sum".to_string()]);

        // GROUP BY 后同样提升。
        let sql = "select department, count(*) as cnt from employees group by ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(context.suggest_select_aliases);
        assert_eq!(context.select_aliases, vec!["cnt".to_string()]);

        // T042：HAVING 后也可见 SELECT 别名。
        let sql = "select department, count(*) as cnt from employees group by department having ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(context.suggest_select_aliases);
        assert_eq!(context.select_aliases, vec!["cnt".to_string()]);
    }

    #[test]
    fn non_order_group_context_does_not_boost_aliases() {
        // 普通列上下文（如 WHERE）不提升别名候选。
        let sql = "select sum(amount) total_sum from orders o where ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(!context.suggest_select_aliases);

        // 无别名的 ORDER BY 不提升。
        let sql = "select name from orders o order by ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(!context.suggest_select_aliases);
    }

    #[test]
    fn select_alias_completion_items_filter_by_prefix() {
        let items =
            select_alias_completion_items(&["total_sum".to_string(), "cnt".to_string()], "to");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "total_sum");
        assert_eq!(items[0].kind, QueryCompletionKind::Column);
        assert_eq!(items[0].detail.as_deref(), Some("select alias"));
    }

    #[test]
    fn star_expansion_target_detects_qualified_and_bare_star() {
        // 限定星号 `o.*` 光标在 `*` 后 → Some(Some("o"))。
        assert_eq!(
            star_expansion_target("select o.*"),
            Some(Some("o".to_string()))
        );
        // 裸星号 `SELECT *` → Some(None)。
        assert_eq!(star_expansion_target("select *"), Some(None));
        // 光标不在 `*` 后时返回 None。
        assert_eq!(star_expansion_target("select o."), None);
        // 纯表达式不触发（`*` 不是当前候选分隔符上下文时保守处理）。
        assert_eq!(star_expansion_target("select 1"), None);
    }

    #[test]
    fn star_expansion_context_enabled_only_with_referenced_tables() {
        // 限定星号：光标紧跟 `*` 之后。
        let sql = "select o.* from orders o";
        let star_at = sql.find('*').unwrap() + 1;
        let context = sql_completion_context(sql, star_at, DatabaseKind::MySql);
        assert_eq!(context.star_expansion, Some(Some("o".to_string())));

        // 裸星号：光标紧跟 `*` 之后。
        let sql = "select * from orders o";
        let star_at = sql.find('*').unwrap() + 1;
        let context = sql_completion_context(sql, star_at, DatabaseKind::MySql);
        assert_eq!(context.star_expansion, Some(None));

        // 无引用表（光标前星号但无 FROM）时不启用。
        let sql = "select * ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.star_expansion, None);
    }

    #[test]
    fn is_join_context_detects_join_keywords() {
        assert!(is_join_context("select * from orders o join "));
        assert!(is_join_context("select * from orders o left join"));
        assert!(is_join_context("from a inner join "));
        assert!(!is_join_context("select * from orders o where "));
        assert!(!is_join_context("select * from orders o"));
    }

    #[test]
    fn fk_join_completion_items_generate_from_real_fks() {
        let foreign_keys = vec![ForeignKeyInfo {
            name: "fk_orders_customer".to_string(),
            column: "customer_id".to_string(),
            ref_schema: None,
            ref_table: "customers".to_string(),
            ref_column: "id".to_string(),
        }];
        let items = fk_join_completion_items("o", &foreign_keys, DatabaseKind::MySql, "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert_text, "JOIN customers ON o.customer_id = customers.id");
        assert_eq!(items[0].kind, QueryCompletionKind::Snippet);
        assert_eq!(items[0].filter_text.as_deref(), Some("customers"));
    }

    #[test]
    fn fk_join_completion_items_empty_and_prefix_filter() {
        // 无外键返回空。
        assert!(fk_join_completion_items("o", &[], DatabaseKind::MySql, "").is_empty());

        // 前缀过滤：仅匹配参考表名。
        let foreign_keys = vec![ForeignKeyInfo {
            name: "fk".to_string(),
            column: "customer_id".to_string(),
            ref_schema: None,
            ref_table: "customers".to_string(),
            ref_column: "id".to_string(),
        }];
        let items = fk_join_completion_items("o", &foreign_keys, DatabaseKind::MySql, "ord");
        assert!(items.is_empty());
        let items = fk_join_completion_items("o", &foreign_keys, DatabaseKind::MySql, "c");
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn fk_join_completion_items_group_composite_constraints() {
        let foreign_keys = vec![
            ForeignKeyInfo {
                name: "fk_pair".into(),
                column: "tenant_id".into(),
                ref_schema: None,
                ref_table: "accounts".into(),
                ref_column: "tenant_id".into(),
            },
            ForeignKeyInfo {
                name: "fk_pair".into(),
                column: "account_id".into(),
                ref_schema: None,
                ref_table: "accounts".into(),
                ref_column: "id".into(),
            },
        ];
        let items = fk_join_completion_items("o", &foreign_keys, DatabaseKind::MySql, "");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].insert_text,
            "JOIN accounts ON o.tenant_id = accounts.tenant_id AND o.account_id = accounts.id"
        );
        assert_eq!(items[0].label, items[0].insert_text);
    }

    #[test]
    fn fk_join_completion_items_quotes_reserved_and_special_names() {
        // T060 验收 5：quote 名称。保留字 / 特殊字符表、列、schema 限定名在 JOIN
        // snippet 中逐段反引号（MySQL）。
        let foreign_keys = vec![
            ForeignKeyInfo {
                name: "fk_order_ref".to_string(),
                column: "order".to_string(),        // 保留字列
                ref_schema: None,
                ref_table: "order".to_string(),     // 保留字表
                ref_column: "id".to_string(),
            },
            ForeignKeyInfo {
                name: "fk_order_ref".to_string(),
                column: "detail key".to_string(),   // 特殊字符列
                ref_schema: None,
                ref_table: "order".to_string(),
                ref_column: "detail id".to_string(), // 特殊字符列
            },
        ];
        // root 别名也是保留字（如 `group`），须引号；schema 限定参考表逐段引号。
        let items = fk_join_completion_items(
            "group",
            &foreign_keys,
            DatabaseKind::MySql,
            "",
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].insert_text,
            "JOIN `order` ON `group`.`order` = `order`.id AND `group`.`detail key` = `order`.`detail id`"
        );
        // label 与 insert_text 一致；filter 仍按裸表名 `order` 匹配前缀。
        assert_eq!(items[0].label, items[0].insert_text);
        assert_eq!(items[0].filter_text.as_deref(), Some("order"));

        // schema 限定参考表：schema 与表名逐段引号（保留字 schema/table）。
        let schemed = vec![ForeignKeyInfo {
            name: "fk".to_string(),
            column: "group_id".to_string(),
            ref_schema: Some("select".to_string()), // 保留字 schema
            ref_table: "order".to_string(),
            ref_column: "id".to_string(),
        }];
        let items = fk_join_completion_items("a", &schemed, DatabaseKind::MySql, "");
        assert_eq!(
            items[0].insert_text,
            "JOIN `select`.`order` ON a.group_id = `select`.`order`.id"
        );
    }

    #[test]
    fn join_context_suggests_plain_tables_besides_fk() {
        // T060 验收 1：普通表候选。`join ` 带引用表时，普通表候选仍给出
        // （不只 FK snippet），关系补全在 JOIN 上下文可用。
        let sql = "select * from customers c join ";
        let items = matrix_request_items(sql, None);
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Table && item.label == "Product"),
            "JOIN 后应给出普通表 Product"
        );
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Table && item.label == "ProductCategory"),
            "JOIN 后应给出普通表 ProductCategory"
        );
    }

    #[test]
    fn fk_join_context_enabled_when_join_keyword_with_referenced_tables() {
        // JOIN 关键字后且有引用表时启用。
        let sql = "select * from orders o join ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(context.suggest_join_keys);

        // 无引用表（未写 FROM）时不启用。
        let sql = "select * join ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(!context.suggest_join_keys);

        // 非 JOIN 上下文不启用。
        let sql = "select * from orders o where ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert!(!context.suggest_join_keys);
    }

    #[test]
    fn join_on_fk_score_boosts_child_fk_column_and_is_stable() {
        // T043：JOIN ON 场景子表外键列提升（设计 +25），非外键列不动。
        assert_eq!(join_on_fk_score(40, true), 15);
        assert_eq!(join_on_fk_score(40, false), 40);
        // 小分数下仍单调低于同分数非外键列，确保外键列恒排前。
        assert!(join_on_fk_score(50, true) < join_on_fk_score(50, false));
    }

    #[test]
    fn qualified_join_on_context_resolves_only_that_alias_column_scope() {
        // T043：`ON u.id = o.|` 只补全 `o` 对应表列，未知/无关 alias 不泄漏全库噪音。
        let sql =
            "select * from users u join orders o on u.id = o.";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        let targets = completion_column_tables(&context);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].alias.as_deref(), Some("o"));
        // 未知 alias 限定：不命中任何表，返回空 -> 不落入全库降级。
        let unknown_sql =
            "select * from users u join orders o on u.id = o.id and z.";
        let unknown_targets =
            completion_column_tables(&sql_completion_context(unknown_sql, unknown_sql.len(), DatabaseKind::MySql));
        assert!(unknown_targets.is_empty());
    }

    #[test]
    fn next_action_predicate_operator_after_column() {
        // `WHERE age |` → 谓词操作符。
        let sql = "select * from users where age ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.intent.action, NextAction::PredicateOperator);
        assert!(context.intent.expected_tokens.iter().any(|tok| *tok == "="));
        assert!(context.intent.expected_tokens.iter().any(|tok| *tok == "LIKE"));
    }

    #[test]
    fn next_action_predicate_value_after_operator() {
        // `WHERE age = |` → 取值。
        let sql = "select * from users where age = ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.intent.action, NextAction::PredicateValue);
        assert!(context.intent.expected_tokens.iter().any(|tok| *tok == "NULL"));
        // 未消费操作符时（`WHERE age |`）仍是操作符而非取值。
        let sql = "select * from users where age ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.intent.action, NextAction::PredicateOperator);
    }

    #[test]
    fn predicate_value_context_never_suggests_not_null_after_operator() {
        // F002：`WHERE id = |` 后取值，非法的 NULL 判定操作符 NOT NULL 不得出现
        // （IS [NOT] NULL 只在操作符上下文才合法）。
        for sql in [
            "select * from users where id = ",
            "select * from users where id <> ",
            "select * from users where name like ",
            "select * from users where id >= ",
        ] {
            let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
            assert_eq!(context.intent.action, NextAction::PredicateValue);
            assert!(
                context
                    .intent
                    .expected_tokens
                    .iter()
                    .all(|tok| *tok != "NOT NULL"),
                "值上下文不应提供 NOT NULL: {sql}"
            );
            assert!(context.intent.expected_tokens.iter().any(|tok| *tok == "NULL"));
        }
    }

    #[test]
    fn predicate_value_context_recognizes_is_and_is_not() {
        // F002：`WHERE col IS [NOT] |` 也取值（NULL/TRUE/FALSE），正确区分 IS 判定。
        for sql in [
            "select * from users where id is ",
            "select * from users where id is not ",
        ] {
            let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
            assert_eq!(context.intent.action, NextAction::PredicateValue);
            for tok in ["NULL", "TRUE", "FALSE"] {
                assert!(context.intent.expected_tokens.contains(&tok), "{sql}: 缺 {tok}");
            }
        }
        // 限定列 `u.is |` 不误判为值上下文（is 是列名而非判定符）。
        let sql = "select * from users u where u.is ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_ne!(context.intent.action, NextAction::PredicateValue);
    }

    #[test]
    fn next_action_relation_alias_after_table() {
        // `FROM users |` → 别名或后续子句。
        let sql = "select * from users ";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(context.intent.action, NextAction::RelationAlias);
        assert!(context.intent.expected_tokens.iter().any(|tok| *tok == "AS"));
        assert!(context.intent.expected_tokens.iter().any(|tok| *tok == "WHERE"));
        // 表尚未结束时（未完的标识符）不判定为关系别名。
        let sql = "select * from us";
        let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_ne!(context.intent.action, NextAction::RelationAlias);
    }

    #[test]
    fn expected_token_items_come_from_intent() {
        // T014：预期 token 候选由意图驱动生成。
        let context = sql_completion_context(
            "select * from users where age ",
            "select * from users where age ".len(),
            DatabaseKind::MySql,
        );
        let items = expected_token_completion_items(&context.intent, "");
        assert!(items.iter().any(|item| item.label == "="));
        assert!(items.iter().any(|item| item.label == "LIKE"));
    }

    #[test]
    fn global_rank_puts_expected_tokens_first() {
        // T020：预期 token（判定结果）置于合并候选最前。
        let context = sql_completion_context(
            "select * from users where age ",
            "select * from users where age ".len(),
            DatabaseKind::MySql,
        );
        let expected = expected_token_completion_items(&context.intent, "");
        assert!(!expected.is_empty());
        let expected_labels: std::collections::HashSet<String> = expected
            .iter()
            .map(|item| item.label.to_ascii_lowercase())
            .collect();
        let generic = vec![QueryCompletionItem {
            label: "id".into(),
            insert_text: "id".into(),
            kind: QueryCompletionKind::Column,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
}];
        let items = globally_rank_completion_items(
            generic,
            expected,
            &context.intent,
            "",
            &|_| false,
            &|_| 0,
        );
        // 预期 token（操作符/取值关键字）整体排在通用候选之前；首项必属预期集合。
        assert!(expected_labels.contains(&items[0].label.to_ascii_lowercase()));
        assert!(expected_labels.contains(&items[1].label.to_ascii_lowercase()));
        assert!(!expected_labels.contains(&items.last().unwrap().label.to_ascii_lowercase()));
    }

    #[test]
    fn global_rank_prefers_clause_relevant_kind_on_empty_prefix() {
        // T022：空前缀下按意图相关度排序——WHERE 子句里列/函数排在表之前。
        let context = sql_completion_context(
            "select * from users where age ",
            "select * from users where age ".len(),
            DatabaseKind::MySql,
        );
        let column = QueryCompletionItem {
            label: "age".into(),
            insert_text: "age".into(),
            kind: QueryCompletionKind::Column,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        let table = QueryCompletionItem {
            label: "users".into(),
            insert_text: "users".into(),
            kind: QueryCompletionKind::Table,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        let items = globally_rank_completion_items(
            vec![table, column],
            Vec::new(),
            &context.intent,
            "",
            &|_| false,
            &|_| 0,
        );
        assert_eq!(items[0].label, "age");
        assert_eq!(items[1].label, "users");
    }

    #[test]
    fn dedupe_keeps_join_columns_from_distinct_tables() {
        // T023：JOIN 场景下不同表的同名列（detail 含来源）保留，完全相同者合并。
        let id_users = QueryCompletionItem {
            label: "id".into(),
            insert_text: "id".into(),
            kind: QueryCompletionKind::Column,
            detail: Some("users".into()),
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        let id_orders = QueryCompletionItem {
            label: "id".into(),
            insert_text: "id".into(),
            kind: QueryCompletionKind::Column,
            detail: Some("orders".into()),
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        let items = dedupe_completion_items(vec![
            id_users.clone(),
            id_orders.clone(),
            id_users.clone(),
        ]);
        // 两个不同来源的 id 保留，重复的 users.id 被合并。
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| item.detail.as_deref() == Some("users")));
        assert!(items.iter().any(|item| item.detail.as_deref() == Some("orders")));
    }

    #[test]
    fn intent_golden_corpus_covers_phase1_scenarios() {
        // T025 intent golden corpus：锁定 Phase 1 各场景的下一步意图判定。
        let cases = [
            ("select * from users where age ", NextAction::PredicateOperator, "="),
            ("select * from users where age = ", NextAction::PredicateValue, "NULL"),
            ("select * from users ", NextAction::RelationAlias, "AS"),
            ("select * from users o join orders on o.id = ", NextAction::PredicateValue, "NULL"),
            ("select * from users o join orders a on o.id = a.", NextAction::JoinCondition, ""),
            ("select * from users order by age ", NextAction::OrderByExpression, "ASC"),
            ("select * from users group by name ", NextAction::GroupByExpression, "ASC"),
            (
                "insert into t(a, b) values (",
                NextAction::InsertValue,
                "NULL",
            ),
            ("select", NextAction::Unknown, ""),
        ];
        for (sql, expected_action, expected_token) in cases {
            let context = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
            assert_eq!(
                context.intent.action, expected_action,
                "intent mismatch for: {sql:?}"
            );
            if !expected_token.is_empty() {
                assert!(
                    context
                        .intent
                        .expected_tokens
                        .iter()
                        .any(|token| *token == expected_token),
                    "expected token {expected_token:?} missing for: {sql:?}"
                );
            }
        }
    }

    #[test]
    fn replace_range_covers_only_trailing_identifier_not_qualifier() {
        // T030：采纳 `o.` 后的列候选只替换光标处的空前缀，保留 qualifier，避免重复 `o.`。
        let sql = "select * from users u join orders o on u.id = o.";
        let ctx = sql_completion_context(sql, sql.len(), DatabaseKind::MySql);
        assert_eq!(ctx.prefix, "");
        assert_eq!(ctx.qualifier.as_deref(), Some("o"));
        assert_eq!(ctx.replace_start, sql.len());
        assert_eq!(ctx.replace_end, sql.len());

        // 已输入部分前缀：replace 覆盖该前缀，仍不含 qualifier。
        let sql2 = "select * from users u where u.ag";
        let ctx = sql_completion_context(sql2, sql2.len(), DatabaseKind::MySql);
        assert_eq!(ctx.prefix, "ag");
        assert_eq!(ctx.replace_start, sql2.find("ag").unwrap());
        assert_eq!(ctx.replace_end, sql2.len());
    }

    #[test]
    fn execute_query_text_for_scope_reports_progress_without_opening_tab() {
        let controller = AppController::with_mock_data();
        let mut summaries = Vec::new();
        let result = controller
            .execute_query_text_for_scope_with_progress(
                ConnectionId(1),
                Some("main".to_string()),
                "select * from Product".to_string(),
                QueryExecutionOptions::default(),
                &mut |summary| summaries.push(summary),
                &|| false,
            )
            .unwrap();

        assert!(controller.state().tabs.is_empty());
        assert_eq!(summaries.len(), 1);
        assert_eq!(result.summaries[0].sql, "select * from Product LIMIT 100");
    }

    #[test]
    fn execute_query_records_history_kind_tables_and_failure_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "update Product set name = 'Bike' where id = 1".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        let history = &controller.state().query_history[0];
        assert_eq!(history.tables, vec!["Product".to_string()]);
        assert_eq!(history.kind, QueryHistoryKind::DataChange);
        assert!(history.success);

        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "create table AuditLog (id int)".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        let history = &controller.state().query_history[1];
        assert_eq!(history.tables, vec!["auditlog".to_string()]);
        assert_eq!(history.kind, QueryHistoryKind::SchemaChange);
        assert!(history.success);

        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select error from Product".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        let history = &controller.state().query_history[2];
        assert_eq!(history.kind, QueryHistoryKind::Query);
        assert!(!history.success);
    }

    #[test]
    fn execute_delete_query_records_original_row_snapshot() {
        let path = temp_sqlite_path("delete-rollback-history");
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
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "DELETE FROM products WHERE id = 1".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        let history = &controller.state().query_history[0];
        assert_eq!(history.kind, QueryHistoryKind::DataChange);
        assert!(history.rollback_snapshot.is_some());
        assert_eq!(
            history.rollback_sql().as_deref(),
            Some("INSERT INTO products (`id`, `name`) VALUES (1, 'Road Bike');")
        );
        assert!(history
            .rollback_snapshot_summary()
            .unwrap()
            .contains("原始行: 1 行"));
    }

    #[test]
    fn sqlite_copy_table_uses_source_ddl_for_structure() {
        let path = temp_sqlite_path("copy-table-structure");
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
                    name TEXT NOT NULL DEFAULT 'new',
                    UNIQUE(name)
                )",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        });

        let object = ObjectPath {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
            schema: None,
            name: "products".to_string(),
            kind: ObjectKind::Table,
        };
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));

        let event = controller.dispatch(AppCommand::CopyTable {
            object,
            new_name: "products_copy".to_string(),
            copy_data: false,
        });

        assert!(matches!(event, AppEvent::TableCopied { .. }));
        runtime.block_on(async {
            let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .connect()
                .await
                .unwrap();
            let ddl: (String,) = sqlx::query_as(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'products_copy'",
            )
            .fetch_one(&mut connection)
            .await
            .unwrap();
            assert!(ddl.0.contains("id INTEGER PRIMARY KEY"));
            assert!(ddl.0.contains("name TEXT NOT NULL DEFAULT 'new'"));
            assert!(ddl.0.contains("UNIQUE"));
            sqlx::query("INSERT INTO products_copy (id) VALUES (1)")
                .execute(&mut connection)
                .await
                .unwrap();
            let name: (String,) = sqlx::query_as("SELECT name FROM products_copy WHERE id = 1")
                .fetch_one(&mut connection)
                .await
                .unwrap();
            assert_eq!(name.0, "new");
            connection.close().await.unwrap();
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn execute_update_query_records_original_values_snapshot() {
        let path = temp_sqlite_path("query-update-rollback");
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
            sqlx::query("CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, payload BLOB)")
                .execute(&mut connection)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO products (id, name, payload) VALUES (1, 'Road Bike', X'010203')",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        });

        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(7),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "UPDATE products SET name = 'Touring Bike' WHERE id = 1".to_string(),
        });

        let event = controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(matches!(event, AppEvent::QueryFinished(TabId(1), _)));
        let history = &controller.state().query_history[0];
        assert!(history.rollback_snapshot.is_some());
        assert_eq!(
            history.rollback_sql().as_deref(),
            Some("UPDATE products SET `name` = 'Road Bike' WHERE `id` = 1;")
        );
        assert!(history
            .rollback_snapshot_summary()
            .unwrap()
            .contains("变更字段: name"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn simple_query_result_can_be_edited_when_primary_key_is_returned() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(active_query_result_editor(active_query_editor(&controller)).is_some());
        let event = controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Changed Bike".to_string()),
        });

        assert!(matches!(event, AppEvent::TabActivated(TabId(1))));
        let editor = active_query_result_editor(active_query_editor(&controller)).unwrap();
        assert_eq!(editor.changes.as_ref().unwrap().dirty_cell_count(), 1);
    }

    #[test]
    fn applying_query_result_changes_clears_dirty_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Changed Bike".to_string()),
        });

        let event = controller.dispatch(AppCommand::ApplyDataChanges(TabId(1)));

        assert!(matches!(event, AppEvent::TabActivated(TabId(1))));
        let editor = active_query_result_editor(active_query_editor(&controller)).unwrap();
        assert!(editor.changes.is_none());
    }

    #[test]
    fn complex_query_result_stays_readonly() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product join Category on Product.category_id = Category.id"
                .to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(active_query_editor(&controller).result_editors.is_empty());
    }

    #[test]
    fn execute_query_runs_multiple_select_statements() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product limit 20;\n\nselect * from Product".to_string(),
        });

        let event = controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(matches!(event, AppEvent::QueryFinished(TabId(1), _)));
        let editor = active_query_editor(&controller);
        assert_eq!(editor.summaries.len(), 2);
        assert_eq!(editor.summaries[0].sql, "select * from Product limit 20");
        assert_eq!(editor.summaries[1].sql, "select * from Product LIMIT 100");
    }

    #[test]
    fn multi_result_query_tracks_editors_per_result_page() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product;\nselect * from Product".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        let editor = active_query_editor(&controller);
        assert_eq!(editor.results.len(), 2);
        assert!(editor.result_editors.contains_key(&0));
        assert!(editor.result_editors.contains_key(&1));
        assert_eq!(editor.active_result_editor, Some(0));

        controller.dispatch(AppCommand::ActivateQueryResultEditor {
            tab_id: TabId(1),
            page_index: Some(1),
        });
        controller.dispatch(AppCommand::EditDataCell {
            tab_id: TabId(1),
            row: 0,
            column: 1,
            value: CellValue::Text("Second Result".to_string()),
        });

        let editor = active_query_editor(&controller);
        assert_eq!(editor.active_result_editor, Some(1));
        assert!(editor.result_editors[&0].changes.is_none());
        assert_eq!(
            editor.result_editors[&1]
                .changes
                .as_ref()
                .unwrap()
                .dirty_cell_count(),
            1
        );
    }

    #[test]
    fn finish_query_result_page_refresh_replaces_only_target_result_page() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product;\nselect * from Product".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));
        let first_page_before = active_query_editor(&controller).results[0].clone();
        let mut refreshed_page = active_query_editor(&controller).results[1].clone();
        refreshed_page.rows.remove(0);

        let event = controller.dispatch(AppCommand::FinishQueryResultPageRefresh {
            tab_id: TabId(1),
            result_index: 1,
            page_index: 1,
            result: Ok(QueryExecutionResult {
                summaries: vec![QueryExecutionSummary {
                    sql: "select * from Product LIMIT 100".to_string(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: true,
                    message: "返回 1 行结果表".to_string(),
                    returned_rows: 1,
                    affected_rows: 0,
                    elapsed_ms: 3,
                }],
                results: vec![refreshed_page.clone()],
                rollback_snapshots: Vec::new(),
            }),
        });

        assert_eq!(
            event,
            AppEvent::QueryResultPageRefreshed {
                tab_id: TabId(1),
                result_index: 1,
                page_index: 1,
                page: refreshed_page.clone(),
            }
        );
        let editor = active_query_editor(&controller);
        assert_eq!(editor.results.len(), 2);
        assert_eq!(editor.results[0], first_page_before);
        assert_eq!(editor.results[1], refreshed_page);
        assert!(editor.result_editors.contains_key(&0));
        assert!(editor.result_editors.contains_key(&1));
        assert_eq!(editor.active_result_editor, Some(1));
    }

    #[test]
    fn query_result_row_actions_update_target_result_editor() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product;\nselect * from Product".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        let event = controller.dispatch(AppCommand::InsertDataRow {
            tab_id: TabId(1),
            result_index: Some(1),
            after_row: Some(0),
        });

        assert_eq!(event, AppEvent::TabActivated(TabId(1)));
        let editor = active_query_editor(&controller);
        assert!(editor.result_editors[&0].changes.is_none());
        let result_editor = &editor.result_editors[&1];
        assert_eq!(result_editor.page.as_ref().unwrap().rows.len(), 3);
        assert_eq!(
            result_editor.editing_cell,
            Some(CellPosition { row: 1, column: 0 })
        );
        assert_eq!(editor.results[1].rows.len(), 3);

        controller.dispatch(AppCommand::DeleteDataRow {
            tab_id: TabId(1),
            result_index: Some(1),
            row: 0,
        });
        let changes = active_query_editor(&controller).result_editors[&1]
            .changes
            .as_ref()
            .unwrap();
        assert_eq!(changes.inserts.len(), 1);
        assert_eq!(changes.deletes.len(), 1);
    }

    #[test]
    fn execute_query_text_runs_selection_without_replacing_editor_text() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select error;\nselect * from Product".to_string(),
        });

        let event = controller.dispatch(AppCommand::ExecuteQueryText {
            tab_id: TabId(1),
            text: "select * from Product".to_string(),
        });

        let AppEvent::QueryFinished(_, execution) = event else {
            panic!("expected query finished");
        };
        assert_eq!(execution.summaries[0].sql, "select * from Product LIMIT 100");
        assert_eq!(
            active_query_editor(&controller).text,
            "select error;\nselect * from Product"
        );
        assert_eq!(
            active_query_editor(&controller).summaries[0].sql,
            "select * from Product LIMIT 100"
        );
        assert_eq!(controller.state().query_history.len(), 1);
    }

    #[test]
    fn execute_query_normalizes_double_quoted_strings() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: r#"select * from Product where name = "Bob's" and note = "ok""#.to_string(),
        });

        let event = controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(matches!(event, AppEvent::QueryFinished(TabId(1), _)));
        let editor = active_query_editor(&controller);
        assert_eq!(
            editor.summaries[0].sql,
            "select * from Product where name = 'Bob''s' and note = 'ok' LIMIT 100"
        );
        assert_eq!(
            active_query_editor(&controller).text,
            r#"select * from Product where name = "Bob's" and note = "ok""#
        );
        assert_eq!(
            controller.state().query_history[0].text,
            "select * from Product where name = 'Bob''s' and note = 'ok' LIMIT 100"
        );
    }

    #[test]
    fn execute_query_keeps_user_limit() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product limit 20".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert_eq!(
            active_query_editor(&controller).summaries[0].sql,
            "select * from Product limit 20"
        );
    }

    #[test]
    fn execute_query_strips_comments_before_running() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "-- Product 产品表\nselect * from Product /* inline */".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert_eq!(
            active_query_editor(&controller).summaries[0].sql,
            "select * from Product LIMIT 100"
        );
    }

    #[test]
    fn execute_query_keeps_comment_tokens_inside_strings() {
        assert_eq!(
            sql_text_for_execution("select '--keep', '#keep', '/*keep*/'"),
            "select '--keep', '#keep', '/*keep*/' LIMIT 100"
        );
    }

    #[test]
    fn execute_query_moves_limit_after_order_by() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product limit 20 order by name desc".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert_eq!(
            active_query_editor(&controller).summaries[0].sql,
            "select * from Product order by name desc limit 20"
        );
    }

    #[test]
    fn execute_query_appends_default_limit_after_order_by() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product order by name desc".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert_eq!(
            active_query_editor(&controller).summaries[0].sql,
            "select * from Product order by name desc LIMIT 100"
        );
    }

    #[test]
    fn execute_query_does_not_limit_non_select_sql() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "update Product set name = 'A'".to_string(),
        });

        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert_eq!(
            active_query_editor(&controller).summaries[0].sql,
            "update Product set name = 'A'"
        );
    }

    #[test]
    fn format_query_sql_aligns_mysql_create_table_columns() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(2)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "create table `demo` (`id` varchar(64) not null comment '主键', `created_at` timestamp(6) not null comment '创建时间', primary key (`id`))".to_string(),
        });

        controller.dispatch(AppCommand::FormatQuerySql(TabId(1)));

        let editor = active_query_editor(&controller);
        assert!(editor.text.contains("`id`          VARCHAR(64)"));
        assert!(editor.text.contains("`created_at`  TIMESTAMP(6)"));
        assert!(editor.text.contains("COMMENT '创建时间'"));
        assert!(editor.text.contains("  PRIMARY KEY (`id`)"));
    }

    #[test]
    fn compress_query_sql_collapses_to_one_line() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select  *\nfrom Product\nwhere name = 'Bob  Lee'\n-- keep\nand id = 1"
                .to_string(),
        });

        controller.dispatch(AppCommand::CompressQuerySql(TabId(1)));

        assert_eq!(
            active_query_editor(&controller).text,
            "select * from Product where name = 'Bob  Lee' /* keep*/ and id = 1"
        );
    }

    #[test]
    fn mark_query_saved_updates_title_and_dirty_state() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select 1".to_string(),
        });

        controller.dispatch(AppCommand::MarkQuerySaved {
            tab_id: TabId(1),
            title: "orders.sql".to_string(),
            origin: QueryOrigin::File {
                path: "orders.sql".into(),
            },
        });

        let tab = controller.state().active_tab().unwrap();
        assert_eq!(tab.title, "orders.sql");
        assert!(!tab.dirty);
    }

    #[test]
    fn execute_error_query_records_failed_summary() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select error; select * from Product".to_string(),
        });

        let event = controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        assert!(matches!(event, AppEvent::QueryFinished(TabId(1), _)));
        let editor = active_query_editor(&controller);
        assert_eq!(editor.results.len(), 1);
        assert_eq!(editor.summaries.len(), 2);
        assert!(!editor.summaries[0].success);
        assert!(editor.summaries[1].success);
        assert!(editor.error.is_none());
        assert_eq!(controller.state().query_history.len(), 2);
    }

    #[test]
    fn query_completion_suggests_tables_after_from() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Pro".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from Pro".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert_eq!(result.replace_start, "select * from ".len());
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Table && item.insert_text == "Product"
        }));
    }

    #[test]
    fn query_completion_fuzzy_matches_tables_after_from() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Pdt".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from Pdt".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Table && item.insert_text == "Product"
        }));
    }

    #[test]
    fn query_completion_suggests_mysql_create_table_types() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(2),
            database: None,
        });
        let text = "CREATE TABLE test (id va";
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: text.to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: text.len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| item.insert_text == "VARCHAR"));
    }

    #[test]
    fn keyword_completion_supports_fuzzy_matching() {
        let items = keyword_completion_items_for(SQL_COMPLETION_KEYWORDS, "sel");

        assert_eq!(items.first().map(|item| item.label.as_str()), Some("SELECT"));
    }

    #[test]
    fn function_completion_supports_fuzzy_matching() {
        let items = function_completion_items("coa");

        assert!(items.iter().any(|item| item.label == "COALESCE"));
    }

    #[test]
    fn sqlite_function_completion_uses_sqlite_function_list() {
        let items = function_completion_items_for(SQLITE_COMPLETION_FUNCTIONS, "strf");

        assert!(items.iter().any(|item| item.label == "STRFTIME"));
        assert!(items.iter().all(|item| item.label != "DATE_FORMAT"));
    }

    #[test]
    fn table_completion_orders_better_matches_first() {
        let items = table_completion_items(
            vec![
                CompletionTable {
                    database: None,
                    schema: None,
                    name: "shining_factory_type".to_string(),
                    kind: ObjectKind::Table,
                },
                CompletionTable {
                    database: None,
                    schema: None,
                    name: "type_config".to_string(),
                    kind: ObjectKind::Table,
                },
                CompletionTable {
                    database: None,
                    schema: None,
                    name: "shining_dental_order".to_string(),
                    kind: ObjectKind::Table,
                },
            ],
            "type",
        );

        assert_eq!(items[0].insert_text, "type_config");
        assert_eq!(items[1].insert_text, "shining_factory_type");
    }

    #[test]
    fn query_completion_suggests_columns_for_alias() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select p. from Product p".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select p.".len(),
            explicit: true,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(
            result.items.iter().any(|item| {
                item.kind == QueryCompletionKind::Column && item.insert_text == "id"
            })
        );
    }

    #[test]
    fn query_completion_for_alias_dot_does_not_mix_keywords() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select u. from 3d_user u".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select u.".len(),
            explicit: true,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Column && item.insert_text == "id"
        }));
        assert!(result.items.iter().all(|item| {
            !matches!(
                item.kind,
                QueryCompletionKind::Keyword | QueryCompletionKind::Function
            )
        }));
    }

    #[test]
    fn query_completion_suggests_columns_for_qualified_alias() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from main.Product as p where p.".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from main.Product as p where p.".len(),
            explicit: true,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(
            result.items.iter().any(|item| {
                item.kind == QueryCompletionKind::Column && item.insert_text == "id"
            })
        );
    }

    #[test]
    fn query_completion_fuzzy_matches_columns_for_alias() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select p.nm from Product p".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select p.nm".len(),
            explicit: true,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Column && item.insert_text == "name"
        }));
    }

    #[test]
    fn query_completion_suggests_columns_after_condition_operator() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product where id = 1 and na".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from Product where id = 1 and na".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Column && item.insert_text == "name"
        }));
    }

    #[test]
    fn mysql_completion_context_suggests_columns_after_expression_operator() {
        let equality = "select * from Product where id = na";
        let context = sql_completion_context(equality, equality.len(), DatabaseKind::MySql);
        assert!(context.suggest_columns);
        assert_eq!(context.prefix, "na");

        let comma = "select id, na from Product";
        let context = sql_completion_context(comma, "select id, na".len(), DatabaseKind::MySql);
        assert!(context.suggest_columns);
        assert_eq!(context.prefix, "na");
    }

    #[test]
    fn mysql_completion_context_does_not_scan_columns_after_projection_star() {
        let projection = "select * f";
        let context = sql_completion_context(projection, projection.len(), DatabaseKind::MySql);
        assert_eq!(context.expectation, CompletionExpectation::FromClause);
        assert!(!context.suggest_columns);

        let multiplication = "select price * f";
        let context =
            sql_completion_context(multiplication, multiplication.len(), DatabaseKind::MySql);
        assert!(context.suggest_columns);
    }

    #[test]
    fn query_completion_select_projection_prefers_from_keyword() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * f".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * f".len(),
            explicit: false,
        });
        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Keyword && item.insert_text == "FROM"
        }));
        assert!(result.items.iter().all(|item| {
            !matches!(
                item.kind,
                QueryCompletionKind::Column
                    | QueryCompletionKind::Table
                    | QueryCompletionKind::Function
            )
        }));
    }

    #[test]
    fn query_completion_suggests_database_columns_without_from() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select na".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select na".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Column
                && item.insert_text == "name"
                && item.detail.as_deref().is_some_and(|detail| detail.contains("Product"))
        }));
    }

    #[test]
    fn query_completion_can_disable_database_wide_column_index() {
        let mut controller = AppController::with_mock_data();
        let mut settings = controller.state().settings.clone();
        settings.enable_completion_index = false;
        controller.dispatch(AppCommand::SaveSettings(settings));
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select na".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select na".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().all(|item| {
            !(item.kind == QueryCompletionKind::Column && item.insert_text == "name")
        }));
    }

    #[test]
    fn cancelled_query_completion_skips_metadata_reads() {
        let controller = AppController::with_mock_data();
        let latest_request = Arc::new(std::sync::atomic::AtomicU64::new(2));
        let result = controller
            .query_completions_for_text_with_cancel(
                ConnectionId(1),
                Some("main".to_string()),
                "select na".to_string(),
                "select na".len(),
                false,
                latest_request,
                1,
            )
            .expect("cancelled completion should return partial local result");
        assert!(result.items.iter().all(|item| {
            !(item.kind == QueryCompletionKind::Column && item.insert_text == "name")
        }));
    }


    #[test]
    fn warm_completion_index_populates_database_wide_columns() {
        let mut controller = AppController::with_mock_data();

        let event = controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        assert_eq!(
            event,
            AppEvent::CompletionIndexWarmed(ConnectionId(1), Some("main".to_string()))
        );
        let index = controller.completion_index.lock().unwrap();
        let columns = index.database_columns(ConnectionId(1), Some("main"), None, "na");
        assert!(columns.iter().any(|column| column.name == "name"));
    }

    #[test]
    fn warm_completion_index_respects_disabled_setting() {
        let mut controller = AppController::with_mock_data();
        let mut settings = controller.state().settings.clone();
        settings.enable_completion_index = false;
        controller.dispatch(AppCommand::SaveSettings(settings));

        let event = controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        assert_eq!(
            event,
            AppEvent::CompletionIndexWarmed(ConnectionId(1), Some("main".to_string()))
        );
        let index = controller.completion_index.lock().unwrap();
        assert!(!index.has_database_index(ConnectionId(1), Some("main"), None));
    }

    #[test]
    fn query_completion_loads_persisted_index_snapshot() {
        let storage_root = temp_sqlite_path("completion-index-storage").with_extension("cache");
        let storage = fluxdb_storage::FileStorage::new(storage_root);
        let mut first = AppController::with_mock_data();
        first.set_completion_index_storage(storage.clone());
        first.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        let mut second = AppController::with_mock_data();
        second.set_completion_index_storage(storage);
        second.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        second.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select na".to_string(),
        });

        let event = second.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select na".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Column && item.insert_text == "name"
        }));
        let index = second.completion_index.lock().unwrap();
        assert!(index.has_database_index(ConnectionId(1), Some("main"), None));
    }

    #[test]
    fn completion_index_version_mismatch_safely_invalidates_snapshot() {
        let controller = AppController::with_mock_data();
        let table = TableRef {
            database: Some("main".to_string()),
            schema: None,
            name: "customer".to_string(),
            kind: ObjectKind::Table,
            rows: Some(1),
            comment: None,
        };
        let meta = CompletionIndexMeta {
            app_index_version: COMPLETION_INDEX_VERSION,
            db_kind: DatabaseKind::MySql,
            last_indexed_at: 0,
            last_verified_at: 0,
            ttl_seconds: 3600,
            dirty: false,
            table_count: 1,
            table_fingerprints: Vec::new(),
        };
        let snapshot = CompletionIndexSnapshot {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            tables: vec![table],
            columns: Vec::new(),
            routines: Vec::new(),
            triggers: Vec::new(),
            meta,
        };

        // 版本升级后写回的旧快照：版本不匹配应被安全拒绝，不加载任何数据
        let mut upgraded = snapshot.clone();
        upgraded.meta.app_index_version = COMPLETION_INDEX_VERSION + 1;
        {
            let mut index = controller.completion_index.lock().unwrap();
            index.insert_snapshot(upgraded);
            assert!(!index.has_database_index(ConnectionId(1), Some("main"), None));
        }

        // 当前版本快照：可正常加载并读取其中表数据
        {
            let mut index = controller.completion_index.lock().unwrap();
            index.insert_snapshot(snapshot);
            assert!(index.has_database_index(ConnectionId(1), Some("main"), None));
        }
    }

    #[test]
    fn ddl_marks_target_table_dirty_and_warmup_refreshes_it() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "alter table Product add column sku text".to_string(),
        });
        controller.dispatch(AppCommand::ExecuteQuery(TabId(1)));

        {
            let index = controller.completion_index.lock().unwrap();
            assert_eq!(
                index.dirty_table_names(ConnectionId(1), Some("main"), None),
                vec!["product".to_string()]
            );
            assert!(index.is_dirty_or_expired(
                ConnectionId(1),
                Some("main"),
                None,
                unix_timestamp_secs()
            ));
        }

        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        let index = controller.completion_index.lock().unwrap();
        assert!(index
            .dirty_table_names(ConnectionId(1), Some("main"), None)
            .is_empty());
        assert!(!index.is_dirty_or_expired(
            ConnectionId(1),
            Some("main"),
            None,
            unix_timestamp_secs()
        ));
    }

    #[test]
    fn background_refresh_worker_replaces_stale_columns_and_clears_dirty() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        // 直接将该库标记为 dirty（模拟 DDL 后的 stale 场景），再走后台 worker 刷新
        {
            let mut index = controller.completion_index.lock().unwrap();
            index.mark_table_dirty(ConnectionId(1), Some("main"), None, "product");
            assert!(index.is_dirty_or_expired(
                ConnectionId(1),
                Some("main"),
                None,
                unix_timestamp_secs()
            ));
        }

        // 直接调后台 worker：在线拉取列写回索引
        let config = controller
            .connection_config(ConnectionId(1))
            .expect("connection")
            .clone();
        let index = Arc::clone(&controller.completion_index);
        let (tables, columns) =
            refresh_index_columns_in_background(&index, &config, ConnectionId(1), Some("main"), None)
                .expect("background refresh should succeed");
        assert!(tables > 0);
        assert!(columns > 0);

        // 刷新后索引转 fresh（stale 标记清除），旧候选已被替换
        let index = controller.completion_index.lock().unwrap();
        assert!(!index.is_dirty_or_expired(
            ConnectionId(1),
            Some("main"),
            None,
            unix_timestamp_secs()
        ));
    }

    #[test]
    fn background_refresh_guard_dedupes_concurrent_triggers() {
        let controller = AppController::with_mock_data();
        let mut index = controller.completion_index.lock().unwrap();
        let key = CompletionIndex::db_key(ConnectionId(1), Some("main"), None);
        assert!(index.begin_refresh(key.clone())); // 首次触发：登记成功
        assert!(!index.begin_refresh(key.clone())); // 并发触发：去重，不再 spawn
        index.end_refresh(&key);
        assert!(index.begin_refresh(key.clone())); // 结束后可再次触发
        index.end_refresh(&key);
    }

    #[test]
    fn sql_ddl_impact_extracts_common_table_names() {
        // 表级 DDL → 命中具体表
        assert_eq!(
            sql_ddl_impact("alter table Product add column sku text").unwrap().tables,
            BTreeSet::from(["product".to_string()])
        );
        assert_eq!(
            sql_ddl_impact("drop table Product").unwrap().tables,
            BTreeSet::from(["product".to_string()])
        );
        assert_eq!(
            sql_ddl_impact("create table Orders (id int)").unwrap().tables,
            BTreeSet::from(["orders".to_string()])
        );
        assert_eq!(
            sql_ddl_impact("truncate table Product").unwrap().tables,
            BTreeSet::from(["product".to_string()])
        );
        assert_eq!(
            sql_ddl_impact("create index idx_name on Customer(name)").unwrap().tables,
            BTreeSet::from(["customer".to_string()])
        );
        let rename = sql_ddl_impact("rename table Product to ProductArchive").unwrap();
        assert_eq!(
            rename.tables,
            BTreeSet::from(["product".to_string(), "productarchive".to_string()])
        );

        // 库级 DDL → database_wide（不落到具体表）
        for sql in [
            "create database analytics",
            "drop database analytics",
            "alter database analytics",
            "create schema public",
            "drop schema public",
        ] {
            let impact = sql_ddl_impact(sql).expect("ddl impact");
            assert!(impact.database_wide, "应库级失效: {sql}");
            assert!(impact.tables.is_empty(), "库级不应有表: {sql}");
        }
    }

    #[test]
    fn background_refresh_follows_table_vs_database_dirty_scope() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        let config = controller
            .connection_config(ConnectionId(1))
            .expect("connection")
            .clone();
        let index = Arc::clone(&controller.completion_index);

        // 表级 dirty：只刷新该表（其余表不碰）
        {
            let mut guard = controller.completion_index.lock().unwrap();
            guard.mark_table_dirty(ConnectionId(1), Some("main"), None, "product");
        }
        let (tables, _columns) =
            refresh_index_columns_in_background(&index, &config, ConnectionId(1), Some("main"), None)
                .expect("table-scope refresh");
        assert_eq!(tables, 1, "表级 dirty 只应刷新 product 一张表");
        {
            let guard = controller.completion_index.lock().unwrap();
            assert!(!guard.is_dirty_or_expired(
                ConnectionId(1),
                Some("main"),
                None,
                unix_timestamp_secs()
            ));
        }

        // 库级 dirty：刷新整库（多张表）
        {
            let mut guard = controller.completion_index.lock().unwrap();
            guard.mark_dirty(ConnectionId(1), Some("main"), None);
        }
        let (tables, _columns) =
            refresh_index_columns_in_background(&index, &config, ConnectionId(1), Some("main"), None)
                .expect("db-scope refresh");
        assert!(tables > 1, "库级 dirty 应刷新整库, got {tables}");
    }

    #[test]
    fn background_refresh_batches_large_schema_columns_in_single_call() {
        // T053：大 schema（200 张表）走后台 worker 刷新时，worker 以一次批量列查询覆盖整库，
        // 而非每表一次查询（N+1）。对比：批量 1 次 vs 逐表 N 次远程查询。
        const N: usize = 200;
        let controller = AppController::with_mock_data();
        let now = unix_timestamp_secs();
        let snapshot = CompletionIndexSnapshot {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
            schema: None,
            tables: (0..N)
                .map(|i| TableRef {
                    database: Some("main".to_string()),
                    schema: None,
                    name: format!("big_table_{i}"),
                    kind: ObjectKind::Table,
                    rows: None,
                    comment: None,
                })
                .collect(),
            columns: Vec::new(),
            routines: Vec::new(),
            triggers: Vec::new(),
            meta: CompletionIndexMeta {
                app_index_version: COMPLETION_INDEX_VERSION,
                db_kind: DatabaseKind::Sqlite,
                last_indexed_at: now,
                last_verified_at: now,
                ttl_seconds: COMPLETION_INDEX_TTL_SECONDS,
                dirty: false,
                table_count: N,
                table_fingerprints: Vec::new(),
            },
        };
        {
            let mut index = controller.completion_index.lock().unwrap();
            index.insert_snapshot(snapshot);
            index.mark_dirty(ConnectionId(1), Some("main"), None);
        }

        let config = controller
            .connection_config(ConnectionId(1))
            .expect("connection")
            .clone();
        let index = Arc::clone(&controller.completion_index);
        let (tables, columns) =
            refresh_index_columns_in_background(&index, &config, ConnectionId(1), Some("main"), None)
                .expect("large-schema refresh");

        // 单次批量调用覆盖全部 N 张表（worker 对批量列 fetch 仅一次调用）
        assert_eq!(tables, N, "整库刷新应覆盖全部 {N} 张表, got {tables}");
        assert!(columns >= N, "每表至少一列, got {columns}");
        {
            let guard = controller.completion_index.lock().unwrap();
            assert!(!guard.is_dirty_or_expired(
                ConnectionId(1),
                Some("main"),
                None,
                unix_timestamp_secs()
            ));
            // 批量写入真正落库：首尾大表均可查询到列
            assert!(
                !guard
                    .table_columns(ConnectionId(1), Some("main"), None, "big_table_0")
                    .is_empty(),
                "big_table_0 应已写入列"
            );
            assert!(
                !guard
                    .table_columns(ConnectionId(1), Some("main"), None, "big_table_199")
                    .is_empty(),
                "big_table_199 应已写入列"
            );
        }
    }

    #[test]
    fn stale_request_serves_candidates_and_registers_background_refresh() {
        // T055 验收 2：stale 数据可用。索引被标记 dirty/过期后，请求路径不阻塞、
        // 仍用旧候选返回补全（stale 可用），同时登记后台刷新（begin_refresh）。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });

        // 模拟 DDL 后的 stale：直接标记库级 dirty。
        {
            let mut index = controller.completion_index.lock().unwrap();
            index.mark_dirty(ConnectionId(1), Some("main"), None);
            assert!(index.is_dirty_or_expired(
                ConnectionId(1),
                Some("main"),
                None,
                unix_timestamp_secs()
            ));
        }

        // 请求路径：stale 时仍返回候选（不阻塞等待刷新完成），而非清空候选。
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Pro".to_string(),
        });
        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from Pro".len(),
            explicit: false,
        });
        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("stale 请求应仍返回候选");
        };
        assert!(
            result.items.iter().any(|item| item.kind == QueryCompletionKind::Table && item.label == "Product"),
            "stale 时仍应给出旧索引的 Product 表候选"
        );
        // refresh 登记/去重机制由 background_refresh_guard_dedupes_concurrent_triggers 单独锁定；
        // 后台线程刷完即清 refresh_inflight（此处为竞态，不在此断言）。验收 = stale 可用、请求不阻塞。
    }

    #[test]
    fn completion_items_do_not_fetch_documentation_per_item() {
        // T054 验收 1：显示候选不触发每项 documentation 查询。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Pro".to_string(),
        });
        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from Pro".len(),
            explicit: false,
        });
        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        // 候选的 documentation 仅来自索引内联 comment（列项）或 None（表项），
        // 没有触发任何逐项远程/额外解析。
        for item in &result.items {
            match item.kind {
                QueryCompletionKind::Column => {
                    // 列项带内联注释（索引里的 column.comment），不是远程抓取。
                    assert!(
                        item.documentation.is_some(),
                        "列项应携带索引内联注释: {}",
                        item.label
                    );
                }
                QueryCompletionKind::Table | QueryCompletionKind::View => {
                    assert!(
                        item.documentation.is_none(),
                        "表/视图候选不应在首屏携带需抓取的 doc: {}",
                        item.label
                    );
                }
                _ => {}
            }
        }
        // 候选读取纯本地：不应触发新的后台刷新登记。
        let index = controller.completion_index.lock().unwrap();
        assert!(index.refresh_inflight.is_empty(), "候选读取不应触发后台刷新");
    }

    #[test]
    fn lazy_completion_documentation_resolves_structure_or_error() {
        // T054 验收 2：选中项详情懒加载，数据全部来自索引（无远程查询），
        // 对象不在索引 / 无可用文档时返回 Error。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        let item = |label: &str, kind: QueryCompletionKind, docs: Option<&str>| QueryCompletionItem {
            label: label.to_string(),
            insert_text: label.to_string(),
            kind,
            detail: None,
            documentation: docs.map(str::to_string),
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};

        // 表 → Ready 结构元数据（列清单），包含列名、类型与列注释。
        let doc = controller.completion_item_documentation(
            ConnectionId(1),
            Some("main"),
            &item("Product", QueryCompletionKind::Table, None),
            &|| false,
        );
        match doc {
            CompletionDocumentationState::Ready(text) => {
                assert!(text.contains("id"), "应含列 id: {text}");
                assert!(text.contains("INTEGER"), "应含类型 INTEGER: {text}");
                assert!(text.contains("name"), "应含列 name: {text}");
                assert!(
                    text.contains("Product display name"),
                    "应含列注释: {text}"
                );
            }
            other => panic!("表项应解析为 Ready, got {other:?}"),
        }

        // 索引中不存在的表 → Error。
        assert!(matches!(
            controller.completion_item_documentation(
                ConnectionId(1),
                Some("main"),
                &item("NopeMissing", QueryCompletionKind::Table, None),
                &|| false
            ),
            CompletionDocumentationState::Error(_)
        ));

        // 列 → Ready 内联注释；无注释 → Error、不依赖索引。
        assert!(matches!(
            controller.completion_item_documentation(
                ConnectionId(1),
                Some("main"),
                &item("cust", QueryCompletionKind::Column, Some("客户编号")),
                &|| false
            ),
            CompletionDocumentationState::Ready(text) if text == "客户编号"
        ));
        assert!(matches!(
            controller.completion_item_documentation(
                ConnectionId(1),
                Some("main"),
                &item("cust", QueryCompletionKind::Column, None),
                &|| false
            ),
            CompletionDocumentationState::Error(_)
        ));

        // 函数/过程 → Ready 名称。
        assert!(matches!(
            controller.completion_item_documentation(
                ConnectionId(1),
                Some("main"),
                &item("normalize_price", QueryCompletionKind::Function, None),
                &|| false
            ),
            CompletionDocumentationState::Ready(name) if name == "normalize_price"
        ));

        // 关键字等无结构元数据 → Error。
        assert!(matches!(
            controller.completion_item_documentation(
                ConnectionId(1),
                Some("main"),
                &item("select", QueryCompletionKind::Keyword, None),
                &|| false
            ),
            CompletionDocumentationState::Error(_)
        ));
    }

    #[test]
    fn completion_documentation_for_resolves_by_kind_and_label() {
        // F005 验收：desktop 公开入口 `completion_documentation_for` 按轻量参数
        // （连接/库/类型/标签）解析选中项说明文档，语义与内部实现一致。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        let doc = |kind, label: &str| {
            controller.completion_documentation_for(
                ConnectionId(1),
                Some("main".to_string()),
                kind,
                label.to_string(),
                None,
            )
        };
        // 列注释走 `comment` 参数（候选补全时可得的自带来内联注释）。
        let doc_comment = |kind, label: &str, comment: &str| {
            controller.completion_documentation_for(
                ConnectionId(1),
                Some("main".to_string()),
                kind,
                label.to_string(),
                Some(comment.to_string()),
            )
        };

        // 表 → Ready 结构元数据（列清单）。
        match doc(QueryCompletionKind::Table, "Product") {
            CompletionDocumentationState::Ready(text) => {
                assert!(text.contains("id"), "应含列 id: {text}");
                assert!(text.contains("Product display name"), "应含列注释: {text}");
            }
            other => panic!("表项应解析为 Ready, got {other:?}"),
        }

        // 索引中没有的表 → Error。
        assert!(matches!(
            doc(QueryCompletionKind::Table, "NopeMissing"),
            CompletionDocumentationState::Error(_)
        ));

        // 列 → Ready 内联注释（注释由 comment 参数带入）。
        assert!(matches!(
            doc_comment(QueryCompletionKind::Column, "cust", "客户编号"),
            CompletionDocumentationState::Ready(text) if text == "客户编号"
        ));
        // 列但无注释 → Error。
        assert!(matches!(
            doc(QueryCompletionKind::Column, "cust"),
            CompletionDocumentationState::Error(_)
        ));

        // 函数 → Ready 名称。
        assert!(matches!(
            doc(QueryCompletionKind::Function, "normalize_price"),
            CompletionDocumentationState::Ready(name) if name == "normalize_price"
        ));

        // 关键字等无结构元数据 → Error。
        assert!(matches!(
            doc(QueryCompletionKind::Keyword, "select"),
            CompletionDocumentationState::Error(_)
        ));
    }

    #[test]
    fn completion_documentation_with_cancel_returns_loading_on_latest_wins() {
        // latest-wins 取消：`should_cancel` 为真时旧详情请求提前返回 Loading，
        // 保证切换选中项时旧结果不覆盖新选择（选中项切换后由 UI 复位为新请求）。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        // 恒真取消回调 → 即使对象存在也返回 Loading（未完成不产出 Ready）。
        assert!(matches!(
            controller.completion_documentation_for_with_cancel(
                ConnectionId(1),
                Some("main".to_string()),
                QueryCompletionKind::Table,
                "Product".to_string(),
                None,
                &|| true,
            ),
            CompletionDocumentationState::Loading
        ));
        // 恒假 → 正常 Ready，与取消关闭时行为一致。
        assert!(matches!(
            controller.completion_documentation_for_with_cancel(
                ConnectionId(1),
                Some("main".to_string()),
                QueryCompletionKind::Table,
                "Product".to_string(),
                None,
                &|| false,
            ),
            CompletionDocumentationState::Ready(_)
        ));
    }

    // T010 场景矩阵的通用请求装配：warm 索引 + 打开编辑器 + 输入 SQL + 请求补全。
    // 返回完整的全局排序后候选。cursor 默认在文本末尾。
    fn matrix_request_items(sql: &str, cursor: Option<usize>) -> Vec<QueryCompletionItem> {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: sql.to_string(),
        });
        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: cursor.unwrap_or_else(|| sql.len()),
            explicit: false,
        });
        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions for: {sql:?}");
        };
        result.items
    }

    #[test]
    fn scenario_matrix_statement_head_keywords_and_snippets() {
        // 语句头：关键字 + snippet 正例；prefix 仅命中两者之一时不混入无关表/列。
        let items = matrix_request_items("sel", None);
        assert!(
            items.iter().any(|item| item.label.eq_ignore_ascii_case("select")),
            "语句头应给出 select 关键字"
        );
        // 映射到真实 mock 无相关关键字时，至少不出无关表/列噪音。
        assert!(
            items.iter().all(|item| matches!(
                item.kind,
                QueryCompletionKind::Keyword | QueryCompletionKind::Snippet
            )),
            "语句头高置信度不应返回无关表/列/函数"
        );
    }

    #[test]
    fn scenario_matrix_from_join_tables_views_and_schema() {
        // FROM 后给出当前库 schema + 表/视图候选。
        let items = matrix_request_items("select * from ", None);
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Schema && item.label == "main"),
            "FROM 后应给出当前库 schema main"
        );
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Table && item.label == "Product"),
            "FROM 后应给出 Product 表"
        );
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Table && item.label == "ProductCategory"),
            "FROM 后应给出 ProductCategory 表"
        );
        // 表上下文中不应混入列候选（列依赖具体表，空前缀 FROM 无列语义）。
        assert!(
            items.iter().all(|item| item.kind != QueryCompletionKind::Column),
            "FROM 空前缀不应返回列候选"
        );

        // 已引用表 + qualifier：`t.` 限定列；qualifier 对应已引用别名则走列，不返表。
        let qualified = matrix_request_items("select t. from Product t", Some(8));
        assert!(
            qualified.iter().any(|item| item.kind == QueryCompletionKind::Column && item.label == "id"),
            "限定已引用表应给出 id 列"
        );
        assert!(
            qualified.iter().all(|item| item.kind != QueryCompletionKind::Table),
            "限定已引用表后不应再返表候选"
        );
    }

    #[test]
    fn scenario_matrix_columns_qualified_and_alias() {
        // 表后 `.` 限定列：给出该表列候选；无关函数/关键字不入混。
        let items = matrix_request_items("select p. from Product p", Some(8));
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Column && item.label == "name"),
            "限定列应给出 name"
        );
        assert!(
            items.iter().any(|item| item.kind == QueryCompletionKind::Column && item.label == "id"),
            "限定列应给出 id"
        );
    }

    #[test]
    fn scenario_matrix_clause_next_constructs() {
        // 表后给出 alias/后续构造（下一动作 token），不混入无关列。
        let items = matrix_request_items("select * from Product ", None);
        assert!(
            items.iter().any(|item| item.label.eq_ignore_ascii_case("as")),
            "表后应给出 alias 关键字 as"
        );
    }

    #[test]
    fn scenario_matrix_predicate_operator_and_value() {
        // `col `（操作数后）为操作符上下文：给出比较操作符，不返函数/列实体候选。
        let op_items = matrix_request_items("select * from Product where id ", None);
        assert!(
            op_items.iter().any(|item| item.kind == QueryCompletionKind::Keyword && item.label == "="),
            "操作数后应给出比较操作符 ="
        );
        assert!(
            op_items.iter().all(|item| item.kind != QueryCompletionKind::Column),
            "比较操作符上下文不应返回列候选"
        );

        // `col =` 后为值上下文：给出 NULL/布尔字面量与内置函数/被引用列，而非返回更多表。
        let value_items = matrix_request_items("select * from Product where id = ", None);
        assert!(
            value_items.iter().any(|item| item.kind == QueryCompletionKind::Keyword && item.label == "NULL"),
            "等号后应给出 NULL 值候选"
        );
        assert!(
            value_items.iter().any(|item| item.kind == QueryCompletionKind::Function),
            "等号后应给出函数候选"
        );
        assert!(
            value_items.iter().all(|item| item.kind != QueryCompletionKind::Table),
            "值上下文不应返回表候选"
        );
    }

    #[test]
    fn scenario_matrix_insert_update_delete() {
        // INSERT 目标列；函数/过程候选在调用上下文给出。
        let insert_items = matrix_request_items("insert into Product(", None);
        assert!(
            insert_items.iter().any(|item| item.kind == QueryCompletionKind::Column && item.label == "name"),
            "INSERT 应给出目标列 name"
        );

        let fn_items = matrix_request_items("select normal", None);
        assert!(
            fn_items.iter().any(|item| item.kind == QueryCompletionKind::Function && item.label == "normalize_price"),
            "函数前缀应给出 normalize_price"
        );
    }

    #[test]
    fn scenario_matrix_star_and_routine_snippet() {
        // 语句头 snippet 前缀。
        let snip_items = matrix_request_items("se", None);
        assert!(
            snip_items.iter().any(|item| item.kind == QueryCompletionKind::Snippet),
            "snippet 前缀应给出 snippet 候选"
        );
        // 触发器候选仅在 DDL 触发器上下文（DROP TRIGGER）给出，非 SELECT 从句。
        let trig_items = matrix_request_items("drop trigger product_a", None);
        assert!(
            trig_items.iter().any(|item| item.kind == QueryCompletionKind::Trigger && item.label == "product_ai"),
            "DROP TRIGGER 上下文应给出 product_ai"
        );
    }

    #[test]
    fn scenario_matrix_star_expansion_only_expands_target_relation() {
        // 限定星号 `c.*`：多引用表（JOIN）下只展开目标 relation（ProductCategory），
        // 不掺入另一引用表 Product 的列。注意用 JOIN 而非逗号连接——逗号连接的
        // 多表在当前 parser 里会被合并成一张带别名的表。
        let sql = "select c.* from Product p join ProductCategory c on p.id = c.id";
        let star_at = sql.find('*').unwrap() + 1;
        let items = matrix_request_items(sql, Some(star_at));
        let star = items
            .iter()
            .find(|item| item.kind == QueryCompletionKind::Snippet && item.filter_text.as_deref() == Some("*"))
            .expect("星号展开候选应存在");
        assert_eq!(star.label, "展开所有列 (*)");
        assert_eq!(star.insert_text, "c.id, c.name, c.sort_order");
        assert!(
            !star.insert_text.contains("p."),
            "限定 c.* 不应展开其它引用表 p 的列"
        );
    }

    #[test]
    fn scenario_matrix_star_expansion_bare_star_dedups_all_tables() {
        // 裸星号：取全部引用表（Product 与 ProductCategory），跨表按列名去重
        // （两表同为 id/name → 只出一份，前缀取首个目标表的别名）。
        let sql = "select * from Product p join ProductCategory c on p.id = c.id";
        let star_at = sql.find('*').unwrap() + 1;
        let items = matrix_request_items(sql, Some(star_at));
        let star = items
            .iter()
            .find(|item| item.kind == QueryCompletionKind::Snippet && item.filter_text.as_deref() == Some("*"))
            .expect("星号展开候选应存在");
        assert_eq!(star.insert_text, "p.id, p.name, p.category_id, p.price, p.active, p.created_at, c.sort_order");
        assert_eq!(star.detail.as_deref(), Some("7 列"));
    }

    #[test]
    fn scenario_matrix_star_expansion_is_suggestion_not_autoedit() {
        // 展开候选是「建议」，不自动改写用户文本：replace 区间仅覆盖星号本身，
        // 而非整个 SELECT 子句；替换发生在用户显式接受候选之后。
        let sql = "select c.* from Product c";
        let star_at = sql.find('*').unwrap();
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: sql.to_string(),
        });
        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: star_at + 1,
            explicit: false,
        });
        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        let star = result
            .items
            .iter()
            .find(|item| item.kind == QueryCompletionKind::Snippet && item.filter_text.as_deref() == Some("*"))
            .expect("星号展开候选应存在");
        // 只覆盖 `*` 附近的替换区间，其余用户文本（select / from / alias）原样保留。
        assert!(result.replace_start >= star_at, "不应从更早位置开始改写用户文本");
        assert!(result.replace_end >= star_at);
        assert_eq!(star.kind, QueryCompletionKind::Snippet);
    }

    #[test]
    fn sql_signature_nested_uses_innermost_call() {
        let sig = sql_signature_at("f(g(a, b, ", "f(g(a, b, ".len()).expect("应取最内层 g 调用");
        assert_eq!(sig.name, "g");
        assert_eq!(sig.active_parameter, 2);
    }

    #[test]
    fn sql_signature_schema_qualified_takes_last_segment() {
        let sql = "select sales.concat(a, b";
        let sig = sql_signature_at(sql, sql.len()).expect("应识别 sales.concat 调用");
        assert_eq!(sig.name, "concat");
        assert_eq!(sig.qualifier.as_deref(), Some("sales"));
        assert_eq!(sig.active_parameter, 1);
        // 无 schema 限定。
        let bare = "select concat(";
        let sig = sql_signature_at(bare, bare.len()).expect("应识别 bare concat");
        assert_eq!(sig.qualifier, None);
        assert_eq!(sig.active_parameter, 0);
    }

    #[test]
    fn sql_signature_multibyte_text_counts_params() {
        // 中文字符与 emoji 属多字节文本；字符串内逗号不计入 active 参数。
        let sql = "select concat('中文, 逗号', '😀,x', ";
        let sig = sql_signature_at(sql, sql.len()).expect("多字节文本下应识别 concat 调用");
        assert_eq!(sig.name, "concat");
        assert_eq!(sig.active_parameter, 2);
    }

    #[test]
    fn sql_signature_ignores_commas_inside_strings() {
        // 单双引号与反引号字符串内逗号不计；相邻重复引号作为转义不结束字符串。
        let mixed = "f('a,b', \"c,d\", ";
        assert_eq!(sql_signature_at(mixed, mixed.len()).map(|s| s.active_parameter), Some(2));
        assert_eq!(sql_signature_at("f('it''s, x', b", "f('it''s, x', b".len()).map(|s| s.active_parameter), Some(1));
        // 嵌套括号内逗号不计。
        assert_eq!(sql_signature_at("f(g(a, b), c", "f(g(a, b), c".len()).map(|s| s.active_parameter), Some(1));
    }

    #[test]
    fn sql_signature_non_identifier_before_paren_is_none() {
        // 控制流 `if (` 左侧不紧邻标识符 → 不判定为函数调用。
        assert!(sql_signature_at("if (x, y", "if (x, y".len()).is_none());
        // 光标不在括号内。
        assert!(sql_signature_at("f()", 3).is_none());
        assert!(sql_signature_at("", 0).is_none());
    }

    #[test]
    fn scenario_matrix_metadata_missing_is_graceful() {
        // metadata 缺失/失败：请求不 panic，候选为空或保守降级（含 Unknown intent 不崩）。
        let items = matrix_request_items("select * from NonExistentTable wh", None);
        // 此处表不存在 → 仍然返回某类候选或空，但绝不 panic；列不应因不存在的表而出现。
        assert!(
            items.iter().all(|item| item.kind != QueryCompletionKind::Column),
            "不存在表的列不应出现"
        );
    }

    #[test]
    fn completion_index_matches_token_prefixes() {
        let mut index = CompletionIndex::default();
        index.replace_table_columns(
            ConnectionId(1),
            Some("main"),
            None,
            "Product",
            vec![CompletionColumn {
                table: "Product".to_string(),
                name: "customer_name".to_string(),
                type_name: Some("TEXT".to_string()),
                nullable: false,
                primary_key: false,
                comment: None,
            }],
            DatabaseKind::Sqlite,
        );

        let columns = index.database_columns(ConnectionId(1), Some("main"), None, "na");

        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].name, "customer_name");
    }

    #[test]
    fn query_completion_does_not_pull_remote_routines_for_plain_keyword_input() {
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select norm".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select norm".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
            panic!("expected query completions");
        };
        assert!(result.items.iter().all(|item| item.insert_text != "normalize_price"));
        assert!(result.items.iter().all(|item| item.insert_text != "refresh_product"));
    }

    #[test]
    fn column_completion_orders_better_matches_first() {
        let items = column_completion_items(
            vec![
                CompletionColumn {
                    table: "Product".to_string(),
                    name: "product_name".to_string(),
                    type_name: Some("TEXT".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
                CompletionColumn {
                    table: "Product".to_string(),
                    name: "name".to_string(),
                    type_name: Some("TEXT".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                CompletionColumn {
                    table: "Product".to_string(),
                    name: "display_name".to_string(),
                    type_name: Some("TEXT".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
            ],
            "name",
        );

        assert_eq!(items[0].insert_text, "name");
        assert_eq!(items[1].insert_text, "display_name");
    }

    #[test]
    fn function_completion_displays_name_without_parentheses() {
        let items = function_completion_items("co");

        assert_eq!(items[0].label, "COUNT");
        assert_eq!(items[0].insert_text, "COUNT()");
    }

    #[test]
    fn completion_cache_clears_for_object_refresh_commands() {
        assert!(should_clear_completion_cache(&AppCommand::OpenConnection(ConnectionId(1))));
        assert!(should_clear_completion_cache(&AppCommand::LoadObjectChildren(
            ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".to_string()),
                schema: None,
                name: "Product".to_string(),
                kind: ObjectKind::Table,
            }
        )));
        assert!(should_clear_completion_cache(&AppCommand::RefreshObject(None)));
        assert!(should_clear_completion_cache(&AppCommand::ExecuteQuery(TabId(1))));
    }

    #[test]
    fn completion_cache_entry_expires_after_ttl() {
        let entry = CompletionCacheEntry {
            value: Vec::<CompletionTable>::new(),
            fetched_at: 100,
        };
        assert!(entry.is_fresh(100 + COMPLETION_INDEX_TTL_SECONDS));
        assert!(!entry.is_fresh(101 + COMPLETION_INDEX_TTL_SECONDS));
    }

    #[test]
    fn query_completion_suggests_routines_and_triggers_in_specific_contexts() {
        let mut controller = AppController::with_mock_data();
        let mut config = controller.state().connections[0].config.clone();
        config.kind = DatabaseKind::MySql;
        controller.dispatch(AppCommand::UpdateConnection(config));
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "call refresh".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "call refresh".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(_, _, result) = event else {
            panic!("expected procedure completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Procedure && item.insert_text == "refresh_product"
        }));

        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "drop trigger product".to_string(),
        });
        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "drop trigger product".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(_, _, result) = event else {
            panic!("expected trigger completions");
        };
        assert!(result.items.iter().any(|item| {
            item.kind == QueryCompletionKind::Trigger && item.insert_text == "product_ai"
        }));
    }

    // ---------- T063 类型感知排序 ----------

    #[test]
    fn comparison_left_operand_matches_qualified_and_bare_columns() {
        // 限定列：`WHERE p.id = |` → Some((Some("p"), "id"))。
        assert_eq!(
            comparison_left_operand("select * from Product p where p.id = "),
            Some((Some("p".to_string()), "id".to_string()))
        );
        // 裸列：`WHERE name = |` → Some((None, "name"))。
        assert_eq!(
            comparison_left_operand("select * from Product where name = "),
            Some((None, "name".to_string()))
        );
        // 比较操作符变体 `<=` / `>=` / `!=`。
        assert_eq!(
            comparison_left_operand("select * from Product where id >= "),
            Some((None, "id".to_string()))
        );
        // 无比较操作符 → None（非比较上下文不参与类型提升）。
        assert_eq!(comparison_left_operand("select * from Product where "), None);
        // 左侧非列（如立即数）→ None。
        assert_eq!(comparison_left_operand("where 5 = "), None);
    }

    #[test]
    fn sql_type_family_groups_common_type_names() {
        use TypeFamily::*;
        assert_eq!(sql_type_family("INTEGER"), Integer);
        assert_eq!(sql_type_family("BIGINT"), Integer);
        assert_eq!(sql_type_family("DECIMAL(10,2)"), Numeric);
        assert_eq!(sql_type_family("VARCHAR(255)"), Text);
        assert_eq!(sql_type_family("TEXT"), Text);
        assert_eq!(sql_type_family("DATETIME"), DateTime);
        assert_eq!(sql_type_family("BOOLEAN"), Boolean);
        assert_eq!(sql_type_family("UNKNOWN_TYPE"), Other);
        assert_eq!(sql_type_family(""), Other);
    }

    #[test]
    fn type_family_compatible_interops_numeric_and_integer() {
        use TypeFamily::*;
        // 数值族互相兼容（INTEGER <-> NUMERIC）。
        assert!(type_family_compatible(Integer, Integer));
        assert!(type_family_compatible(Integer, Numeric));
        assert!(type_family_compatible(Numeric, Integer));
        assert!(type_family_compatible(Numeric, Numeric));
        // 同族兼容。
        assert!(type_family_compatible(Text, Text));
        // 跨族不兼容。
        assert!(!type_family_compatible(Integer, Text));
        assert!(!type_family_compatible(Text, DateTime));
        // Other 恒不兼容（无法判定类型不产生偏好）。
        assert!(!type_family_compatible(Integer, Other));
        assert!(!type_family_compatible(Other, Text));
    }

    fn item(label: &str, kind: QueryCompletionKind) -> QueryCompletionItem {
        QueryCompletionItem {
            label: label.into(),
            insert_text: label.into(),
            kind,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
}
    }

    #[test]
    fn rank_type_match_boosts_reorders_but_never_filters() {
        let context = sql_completion_context(
            "select * from Product where id = ",
            "select * from Product where id = ".len(),
            DatabaseKind::MySql,
        );
        let intent = context.intent;
        let expected = Vec::new();

        // 背景：两个「同属数值族」的候选前后声明；若启用类型提升，「数值匹配」的
        // 应越过「文本」，即使文本在后声明也能提前；关闭提升（type_match 恒 false）
        // 时保持稳定字母序。
        let numeric = item("price", QueryCompletionKind::Column);
        let text = item("name", QueryCompletionKind::Column);

        // 1) 有类型信息：匹配 numeric 的 column 提升到文本列之前。
        let boosted = globally_rank_completion_items(
            vec![text.clone(), numeric.clone()],
            expected.clone(),
            &intent,
            "",
            &|candidate| candidate.label == "price",
            &|_| 0,
        );
        assert_eq!(boosted[0].label, "price", "类型匹配列应前置");
        assert_eq!(boosted.len(), 2, "排序只重排，不得过滤候选");
        assert!(boosted.iter().any(|candidate| candidate.label == "name"));

        // 2) 无类型信息：type_match 恒 false，行为与之前一致（稳定字母序）。
        let baseline = globally_rank_completion_items(
            vec![text.clone(), numeric.clone()],
            expected.clone(),
            &intent,
            "",
            &|_| false,
            &|_| 0,
        );
        assert_eq!(baseline[0].label, "name", "无类型信息时不产生偏好");

        // 3) 提升只影响顺序；所有候选仍全部保留。
        let labels: Vec<&str> = boosted.iter().map(|candidate| candidate.label.as_str()).collect();
        assert!(labels.contains(&"price") && labels.contains(&"name"));
    }

    #[test]
    fn query_completion_type_boost_in_comparison_context() {
        // 端到端（controller）：`Product WHERE id = |` 时，参照列 id（INTEGER）与候选列
        // 类型族相比——Product 仅 id(INTEGER)/name(TEXT) 两列，id 与自身同族应排前。
        let mut controller = AppController::with_mock_data();
        let mut config = controller.state().connections[0].config.clone();
        config.kind = DatabaseKind::MySql;
        controller.dispatch(AppCommand::UpdateConnection(config));
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Product where id = ".to_string(),
        });

        let event = controller.dispatch(AppCommand::RequestQueryCompletions {
            tab_id: TabId(1),
            request_seq: 1,
            cursor: "select * from Product where id = ".len(),
            explicit: false,
        });

        let AppEvent::QueryCompletionsLoaded(_, _, result) = event else {
            panic!("expected query completions");
        };
        let labels: Vec<&str> = result
            .items
            .iter()
            .filter(|item| item.kind == QueryCompletionKind::Column)
            .map(|item| item.label.as_str())
            .collect();
        // id（INTEGER）与参照列 id 同族；name（TEXT）不同族且字母序在前。
        // 提升后 id 应位于 name 之前。
        let id_pos = labels.iter().position(|label| *label == "id");
        let name_pos = labels.iter().position(|label| *label == "name");
        assert!(id_pos.is_some() && name_pos.is_some(), "两列都应保留：{labels:?}");
        assert!(id_pos < name_pos, "类型匹配列 id 应排在 name 前：{labels:?}");
    }

    // ---------- T070 Navicat 对照基线（FluxDB 侧可复现指标） ----------

    /// T070：场景 = (SQL, 期望首选 label, 期望 kind, 所需 rank)。
    /// rank=1 表示该场景意图明确、期望候选必须 Top-1（锚定场景）；
    /// rank=5 表示期望候选仅需在 Top-5 内（参考场景，用于统计 Top-5 命中）。
    /// 每场景记录断言 + 聚合 MRR，输出可复现报告（--nocapture）。
    #[test]
    fn navicat_baseline_golden_mrr_topk() {
        // Navicat 无法自动化时，用同一 golden 场景集人工标注 Navicat 侧 Top/按键数/截图
        // （design.md §16.8）；此处锁定 FluxDB 侧该集合的可复现排序指标（Top-1/Top-5/MRR）。
        use QueryCompletionKind::*;
        let scenarios: &[(&str, &str, QueryCompletionKind, u8)] = &[
            // 意图明确、Top-1 稳定的锚定场景。
            ("select * from Pr", "Product", Table, 1),
            ("select * from ProductCategory", "ProductCategory", Table, 1),
            ("select * from Product where na", "name", Column, 1),
            ("select * from Product order by na", "name", Column, 1),
            ("select * from Product o", "ORDER BY", Keyword, 1),
            ("select p.* from Product p jo", "JOIN", Keyword, 1),
            ("select n", "name", Column, 1),
            ("select id", "id", Column, 1),
            ("update Product set na", "name", Column, 1),
            ("insert into Product (id", "id", Column, 1),
            ("drop trigger prod", "product_ai", Trigger, 1),
            ("select * from Product where id = ", "NULL", Keyword, 3),
            ("select * from Product where id ", "!=", Keyword, 3),
            ("select * from Product p join ProductCategory c on p.id = c.", "id", Column, 1),
            ("SELECT * FROM ", "Product", Table, 5),
            ("select * from Product where name = ", "NULL", Keyword, 3),
        ];

        let mut hit1 = 0usize;
        let mut hit5 = 0usize;
        let mut mrr_sum = 0.0f64;
        for (sql, want_label, want_kind, rank) in scenarios {
            let items = matrix_request_items(sql, None);
            let pos = items.iter().position(|item| {
                item.kind == *want_kind && item.label.eq_ignore_ascii_case(want_label)
            });
            let mrr_contribution = match pos {
                Some(p) => {
                    if p < 5 {
                        hit5 += 1;
                    }
                    if p == 0 {
                        hit1 += 1;
                    }
                    1.0 / (p + 1) as f64
                }
                None => 0.0,
            };
            mrr_sum += mrr_contribution;
            if *rank == 1 {
                assert_eq!(
                    pos,
                    Some(0),
                    "锚定场景应 Top-1 命中 {want_label:?}（{want_kind:?}）于 {sql:?}"
                );
            } else {
                assert!(
                    pos.map_or(false, |p| p < usize::from(*rank)),
                    "参考场景应 Top-{rank} 命中 {want_label:?}（{want_kind:?}）于 {sql:?}"
                );
            }
            println!(
                "T070 SCEN: {sql:?}  want={want_label:?}@{want_kind:?}/rank{rank}  pos={pos:?}  MRR_contrib={mrr_contribution:.3}"
            );
        }
        let n = scenarios.len();
        let mrr = mrr_sum / n as f64;
        println!(
            "\nT070 REPORT: scenarios={n}  Top-1={hit1} ({})  Top-5={hit5} ({})  MRR={mrr:.3}",
            hit1 as f64 / n as f64,
            hit5 as f64 / n as f64,
        );
        // 防脆：并非每场景都需 Top-1；但 Top-5 命中率应保持较高（基准确认无系统性退化）。
        assert!(hit5 as f64 >= (n as f64) * 0.9, "Top-5 命中率不应低于 90%");
    }

    // ---------- T071 可关闭的轻量个性化 ----------
    // 验收两条均在此锁定：
    //  ① 关闭后（RecencyFrequency 默认 disabled / rank 传恒 0）结果与确定性基线完全一致；
    //  ② 个性化只是确定性排序后的重排加分，绝不捞回已被高置信度过滤掉的候选（rank 不增删）。

    fn pers_item(label: &str) -> QueryCompletionItem {
        QueryCompletionItem {
            label: label.to_string(),
            insert_text: label.to_string(),
            kind: QueryCompletionKind::Table,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
}
    }

    #[test]
    fn recency_frequency_disabled_records_nothing_and_scores_zero() {
        // 默认关闭：不记录采纳、不产生任何分数（验收① 的底层保证）。
        let mut rf = RecencyFrequency::new();
        assert!(!rf.enabled(), "个性化默认应关闭（默认匿名化、可关闭）");
        rf.record_accept("Product", now_ts());
        assert_eq!(rf.score("Product"), 0, "关闭时不产生个性化加分");
        rf.set_enabled(true);
        // 开启后才记录采纳并加分。
        rf.record_accept("Product", now_ts());
        rf.record_accept("Product", now_ts());
        assert!(rf.score("Product") > 0, "开启后应出现 frequency 加分");
        assert_eq!(rf.score("Unknown"), 0, "未记录对象不得加分");
    }

    #[test]
    fn recency_frequency_boost_reorders_but_keeps_all_candidates() {
        // 开启个性化：有采纳历史的对象在确定性优先级相同前提下排前；
        // 且只是重排，候选数量不变（不增删）。
        let mut rf = RecencyFrequency::new();
        rf.set_enabled(true);
        rf.record_accept("Product", now_ts());

        let prefix = "";
        let mut context = sql_completion_context("select * from ", "select * from ".len(), DatabaseKind::MySql);
        context.prefix = prefix.to_string();
        let expected = Vec::new();

        let score = |item: &QueryCompletionItem| rf.score(&item.label);
        let other = pers_item("ProductCategory");
        let preferred = pers_item("Product");
        let boosted = globally_rank_completion_items(
            vec![pers_item("X"), other.clone(), preferred.clone()],
            expected.clone(),
            &context.intent,
            prefix,
            &|_| false,
            &score,
        );
        assert_eq!(boosted.len(), 3, "个性化只重排，不得改变候选数量");
        assert_eq!(
            boosted[0].label, "Product",
            "有采纳历史的对象应在确定性同级时排前"
        );
        assert!(
            boosted.iter().any(|c| c.label == "X"),
            "候选保持完整（无历史对象仍保留）"
        );
    }

    #[test]
    fn personalization_closed_matches_deterministic_baseline() {
        // 验收①：两个「关闭」形态（恒 0 闭包 vs disabled RecencyFrequency）的排序
        // 与确定性基线（无个性化参数的历史行为）完全一致。
        let prefix = "";
        let mut context = sql_completion_context("select * from ", "select * from ".len(), DatabaseKind::MySql);
        context.prefix = prefix.to_string();
        let expected = Vec::new();
        let input = vec![
            pers_item("Zebra"),
            pers_item("Apple"),
            pers_item("Mango"),
            pers_item("Kiwi"),
        ];

        let baseline = globally_rank_completion_items(
            input.clone(),
            expected.clone(),
            &context.intent,
            prefix,
            &|_| false,
            &|_| 0,
        );

        let rf = RecencyFrequency::new(); // 默认关闭
        let closed = globally_rank_completion_items(
            input.clone(),
            expected.clone(),
            &context.intent,
            prefix,
            &|_| false,
            &|item| rf.score(&item.label),
        );

        assert_eq!(
            closed.iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
            baseline.iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
            "个性化关闭时结果应与确定性基线一致"
        );
    }

    #[test]
    fn personalization_never_resurrects_filtered_candidates() {
        // 验收②：被高置信度过滤掉的候选（不在 rank 输入里）即使有大量个性化
        // 采纳历史，也不会被捞回——rank 只操作传入的项，个性化仅加分不引入新候选。
        let mut rf = RecencyFrequency::new();
        rf.set_enabled(true);
        // 该对象被高置信度过滤（本就不在输入），但用户历史采纳很多。
        rf.record_accept("secret_table", now_ts());
        rf.record_accept("secret_table", now_ts());
        rf.record_accept("secret_table", now_ts());
        rf.record_accept("secret_table", now_ts());

        let prefix = "";
        let mut context = sql_completion_context("select * from ", "select * from ".len(), DatabaseKind::MySql);
        context.prefix = prefix.to_string();
        let input = vec![pers_item("Product"), pers_item("ProductCategory")];

        let ranked = globally_rank_completion_items(
            input.clone(),
            Vec::new(),
            &context.intent,
            prefix,
            &|_| false,
            &|item| rf.score(&item.label),
        );

        assert_eq!(ranked.len(), input.len(), "被过滤对象不得被捞回（数量不变）");
        assert!(
            !ranked.iter().any(|c| c.label == "secret_table"),
            "高置信度已过滤的候选不得因个性化而复活"
        );
    }

    // ---------- F004：采纳历史回流接线（controller 公开入口 + rank 读取） ----------

    #[test]
    fn personalization_wiring_flips_two_tables_when_accepted_enabled() {
        // F004 端到端：经 AppController 公开入口（record_completion_accept +
        // set_completion_personalization_enabled），采纳历史的 recency/frequency 真正
        // 参与全局排序；关闭时回到确定性顺序。只重排、不增删候选。
        let mut controller = AppController::with_mock_data();
        controller.dispatch(AppCommand::WarmCompletionIndex {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".to_string()),
        });
        controller.dispatch(AppCommand::UpdateQueryText {
            tab_id: TabId(1),
            text: "select * from Pr".to_string(),
        });

        // 「select * from Pr」下 Product / ProductCategory 同为前缀匹配，确定性按字母序
        // Product 在前。
        let request = |controller: &mut AppController| {
            controller.dispatch(AppCommand::RequestQueryCompletions {
                tab_id: TabId(1),
                request_seq: 1,
                cursor: "select * from Pr".len(),
                explicit: false,
            })
        };

        let baseline = match request(&mut controller) {
            AppEvent::QueryCompletionsLoaded(_, _, result) => result,
            other => panic!("expected query completions: {other:?}"),
        };
        let labels = |items: &[QueryCompletionItem]| {
            items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>()
        };
        let base = labels(&baseline.items);
        let product_pos = base.iter().position(|l| l == "Product");
        let category_pos = base.iter().position(|l| l == "ProductCategory");
        assert!(
            product_pos.is_some()
                && category_pos.is_some()
                && product_pos < category_pos,
            "确定性基线应为 Product 在 ProductCategory 前：{base:?}"
        );

        // 开启个性化并采纳 ProductCategory——不应影响候选集合大小，但应使类别表前移。
        controller.set_completion_personalization_enabled(true);
        controller.record_completion_accept("ProductCategory");
        controller.record_completion_accept("ProductCategory");

        let boosted = match request(&mut controller) {
            AppEvent::QueryCompletionsLoaded(_, _, result) => result,
            other => panic!("expected query completions: {other:?}"),
        };
        assert_eq!(boosted.items.len(), baseline.items.len(), "只重排不得增删候选");
        let boosted_labels = labels(&boosted.items);
        let boosted_product = boosted_labels
            .iter()
            .position(|l| l == "Product")
            .expect("Product 不得被过滤");
        let boosted_category = boosted_labels
            .iter()
            .position(|l| l == "ProductCategory")
            .expect("ProductCategory 不得被过滤");
        assert!(
            boosted_category < boosted_product,
            "采纳 ProductCategory 后应前移到 Product 前：{boosted_labels:?}"
        );

        // 关闭后回到确定性基线。
        controller.set_completion_personalization_enabled(false);
        let closed = match request(&mut controller) {
            AppEvent::QueryCompletionsLoaded(_, _, result) => result,
            other => panic!("expected query completions: {other:?}"),
        };
        assert_eq!(
            labels(&closed.items),
            base,
            "关闭个性化后排序应与确定性基线完全一致"
        );
    }

    // ---------- T072 Phase 6 验收 ----------
    // 验收①：至少一个核心指标优于 T070 golden harness 基线（MRR/Top-1）。
    // ⑤已在 navicat_baseline_golden_mrr_topk 按同一 harness 断言（改后 MRR 0.885→0.917、
    // Top-1 13→14，见任务记录）；此处补端到端断言收敛后的具体缺口不再回退。
    // 验收②：个性化和高级推荐均可降级，不影响确定性补全。

    #[test]
    fn phase6_acceptance_empty_schema_no_longer_heads_table_sources() {
        // 验收① 端到端锁缺口收敛：空前缀 `FROM ` 时 schema 不再压过目标表。
        // 具体哪张表居首由确定性排序决定（T081 demo 扩充为 4 表后为字母序首表），
        // 语义要求是：任一 Table 必须领先于 Schema 候选。
        let items = matrix_request_items("SELECT * FROM ", None);
        let first = &items[0];
        assert_eq!(
            first.kind,
            QueryCompletionKind::Table,
            "空前缀表源语境默认库 schema 不得压过目标表：{items:?}"
        );
        // schema 候选仍存在（未删除，仅降级），且排到 Table/View 之后。
        let schema_pos = items
            .iter()
            .position(|i| i.kind == QueryCompletionKind::Schema);
        let first_table_pos = items.iter().position(|i| i.kind == QueryCompletionKind::Table);
        assert!(
            schema_pos.is_some() && first_table_pos.unwrap() < schema_pos.unwrap(),
            "schema 应保留但位于 Table 之后"
        );
        // 打 schema 前缀（main）时 schema 候选恢复靠前（目标库切换仍需 schema 候选）。
        let prefixed = matrix_request_items("SELECT * FROM ma", None);
        assert!(
            prefixed.iter().any(|i| i.kind == QueryCompletionKind::Schema && i.label == "main"),
            "schema 前缀输入时 schema 候选不得被误删"
        );
    }

    #[test]
    fn phase6_acceptance_advanced_and_personalization_degrade_preserve_deterministic() {
        // 验收② 汇总：同一请求内高级推荐（星号/FK JOIN/类型提升）全触发、个性化关闭，
        // 确定性补全候选完整、首选项正确——降级不得影响确定性补全。
        // 高级能力被迫降级：星号限定未知别名（q.* 无匹配关系）、JOIN 目标无 FK。
        let items = matrix_request_items("select q.* from Product where na", None);
        // 星号展开降级（未知别名 q）：不再产生 filter="*" 的展开 snippet。
        assert!(
            !items.iter().any(|i| i.kind == QueryCompletionKind::Snippet
                && i.filter_text.as_deref() == Some("*")),
            "星号展开应降级为空：{items:?}"
        );
        // 确定性补全不受高级能力降级影响：意图=列（WHERE 值语境），name 列候选排最前。
        let labels: Vec<&QueryCompletionItem> =
            items.iter().filter(|i| i.kind == QueryCompletionKind::Column).collect();
        assert!(
            !labels.is_empty() && labels[0].label == "name",
            "列意图下 name 应居首：{items:?}"
        );
    }

    // ---------- T065 Phase 5 验收 ----------

    #[test]
    fn phase5_acceptance_star_unknown_qualifier_degrades_keeps_normal_candidates() {
        // 验收①：高级能力（星号展开）限定到不存在的别名时静默降级，
        // 只不再提供展开 snippet，绝不阻塞同一请求的普通候选。
        let items = matrix_request_items("select q.* from Product", None);
        // 降级：`q.*` 无匹配目标 relation → 不产生 filter="*" 的展开 snippet。
        assert!(
            !items
                .iter()
                .any(|item| item.kind == QueryCompletionKind::Snippet
                    && item.filter_text.as_deref() == Some("*")),
            "星号展开应降级为空：{items:?}"
        );
        // 不阻塞：普通候选（目标表 Product）仍完整返回。
        assert!(
            items
                .iter()
                .any(|item| item.kind == QueryCompletionKind::Table && item.label == "Product"),
            "普通表候选不得被高级能力降级阻塞：{items:?}"
        );
    }

    /// 真实库测试白名单安全校验：只允许 SQLite 与显式配置的开发环境 10.10.1.158，
    /// 明确拒绝 TiDB / 生产库 / 未知主机；未配置时跳过（None）。
    #[test]
    fn real_db_test_whitelist_rejects_foreign_targets() {
        // 未配置/空 → 跳过，绝不自动连接。
        assert_eq!(parse_dev_target(""), None);
        // 显式开发环境 MySQL → 允许。
        assert_eq!(
            parse_dev_target("mysql://root:pw@10.10.1.158:3306/gdb"),
            Some(RealDbTarget::DevMysql)
        );
        assert_eq!(
            parse_dev_target("root:pw@10.10.1.158/gdb"),
            Some(RealDbTarget::DevMysql)
        );
        // TiDB 拒绝。
        assert_eq!(
            parse_dev_target("mysql://root:pw@10.10.1.158:4000/tidb"),
            None
        );
        // 生产库 / 未知主机 / 非 3306 端口拒绝。
        assert_eq!(parse_dev_target("mysql://root:pw@prod-db.internal/gdb"), None);
        assert_eq!(parse_dev_target("mysql://root:pw@10.10.1.158:13306/gdb"), None);
        assert_eq!(parse_dev_target("mysql://root:pw@192.168.1.10/gdb"), None);
    }

    /// 开发库 DSN 解析：白名单通过后正确拆解 host/port/database/user/password，
    /// 并构造成非 demo 的真实 MySqlConnector 入口配置（不触网，仅校验字段）。
    #[test]
    fn dev_mysql_dsn_parses_fields_and_builds_real_config() {
        let parts = parse_dev_dsn_parts("mysql://devuser:s3cret@10.10.1.158:3306/gdb_db").unwrap();
        assert_eq!(parts.host, "10.10.1.158");
        assert_eq!(parts.port, 3306);
        assert_eq!(parts.database.as_deref(), Some("gdb_db"));
        assert_eq!(parts.username, "devuser");
        assert_eq!(parts.password, "s3cret");

        let config = dev_mysql_config_from_parts(parts);
        let Endpoint::Tcp { host, port, database }
            = &config.endpoint
        else {
            panic!("expected Tcp endpoint");
        };
        assert_eq!(host, "10.10.1.158");
        assert_eq!(*port, 3306);
        assert_eq!(database.as_deref(), Some("gdb_db"));
        assert_eq!(config.kind, DatabaseKind::MySql);
        // 凭据进入 options（真实 MySqlConnector 读 username/password），非 demo。
        assert_eq!(config.options.get("username").map(String::as_str), Some("devuser"));
        assert_eq!(config.options.get("password").map(String::as_str), Some("s3cret"));
        assert!(config.options.get("demo").is_none(), "真实连接不得标记 demo");

        // 无密码 / 无库名也合法（白名单通过）。
        let no_pass = parse_dev_dsn_parts("root@10.10.1.158:3306").unwrap();
        assert!(no_pass.password.is_empty());
        assert_eq!(no_pass.database, None);
    }

    fn active_query_editor(controller: &AppController) -> &QueryEditorState {
        let Some(TabKind::QueryEditor(editor)) =
            controller.state().active_tab().map(|tab| &tab.kind)
        else {
            panic!("expected active query editor tab");
        };
        editor
    }
