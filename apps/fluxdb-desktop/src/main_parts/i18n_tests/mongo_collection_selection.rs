// MongoDB 集合页文档卡片的窗口级文本选择：真实生产视图的无头鼠标驱动。
//
// 不连库：直接写入一页文档快照，验证在小幅拖动时选区仍停留在按下的那一行，
// 不会因为行序重复或端点解析错误跳到别的文档。

#[gpui::test]
fn mongo_collection_drag_selection_stays_in_the_pressed_row(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoDocument, MongoDocumentsPage,
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
        name: "mongo collection selection".into(),
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
                        name: "customers".into(),
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
    let (tab_id, generation) = cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let tab_id = state.active_tab.expect("collection tab is active");
        // 打开集合会先发起一次真实首屏加载，令牌已经递增；用当前令牌才不会被当成过期结果丢弃。
        let generation = state
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                fluxdb_app::TabKind::MongoCollection(collection)
                    if tab.id == tab_id =>
                {
                    Some(collection.load_generation)
                }
                _ => None,
            })
            .expect("collection tab state");
        (tab_id, generation)
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::FinishMongoDocuments {
                    tab_id,
                    generation,
                    result: Ok(MongoDocumentsPage {
                        // 7 篇长文档：卡片列表超出视口，真实场景里拖动会经过可滚动范围。
                        documents: (0..7)
                            .map(|index| {
                                let id = format!("66a00000000000000000000{}", index + 5);
                                MongoDocument {
                                    id: id.clone(),
                                    json: format!(
                                        r#"{{"_id":{{"$oid":"{id}"}},"customer_no":"CUST-00{index}","name":{{"first":"Tang","last":"Ke"}},"email":"tang.ke@example.test","active":true,"age":41,"loyalty_points":2147483648,"balance":"8888.88","tags":["a","b"],"phones":["1"],"profile":{{"tier":"gold"}},"last_login":"2026-10-24T01:15:00Z","created_at":"2023-07-07T07:07:00Z"}}"#
                                    ),
                                }
                            })
                            .collect(),
                        returned: 7,
                        total: Some(7),
                        has_more: false,
                    }),
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    let first_row = cx
        .debug_bounds(
            "mongo-document:field-text:66a000000000000000000005:customer_no",
        )
        .expect("first document field participates in selection");
    let card_top_before = cx
        .debug_bounds("mongo-document-card:0")
        .expect("first card renders")
        .top();
    // 从行的下缘附近按下，再往下挪几像素：真实用户“轻微移动”就会跨出这一行。
    let pressed = gpui::point(first_row.left() + gpui::px(2.), first_row.bottom() - gpui::px(2.));
    cx.simulate_event(gpui::MouseDownEvent {
        position: pressed,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    // 小幅拖动：横向几十像素、纵向几像素。
    let released =
        gpui::point(first_row.left() + gpui::px(60.), first_row.bottom() + gpui::px(2.));
    cx.simulate_event(gpui::MouseMoveEvent {
        position: released,
        modifiers: Modifiers::default(),
        pressed_button: Some(gpui::MouseButton::Left),
    });
    // 按住不放时让拖动自动滚动的定时器真的跑几拍：误触发会让列表滚走。
    for _ in 0..3 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(20));
        if !cx.background_executor.tick() {
            break;
        }
    }
    cx.simulate_event(gpui::MouseUpEvent {
        position: released,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    let selected = cx.update(|window, cx| {
        let _ = window.draw(cx);
        gpui_kit::base::TextSelection::selected_text(window, cx)
    });
    assert!(
        selected.starts_with("customer"),
        "the drag selects the pressed row, got {selected:?}"
    );
    assert!(
        !selected.contains("CUST-0006") && !selected.contains("email"),
        "a small drag must not reach other rows or documents, got {selected:?}"
    );
    // 轻拖不应触发自动滚动：卡片位置一旦变化，用户看到的就是“选区跳到别处”。
    let card_top_after = cx
        .debug_bounds("mongo-document-card:0")
        .expect("first card stays rendered")
        .top();
    assert_eq!(
        card_top_after, card_top_before,
        "a small drag must not scroll the document list"
    );
}
