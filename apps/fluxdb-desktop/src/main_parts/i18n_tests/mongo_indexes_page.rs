// MongoDB 索引页（Compass 式表格）的生产视图无头用例。
//
// 不连库：直接用 `FinishMongoIndexes` 摆进两条索引，验证表格行渲染、刷新入口、
// 以及点击行首箭头展开/收起键定义。

#[gpui::test]
fn mongo_indexes_page_lists_and_expands_definitions(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoIndexField, MongoIndexInfo,
        ObjectKind, ObjectPath,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    let connection_id = ConnectionId(23);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "mongo indexes".into(),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 27017,
            database: None,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }]));
    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
    });
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            navicat_main_new(
                window,
                cx,
                controller,
                storage,
                gpui_component::ThemeMode::Light,
            )
        });
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
    let view = cx.update(|window, cx| {
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
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenMongoCollection {
                    object: ObjectPath {
                        connection_id,
                        database: Some("shop".into()),
                        schema: None,
                        name: "orders".into(),
                        kind: ObjectKind::Collection,
                    },
                    read_only: false,
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    let tab_id = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .active_tab
            .expect("mongo collection tab is active")
    });

    // 两条索引：`_id_`（唯一、有使用统计）与复合索引（无统计，走不可用分支）。
    let now = chrono::Utc::now().timestamp_millis();
    let indexes = vec![
        MongoIndexInfo {
            name: "_id_".into(),
            keys_json: r#"{"_id": {"$numberInt":"1"}}"#.into(),
            fields: vec![MongoIndexField {
                field: "_id".into(),
                value: "1".into(),
            }],
            index_type: "regular".into(),
            unique: true,
            sparse: false,
            partial_filter: None,
            expire_after_seconds: None,
            has_collation: false,
            hidden: false,
            size_bytes: Some(36_900),
            usage_count: Some(2),
            usage_since: Some(now - 60_000),
            ready: true,
        },
        MongoIndexInfo {
            name: "profile_city_1".into(),
            keys_json: r#"{"profile.city": {"$numberInt":"1"}}"#.into(),
            fields: vec![MongoIndexField {
                field: "profile.city".into(),
                value: "1".into(),
            }],
            index_type: "regular".into(),
            unique: false,
            sparse: false,
            partial_filter: None,
            expire_after_seconds: None,
            has_collation: false,
            hidden: false,
            size_bytes: None,
            usage_count: None,
            usage_since: None,
            ready: true,
        },
        MongoIndexInfo {
            name: "tag_1".into(),
            keys_json: r#"{"tag": {"$numberInt":"1"}}"#.into(),
            fields: vec![MongoIndexField {
                field: "tag".into(),
                value: "1".into(),
            }],
            index_type: "regular".into(),
            unique: false,
            sparse: false,
            partial_filter: None,
            expire_after_seconds: None,
            has_collation: false,
            hidden: false,
            size_bytes: Some(1_024),
            usage_count: None,
            usage_since: None,
            ready: true,
        },
    ];
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::FinishMongoIndexes {
                    tab_id,
                    result: Ok(indexes),
                },
                cx,
            );
            this.dispatch(
                AppCommand::SelectMongoSection {
                    tab_id,
                    section: crate::MongoSection::Indexes,
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    let window_bounds = cx.update(|window, _| window.bounds());
    let refresh = cx
        .debug_bounds("mongo-indexes-refresh")
        .expect("refresh button renders");
    assert!(
        window_bounds.contains(&refresh.center()),
        "refresh button must stay inside the window: {refresh:?}"
    );
    for (selector, name) in [
        ("mongo-index-row:_id_", "_id_"),
        ("mongo-index-row:profile_city_1", "profile_city_1"),
        ("mongo-index-row:tag_1", "tag_1"),
    ] {
        let row = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("index row `{name}` must render"));
        assert!(
            window_bounds.contains(&row.center()),
            "index row `{name}` must stay inside the window: {row:?}"
        );
    }
    assert!(
        cx.debug_bounds("mongo-index-keys:_id_").is_none(),
        "definitions start collapsed"
    );

    // 表头和数据行共用列定义：每一列必须同起点同宽度，否则点表头排序的列会错位。
    for key in [
        "name", "type", "size", "usage", "rate", "properties", "status",
    ] {
        let header_selector: &'static str =
            Box::leak(format!("mongo-index-sort:{key}").into_boxed_str());
        let cell_selector: &'static str =
            Box::leak(format!("mongo-index-cell:_id_:{key}").into_boxed_str());
        let header = cx
            .debug_bounds(header_selector)
            .unwrap_or_else(|| panic!("header cell `{key}` renders"));
        let cell = cx
            .debug_bounds(cell_selector)
            .unwrap_or_else(|| panic!("data cell `{key}` renders"));
        assert_eq!(
            (header.origin.x, header.size.width),
            (cell.origin.x, cell.size.width),
            "column `{key}` must line up between header and row"
        );
    }

    // 操作列：`_id_` 删不掉，其余索引各有删除按钮。
    assert!(
        cx.debug_bounds("mongo-index-drop:_id_").is_none(),
        "the built-in _id_ index must not offer a drop action"
    );
    for name in ["profile_city_1", "tag_1"] {
        let selector = match name {
            "profile_city_1" => "mongo-index-drop:profile_city_1",
            _ => "mongo-index-drop:tag_1",
        };
        assert!(
            cx.debug_bounds(selector).is_some(),
            "index `{name}` must offer a drop action"
        );
    }

    // 点表头排序：大小升序把无统计的索引排到最后，再点一次翻转方向。
    let row_top = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("row `{selector}` must render"))
            .origin
            .y
    };
    let size_header = cx
        .debug_bounds("mongo-index-sort:size")
        .expect("size column is sortable");
    cx.simulate_click(size_header.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(
        row_top(&mut *cx, "mongo-index-row:tag_1") < row_top(&mut *cx, "mongo-index-row:_id_")
            && row_top(&mut *cx, "mongo-index-row:_id_")
                < row_top(&mut *cx, "mongo-index-row:profile_city_1"),
        "ascending size sort puts the smallest first and the unknown size last"
    );
    let size_header = cx
        .debug_bounds("mongo-index-sort:size")
        .expect("size column still renders");
    cx.simulate_click(size_header.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(
        row_top(&mut *cx, "mongo-index-row:profile_city_1")
            < row_top(&mut *cx, "mongo-index-row:_id_"),
        "clicking the same header again reverses the order"
    );

    // 删除按钮先弹确认框；取消后索引仍然在，不发任何请求。
    let drop = cx
        .debug_bounds("mongo-index-drop:profile_city_1")
        .expect("drop action renders");
    cx.simulate_click(drop.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let cancel = cx
        .debug_bounds("mongo-index-drop-cancel")
        .expect("drop confirmation dialog opens");
    cx.simulate_click(cancel.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-index-drop-confirm").is_none(),
        "cancel closes the confirmation dialog"
    );
    assert!(cx.debug_bounds("mongo-index-row:profile_city_1").is_some());

    // 点行首箭头展开键定义，再点一次收起。
    let toggle = cx
        .debug_bounds("mongo-index-toggle:_id_")
        .expect("expand toggle renders");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("mongo-index-keys:_id_").is_some(),
        "clicking the chevron expands the key definition"
    );
    let toggle = cx
        .debug_bounds("mongo-index-toggle:_id_")
        .expect("expand toggle still renders when expanded");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("mongo-index-keys:_id_").is_none(),
        "clicking again collapses the key definition"
    );
}
