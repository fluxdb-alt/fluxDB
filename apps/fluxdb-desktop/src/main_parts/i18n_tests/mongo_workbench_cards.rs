// MongoDB 命令工作台的文档结果卡片：真实生产视图的无头点击驱动。
//
// 不连库：直接写入一条带结构化文档的执行记录，验证卡头 / 字段列表 / 文档↔JSON 切换
// 与折叠交互都真的画出来，而不是只编译通过。

#[gpui::test]
fn mongo_workbench_renders_document_cards_with_collapsible_json(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoDocument, MongoShellResult,
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
    let connection_id = ConnectionId(22);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "mongo workbench".into(),
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
                AppCommand::OpenMongoWorkbench {
                    connection_id,
                    database: "shop".into(),
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
            .expect("mongo workbench tab is active")
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::FinishMongoWorkbenchExecution {
                    tab_id,
                    text: "db.orders.find()".into(),
                    result: Ok(MongoShellResult {
                        output: "{}\n".into(),
                        document_count: Some(1),
                        documents: vec![MongoDocument {
                            id: "66e000000000000000000001".into(),
                            json: r#"{"_id":{"$oid":"66e000000000000000000001"},"name":"A-1","nested":{"qty":2}}"#.into(),
                        }],
                        ..Default::default()
                    }),
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    assert!(
        cx.debug_bounds("mongo-workbench-documents:1").is_some(),
        "document results render as a card list"
    );
    assert!(
        cx.debug_bounds("mongo-workbench-document:1:0").is_some(),
        "one document means one card"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_some(),
        "the card body is the field list"
    );

    // 展示区文本接入窗口级选择（真生产视图，不连库）：双击字段名只选中单词。
    let field_text = cx
        .debug_bounds("mongo-wb-json:1:field-text:wb:1:66e000000000000000000001:name")
        .expect("field text must participate in text selection");
    cx.simulate_event(gpui::MouseDownEvent {
        position: gpui::point(field_text.left() + gpui::px(2.), field_text.center().y),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: gpui::point(field_text.left() + gpui::px(2.), field_text.center().y),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
    });
    // 断言放在 gpui 更新闭包外：失败信息不会被窗口更新期间的双重 panic 吞掉。
    let selected = cx.update(|window, cx| {
        let _ = window.draw(cx);
        gpui_kit::base::TextSelection::selected_text(window, cx)
    });
    assert_eq!(
        selected, "name",
        "double-click selects the field name in the production view"
    );

    // 嵌套行屏上是 `nested: { … }`，复制时给出真实内容而不是占位。
    let nested_text = cx
        .debug_bounds("mongo-wb-json:1:field-text:wb:1:66e000000000000000000001:nested")
        .expect("nested field text participates in selection");
    let nested_position = gpui::point(nested_text.left() + gpui::px(2.), nested_text.center().y);
    cx.simulate_event(gpui::MouseDownEvent {
        position: nested_position,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: nested_position,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
    });
    let selected_nested = cx.update(|window, cx| {
        let _ = window.draw(cx);
        gpui_kit::base::TextSelection::selected_text(window, cx)
    });
    assert!(
        selected_nested.contains("qty") && !selected_nested.contains('…'),
        "folded field copies its real content, got {selected_nested:?}"
    );
    assert!(
        cx.debug_bounds("mongo-wb-json:1-json:0").is_none(),
        "JSON stays collapsed until requested"
    );

    let view_toggle = cx
        .debug_bounds("mongo-workbench-doc-view:1:0")
        .expect("view toggle renders");
    cx.simulate_click(view_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-wb-json:1-json:0").is_some(),
        "switching to JSON shows the foldable JSON block"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_none(),
        "JSON mode replaces the field list"
    );

    // JSON 行也接入窗口级选择：双击选词，cmd-c 只复制选中的词。
    let json_line = cx
        .debug_bounds("mongo-wb-json:1-json-fold:0-text:1")
        .expect("JSON text row participates in selection");
    let word_position = gpui::point(json_line.left() + gpui::px(30.), json_line.center().y);
    cx.simulate_event(gpui::MouseDownEvent {
        position: word_position,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: word_position,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
    });
    let selected_word = cx.update(|window, cx| {
        let _ = window.draw(cx);
        gpui_kit::base::TextSelection::selected_text(window, cx)
    });
    assert!(
        selected_word.contains("$oid") && !selected_word.contains('…'),
        "folded JSON line copies its real content, got {selected_word:?}"
    );
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(selected_word.trim().to_string()),
        "cmd-c copies the JSON selection"
    );

    // 跨行拖选按 document_order 拼接，行间以换行分隔。
    let json_line_2 = cx
        .debug_bounds("mongo-wb-json:1-json-fold:0-text:2")
        .expect("second JSON text row participates in selection");
    let drag_start = gpui::point(json_line.left() + gpui::px(30.), json_line.center().y);
    let drag_end = gpui::point(json_line_2.left() + gpui::px(30.), json_line_2.center().y);
    cx.simulate_event(gpui::MouseDownEvent {
        position: drag_start,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseMoveEvent {
        position: drag_end,
        modifiers: Modifiers::default(),
        pressed_button: Some(gpui::MouseButton::Left),
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: drag_end,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    let dragged = cx.update(|window, cx| {
        let _ = window.draw(cx);
        gpui_kit::base::TextSelection::selected_text(window, cx)
    });
    assert!(
        dragged.contains('\n'),
        "dragging across JSON rows selects multiple lines, got {dragged:?}"
    );

    let view_toggle = cx
        .debug_bounds("mongo-workbench-doc-view:1:0")
        .expect("view toggle stays in the header");
    cx.simulate_click(view_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-wb-json:1-json:0").is_none(),
        "switching back hides the JSON block"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_some(),
        "switching back restores the field list"
    );
    assert!(
        cx.debug_bounds("mongo-workbench-doc-copy:1:0").is_some(),
        "the copy button renders next to the view toggle"
    );

    let document_toggle = cx
        .debug_bounds("mongo-workbench-doc-toggle:1:0")
        .expect("document collapse toggle renders");
    cx.simulate_click(document_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_none(),
        "collapsing the document drops its field list"
    );
    assert!(
        cx.debug_bounds("mongo-workbench-document:1:0").is_some(),
        "the card header stays after collapsing"
    );

    // 同一页再跑一条记录：两条记录的行序必须接续，否则拖选会把上一条记录
    // 同位置的行一起算进选区（表现为选区“跳”到别的卡片）。
    // 先展开第一条记录的卡片，让两批字段同时参与选择。
    let document_toggle = cx
        .debug_bounds("mongo-workbench-doc-toggle:1:0")
        .expect("document collapse toggle stays in the header");
    cx.simulate_click(document_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds(
            "mongo-wb-json:1:field-text:wb:1:66e000000000000000000001:name"
        )
        .is_some(),
        "expanding the first record brings its rows back"
    );
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::FinishMongoWorkbenchExecution {
                    tab_id,
                    text: "db.orders.find()".into(),
                    result: Ok(MongoShellResult {
                        output: "{}\n".into(),
                        document_count: Some(1),
                        documents: vec![MongoDocument {
                            id: "66e000000000000000000002".into(),
                            json: r#"{"_id":{"$oid":"66e000000000000000000002"},"name":"B-2","nested":{"qty":9}}"#.into(),
                        }],
                        ..Default::default()
                    }),
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    let second_name = cx
        .debug_bounds(
            "mongo-wb-json:2:field-text:wb:2:66e000000000000000000002:name",
        )
        .expect("second record field participates in selection");
    let second_nested = cx
        .debug_bounds(
            "mongo-wb-json:2:field-text:wb:2:66e000000000000000000002:nested",
        )
        .expect("second record nested field participates in selection");
    let second_start =
        gpui::point(second_name.left() + gpui::px(2.), second_name.center().y);
    let second_end = gpui::point(
        second_nested.left() + gpui::px(2.),
        second_nested.center().y,
    );
    cx.simulate_event(gpui::MouseDownEvent {
        position: second_start,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseMoveEvent {
        position: second_end,
        modifiers: Modifiers::default(),
        pressed_button: Some(gpui::MouseButton::Left),
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: second_end,
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    let second_dragged = cx.update(|window, cx| {
        let _ = window.draw(cx);
        gpui_kit::base::TextSelection::selected_text(window, cx)
    });
    assert!(
        second_dragged.contains("B-2")
            && !second_dragged.contains("A-1")
            && !second_dragged.contains("\"qty\": 2"),
        "dragging in the second record must not select the first record's rows, got {second_dragged:?}"
    );
    assert!(
        second_dragged.contains("qty"),
        "the drag reaches the second record's nested row, got {second_dragged:?}"
    );
}
