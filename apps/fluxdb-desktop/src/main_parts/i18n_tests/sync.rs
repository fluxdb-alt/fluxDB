// 数据同步向导：真实生产视图中右键连接 → 打开菜单 → 三步向导 → 预览 → 执行。
// 差异算法与写入细节由 fluxdb-app 的 SQLite 往返测试覆盖，这里只验证界面接线与真实落库。

fn sync_redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn sync_execute_sql(config: &fluxdb_core::ConnectionConfig, sql: &str) {
    use fluxdb_connectors::connector_for;
    use fluxdb_core::{QueryExecutionOptions, QueryMode, QueryRequest};

    let result = connector_for(config)
        .expect("sqlite connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            session_id: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("sqlite execute");
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "SQL 执行失败: {sql}"
    );
}

fn sync_count_rows(config: &fluxdb_core::ConnectionConfig, table: &str) -> usize {
    use fluxdb_connectors::connector_for;
    use fluxdb_core::{QueryExecutionOptions, QueryMode, QueryRequest};

    let result = connector_for(config)
        .expect("sqlite connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            session_id: None,
            text: format!("SELECT id FROM {table} ORDER BY id"),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("sqlite select");
    result.results.iter().map(|page| page.rows.len()).sum()
}

#[test]
fn sync_column_map_and_skip_parsing() {
    let (map, per_table) = crate::parse_column_maps(
        "old_id=id, old_name = name; bad, =x, y=, orders.old_id=order_id, public.items.sku=code",
    );
    assert_eq!(map.get("old_id").map(String::as_str), Some("id"));
    assert_eq!(map.get("old_name").map(String::as_str), Some("name"));
    assert_eq!(map.len(), 2, "无等号或空侧的项必须忽略");
    assert_eq!(
        per_table
            .get("orders")
            .and_then(|map| map.get("old_id"))
            .map(String::as_str),
        Some("order_id")
    );
    assert_eq!(
        per_table
            .get("public.items")
            .and_then(|map| map.get("sku"))
            .map(String::as_str),
        Some("code"),
        "schema 限定的表键按最后一个点切分"
    );

    let overrides =
        crate::parse_type_overrides("orders.price=numeric(12,3); bad\nx=1\norders.qty=bigint");
    assert_eq!(
        overrides
            .get("orders")
            .and_then(|map| map.get("price"))
            .map(String::as_str),
        Some("numeric(12,3)")
    );
    assert_eq!(overrides.len(), 1, "缺少表前缀的项必须忽略");

    assert_eq!(
        crate::split_column_list("a, b;\nc\n, "),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );

    // 比对键：表级用 `表键:列,列`，组之间分号或换行；其余并入全局。
    let (global, per_table) = crate::parse_key_columns(
        "id, tenant_id; orders:order_id, tenant_id\npublic.items:sku; :bad",
    );
    assert_eq!(global, vec!["id".to_string(), "tenant_id".to_string()]);
    assert_eq!(
        per_table.get("orders").map(Vec::as_slice),
        Some(["order_id".to_string(), "tenant_id".to_string()].as_slice())
    );
    assert_eq!(
        per_table.get("public.items").map(Vec::as_slice),
        Some(["sku".to_string()].as_slice())
    );
    assert_eq!(per_table.len(), 2, "缺少表名的项忽略");
    let (global, per_table) = crate::parse_key_columns("  ");
    assert!(global.is_empty() && per_table.is_empty());

    // 单表写回：保留全局与其它表项；传 None 删除该表项。
    let written = crate::write_table_column_list(
        "id, tenant_id; orders:order_id",
        "items",
        Some(vec!["sku".to_string(), "tenant_id".to_string()]),
    );
    assert_eq!(written, "id, tenant_id; items:sku, tenant_id; orders:order_id");
    let written = crate::write_table_column_list(&written, "items", None);
    assert_eq!(written, "id, tenant_id; orders:order_id");
    assert_eq!(crate::write_table_column_list("", "t", Some(Vec::new())), "");
}

#[test]
fn sync_pair_auto_target_falls_back_to_numbered_name() {
    let table = |name: &str| fluxdb_app::SyncTableInfo {
        key: name.into(),
        name: name.into(),
        schema: None,
    };
    let source = table("3d_ip_region");

    // 同名优先。
    let targets = [table("3d_ip_region"), table("3d_ip_region_1")];
    assert_eq!(
        crate::auto_target_in(&targets, &source).map(|t| t.key.as_str()),
        Some("3d_ip_region"),
        "同名表优先于近似名"
    );

    // 无同名时取序号最小的近似表，_1 与 1 都算。
    let targets = [
        table("3d_ip_region_2"),
        table("3d_ip_region1"),
        table("3d_ip_region_log"),
    ];
    assert_eq!(
        crate::auto_target_in(&targets, &source).map(|t| t.key.as_str()),
        Some("3d_ip_region1"),
        "近似名按数字后缀最小者配对，非数字后缀不参与"
    );

    // 没有可接受候选时返回 None，交给界面显示「自动配对」。
    let targets = [table("3d_ip_region_log")];
    assert!(crate::auto_target_in(&targets, &source).is_none());
    assert!(crate::numbered_suffix("3d_ip_region", "3d_ip_region_1").is_some());
    assert!(crate::numbered_suffix("3d_ip_region", "3D_IP_REGION_1").is_some());
    assert!(crate::numbered_suffix("3d_ip_region", "3d_ip_region_log").is_none());
    assert!(crate::numbered_suffix("3d_ip_region", "3d_ip_region").is_none());
}

#[gpui::test]
fn sync_wizard_previews_and_executes_data_sync(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    // 与真实启动一致：执行账本落到同一 profile 存储。
    controller.set_sync_run_storage(storage.clone());
    let connection = |id: u64, name: &str, file: &str| ConnectionConfig {
        id: ConnectionId(id),
        name: name.into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join(file),
            read_only: false,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    let source = connection(1, "sync source", "source.db");
    let target = connection(2, "sync target", "target.db");
    // 宽表：字段多到超过弹框高度，用来验证字段列表能滚动。
    let extra_columns: String = (1..=20).map(|i| format!(", c{i:02} REAL")).collect();
    sync_execute_sql(
        &source,
        &format!(
            "CREATE TABLE sync_orders (id INTEGER PRIMARY KEY, amount REAL NOT NULL{extra_columns})"
        ),
    );
    sync_execute_sql(
        &source,
        "INSERT INTO sync_orders (id, amount) VALUES (1, 10.0), (2, 20.0)",
    );
    sync_execute_sql(
        &target,
        &format!(
            "CREATE TABLE sync_orders (id INTEGER PRIMARY KEY, amount REAL NOT NULL{extra_columns})"
        ),
    );
    sync_execute_sql(
        &target,
        "INSERT INTO sync_orders (id, amount) VALUES (1, 10.0)",
    );
    controller.dispatch(AppCommand::ReplaceConnections(vec![source, target.clone()]));
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
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(900.)));
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
    sync_redraw(cx);

    let row = cx.debug_bounds("connection-row:1").expect("connection row");
    cx.simulate_event(gpui::MouseDownEvent {
        position: row.center(),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    sync_redraw(cx);
    let entry = cx
        .debug_bounds("connection-menu-item:Cross-connection data transfer")
        .expect("data sync entry in connection menu");
    cx.simulate_click(entry.center(), Modifiers::default());
    sync_redraw(cx);

    assert!(
        cx.debug_bounds("sync-wizard-modal").is_some(),
        "wizard must open from the real connection menu"
    );
    assert!(cx.debug_bounds("sync-mode-tab:structure").is_some());
    assert!(cx.debug_bounds("sync-mode-tab:data").is_some());
    // SQLite 没有 schema 概念，两端都不该出现 Schema 选择器。
    assert!(
        cx.debug_bounds("sync-schema-source").is_none(),
        "source endpoint must hide the schema select for SQLite"
    );
    assert!(
        cx.debug_bounds("sync-schema-target").is_none(),
        "target endpoint must hide the schema select for SQLite"
    );

    // 点遮罩不能关向导：只允许「取消」或右上角关闭按钮退出。
    let modal = cx.debug_bounds("sync-wizard-modal").expect("modal");
    cx.simulate_click(
        gpui::point(modal.origin.x + px(6.), modal.origin.y + px(6.)),
        Modifiers::default(),
    );
    sync_redraw(cx);
    assert!(
        cx.debug_bounds("sync-wizard-modal").is_some(),
        "clicking the backdrop must not close the wizard"
    );

    cx.run_until_parked();
    sync_redraw(cx);
    let tables = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .tables
            .clone()
    });
    assert_eq!(tables.len(), 1, "source table list must load");
    let selected = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .selected_tables
            .len()
    });
    assert_eq!(selected, 0, "tables must not be preselected");
    let select_all = cx.debug_bounds("sync-select-all").expect("select all button");
    cx.simulate_click(select_all.center(), Modifiers::default());
    sync_redraw(cx);
    let selected = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .selected_tables
            .len()
    });
    assert_eq!(selected, 1, "select all ticks every visible table");

    // 布局对齐设计稿：左栏端点卡片，右侧表清单，底部固定操作栏。
    let rail = cx.debug_bounds("sync-pick-rail").expect("endpoint rail");
    let panel = cx.debug_bounds("sync-table-panel").expect("table panel");
    assert!(
        rail.origin.x + rail.size.width <= panel.origin.x,
        "endpoint rail must sit left of the table panel: rail={rail:?} panel={panel:?}"
    );
    assert!(
        panel.size.width > rail.size.width * 2.,
        "table panel must take the remaining width: rail={rail:?} panel={panel:?}"
    );
    assert!(
        cx.debug_bounds("sync-selected-bar").is_some(),
        "table panel keeps a footer with the selected count"
    );
    assert!(
        cx.debug_bounds("sync-table-search").is_some(),
        "table panel keeps the table search box"
    );
    let footer = cx.debug_bounds("sync-wizard-footer").expect("wizard footer");
    assert!(
        panel.origin.y + panel.size.height <= footer.origin.y + px(1.),
        "table panel must end above the footer: panel={panel:?} footer={footer:?}"
    );

    // 第一步 → 表配对 → 选项 → 预览；第四步的比对在后台完成。
    for index in 0..3 {
        let next = cx.debug_bounds("sync-wizard-next").expect("next button");
        cx.simulate_click(next.center(), Modifiers::default());
        sync_redraw(cx);
        if index == 0 {
            cx.run_until_parked();
            sync_redraw(cx);
            assert!(
                cx.debug_bounds("sync-pair-row:sync_orders").is_some(),
                "selected source tables expose a target pairing dropdown"
            );
            let pairs = cx.update(|_, cx| view.read(cx).sync_pair_rows.len());
            assert_eq!(pairs, 1, "one pairing row per selected source table");
            // 下拉默认选中自动匹配到的同名表，不再是空白「请选择」。
            let selected_target = cx.update(|_, cx| {
                view.read(cx).sync_pair_rows[0]
                    .select
                    .read(cx)
                    .selected_value()
                    .cloned()
            });
            assert_eq!(
                selected_target.as_deref(),
                Some("sync_orders"),
                "pair row preselects the auto-matched target table"
            );
            // 表头列与行内下拉同宽同起点，避免出现「目标表」标题错位。
            let header = cx
                .debug_bounds("sync-pair-header-target")
                .expect("pair header target");
            let target = cx
                .debug_bounds("sync-pair-target:sync_orders")
                .expect("pair target select");
            assert!(
                (header.origin.x - target.origin.x).abs() < px(1.)
                    && (header.size.width - target.size.width).abs() < px(1.),
                "pair header must align with the dropdown column: header={header:?} target={target:?}"
            );

            // 字段映射弹框：按真实两端列读取，逐列下拉，取消后回到配对列表。
            let button = cx
                .debug_bounds("sync-field-map:sync_orders")
                .expect("field mapping button");
            cx.simulate_click(button.center(), Modifiers::default());
            sync_redraw(cx);
            cx.run_until_parked();
            sync_redraw(cx);
            assert!(
                cx.debug_bounds("sync-field-map-dialog").is_some(),
                "field mapping dialog must open over the wizard"
            );
            let column_row = cx
                .debug_bounds("sync-field-map-row:id")
                .expect("dialog lists real source columns");
            assert!(
                column_row.size.height > px(20.),
                "field mapping rows must have real height, not a collapsed list: {column_row:?}"
            );
            // 源字段文字要有宽度，目标字段列头与行内下拉同起点同宽。
            let source_label = cx
                .debug_bounds("sync-field-map-source:id")
                .expect("field mapping source label");
            assert!(
                source_label.size.width > px(10.),
                "field mapping source column must not collapse: {source_label:?}"
            );
            let map_header = cx
                .debug_bounds("sync-field-map-header-target")
                .expect("field mapping header");
            let map_target = cx
                .debug_bounds("sync-field-map-target:id")
                .expect("field mapping target select");
            assert!(
                (map_header.origin.x - map_target.origin.x).abs() < px(1.)
                    && (map_header.size.width - map_target.size.width).abs() < px(1.),
                "field mapping header must align with the dropdown column: header={map_header:?} target={map_target:?}"
            );
            // 比对键默认取目标主键；跳过字段默认为空。
            let (keys, skips) = cx.update(|_, cx| {
                let dialog = view
                    .read(cx)
                    .sync_field_map
                    .as_ref()
                    .expect("field map state");
                (dialog.key_columns.clone(), dialog.skip_columns.clone())
            });
            assert_eq!(
                keys,
                std::collections::BTreeSet::from(["id".to_string()]),
                "key columns default to the target primary key"
            );
            assert!(skips.is_empty(), "skip columns start empty");

            // 勾选额外的比对键与跳过字段，保存后写回表级文本。
            let amount_key = cx
                .debug_bounds("sync-field-key:amount")
                .expect("key chip");
            cx.simulate_click(amount_key.center(), Modifiers::default());
            sync_redraw(cx);
            let c01_skip = cx
                .debug_bounds("sync-field-skip:c01")
                .expect("skip chip");
            cx.simulate_click(c01_skip.center(), Modifiers::default());
            sync_redraw(cx);
            // 字段多于可视高度时列表要能滚动，不能被弹框裁掉。
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: map_target.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-120.))),
                modifiers: Modifiers::default(),
                touch_phase: gpui::TouchPhase::Moved,
            });
            sync_redraw(cx);
            let (offset, max_offset) = cx.update(|_, cx| {
                let dialog = view
                    .read(cx)
                    .sync_field_map
                    .as_ref()
                    .expect("field map state");
                (dialog.scroll.offset(), dialog.scroll.max_offset())
            });
            assert!(
                max_offset.y > px(0.) && offset.y < px(0.),
                "field mapping list must scroll: offset={offset:?} max={max_offset:?}"
            );
            let save = cx
                .debug_bounds("sync-field-map-save")
                .expect("field settings save");
            cx.simulate_click(save.center(), Modifiers::default());
            sync_redraw(cx);
            assert!(
                cx.debug_bounds("sync-field-map-dialog").is_none(),
                "save closes the field settings dialog"
            );
            let (key_text, skip_text) = cx.update(|_, cx| {
                let state = view.read(cx).sync_wizard.as_ref().expect("wizard state");
                (state.key_columns_text.clone(), state.skip_columns_text.clone())
            });
            assert!(
                key_text.contains("sync_orders:") && key_text.contains("amount"),
                "saving writes the per-table key text: {key_text:?}"
            );
            assert!(
                skip_text.contains("sync_orders:") && skip_text.contains("c01"),
                "saving writes the per-table skip text: {skip_text:?}"
            );
        }
        if index == 1 {
            let column = cx
                .debug_bounds("sync-options-column")
                .expect("options column");
            let modal = cx.debug_bounds("sync-wizard-modal").expect("modal");
            let column_center = column.origin.x + column.size.width / 2.;
            let modal_center = modal.origin.x + modal.size.width / 2.;
            assert!(
                (column_center - modal_center).abs() < px(2.),
                "options column must be centered: column={column:?} modal={modal:?}"
            );
            assert!(
                cx.debug_bounds("sync-footer-hint").is_some(),
                "footer keeps the step hint"
            );
            // 冲突策略：默认跳过失败行，可切到「停止当前表」再切回。
            let policy = |cx: &mut gpui::VisualTestContext| {
                cx.update(|_, cx| {
                    view.read(cx)
                        .sync_wizard
                        .as_ref()
                        .expect("wizard state")
                        .data_options
                        .row_error_policy
                })
            };
            assert_eq!(
                policy(cx),
                fluxdb_app::RowErrorPolicy::Skip,
                "默认跳过失败行"
            );
            let stop = cx
                .debug_bounds("sync-row-error-stop")
                .expect("stop policy row");
            cx.simulate_click(stop.center(), Modifiers::default());
            sync_redraw(cx);
            assert_eq!(policy(cx), fluxdb_app::RowErrorPolicy::Stop);
            let skip = cx
                .debug_bounds("sync-row-error-skip")
                .expect("skip policy row");
            cx.simulate_click(skip.center(), Modifiers::default());
            sync_redraw(cx);
            assert_eq!(policy(cx), fluxdb_app::RowErrorPolicy::Skip);
            // 唯一键冲突：默认按失败行处理，可切到忽略/覆盖再切回。
            let conflict = |cx: &mut gpui::VisualTestContext| {
                cx.update(|_, cx| {
                    view.read(cx)
                        .sync_wizard
                        .as_ref()
                        .expect("wizard state")
                        .data_options
                        .insert_conflict
                })
            };
            assert_eq!(
                conflict(cx),
                fluxdb_core::InsertConflictMode::Error,
                "默认按失败行处理"
            );
            let ignore = cx
                .debug_bounds("sync-insert-conflict-ignore")
                .expect("ignore conflict row");
            cx.simulate_click(ignore.center(), Modifiers::default());
            sync_redraw(cx);
            assert_eq!(conflict(cx), fluxdb_core::InsertConflictMode::Ignore);
            let overwrite = cx
                .debug_bounds("sync-insert-conflict-overwrite")
                .expect("overwrite conflict row");
            cx.simulate_click(overwrite.center(), Modifiers::default());
            sync_redraw(cx);
            assert_eq!(conflict(cx), fluxdb_core::InsertConflictMode::Overwrite);
            let error = cx
                .debug_bounds("sync-insert-conflict-error")
                .expect("error conflict row");
            cx.simulate_click(error.center(), Modifiers::default());
            sync_redraw(cx);
            assert_eq!(conflict(cx), fluxdb_core::InsertConflictMode::Error);
            // 表级事务：默认按批提交，可切到整表原子再切回。
            let atomic = |cx: &mut gpui::VisualTestContext| {
                cx.update(|_, cx| {
                    view.read(cx)
                        .sync_wizard
                        .as_ref()
                        .expect("wizard state")
                        .data_options
                        .atomic_table
                })
            };
            assert!(!atomic(cx), "默认按批提交");
            // 表级事务卡位于滚动区底部，先滚到可见范围再点；
            // 落在窗口/底栏覆盖区的点击会被静默丢弃。
            let mut last_y = f32::NAN;
            let mut lines = -3.;
            for _ in 0..12 {
                let row = cx
                    .debug_bounds("sync-atomic-on")
                    .expect("atomic on row");
                let footer = cx
                    .debug_bounds("sync-footer-hint")
                    .expect("footer hint");
                let y = f32::from(row.center().y);
                if y < f32::from(footer.origin.y) - 8. {
                    break;
                }
                if (y - last_y).abs() < 0.5 {
                    lines = -lines;
                }
                last_y = y;
                let center = cx.update(|window, _| window.bounds().center());
                cx.simulate_event(gpui::ScrollWheelEvent {
                    position: center,
                    delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
                    ..Default::default()
                });
                sync_redraw(cx);
            }
            let atomic_on = cx
                .debug_bounds("sync-atomic-on")
                .expect("atomic on row");
            cx.simulate_click(atomic_on.center(), Modifiers::default());
            sync_redraw(cx);
            assert!(atomic(cx));
            let atomic_off = cx
                .debug_bounds("sync-atomic-off")
                .expect("atomic off row");
            cx.simulate_click(atomic_off.center(), Modifiers::default());
            sync_redraw(cx);
            assert!(!atomic(cx));
            // 导出迁移 SQL：默认不导出；已选路径可清回「不导出」。
            let export_path = |cx: &mut gpui::VisualTestContext| {
                cx.update(|_, cx| {
                    view.read(cx)
                        .sync_wizard
                        .as_ref()
                        .expect("wizard state")
                        .data_options
                        .export_sql_path
                        .clone()
                })
            };
            assert!(export_path(cx).is_none(), "默认不导出");
            cx.update(|_, cx| {
                view.update(cx, |this, cx| {
                    this.sync_wizard
                        .as_mut()
                        .expect("wizard state")
                        .data_options
                        .export_sql_path = Some("/tmp/sync-migration.sql".into());
                    cx.notify();
                });
            });
            let mut last_y = f32::NAN;
            let mut lines = -3.;
            for _ in 0..12 {
                let row = cx
                    .debug_bounds("sync-export-sql-off")
                    .expect("export off row");
                let footer = cx
                    .debug_bounds("sync-footer-hint")
                    .expect("footer hint");
                let y = f32::from(row.center().y);
                if y < f32::from(footer.origin.y) - 8. {
                    break;
                }
                if (y - last_y).abs() < 0.5 {
                    lines = -lines;
                }
                last_y = y;
                let center = cx.update(|window, _| window.bounds().center());
                cx.simulate_event(gpui::ScrollWheelEvent {
                    position: center,
                    delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
                    ..Default::default()
                });
                sync_redraw(cx);
            }
            let export_off = cx
                .debug_bounds("sync-export-sql-off")
                .expect("export off row");
            cx.simulate_click(export_off.center(), Modifiers::default());
            sync_redraw(cx);
            assert!(export_path(cx).is_none(), "「不导出」必须清掉路径");
            // 比对键/字段映射/跳过字段等批量文本收进「高级」，默认收起。
            let expanded = |cx: &mut gpui::VisualTestContext| {
                cx.update(|_, cx| {
                    view.read(cx)
                        .sync_wizard
                        .as_ref()
                        .expect("wizard state")
                        .advanced_expanded
                })
            };
            assert!(!expanded(cx), "advanced card starts collapsed");
            // 新增策略卡后「高级」在滚动区底部，先滚到可见范围再点；
            // 落在窗口外的点击会被静默丢弃。
            let mut last_y = f32::NAN;
            let mut lines = -3.;
            for _ in 0..12 {
                let toggle = cx
                    .debug_bounds("sync-advanced-toggle")
                    .expect("advanced toggle");
                // 底栏覆盖滚动区下沿：中心落在底栏上方的点击才不会被底栏吞掉。
                let footer = cx
                    .debug_bounds("sync-footer-hint")
                    .expect("footer hint");
                let y = f32::from(toggle.center().y);
                if y < f32::from(footer.origin.y) - 8. {
                    break;
                }
                // 滚不动就反向：不同平台的滚轮方向约定不一致。
                if (y - last_y).abs() < 0.5 {
                    lines = -lines;
                }
                last_y = y;
                let center = cx.update(|window, _| window.bounds().center());
                cx.simulate_event(gpui::ScrollWheelEvent {
                    position: center,
                    delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
                    ..Default::default()
                });
                sync_redraw(cx);
            }
            let toggle = cx
                .debug_bounds("sync-advanced-toggle")
                .expect("advanced toggle");
            cx.simulate_click(toggle.center(), Modifiers::default());
            sync_redraw(cx);
            assert!(expanded(cx), "advanced card expands on click");
        }
    }
    cx.run_until_parked();
    let preview = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .data_preview
            .clone()
            .expect("data preview")
    });
    assert_eq!(preview.total_inserted(), 1);
    assert_eq!(preview.total_updated(), 0);
    assert_eq!(sync_count_rows(&target, "sync_orders"), 1);

    // 第三步：统计条在上，左侧对象列表固定窄列，右侧详情占剩余宽度。
    let summary = cx.debug_bounds("sync-preview-summary").expect("summary");
    let list = cx.debug_bounds("sync-preview-list").expect("object list");
    let stats = cx.debug_bounds("sync-preview-footer").expect("stats bar");
    assert!(summary.origin.y + summary.size.height <= list.origin.y + px(1.));
    assert!(
        stats.origin.y >= list.origin.y + list.size.height - px(1.),
        "stats bar sits below the split: list={list:?} stats={stats:?}"
    );
    assert!(
        (list.size.width - px(280.)).abs() < px(1.),
        "object list keeps the narrow column: {list:?}"
    );

    // 运行中关窗只收起界面：同步继续在后台跑，任务中心能回到同一份运行状态。
    const RUNNING_TASK_ID: u64 = 987_654;
    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            let state = this.sync_wizard.as_mut().expect("wizard state");
            state.running = true;
            state.task_center_id = Some(RUNNING_TASK_ID);
        });
    });
    sync_redraw(cx);
    let close = cx
        .debug_bounds("sync-wizard-close-icon")
        .expect("close icon");
    cx.simulate_click(close.center(), Modifiers::default());
    sync_redraw(cx);
    let hidden = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .map(|state| {
                (
                    state.running,
                    state.visible,
                    state.cancel.load(std::sync::atomic::Ordering::Relaxed),
                )
            })
    });
    assert_eq!(
        hidden,
        Some((true, false, false)),
        "运行中关窗必须保留运行状态且不置取消标志"
    );
    assert!(
        cx.debug_bounds("sync-wizard-modal").is_none(),
        "收起的向导不再渲染遮罩"
    );
    cx.update(|_, cx| {
        view.update(cx, |this, cx| this.reopen_sync_wizard(RUNNING_TASK_ID, cx));
    });
    sync_redraw(cx);
    assert!(
        cx.debug_bounds("sync-wizard-modal").is_some(),
        "重新打开的向导回到运行中的预览步骤"
    );
    assert_eq!(
        cx.update(|_, cx| view.read(cx).sync_wizard.as_ref().expect("wizard").step),
        4
    );
    // 复位注入的运行标记，继续走真实执行路径。
    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            let state = this.sync_wizard.as_mut().expect("wizard state");
            state.running = false;
            state.task_center_id = None;
        });
    });
    sync_redraw(cx);

    let run = cx.debug_bounds("sync-wizard-run").expect("run button");
    cx.simulate_click(run.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        sync_count_rows(&target, "sync_orders"),
        2,
        "execute must write the missing row through the real UI path"
    );

    // 执行日志面板：预览步骤底部按「时间 + 文本」逐行展示，而不是只留一行进度。
    sync_redraw(cx);
    let log_panel = cx
        .debug_bounds("sync-log-panel")
        .expect("execution log panel must render");
    assert!(
        log_panel.size.width > px(200.),
        "log panel must span the wizard width: {log_panel:?}"
    );
    let log_lines = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .log
            .iter()
            .map(|entry| (entry.at.clone(), entry.text.clone()))
            .collect::<Vec<_>>()
    });
    assert!(!log_lines.is_empty(), "execution must append log lines");
    assert!(
        log_lines
            .iter()
            .all(|(at, text)| !at.is_empty() && !text.is_empty()),
        "log lines carry time and text: {log_lines:?}"
    );

    // 执行记录回看：落盘后按钮出现在预览底部，点击后弹框展示该表记录。
    let run_id = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .last_run_id
    });
    assert!(run_id.is_some(), "execution must record a run id");
    let open_record = cx
        .debug_bounds("sync-view-run-record")
        .expect("view run record button");
    cx.simulate_click(open_record.center(), Modifiers::default());
    cx.run_until_parked();
    sync_redraw(cx);
    assert!(
        cx.debug_bounds("sync-run-record-modal").is_some(),
        "clicking the footer button opens the run record modal"
    );
    assert!(
        cx.debug_bounds("sync-run-record-table:sync_orders")
            .is_some(),
        "the modal lists the synced table from the local ledger"
    );
    // 块明细按行渲染：行高统一、整行落在弹框内，长键不会把计数挤出可视区。
    let modal_bounds = cx
        .debug_bounds("sync-run-record-modal")
        .expect("record modal");
    let chunk_row = cx
        .debug_bounds("sync-run-chunk:sync_orders:1")
        .expect("first chunk row");
    assert!(
        chunk_row.size.height >= px(20.),
        "chunk row keeps a readable row height: {chunk_row:?}"
    );
    assert!(
        chunk_row.origin.x >= modal_bounds.origin.x
            && chunk_row.origin.x + chunk_row.size.width
                <= modal_bounds.origin.x + modal_bounds.size.width + px(1.),
        "chunk row stays inside the modal: row={chunk_row:?} modal={modal_bounds:?}"
    );
    assert!(
        cx.debug_bounds("sync-run-sample:sync_orders:0").is_some(),
        "row samples render as their own rows"
    );
    let close = cx
        .debug_bounds("sync-run-record-close")
        .expect("record modal close button");
    cx.simulate_click(close.center(), Modifiers::default());
    sync_redraw(cx);
    assert!(
        cx.debug_bounds("sync-run-record-modal").is_none(),
        "close button dismisses the run record modal"
    );

    // 连接菜单入口：不经过向导也能回看最近记录（本地账本）。
    let cancel = cx
        .debug_bounds("sync-wizard-cancel")
        .expect("wizard cancel");
    cx.simulate_click(cancel.center(), Modifiers::default());
    sync_redraw(cx);
    assert!(
        cx.debug_bounds("sync-wizard-modal").is_none(),
        "cancel closes the wizard"
    );
    cx.simulate_event(gpui::MouseDownEvent {
        position: row.center(),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    sync_redraw(cx);
    let run_log_entry = cx
        .debug_bounds("connection-menu-item:Data sync run log")
        .expect("run log entry in connection menu");
    cx.simulate_click(run_log_entry.center(), Modifiers::default());
    cx.run_until_parked();
    sync_redraw(cx);
    assert!(
        cx.debug_bounds("sync-run-record-modal").is_some(),
        "connection menu opens the run log without the wizard"
    );
    // debug_bounds 需要 'static 选择器；测试进程内泄漏一条字符串无碍。
    let expected_row: &'static str =
        format!("sync-run-record-run:{}", run_id.expect("run id")).leak();
    assert!(
        cx.debug_bounds(expected_row).is_some(),
        "run list shows the recorded run"
    );
    assert!(
        cx.debug_bounds("sync-run-record-table:sync_orders")
            .is_some(),
        "latest run is selected by default and shows its table"
    );
    let close = cx
        .debug_bounds("sync-run-record-close")
        .expect("record modal close button");
    cx.simulate_click(close.center(), Modifiers::default());
    sync_redraw(cx);
    assert!(cx.debug_bounds("sync-run-record-modal").is_none());
}

/// 大表预览：超过 10 万行的表只比首块，点一次「下一页」只推进一块，不会自动扫完整表。
#[gpui::test]
fn sync_wizard_previews_big_table_by_page(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    let connection = |id: u64, name: &str, file: &str| ConnectionConfig {
        id: ConnectionId(id),
        name: name.into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join(file),
            read_only: false,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    };
    let source = connection(1, "bulk source", "source.db");
    let target = connection(2, "bulk target", "target.db");
    sync_execute_sql(
        &source,
        "CREATE TABLE bulk_items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    );
    // 源端 12 万行 > 预览自动比对上限（10 万）：进第四步只比首块。
    sync_execute_sql(
        &source,
        "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 120000)
         INSERT INTO bulk_items (id, name) SELECT n, 'v' || n FROM seq",
    );
    // 目标端 10 万行（前 10 万行与源端一致）：目标端比源端先读完，尾部只剩新增。
    sync_execute_sql(
        &target,
        "CREATE TABLE bulk_items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    );
    sync_execute_sql(
        &target,
        "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 100000)
         INSERT INTO bulk_items (id, name) SELECT n, 'v' || n FROM seq",
    );
    controller.dispatch(AppCommand::ReplaceConnections(vec![source, target]));
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
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(900.)));
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
    sync_redraw(cx);

    let row = cx.debug_bounds("connection-row:1").expect("connection row");
    cx.simulate_event(gpui::MouseDownEvent {
        position: row.center(),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    sync_redraw(cx);
    let entry = cx
        .debug_bounds("connection-menu-item:Cross-connection data transfer")
        .expect("data sync entry in connection menu");
    cx.simulate_click(entry.center(), Modifiers::default());
    sync_redraw(cx);
    // 表清单默认不勾选：先「全选」再进入配对。
    cx.run_until_parked();
    sync_redraw(cx);
    let select_all = cx.debug_bounds("sync-select-all").expect("select all");
    cx.simulate_click(select_all.center(), Modifiers::default());
    sync_redraw(cx);
    for _ in 0..3 {
        let next = cx.debug_bounds("sync-wizard-next").expect("next button");
        cx.simulate_click(next.center(), Modifiers::default());
        sync_redraw(cx);
        cx.run_until_parked();
        sync_redraw(cx);
    }

    let page = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, cx| {
            view.read(cx)
                .sync_wizard
                .as_ref()
                .expect("wizard state")
                .data_preview
                .as_ref()
                .expect("data preview")
                .tables
                .iter()
                .find(|table| table.key == "bulk_items")
                .cloned()
                .expect("bulk table preview")
        })
    };
    let first = page(cx);
    assert!(!first.page_done, "大表首块不应一次比完: {first:?}");
    assert_eq!(
        first.compared_rows(),
        4096,
        "大表预览只比首块: {first:?}"
    );
    assert!(
        cx.debug_bounds("sync-data-pager").is_some(),
        "未比完时显示分页器"
    );

    // 点一次「下一页」只再比一块，不自动续跑。
    // 100 条样本把分页器推到滚动区底部，先滚动右侧详情再点击（窗口外的点击会被丢弃）。
    let next_page: &'static str = "sync-next-page:bulk_items";
    let list = cx.debug_bounds("sync-preview-list").expect("object list");
    let detail_point = gpui::point(
        list.origin.x + list.size.width + px(200.),
        list.origin.y + list.size.height / 2.,
    );
    for _ in 0..12 {
        let Some(button) = cx.debug_bounds(next_page) else {
            panic!("next page button missing");
        };
        if button.origin.y + button.size.height < px(890.) {
            break;
        }
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: detail_point,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-400.))),
            modifiers: Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        sync_redraw(cx);
    }
    let button = cx.debug_bounds(next_page).expect("next page button");
    cx.simulate_click(button.center(), Modifiers::default());
    cx.run_until_parked();
    sync_redraw(cx);

    let second = page(cx);
    let (page_running, preview_error) = cx.update(|_, cx| {
        let state = view.read(cx).sync_wizard.as_ref().expect("wizard state");
        (state.page_running, state.preview_error.clone())
    });
    assert!(
        !second.page_done,
        "点一次只推进一块: page_running={page_running} preview_error={preview_error:?}"
    );
    assert_eq!(
        second.compared_rows(),
        8192,
        "第二块只多比 4096 行: {second:?}"
    );
    assert!(
        cx.debug_bounds("sync-data-pager").is_some(),
        "未比完时保留分页器"
    );
}

