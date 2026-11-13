// Oracle / 达梦在 SQL 编辑器里的端到端用例：真实 `NavicatMain` + 真实 SQL 编辑器，
// 无头事件驱动（模拟点击、键入、点击候选），断言用户能观察到的结果。
//
// 覆盖 docs/oracle.md §9「补全与高亮按连接类型分方言」：
// - 编辑器拿到的补全方言必须是 Oracle，不是 PostgreSQL（否则会补出 `NOW`）；
// - Oracle 专有函数（`NVL`/`TO_TIMESTAMP`）能补出来，PostgreSQL 专有的补不出；
// - 表候选按方言引用（Oracle 双引号）；
// - code lens 的语句边界与 PL/SQL 块一致（Oracle / 达梦都不按 `;` 切碎匿名块）。

/// 一条待打开的查询标签页：连接 + 库/schema 作用域 + 该 schema 下已加载的表。
struct CompletionTab {
    connection: fluxdb_core::ConnectionConfig,
    database: String,
    schema: String,
    table: &'static str,
}

/// 构造 Oracle 连接配置。不连库：这些用例只读本地补全 / 元数据，不发起网络请求。
fn oracle_completion_tab(connection_id: fluxdb_core::ConnectionId) -> CompletionTab {
    use fluxdb_core::{
        ConnectionConfig, DatabaseKind, Endpoint, OracleBasicOptions, OracleConnectionProfile,
        SecretRef,
    };

    CompletionTab {
        connection: ConnectionConfig {
            id: connection_id,
            name: "Oracle completion".into(),
            kind: DatabaseKind::Oracle,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1521,
                database: Some("FREEPDB1".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            oracle_profile: Some(OracleConnectionProfile {
                basic: OracleBasicOptions {
                    host: "127.0.0.1".into(),
                    port: 1521,
                    service_name: "FREEPDB1".into(),
                    username: "fluxdb".into(),
                    password: SecretRef::inline("not-used"),
                    ..OracleBasicOptions::default()
                },
                ..OracleConnectionProfile::default()
            }),
            mongo_profile: None,
        },
        database: "FREEPDB1".into(),
        schema: "FLUXDB".into(),
        table: "EMPLOYEES",
    }
}

/// 构造 PostgreSQL 连接配置（对照用）。
fn postgres_completion_tab(connection_id: fluxdb_core::ConnectionId) -> CompletionTab {
    use fluxdb_core::{
        ConnectionConfig, DatabaseKind, Endpoint, PostgresConnectionProfile,
    };

    CompletionTab {
        connection: ConnectionConfig {
            id: connection_id,
            name: "PG completion".into(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 5432,
                database: Some("fluxdb".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(PostgresConnectionProfile::default()),
            dameng_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
            mongo_profile: None,
        },
        database: "fluxdb".into(),
        schema: "public".into(),
        table: "EMPLOYEES",
    }
}

/// 构造达梦连接配置。
fn dameng_completion_tab(connection_id: fluxdb_core::ConnectionId) -> CompletionTab {
    use fluxdb_core::{
        ConnectionConfig, DamengAdvancedOptions, DamengBasicOptions, DamengConnectionProfile,
        DatabaseKind, Endpoint, SecretRef,
    };

    CompletionTab {
        connection: ConnectionConfig {
            id: connection_id,
            name: "Dameng completion".into(),
            kind: DatabaseKind::Dameng,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 5236,
                database: Some("DAMENG".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: Some(DamengConnectionProfile {
                basic: DamengBasicOptions {
                    host: "127.0.0.1".into(),
                    port: 5236,
                    database: "DAMENG".into(),
                    schema: "FLUXDB".into(),
                    username: "FLUXDB".into(),
                    password: SecretRef::inline("not-used"),
                },
                advanced: DamengAdvancedOptions {
                    connect_timeout_secs: 10,
                    query_timeout_secs: 30,
                    encoding: String::new(),
                    application_name: "FluxDB E2E".into(),
                },
                ssh: Default::default(),
            }),
            sqlserver_profile: None,
            oracle_profile: None,
            mongo_profile: None,
        },
        database: "DAMENG".into(),
        schema: "FLUXDB".into(),
        table: "EMPLOYEES",
    }
}

/// 起一个生产视图，为每条 tab 打开查询编辑器，并把该 schema 下的表挂进树。
///
/// 连接不打开（`OpenConnection` 会真的拨号），只把已加载对象塞进 controller：
/// 编辑器与补全只消费这份内存状态。返回的 `VisualTestContext` 借用入参，
/// 调用方直接遮蔽原 `cx` 使用（与仓库其它 E2E 一致）。
///
/// 编辑器实体不在这里取：宿主只为**当前渲染的**标签页建编辑器，
/// 非活动页要等切过去并重绘后才存在（见 [`focus_editor`]）。
fn open_completion_view(
    cx: &mut TestAppContext,
    tabs: Vec<CompletionTab>,
) -> (
    gpui::Entity<NavicatMain>,
    Vec<fluxdb_app::TabId>,
    &mut gpui::VisualTestContext,
) {
    use fluxdb_core::{ObjectKind, ObjectPath, ObjectSummary};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
    });

    let mut controller = AppController::new();
    let connections = tabs
        .iter()
        .map(|tab| tab.connection.clone())
        .collect::<Vec<_>>();
    let _ = controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(connections));
    // 每条 tab 的历史文本留空；活动页固定为第一条，用例里按需切换。
    let mut tab_ids = Vec::new();
    for tab in &tabs {
        let event = controller.dispatch(fluxdb_app::AppCommand::OpenQueryEditorInDatabase {
            connection_id: tab.connection.id,
            database: Some(tab.database.clone()),
            schema: Some(tab.schema.clone()),
        });
        let fluxdb_app::AppEvent::TabOpened(tab_id) = event else {
            panic!("打开查询编辑器失败");
        };
        tab_ids.push(tab_id);
        controller.merge_loaded_children(
            &ObjectPath {
                connection_id: tab.connection.id,
                database: Some(tab.database.clone()),
                schema: Some(tab.schema.clone()),
                name: tab.schema.clone(),
                kind: ObjectKind::Schema,
            },
            vec![ObjectSummary {
                path: ObjectPath {
                    connection_id: tab.connection.id,
                    database: Some(tab.database.clone()),
                    schema: Some(tab.schema.clone()),
                    name: tab.table.into(),
                    kind: ObjectKind::Table,
                },
                rows: None,
                size_bytes: None,
                modified_at: None,
                comment: None,
                stable: None,
            }],
        );
    }
    let _ = controller.dispatch(fluxdb_app::AppCommand::ActivateTab(tab_ids[0]));

    let storage_for_view = storage.clone();
    let (_, vcx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            navicat_main_new(
                window,
                cx,
                controller,
                storage_for_view,
                gpui_component::ThemeMode::Light,
            )
        });
        Root::new(view, window, cx)
    });
    vcx.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(900.)));
    let view = vcx.update(|window, cx| {
        window
            .root::<Root>()
            .expect("root")
            .expect("typed root")
            .read(cx)
            .view()
            .clone()
            .downcast::<NavicatMain>()
            .expect("production view")
    });
    vcx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });

    let editors = tab_ids;
    (view, editors, vcx)
}

