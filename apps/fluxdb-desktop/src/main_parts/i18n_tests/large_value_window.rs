#[gpui::test]
fn large_value_detail_window_loads_chunks_from_content_store(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, CellContentFileStore, TabKind};
    use fluxdb_core::{
        CellContentStore as _, CellValue, Column, ConnectionConfig, ConnectionId, DataPage,
        DatabaseKind, Endpoint, LargeCellValue, LargeValueKind, ObjectKind, ObjectPath, Row,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    // 大字段内容走真实受控存储：用例不伪造读取结果，块必须能拼回原内容。
    let store_root = temp.path().join("content-store");
    let store = std::sync::Arc::new(CellContentFileStore::new(&store_root));
    // 开头是连续 ASCII，没有空格断点，正是「大字段长行不换行」的复现场景。
    let mut payload = "a".repeat(120_000);
    payload.push_str(&"汉字与ASCII混排".repeat(60_000));
    let content_id = store
        .put("result-test-1", LargeValueKind::Text, payload.as_bytes())
        .expect("store large value");

    let mut controller = AppController::new();
    controller.set_cell_content_storage(storage.clone());
    fluxdb_core::register_cell_content_store(store.clone());
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "large value fixture".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("large.sqlite"),
            read_only: true,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
        mongo_profile: None,
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
    let tab_id = cx.update(|_, cx| {
        let event = view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenDataEditor(ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".into()),
                    schema: None,
                    name: "Notes".into(),
                    kind: ObjectKind::Table,
                }),
                cx,
            )
        });
        let AppEvent::TabOpened(tab_id) = event else {
            panic!("open data editor: {event:?}");
        };
        tab_id
    });
    // 用真实加载回写路径装一页结果：大字段单元格只有预览 + 内容引用。
    let big = CellValue::LargeValue(LargeCellValue {
        content_id: content_id.clone(),
        kind: LargeValueKind::Text,
        byte_length: Some(payload.len() as u64),
        preview: payload.chars().take(64).collect(),
        preview_truncated: true,
    });
    let generation = cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let TabKind::DataEditor(editor) =
            &state.tabs.iter().find(|t| t.id == tab_id).unwrap().kind
        else {
            panic!("data editor tab");
        };
        editor.load_generation
    });
    cx.update(|_, cx| {
        let event = view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::FinishDataPageLoad {
                    tab_id,
                    generation,
                    result: Ok(DataPage {
                        columns: vec![Column {
                            name: "note".into(),
                            type_name: Some("text".into()),
                            nullable: true,
                            primary_key: false,
                            comment: None,
                        }],
                        rows: vec![Row {
                            values: vec![big.clone()],
                        }],
                        offset: 0,
                        limit: 100,
                        has_more: false,
                        incomplete: None,
                    }),
                },
                cx,
            )
        });
        assert!(
            matches!(event, AppEvent::DataLoaded(..)),
            "page load: {event:?}"
        );
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(900.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));

    // 悬停出详情入口，再由真实点击打开详情（不直接调处理函数）。
    // 第 0 列是行号列，数据列从 1 开始（`source_column_indexes` 首项为 None）。
    let cell = cx.debug_bounds("data-cell:0:1").expect("data cell");
    cx.simulate_mouse_move(cell.center(), None, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let detail = cx
        .debug_bounds("data-cell-detail:0:1")
        .expect("cell detail action");
    cx.simulate_click(detail.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let opened = cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let TabKind::DataEditor(editor) = &state.tabs.iter().find(|t| t.id == tab_id).unwrap().kind
        else {
            panic!("data editor tab");
        };
        (editor.cell_detail_panel.open, editor.cell_detail_panel.active_cell)
    });
    let windowed = cx.update(|_, cx| view.read(cx).large_value_windows.contains_key(&fluxdb_app::QueryPaneKey::new(tab_id, crate::DATA_EDITOR_PANE)));
    assert!(
        cx.debug_bounds("large-value-window").is_some(),
        "大字段必须进入分段只读窗口（panel={opened:?}, window={windowed}）"
    );
    cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let TabKind::DataEditor(editor) =
            &state.tabs.iter().find(|t| t.id == tab_id).unwrap().kind
        else {
            panic!("data editor tab");
        };
        assert_eq!(editor.cell_detail_panel.mode, fluxdb_app::CellDetailMode::View);
    });

    // 首块由窗口打开后自动读入；等到后台任务落地。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let loaded = cx.update(|_, cx| {
            view.read(cx)
                .large_value_windows
                .get(&fluxdb_app::QueryPaneKey::new(tab_id, crate::DATA_EDITOR_PANE))
                .is_some_and(|window| !window.view.chunks.is_empty())
        });
        if loaded {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "首块未读入详情窗口"
        );
        std::thread::yield_now();
    }
    // 继续分段读：点真实「读取更多」按钮，窗口必须按字节偏移推进。
    let before = cx.update(|_, cx| {
        view.read(cx)
            .large_value_windows
            .get(&fluxdb_app::QueryPaneKey::new(tab_id, crate::DATA_EDITOR_PANE))
            .map(|window| window.view.next_read_offset())
            .unwrap_or(0)
    });
    assert!(before > 0, "首块应有实际字节");
    if let Some(button) = cx.debug_bounds("large-value-load-more") {
        cx.simulate_click(button.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // 读取中按钮不能消失，要原地变成转圈，否则点完看不出是在读还是出错。
        let loading = cx.update(|_, cx| {
            view.read(cx)
                .large_value_windows
                .get(&fluxdb_app::QueryPaneKey::new(tab_id, crate::DATA_EDITOR_PANE))
                .is_some_and(|window| window.view.loading)
        });
        if loading {
            assert!(
                cx.debug_bounds("large-value-load-more").is_some(),
                "读取中「读取更多」按钮不能消失"
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            let advanced = cx.update(|_, cx| {
                view.read(cx)
                    .large_value_windows
                    .get(&fluxdb_app::QueryPaneKey::new(tab_id, crate::DATA_EDITOR_PANE))
                    .is_some_and(|window| window.view.next_read_offset() > before)
            });
            if advanced {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "「读取更多」没有推进窗口"
            );
            std::thread::yield_now();
        }
    }
    // 窗口拼接内容必须等于原内容前缀：块来自同一份不可变内容。
    cx.update(|_, cx| {
        let window = view.read(cx).large_value_windows.get(&fluxdb_app::QueryPaneKey::new(tab_id, crate::DATA_EDITOR_PANE)).unwrap();
        let text = window.view.window_text();
        assert!(
            payload.starts_with(&text),
            "分块拼接必须等于原内容前缀（{} vs {}）",
            text.len(),
            payload.len()
        );
        assert!(!window.view.is_complete(), "远未读完，不能标为完整");
    });
    // 长文本必须折行：内容宽度不超出窗口，行高远超单行；否则会变成一条横向长条。
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let text_bounds = cx.debug_bounds("large-value-text").expect("大字段文本");
    let window_width = cx
        .debug_bounds("large-value-window")
        .expect("大字段窗口")
        .size
        .width;
    assert!(
        text_bounds.size.width <= window_width,
        "文本宽度 {} 超过窗口 {}，没有折行",
        text_bounds.size.width,
        window_width
    );
    assert!(
        text_bounds.size.height > gpui::px(200.),
        "文本高度 {} 说明仍是单行",
        text_bounds.size.height
    );
    // 操作按钮与其它单元格类型一致，放在「值」标题行：必须在文本区上方。
    let save = cx.debug_bounds("large-value-save").expect("下载按钮");
    assert!(
        save.bottom() <= text_bounds.top(),
        "下载按钮底部 {} 不在标题行（文本区顶部 {}）",
        save.bottom(),
        text_bounds.top()
    );
    if let Some(button) = cx.debug_bounds("large-value-load-more") {
        assert!(
            button.bottom() <= text_bounds.top(),
            "「读取更多」不在标题行"
        );
    }
    // 大小/范围状态不能另占一行：它必须和标题、按钮同处标题行。
    let status = cx
        .debug_bounds("large-value-status")
        .expect("状态文字必须与标题同行");
    assert!(
        status.bottom() <= text_bounds.top(),
        "状态文字跑到标题行下面了"
    );
    assert!(
        (status.top() - save.top()).abs() < gpui::px(6.),
        "状态文字与按钮不在同一行：{} vs {}",
        status.top(),
        save.top()
    );
    // 大文本列必须给出上传替换入口，且和下载一样在标题行（点开是系统文件选择框，
    // 无头环境不点，只验证真实渲染出来的按钮位置）。
    let upload = cx
        .debug_bounds("large-value-upload")
        .expect("大文本列的「上传替换」按钮");
    assert!(
        upload.bottom() <= text_bounds.top(),
        "「上传替换」不在标题行"
    );
    // 大字段是「只是预览」的值：标题行不能出现复制与就地编辑。
    assert!(
        cx.debug_bounds("cell-detail-copy").is_none(),
        "大字段不能提供复制整值"
    );
}