#[gpui::test]
fn sync_wizard_switches_to_structure_mode(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    let connection = |id: u64, name: &str, file: &str| ConnectionConfig {
        id: ConnectionId(id),
        name: name.into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join(file),
            read_only: false,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    let source = connection(1, "structure source", "source.db");
    let target = connection(2, "structure target", "target.db");
    sync_execute_sql(
        &source,
        "CREATE TABLE structure_orders (id INTEGER PRIMARY KEY, amount REAL NOT NULL)",
    );
    sync_execute_sql(
        &target,
        "CREATE TABLE structure_orders (id INTEGER PRIMARY KEY)",
    );
    controller.dispatch(AppCommand::ReplaceConnections(vec![source, target.clone()]));
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
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(900.)));
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
    sync_redraw(cx);

    let row = cx.debug_bounds("connection-row:1").expect("connection row");
    cx.simulate_event(gpui::MouseDownEvent {
        position: row.center(),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    sync_redraw(cx);
    let entry = cx
        .debug_bounds("connection-menu-item:Structure compare & sync")
        .expect("structure sync entry in connection menu");
    cx.simulate_click(entry.center(), Modifiers::default());
    sync_redraw(cx);
    assert!(cx.debug_bounds("sync-wizard-modal").is_some());

    cx.run_until_parked();
    sync_redraw(cx);
    // 表清单默认不勾选：先「全选」再进入配对。
    let select_all = cx.debug_bounds("sync-select-all").expect("select all");
    cx.simulate_click(select_all.center(), Modifiers::default());
    sync_redraw(cx);
    for index in 0..3 {
        let next = cx.debug_bounds("sync-wizard-next").expect("next button");
        cx.simulate_click(next.center(), Modifiers::default());
        sync_redraw(cx);
        if index == 0 {
            // 结构同步的表配对行给「类型映射」：逐列覆盖目标类型。
            let button = cx
                .debug_bounds("sync-type-map:structure_orders")
                .expect("type mapping button");
            cx.simulate_click(button.center(), Modifiers::default());
            sync_redraw(cx);
            cx.run_until_parked();
            sync_redraw(cx);
            assert!(
                cx.debug_bounds("sync-type-map-dialog").is_some(),
                "type mapping dialog must open"
            );
            let row = cx
                .debug_bounds("sync-type-map-row:amount")
                .expect("type mapping row");
            assert!(
                row.size.height > px(20.),
                "type mapping rows must have real height: {row:?}"
            );
            let input = cx
                .debug_bounds("sync-type-map-input:amount")
                .expect("type mapping input");
            cx.simulate_click(input.center(), Modifiers::default());
            sync_redraw(cx);
            cx.simulate_keystrokes("t e x t");
            sync_redraw(cx);
            let save = cx
                .debug_bounds("sync-type-map-save")
                .expect("type mapping save");
            cx.simulate_click(save.center(), Modifiers::default());
            sync_redraw(cx);
            assert!(
                cx.debug_bounds("sync-type-map-dialog").is_none(),
                "save closes the type mapping dialog"
            );
            let text = cx.update(|_, cx| {
                view.read(cx)
                    .sync_wizard
                    .as_ref()
                    .expect("wizard state")
                    .type_overrides_text
                    .clone()
            });
            assert_eq!(
                text, "structure_orders.amount=text",
                "saving writes the per-table type override"
            );
        }
        if index == 1 {
            let column = cx
                .debug_bounds("sync-options-column")
                .expect("options column");
            let modal = cx.debug_bounds("sync-wizard-modal").expect("modal");
            assert!(
                ((column.origin.x + column.size.width / 2.)
                    - (modal.origin.x + modal.size.width / 2.))
                    .abs()
                    < px(2.),
                "structure options column must be centered: column={column:?}"
            );
        }
    }
    cx.run_until_parked();
    let preview = cx.update(|_, cx| {
        view.read(cx)
            .sync_wizard
            .as_ref()
            .expect("wizard state")
            .structure_preview
            .clone()
            .expect("structure preview")
    });
    let table = preview
        .tables
        .iter()
        .find(|table| table.key == "structure_orders")
        .expect("table diff");
    // 目标端缺 amount 列：默认选项只新增，选中 1 项变更。
    assert_eq!(table.selected, 1);
    assert_eq!(table.changes.len(), 1);
    // 差异视图依赖两端整表定义：目标端现状没有 amount，源端目标态有。
    let current = table.current_ddl.as_deref().expect("target ddl");
    let desired = table.desired_ddl.as_deref().expect("source ddl");
    assert!(!current.contains("amount"), "current ddl: {current}");
    assert!(desired.contains("amount"), "desired ddl: {desired}");

    let run = cx.debug_bounds("sync-wizard-run").expect("run button");
    cx.simulate_click(run.center(), Modifiers::default());
    cx.run_until_parked();
    // 目标表能查询 amount，说明 ALTER 真正落库。
    sync_execute_sql(&target, "SELECT amount FROM structure_orders");
}