/// 切到指定标签页，把焦点放进它的编辑器，返回该页的编辑器实体。
///
/// 编辑器实体要到该页首次渲染后才存在，所以必须「先切页、再重绘、最后取」。
fn focus_editor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<NavicatMain>,
    tab_id: fluxdb_app::TabId,
) -> gpui::Entity<crate::editor_component::Editor> {
    // 走视图自己的 dispatch（`NavicatMain::dispatch`），而不是 controller 的：
    // 切页要经过视图层才会 `notify` 触发重绘，重绘时才为新页建编辑器。
    cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(fluxdb_app::AppCommand::ActivateTab(tab_id), cx);
        });
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let editor = cx
        .update(|_, cx| view.read(cx).query_editors.get(&tab_id).cloned())
        .expect("SQL editor entity");
    let bounds = cx.debug_bounds("query-editor").expect("SQL editor");
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    editor
}

/// 键入文本并等补全弹层出现（或超时）。补全有 debounce，需要推进时钟。
fn type_and_wait_for_completion(
    cx: &mut gpui::VisualTestContext,
    editor: &gpui::Entity<crate::editor_component::Editor>,
    text: &str,
) {
    use std::time::{Duration, Instant};

    cx.simulate_input(text);
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).text(), text);
    });
    cx.background_executor.advance_clock(Duration::from_millis(100));
    cx.run_until_parked();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let ready = cx.update(|_, cx| {
            let editor = editor.read(cx);
            editor.completion_visible() && !editor.completion_items.is_empty()
        });
        if ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "补全弹层 5s 内没有出现，文本：{text:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
        cx.background_executor.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
}

