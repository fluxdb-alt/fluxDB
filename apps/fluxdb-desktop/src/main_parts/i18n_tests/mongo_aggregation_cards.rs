// MongoDB 聚合管道的卡片视图：真实生产视图的无头点击驱动。
//
// 不连库：只验卡片 ↔ 管道文本的往返、增删改移与运算符下拉。
// 单阶段预览不在这里点：它会真的建连，无服务时 mongo 驱动要等服务端选择超时
// （默认 30s）才失败，用例会白白卡住。预览与正式聚合共用 `mongo_aggregate`，
// 结果链路归连接器用例。

#[gpui::test]
fn mongo_aggregation_cards_edit_the_pipeline(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, LoadState};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
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
    let connection_id = ConnectionId(21);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "mongo aggregation".into(),
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
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(AppCommand::SelectMongoSection {
                tab_id,
                section: crate::MongoSection::Aggregation,
            }, cx);
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    // 默认管道只有一个 `$group`，卡片模式必须把它摊成一张卡。
    let pipeline_text = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, cx| {
            view.read(cx)
                .mongo_query_inputs
                .get(&tab_id)
                .expect("query inputs")
                .read(cx)
                .aggregation
                .read(cx)
                .value()
                .to_string()
        })
    };
    assert!(
        cx.debug_bounds("mongo-aggregation-stage:0").is_some(),
        "the default pipeline renders as one stage card"
    );
    assert!(
        cx.debug_bounds("mongo-aggregation-stage:1").is_none(),
        "one stage means exactly one card"
    );
    assert!(pipeline_text(cx).contains("$group"));
    let window_bounds = cx.update(|window, _| window.bounds());

    // 卡身左栏是可编辑的参数框：宽度必须真的分到空间。分隔线换成
    // `h_resizable` 之后左栏一度拿到 0 宽——文字看不见、点也点不进去，
    // 且不报错，只有量边界能发现。
    let body_bounds = cx
        .debug_bounds("mongo-aggregation-body:0")
        .expect("the stage body editor renders");
    assert!(
        body_bounds.size.width > gpui::px(200.),
        "the body editor must get real width: {body_bounds:?}"
    );
    assert!(window_bounds.contains(&body_bounds.center()));

    // 添加阶段：新卡追加在末尾，并立刻写进管道文本。
    let add = cx
        .debug_bounds("mongo-aggregation-add")
        .expect("add stage button renders");
    assert!(
        window_bounds.contains(&add.center()),
        "the add button must stay inside the window: {add:?} {window_bounds:?}"
    );
    cx.simulate_click(add.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("mongo-aggregation-stage:1").is_some());
    let text = pipeline_text(cx);
    assert!(
        text.contains("$group") && text.contains("$match"),
        "the new stage is written into the pipeline: {text}"
    );
    // 新卡片的参数框带「说明注释 + 可直接跑的示例值」，抄 Compass 的写法。
    let body_text = |cx: &mut gpui::VisualTestContext, index: usize| {
        cx.update(|_, cx| {
            view.read(cx)
                .mongo_aggregation_builders
                .get(&tab_id)
                .expect("builder")
                .stages[index]
                .body
                .read(cx)
                .value()
                .to_string()
        })
    };
    let body = body_text(cx, 1);
    assert!(
        body.starts_with("/**") && body.trim_end().ends_with("{}"),
        "a new stage starts from its comment template: {body}"
    );
    assert!(
        !text.contains("/**"),
        "the comment stays in the editor, not in the pipeline text: {text}"
    );

    // 停用第一张卡：卡片留着，管道里不再有它。
    let toggle = cx
        .debug_bounds("mongo-aggregation-toggle:0")
        .expect("toggle button renders");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let text = pipeline_text(cx);
    assert!(
        !text.contains("$group") && text.contains("$match"),
        "a disabled stage stays on screen but leaves the pipeline: {text}"
    );
    assert!(
        cx.debug_bounds("mongo-aggregation-stage:1").is_some(),
        "disabling keeps the card"
    );
    let toggle = cx
        .debug_bounds("mongo-aggregation-toggle:0")
        .expect("toggle button renders");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(pipeline_text(cx).contains("$group"));

    // 下移第一张卡：`$match` 排到前面，文本顺序跟着换。
    let down = cx
        .debug_bounds("mongo-aggregation-down:0")
        .expect("move down button renders");
    cx.simulate_click(down.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let text = pipeline_text(cx);
    let match_at = text.find("$match").expect("$match still present");
    let group_at = text.find("$group").expect("$group still present");
    assert!(
        match_at < group_at,
        "moving a card down reorders the pipeline: {text}"
    );
    // 第一张卡的上移按钮必须置灰（不可点）：它在最顶上。
    let up = cx
        .debug_bounds("mongo-aggregation-up:0")
        .expect("move up button renders");
    cx.simulate_click(up.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        pipeline_text(cx),
        text,
        "moving the first card up is a no-op"
    );

    // 预览区常驻在卡片右栏（对齐 Compass）：运行按钮在窗口内、面板已渲染。
    let preview = cx
        .debug_bounds("mongo-aggregation-preview:0")
        .expect("preview button renders");
    assert!(window_bounds.contains(&preview.center()));
    assert!(
        cx.debug_bounds("mongo-aggregation-preview-body:0").is_some(),
        "the stage preview pane is always on screen"
    );
    // 管道一变，已有预览必须失效：留着旧结果会让人以为「改完没生效」。
    // 先摆一个「已有预览」的状态，再走真实的停用点击触发一次管道重写。
    // （参数框的 `set_value` 不会发 Change 事件——那是程序化写入的语义，
    // 走它测不到东西；停用与改参数最终都落到同一个 sync。）
    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            let cell = &mut this
                .mongo_aggregation_builders
                .get_mut(&tab_id)
                .expect("builder")
                .stages[0];
            cell.preview = Some(LoadState::Loaded(fluxdb_core::MongoDocumentsPage {
                documents: Vec::new(),
                returned: 0,
                total: None,
                has_more: false,
            }));
            cell.preview_pipeline = Some(String::new());
        });
    });
    let toggle = cx
        .debug_bounds("mongo-aggregation-toggle:0")
        .expect("toggle button renders");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    let stale = cx.update(|_, cx| {
        view.read(cx)
            .mongo_aggregation_builders
            .get(&tab_id)
            .expect("builder")
            .stages[0]
            .preview
            .clone()
    });
    assert_eq!(
        stale,
        Some(LoadState::NotLoaded),
        "changing the pipeline invalidates the stage preview"
    );
    // 预览结果按文档卡片排（复用文档页字段列表），不是整段 JSON：
    // 字段行能逐层展开、类型着色，和 Compass 的 output preview 同一形态。
    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            this.mongo_aggregation_builders.get_mut(&tab_id).expect("builder").stages[0]
                .preview = Some(LoadState::Loaded(fluxdb_core::MongoDocumentsPage {
                documents: vec![fluxdb_core::MongoDocument {
                    id: "66f000000000000000000001".into(),
                    json: r#"{"_id": {"$oid": "66f000000000000000000001"}, "name": "probe", "nested": {"x": 1}}"#
                        .into(),
                }],
                returned: 1,
                total: None,
                has_more: false,
            }));
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-aggregation-preview-card:0").is_some(),
        "a loaded preview renders document cards"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_some(),
        "the preview card body is the document field list"
    );

    // 还原启用态，后面的用例按「两张卡都启用」继续。
    let toggle = cx
        .debug_bounds("mongo-aggregation-toggle:0")
        .expect("toggle button renders");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();

    // 折叠只收起内容，卡片与管道都不动。
    let collapse = cx
        .debug_bounds("mongo-aggregation-collapse:0")
        .expect("collapse arrow renders");
    cx.simulate_click(collapse.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-aggregation-preview-body:0").is_none(),
        "collapsing hides the body"
    );
    assert!(cx.debug_bounds("mongo-aggregation-stage:0").is_some());
    let collapse = cx
        .debug_bounds("mongo-aggregation-collapse:0")
        .expect("collapse arrow renders");
    cx.simulate_click(collapse.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-aggregation-preview-body:0").is_some(),
        "expanding brings the body back"
    );

    // 移除第一张卡：管道只剩一张。
    let remove = cx
        .debug_bounds("mongo-aggregation-remove:0")
        .expect("remove button renders");
    cx.simulate_click(remove.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("mongo-aggregation-stage:1").is_none());
    let text = pipeline_text(cx);
    assert!(
        text.contains("$group") && !text.contains("$match"),
        "removing the first card drops exactly that stage: {text}"
    );

    // 运算符下拉：能输入筛选，且只在参数框还是默认模板时才换模板（同 Compass 的
    // `changeStageOperator`）。两条都是「看着能用、实际改坏用户输入」的回归点，
    // 所以走真实点击 + 真实键盘。
    //
    // 拿新加的卡来验替换：它的参数就是该运算符的默认模板；管道里原有的 `$group`
    // 是用户/默认管道写的参数，本来就不该被换掉。
    let stage_key = |cx: &mut gpui::VisualTestContext, index: usize| -> String {
        cx.update(|_, cx| {
            view.read(cx)
                .mongo_aggregation_builders
                .get(&tab_id)
                .expect("builder")
                .stages[index]
                .key
                .clone()
        })
    };
    let add = cx
        .debug_bounds("mongo-aggregation-add")
        .expect("add stage button renders");
    cx.simulate_click(add.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // 收起第一张卡，让第二张的卡头（运算符下拉）落进卡片区的可视范围：
    // 卡片区封顶 320px，两张展开的卡会把下面的按钮和卡头挤出视口。
    let collapse = cx
        .debug_bounds("mongo-aggregation-collapse:0")
        .expect("collapse arrow renders");
    cx.simulate_click(collapse.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let key_cell = cx
        .debug_bounds("mongo-aggregation-key:1")
        .expect("operator dropdown renders");
    assert!(
        window_bounds.contains(&key_cell.center()),
        "the operator dropdown must stay inside the window: {key_cell:?} {window_bounds:?}"
    );
    cx.simulate_click(key_cell.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // 输入 `lim`：候选里只有 `$limit` 命中。筛选没生效的话，回车会落在第一项
    // `$match`，下面的断言就会失败。
    cx.simulate_keystrokes("l i m");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        stage_key(cx, 1),
        "$limit",
        "typing filters the operator list"
    );
    // 没动过的默认参数跟着换：`$match` 的模板换成 `$limit` 的（注释 + `10`）。
    let replaced = body_text(cx, 1);
    assert!(
        replaced.starts_with("/**") && replaced.trim_end().ends_with("10"),
        "an untouched default body is replaced with the new operator's template: {replaced}"
    );
    assert!(
        pipeline_text(cx).contains("\"$limit\": 10"),
        "the new operator's body reaches the pipeline: {}",
        pipeline_text(cx)
    );

    // 手写过的参数不许被换运算符冲掉：换个运算符就把用户刚敲的 JSON 清空，
    // 是最难恢复的一类误操作。
    let hand_written = "{\"edited\": 1}".to_string();
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            let body = this.mongo_aggregation_builders[&tab_id].stages[1]
                .body
                .clone();
            body.update(cx, |editor, cx| {
                editor.set_value(hand_written.clone(), window, cx);
            });
        });
    });
    let key_cell = cx
        .debug_bounds("mongo-aggregation-key:1")
        .expect("operator dropdown renders");
    cx.simulate_click(key_cell.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("s k i");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(stage_key(cx, 1), "$skip", "the dropdown picks $skip");
    assert_eq!(
        body_text(cx, 1),
        hand_written,
        "a hand-edited body survives an operator change"
    );
}
