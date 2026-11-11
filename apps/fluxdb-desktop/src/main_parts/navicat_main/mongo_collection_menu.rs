// MongoDB 集合右键菜单的业务编排：刷新、重命名、删除、导入入口。
//
// 菜单本身由 `menus_dialogs/context_menus.rs` 渲染，动作在这里落成命令；
// 导出（全量 / 条件 / 消息日志）单独放在 `mongo_export.rs`。
// 本文件不直连驱动：重命名/删除走 `AppCommand`，导入复用集合工作台的导入弹窗。

impl NavicatMain {
    /// 刷新集合：结果条重跑当前查询，侧边栏该库子节点重取（集合信息/索引一并失效）。
    fn refresh_mongo_collection_object(&mut self, object_path: &ObjectPath, cx: &mut Context<Self>) {
        let tab_id = self
            .controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::MongoCollection(state) if state.object == *object_path => Some(tab.id),
                _ => None,
            });
        if let Some(tab_id) = tab_id {
            self.run_mongo_query(tab_id, cx);
        }
        if let Some(database) = object_path.database.clone() {
            self.refresh_database_children(
                ObjectPath {
                    connection_id: object_path.connection_id,
                    database: Some(database.clone()),
                    schema: None,
                    name: database.clone(),
                    kind: ObjectKind::Database,
                },
                database_tree_key(object_path.connection_id, &database),
                cx,
            );
        }
        self.show_message(
            tr("app.table.toast.refresh_requested"),
            AppMessageKind::Success,
            cx,
        );
    }

    /// 打开重命名弹框（仅集合；视图不提供该入口）。
    fn open_rename_collection_modal(
        &mut self,
        object_path: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_rename_collection = Some(PendingRenameCollection {
            new_name: object_path.name.clone(),
            object_path,
            error: None,
        });
        let name = self
            .pending_rename_collection
            .as_ref()
            .map(|form| form.new_name.clone())
            .unwrap_or_default();
        self.rename_collection_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_rename_collection_modal(&mut self, cx: &mut Context<Self>) {
        if self._rename_mongo_collection_task.is_some() {
            return;
        }
        self.pending_rename_collection = None;
        cx.notify();
    }

    /// 提交重命名：成功后重取该库子节点，并提示新名字。
    fn confirm_rename_collection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_rename_collection.clone() else {
            return;
        };
        if self._rename_mongo_collection_task.is_some() {
            return;
        }
        let new_name = form.new_name.trim().to_string();
        if new_name.is_empty() {
            form.error = Some(tr("app.mongo.rename.empty").to_string());
            self.pending_rename_collection = Some(form);
            cx.notify();
            return;
        }
        if new_name == form.object_path.name {
            self.pending_rename_collection = None;
            cx.notify();
            return;
        }
        let Some(database) = form.object_path.database.clone() else {
            return;
        };
        let connection_id = form.object_path.connection_id;
        let collection = form.object_path.name.clone();
        // 后台控制器跑完必须把快照合并回 live 控制器：已打开标签页的标题与对象路径
        // 由 `merge_renamed_mongo_collection_from` 改写，否则标签页仍指向旧集合名。
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (task_database, task_collection, task_new_name) =
                (database.clone(), collection.clone(), new_name.clone());
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::RenameMongoCollection {
                        connection_id,
                        database: task_database,
                        collection: task_collection,
                        new_name: task_new_name,
                    });
                    (controller, event)
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._rename_mongo_collection_task = None;
                    match event {
                        AppEvent::MongoCollectionRenamed { .. } => {
                            this.controller.merge_renamed_mongo_collection_from(
                                &controller,
                                connection_id,
                                &database,
                                &collection,
                                &new_name,
                            );
                            this.pending_rename_collection = None;
                            // 控制器只替换第一层（库列表），该库集合仍是旧快照，必须重取子节点。
                            this.refresh_database_children(
                                ObjectPath {
                                    connection_id,
                                    database: Some(database.clone()),
                                    schema: None,
                                    name: database.clone(),
                                    kind: ObjectKind::Database,
                                },
                                database_tree_key(connection_id, &database),
                                cx,
                            );
                            this.show_message(
                                tr("app.mongo.rename.done")
                                    .replace("%{from}", &collection)
                                    .replace("%{to}", &new_name),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            if let Some(form) = &mut this.pending_rename_collection {
                                form.error = Some(error.message.clone());
                            }
                            this.show_message(
                                tr("app.mongo.rename.failed")
                                    .replace("%{error}", &error.message),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                        _ => {
                            if let Some(form) = &mut this.pending_rename_collection {
                                form.error = Some(tr("app.mongo.error.no_result").to_string());
                            }
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._rename_mongo_collection_task = Some(task);
        cx.notify();
    }

    /// 打开删除集合确认弹框；文档数取自已加载的集合信息（未知则不展示）。
    fn open_drop_collection_modal(&mut self, object_path: ObjectPath, cx: &mut Context<Self>) {
        let document_count = self
            .controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::MongoCollection(state) if state.object == object_path => {
                    match &state.info {
                        LoadState::Loaded(info) => info.document_count,
                        _ => None,
                    }
                }
                _ => None,
            });
        self.pending_drop_collection = Some(PendingDropCollection {
            object_path,
            document_count,
        });
        cx.notify();
    }

    fn cancel_drop_collection_modal(&mut self, cx: &mut Context<Self>) {
        if self._drop_mongo_collection_task.is_some() {
            return;
        }
        self.pending_drop_collection = None;
        cx.notify();
    }

    /// 确认删除：控制器删除集合并关闭对应标签页，这里只负责提示与刷新。
    fn confirm_drop_collection(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_drop_collection.clone() else {
            return;
        };
        if self._drop_mongo_collection_task.is_some() {
            return;
        }
        let Some(database) = form.object_path.database.clone() else {
            return;
        };
        let connection_id = form.object_path.connection_id;
        let collection = form.object_path.name.clone();
        // 删除会关闭对应标签页，合并时必须走 `merge_dropped_mongo_collection_from`
        // 并补 `apply_closed_tabs`，否则界面还留着已删集合的标签。
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (task_database, task_collection) = (database.clone(), collection.clone());
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::DropMongoCollection {
                        connection_id,
                        database: task_database,
                        collection: task_collection,
                    });
                    (controller, event)
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._drop_mongo_collection_task = None;
                    match event {
                        AppEvent::MongoCollectionDropped { .. } => {
                            let previous_tabs = this
                                .controller
                                .state()
                                .tabs
                                .iter()
                                .map(|tab| tab.id)
                                .collect::<Vec<_>>();
                            this.controller.merge_dropped_mongo_collection_from(
                                &controller,
                                connection_id,
                                &database,
                                &collection,
                            );
                            this.apply_closed_tabs(previous_tabs, cx);
                            this.pending_drop_collection = None;
                            this.refresh_database_children(
                                ObjectPath {
                                    connection_id,
                                    database: Some(database.clone()),
                                    schema: None,
                                    name: database.clone(),
                                    kind: ObjectKind::Database,
                                },
                                database_tree_key(connection_id, &database),
                                cx,
                            );
                            this.show_message(
                                tr("app.mongo.drop_collection.done")
                                    .replace("%{name}", &collection),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            this.show_message(
                                tr("app.mongo.drop_collection.failed")
                                    .replace("%{error}", &error.message),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                        _ => {
                            this.show_message(
                                tr("app.mongo.error.no_result"),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._drop_mongo_collection_task = Some(task);
        cx.notify();
    }

    /// 右键「导入」：先打开（或激活）集合标签页，再复用结果条上的导入弹窗。
    fn open_mongo_import_for_object(&mut self, object_path: ObjectPath, cx: &mut Context<Self>) {
        let read_only = object_path.kind == ObjectKind::View;
        let event = self.dispatch(
            AppCommand::OpenMongoCollection {
                object: object_path,
                read_only,
            },
            cx,
        );
        let tab_id = match event {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => Some(tab_id),
            _ => self.controller.state().active_tab,
        };
        let Some(tab_id) = tab_id else {
            self.show_message(
                tr("app.mongo.import.open_failed"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        };
        self.open_mongo_import_dialog(tab_id, cx);
    }
}

/// 「重命名集合」弹框：单个输入框，回车即提交。
fn mongo_rename_collection_modal(
    form: PendingRenameCollection,
    name_input: Entity<InputState>,
    running: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_submit = !running && !form.new_name.trim().is_empty();
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_rename_collection_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(440.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .debug_selector(|| "mongo-rename-collection-modal".into())
                .key_context("MongoRenameCollectionModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_rename_collection_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .py_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(app_icon(AppIcon::Edit, 16., colors.text))
                                .child(
                                    div()
                                        .text_size(ui_text_size(17.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(tr("app.mongo.rename.title")),
                                ),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_rename_collection_modal(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.muted)
                                .child(
                                    tr("app.mongo.rename.current")
                                        .replace("%{name}", &form.object_path.name),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "mongo-rename-collection-input".into())
                                .child(create_database_text_field(
                                    tr("app.mongo.rename.field"),
                                    name_input,
                                    running,
                                    colors,
                                )),
                        )
                        .when_some(form.error.clone(), |this, error| {
                            this.child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(colors.danger)
                                    .child(error),
                            )
                        }),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("mongo-rename-collection-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_rename_collection_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("mongo-rename-collection-confirm")
                                .debug_selector(|| "mongo-rename-collection-confirm".into())
                                .label(if running {
                                    tr("app.common.running")
                                } else {
                                    tr("app.common.confirm")
                                })
                                .primary()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_rename_collection(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

/// 「删除集合」确认弹框：不可撤销，删除按钮用危险样式。
fn mongo_drop_collection_modal(
    form: PendingDropCollection,
    running: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let detail = match form.document_count {
        Some(count) => tr("app.mongo.drop_collection.count")
            .replace("%{count}", &count.to_string()),
        None => tr("app.mongo.drop_collection.count_unknown").to_string(),
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_drop_collection_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(420.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .debug_selector(|| "mongo-drop-collection-modal".into())
                .key_context("MongoDropCollectionModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_drop_collection_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.mongo.drop_collection.title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_drop_collection_modal(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(ui_text_size(13.))
                        .child(
                            div()
                                .text_color(colors.text)
                                .child(
                                    tr("app.mongo.drop_collection.message")
                                        .replace("%{name}", &form.object_path.name),
                                ),
                        )
                        .child(div().text_color(colors.muted).child(detail))
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.danger)
                                .child(tr("app.mongo.drop_collection.warning")),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("mongo-drop-collection-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_drop_collection_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("mongo-drop-collection-confirm")
                                .debug_selector(|| "mongo-drop-collection-confirm".into())
                                .label(if running {
                                    tr("app.common.running")
                                } else {
                                    tr("app.mongo.drop_collection.confirm")
                                })
                                .danger()
                                .w(px(96.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_drop_collection(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}