fn completion_labels(
    cx: &mut gpui::VisualTestContext,
    editor: &gpui::Entity<crate::editor_component::Editor>,
) -> Vec<String> {
    cx.update(|_, cx| {
        editor
            .read(cx)
            .completion_items
            .iter()
            .map(|item| item.label.to_string())
            .collect()
    })
}

/// 编辑器认定的语句数 = code lens 里 `Run` 的个数（每语句一对 Run/Select）。
fn editor_statement_count(
    cx: &mut gpui::VisualTestContext,
    editor: &gpui::Entity<crate::editor_component::Editor>,
) -> usize {
    cx.update(|_, cx| {
        editor
            .read(cx)
            .code_lens_hits
            .iter()
            .filter(|hit| hit.action == "sql.run")
            .count()
    })
}

/// Oracle 连接里敲函数前缀：补出 Oracle 的函数，不补 PostgreSQL 独有的。
///
/// 这条用例走完 `SqlAdapter::complete` → 宿主 resolver → `app_completion_dialect`
/// 的整条链；`app_completion_dialect(DatabaseKind::Oracle)` 一旦退回 POSTGRES，
/// `NVL`/`NVL2` 消失、`NOW` 出现，用例立刻失败。
#[gpui::test]
fn oracle_connection_completes_oracle_functions_not_postgres_ones(cx: &mut TestAppContext) {
    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) = open_completion_view(cx, vec![oracle_completion_tab(ConnectionId(91))]);
    let editor = focus_editor(cx, &view, tabs[0]);
    // `SELECT N` 落在表达式槽位：Oracle 词表里 N 开头的是 NULLIF/NVL/NVL2 等。
    type_and_wait_for_completion(cx, &editor, "SELECT N");
    let labels = completion_labels(cx, &editor);
    assert!(
        labels.iter().any(|label| label == "NVL"),
        "Oracle 连接应补 NVL，实际：{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "NVL2"),
        "Oracle 连接应补 NVL2，实际：{labels:?}"
    );
    assert!(
        !labels.iter().any(|label| label == "NOW"),
        "Oracle 没有 NOW，出现说明方言退回了 PostgreSQL：{labels:?}"
    );
}

