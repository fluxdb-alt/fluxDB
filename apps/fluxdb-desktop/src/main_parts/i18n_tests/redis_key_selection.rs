// 新建 Key 后留在当前 Redis 库标签页：刷新结果到达时按键名选中新键，
// 不再像以前那样为单个键另开标签页（对齐 RedisInsight 的 AddKey 行为）。
//
// 不连真实 Redis：直接写入一条已加载的键列表，再由真实生产视图消费 DataLoaded。

#[gpui::test]
fn redis_created_key_is_selected_in_the_current_db_tab(cx: &mut TestAppContext) {
    use crate::RedisKeyListMode;
    use fluxdb_app::{AppCommand, AppEvent, TabKind};
    use fluxdb_core::{
        CellValue, Column, ConnectionConfig, ConnectionId, DataPage, DatabaseKind, Endpoint,
        ObjectKind, ObjectPath, Row,
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
    let connection_id = ConnectionId(7);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "redis selection".into(),
        kind: DatabaseKind::Redis,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 6379,
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
    let AppEvent::TabOpened(tab_id) =
        controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id,
            database: Some("0".into()),
            schema: None,
            name: "0".into(),
            kind: ObjectKind::RedisDb,
        }))
    else {
        panic!("open redis db tab without connecting")
    };

    // 刷新回来的键列表：新键不在首位，验证是按「键名」定位而不是默认选第一行。
    let page = DataPage {
        columns: ["键", "类型", "值", "TTL", "大小"]
            .into_iter()
            .map(|name| Column {
                name: name.into(),
                type_name: None,
                nullable: false,
                primary_key: false,
                comment: None,
            })
            .collect(),
        rows: vec![
            Row {
                values: ["alpha", "string", "v", "120", "1 B"]
                    .into_iter()
                    .map(|value| CellValue::Text(value.into()))
                    .collect(),
            },
            Row {
                values: ["orders:new", "hash", "", "无 TTL", "2 B"]
                    .into_iter()
                    .map(|value| CellValue::Text(value.into()))
                    .collect(),
            },
        ],
        offset: 0,
        limit: 100,
        has_more: false,
        incomplete: None,
    };
    assert!(matches!(
        controller.dispatch(AppCommand::FinishDataPageLoad {
            tab_id,
            generation: 0,
            result: Ok(page.clone()),
        }),
        AppEvent::DataLoaded(..)
    ));

    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
    });
    let page_for_view = page.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = navicat_main_new(
                window,
                cx,
                controller,
                storage,
                gpui_component::ThemeMode::Light,
            );
            view.data_table_state(tab_id, &page_for_view, window, cx);
            view
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

    // 平铺模式：刷新落地后自动选中新键，标签页数量保持不变。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.set_redis_key_pending_selection(tab_id, "orders:new".into());
            this.apply_app_event(&AppEvent::DataLoaded(tab_id, page.clone()), cx);
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let this = view.read(cx);
        let state = this.controller.state();
        assert_eq!(state.tabs.len(), 1, "must stay in the current DB tab");
        assert!(matches!(
            &state.tabs[0].kind,
            TabKind::DataEditor(editor) if editor.object.kind == ObjectKind::RedisDb
        ));
        assert!(
            this.redis_key_pending_selection.is_empty(),
            "pending selection is consumed once"
        );
        let table = this
            .data_table_states
            .get(&tab_id)
            .expect("table state")
            .read(cx);
        let delegate = table.delegate();
        let selected = delegate
            .selected_row
            .or_else(|| delegate.selected_cell.map(|(row, _)| row));
        let selected_key = selected
            .and_then(|row| delegate.source_row_indexes.get(row).copied())
            .and_then(|row| page.rows.get(row))
            .and_then(|row| row.values.first())
            .map(|value| value.display_label());
        assert_eq!(selected_key.as_deref(), Some("orders:new"));
    });

    // Folder 模式：展开祖先 folder 后按叶子键选中，详情抽屉仍跟随同一个键。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.redis_key_list_modes
                .insert(tab_id, RedisKeyListMode::Folder);
            this.set_redis_key_pending_selection(tab_id, "orders:new".into());
            this.apply_app_event(&AppEvent::DataLoaded(tab_id, page.clone()), cx);
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let this = view.read(cx);
        assert_eq!(
            this.redis_key_list_selected_leaf(tab_id).as_deref(),
            Some("orders:new")
        );
        assert!(
            this.redis_key_list_expanded(tab_id).contains("orders"),
            "ancestor folders expand so the leaf is visible"
        );
        assert!(this.redis_key_pending_selection.is_empty());
        assert_eq!(this.controller.state().tabs.len(), 1);
    });
}
