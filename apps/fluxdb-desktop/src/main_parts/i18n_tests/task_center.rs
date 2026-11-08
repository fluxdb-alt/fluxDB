// 任务中心：真实生产视图渲染 + 状态栏入口 + 详情抽屉。
//
// 只验证界面装配和选择联动；具体任务执行仍由各功能自己的单测覆盖。

#[gpui::test]
fn task_center_renders_rows_and_detail_drawer(cx: &mut TestAppContext) {
    use crate::{TaskCenterKind, TaskCenterRowKey, TaskCenterStatus};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let storage_probe = storage.clone();
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let controller = AppController::new();
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
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(680.)));
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
            let running_id = this.task_center.begin(
                TaskCenterKind::Restore,
                "Restore database".into(),
                "shop".into(),
                None,
            );
            this.task_center.update_progress(
                running_id,
                None,
                Some("Writing".into()),
                Some("Writing: orders".into()),
            );
            let done_id = this.task_center.begin(
                TaskCenterKind::TableImport,
                "Import table orders".into(),
                "/tmp/orders.csv".into(),
                None,
            );
            this.finish_task_center_record(
                done_id,
                TaskCenterStatus::Success,
                "Completed".into(),
                None,
            );
            this.task_center.open = true;
            this.task_center.selected =
                Some(TaskCenterRowKey::history(TaskCenterKind::Restore, running_id));
            cx.notify();
            running_id
        });
        window.draw(cx).clear(cx);
    });

    assert!(
        cx.debug_bounds("task-center-statusbar-entry").is_some(),
        "status bar must expose the task center entry"
    );
    assert!(
        cx.debug_bounds("task-center-panel").is_some(),
        "task center panel must render"
    );
    // 记录 id 从 1 递增，第一条恢复任务的类型编号固定为 1（Restore），来源为历史。
    assert!(
        cx.debug_bounds("task-center-row:1:1:1").is_some(),
        "running task row must render"
    );
    assert_eq!(
        storage_probe.load_task_history().expect("history").len(),
        1,
        "finished record must be persisted"
    );
    let detail = cx
        .debug_bounds("task-center-detail")
        .expect("selected task detail drawer");
    let panel = cx.debug_bounds("task-center-panel").expect("panel");
    assert!(
        detail.origin.x > panel.origin.x,
        "detail drawer must sit inside the right side of the panel"
    );
    assert!(
        detail.origin.x + detail.size.width <= panel.origin.x + panel.size.width,
        "detail drawer must not be pushed outside the panel"
    );

    // 运行中的任务不出现删除入口；结束后的任务可删除记录。
    assert!(
        cx.debug_bounds("task-center-delete-task").is_none(),
        "running task must not expose delete"
    );
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.task_center.selected =
                Some(TaskCenterRowKey::history(TaskCenterKind::TableImport, 2));
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let delete = cx
        .debug_bounds("task-center-delete-task")
        .expect("delete button for finished task");
    assert!(
        delete.size.width >= gpui::px(60.),
        "delete button must keep its full width"
    );
    cx.simulate_click(delete.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert!(
            app.task_center
                .records
                .iter()
                .all(|record| record.kind != TaskCenterKind::TableImport),
            "finished record must be removed"
        );
        assert!(app.task_center.selected.is_none());
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("task-center-row:4:2:1").is_none(),
        "deleted row must disappear"
    );
    assert!(
        storage_probe
            .load_task_history()
            .expect("history")
            .is_empty(),
        "deleted record must be removed from storage"
    );
}