/// 同一个窗口里 Oracle 与 PostgreSQL 两个标签页对同一前缀给出不同候选。
///
/// 只按「生成结果不同」不够——必须证明差异来自各自的连接类型，
/// 所以两个编辑器在同一个视图、同一次运行里对比。
#[gpui::test]
fn oracle_and_postgres_tabs_diverge_on_the_same_prefix(cx: &mut TestAppContext) {
    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) = open_completion_view(
        cx,
        vec![
            oracle_completion_tab(ConnectionId(92)),
            postgres_completion_tab(ConnectionId(93)),
        ],
    );

    // 先 Oracle 标签页。
    let oracle_editor = focus_editor(cx, &view, tabs[0]);
    type_and_wait_for_completion(cx, &oracle_editor, "SELECT TO_");
    let oracle_labels = completion_labels(cx, &oracle_editor);
    assert!(
        oracle_labels.iter().any(|label| label == "TO_TIMESTAMP"),
        "Oracle 有 TO_TIMESTAMP：{oracle_labels:?}"
    );

    // 再 PostgreSQL 标签页：同前缀，词表必须不同。
    let postgres_editor = focus_editor(cx, &view, tabs[1]);
    type_and_wait_for_completion(cx, &postgres_editor, "SELECT TO_");
    let postgres_labels = completion_labels(cx, &postgres_editor);
    assert!(
        postgres_labels.iter().any(|label| label == "TO_CHAR"),
        "PG 也有 TO_CHAR：{postgres_labels:?}"
    );
    assert!(
        !postgres_labels.iter().any(|label| label == "TO_TIMESTAMP"),
        "PostgreSQL 词表里没有 TO_TIMESTAMP，出现说明方言混了：{postgres_labels:?}"
    );
}

/// 表候选的插入文本按方言引用：Oracle 用双引号包表名。
///
/// 断言的是用户实际点下的候选结果，而不是 provider 的内部字段。
#[gpui::test]
fn oracle_table_completion_inserts_quoted_qualified_name(cx: &mut TestAppContext) {
    use std::time::{Duration, Instant};

    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) = open_completion_view(cx, vec![oracle_completion_tab(ConnectionId(94))]);
    let editor = focus_editor(cx, &view, tabs[0]);

    type_and_wait_for_completion(cx, &editor, "SELECT * FROM EMP");
    let labels = completion_labels(cx, &editor);
    assert!(
        labels.iter().any(|label| label == "EMPLOYEES"),
        "应补出已加载的 EMPLOYEES：{labels:?}"
    );

    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let row = cx
        .debug_bounds("completion-item:EMPLOYEES")
        .expect("EMPLOYEES 候选行");
    cx.simulate_click(row.center(), Modifiers::default());

    // 接受候选会触发后续补全，等文本稳定再断言。
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = cx.update(|_, cx| editor.read(cx).text());
        if text.contains("EMPLOYEES") {
            assert!(
                text.contains("\"EMPLOYEES\""),
                "Oracle 表名应按双引号插入，实际文本：{text:?}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "接受候选后文本未更新：{text:?}");
        std::thread::sleep(Duration::from_millis(50));
        cx.background_executor.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
}

/// Oracle 的语句边界要走进编辑器：`BEGIN … END;` 是一条语句，后面才是第二条。
///
/// code lens 每个语句一对 Run/Select，数量即编辑器认定的语句数。退回 PostgreSQL
/// 规则时匿名块会被 `;` 切碎，这里立刻变成 4 条。
#[gpui::test]
fn oracle_block_stays_one_statement_in_the_editor(cx: &mut TestAppContext) {
    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) = open_completion_view(cx, vec![oracle_completion_tab(ConnectionId(95))]);
    let editor = focus_editor(cx, &view, tabs[0]);

    cx.simulate_input("BEGIN NULL; END;\nSELECT 1 FROM dual;");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(
        editor_statement_count(cx, &editor),
        2,
        "匿名块 + SELECT 应只有两条语句"
    );
}

/// 达梦与 Oracle 同源：同一个匿名块在达梦连接下也必须是一条语句。
///
/// 达梦此前在切分器里落在 `DefaultSyntax`（只认 `CREATE TRIGGER`），会把块按 `;` 切开。
#[gpui::test]
fn dameng_block_stays_one_statement_in_the_editor(cx: &mut TestAppContext) {
    use fluxdb_core::ConnectionId;

    let (view, tabs, cx) = open_completion_view(cx, vec![dameng_completion_tab(ConnectionId(96))]);
    let editor = focus_editor(cx, &view, tabs[0]);

    cx.simulate_input("BEGIN NULL; END;\nSELECT 1;");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(
        editor_statement_count(cx, &editor),
        2,
        "达梦匿名块应只有两条语句"
    );
}
