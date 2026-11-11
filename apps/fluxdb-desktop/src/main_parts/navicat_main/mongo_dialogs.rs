// MongoDB 侧边栏入口的两个弹框：连接信息（服务端拓扑/版本/库大小）与创建集合。
//
// 数据来源均为服务端真实返回：连接信息经 `LoadMongoServerInfo` → `MongoServerInfoLoaded`，
// 创建集合经 `CreateMongoCollection` → `MongoCollectionCreated`（成功后刷新对象树）。
// 本文件只负责编排与渲染，不直连驱动。

impl NavicatMain {
    /// 打开「显示连接信息」弹框并异步读取服务端信息。
    fn show_mongo_info_dialog(&mut self, connection_id: ConnectionId, cx: &mut Context<Self>) {
        self.connection_context_menu = None;
        self.mongo_info_dialog = Some(MongoInfoDialog {
            connection_id,
            state: LoadState::Loading,
        });
        self.load_mongo_server_info(connection_id, cx);
        cx.notify();
    }

    /// 读取（或刷新）连接信息：成功后回写弹框，失败保留错误态可重试。
    fn load_mongo_server_info(&mut self, connection_id: ConnectionId, cx: &mut Context<Self>) {
        if self._mongo_info_tasks.contains_key(&connection_id.0) {
            return;
        }
        let task = self.spawn_controller_command(
            AppCommand::LoadMongoServerInfo(connection_id),
            move |this, event, cx| {
                this._mongo_info_tasks.remove(&connection_id.0);
                let Some(dialog) = this.mongo_info_dialog.as_mut() else {
                    return;
                };
                if dialog.connection_id != connection_id {
                    return;
                }
                dialog.state = match event {
                    AppEvent::MongoServerInfoLoaded(_, result) => match result {
                        Ok(info) => LoadState::Loaded(info),
                        Err(error) => LoadState::Failed(error),
                    },
                    AppEvent::Failed(error) => LoadState::Failed(error),
                    _ => LoadState::Failed(fluxdb_core::UserFacingError::from(
                        fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            tr("app.mongo.info_dialog.error").to_string(),
                        ),
                    )),
                };
                cx.notify();
            },
            cx,
        );
        self._mongo_info_tasks.insert(connection_id.0, task);
    }

    fn close_mongo_info_dialog(&mut self, cx: &mut Context<Self>) {
        self.mongo_info_dialog = None;
        cx.notify();
    }

    /// 打开「创建集合」弹框（仅建集合，不改库）。
    fn show_create_collection_modal(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.database_context_menu = None;
        self.pending_create_collection = Some(MongoCreateCollectionForm {
            connection_id,
            database,
            collection: String::new(),
        });
        self.create_collection_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_create_collection_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_create_collection = None;
        cx.notify();
    }

    /// 提交创建集合：成功后对象树刷新（控制器已替换第一层），弹框关闭。
    fn confirm_create_collection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.pending_create_collection.clone() else {
            return;
        };
        if self.create_collection_running {
            return;
        }
        let collection = form.collection.trim().to_string();
        if collection.is_empty() {
            self.show_message(
                tr("app.mongo.create_collection.field"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.create_collection_running = true;
        let database = form.database.clone();
        let collection_for_task = collection.clone();
        let task = self.spawn_controller_command(
            AppCommand::CreateMongoCollection {
                connection_id: form.connection_id,
                database: form.database.clone(),
                collection,
            },
            move |this, event, cx| {
                this.create_collection_running = false;
                this._create_mongo_collection_task = None;
                match event {
                    AppEvent::MongoCollectionCreated { .. } => {
                        this.pending_create_collection = None;
                        // 控制器只替换了第一层（库列表），该库的集合仍是创建前的快照。
                        // 必须重取子节点，否则新集合要等手动刷新才出现；这里不能用
                        // 直接标记 loaded 的方式，那只是把过期数据当成已加载。
                        let key = database_tree_key(form.connection_id, &database);
                        this.refresh_database_children(
                            ObjectPath {
                                connection_id: form.connection_id,
                                database: Some(database.clone()),
                                schema: None,
                                name: database.clone(),
                                kind: ObjectKind::Database,
                            },
                            key,
                            cx,
                        );
                        this.show_message(
                            tr("app.mongo.create_collection.created")
                                .replace("%{name}", &collection_for_task),
                            AppMessageKind::Success,
                            cx,
                        );
                    }
                    AppEvent::Failed(error) => {
                        this.show_message(
                            tr("app.mongo.create_collection.failed")
                                .replace("%{error}", &error.message),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    _ => {
                        this.show_message(
                            tr("app.mongo.create_collection.failed")
                                .replace("%{error}", tr("app.mongo.error.no_result").as_ref()),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                }
                cx.notify();
            },
            cx,
        );
        self._create_mongo_collection_task = Some(task);
        cx.notify();
    }
}

/// 连接信息弹框：三块（服务器 / 拓扑 / 数据库），字段全部来自服务端。
fn mongo_info_dialog_view(
    dialog: &MongoInfoDialog,
    connection_name: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connection_id = dialog.connection_id;
    let body = match &dialog.state {
        LoadState::NotLoaded | LoadState::Loading => div()
            .flex()
            .items_center()
            .gap_2()
            .py_6()
            .text_size(ui_text_size(13.))
            .text_color(colors.muted)
            .child(loading_spinner_with_color(14., colors.muted))
            .child(tr("app.mongo.info_dialog.loading")),
        LoadState::Failed(error) => div()
            .py_6()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(ui_text_size(13.))
            .child(div().text_color(colors.danger).child(error.message.clone()))
            .child(backup_text_button(
                tr("app.mongo.info_dialog.refresh"),
                colors,
                cx.listener(move |this, _, _, cx| {
                    if let Some(dialog) = this.mongo_info_dialog.as_mut() {
                        dialog.state = LoadState::Loading;
                    }
                    this.load_mongo_server_info(connection_id, cx);
                }),
            )),
        LoadState::Loaded(info) => mongo_info_sections(info, colors),
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
                this.close_mongo_info_dialog(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(520.))
                .max_h(px(560.))
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
                .debug_selector(|| "mongo-info-dialog".into())
                .key_context("MongoInfoModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_mongo_info_dialog(cx);
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
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(app_icon(AppIcon::Info, 16., colors.text))
                                .child(
                                    div()
                                        .text_size(ui_text_size(17.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(tr("app.mongo.info_dialog.title")),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(colors.muted)
                                        .child(connection_name),
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
                                        this.close_mongo_info_dialog(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .py_4()
                        .id("mongo-info-body")
                        .max_h(px(440.))
                        .overflow_y_scrollbar()
                        .child(body),
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
                            backup_text_button(
                                tr("app.mongo.info_dialog.refresh"),
                                colors,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(dialog) = this.mongo_info_dialog.as_mut() {
                                        dialog.state = LoadState::Loading;
                                    }
                                    this.load_mongo_server_info(connection_id, cx);
                                }),
                            )
                            .debug_selector(|| "mongo-info-refresh".into()),
                        )
                        .child(
                            Button::new("mongo-info-close")
                                .debug_selector(|| "mongo-info-close".into())
                                .label(tr("app.mongo.info_dialog.close"))
                                .primary()
                                .w(px(88.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_mongo_info_dialog(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

/// 已加载内容：服务器 / 拓扑 / 数据库三块。缺字段显示「未知」，不编造值。
fn mongo_info_sections(info: &fluxdb_core::MongoServerInfo, colors: UiColors) -> Div {
    let unknown = tr("app.mongo.info.unknown").to_string();
    let topology = match info.topology {
        fluxdb_core::MongoTopology::Standalone => {
            tr("app.mongo.info_dialog.topology_kind.standalone")
        }
        fluxdb_core::MongoTopology::ReplicaSet => {
            tr("app.mongo.info_dialog.topology_kind.replica_set")
        }
        fluxdb_core::MongoTopology::Sharded => tr("app.mongo.info_dialog.topology_kind.sharded"),
        fluxdb_core::MongoTopology::Unknown => tr("app.mongo.info_dialog.topology_kind.unknown"),
    };
    let mut server_rows = vec![
        (
            tr("app.mongo.info_dialog.version").to_string(),
            info.version.clone().unwrap_or_else(|| unknown.clone()),
        ),
        (
            tr("app.mongo.info_dialog.edition").to_string(),
            info.edition.clone().unwrap_or_else(|| unknown.clone()),
        ),
        (
            tr("app.mongo.info_dialog.wire").to_string(),
            info.max_wire_version
                .map(|value| value.to_string())
                .unwrap_or_else(|| unknown.clone()),
        ),
        (
            tr("app.mongo.info_dialog.uptime").to_string(),
            info.uptime_secs
                .map(|secs| {
                    let days = secs / 86_400;
                    let hours = (secs % 86_400) / 3_600;
                    if days > 0 {
                        tr("app.mongo.info_dialog.uptime_value")
                            .replace("%{days}", &days.to_string())
                            .replace("%{hours}", &hours.to_string())
                    } else {
                        tr("app.mongo.info_dialog.uptime_short")
                            .replace("%{hours}", &hours.to_string())
                    }
                })
                .unwrap_or_else(|| unknown.clone()),
        ),
    ];
    server_rows.shrink_to_fit();

    let mut topology_rows = vec![(
        tr("app.mongo.info_dialog.topology").to_string(),
        topology.to_string(),
    )];
    if let Some(name) = &info.replica_set_name {
        topology_rows.push((
            tr("app.mongo.info_dialog.replica_set").to_string(),
            name.clone(),
        ));
    }
    if let Some(primary) = &info.primary {
        topology_rows.push((
            tr("app.mongo.info_dialog.primary").to_string(),
            primary.clone(),
        ));
    }
    if !info.hosts.is_empty() {
        topology_rows.push((
            tr("app.mongo.info_dialog.hosts").to_string(),
            info.hosts.join(", "),
        ));
    }

    let databases = if info.databases.is_empty() {
        div()
            .text_size(ui_text_size(12.))
            .text_color(colors.muted)
            .child(tr("app.mongo.info_dialog.databases_empty"))
    } else {
        let mut list = div().flex().flex_col().gap_1();
        for database in &info.databases {
            let size = database
                .size_on_disk
                .map(|bytes| {
                    tr("app.mongo.info_dialog.database_size")
                        .replace("%{size}", &mongo_human_bytes(bytes))
                })
                .unwrap_or_else(|| tr("app.mongo.info_dialog.database_no_size").to_string());
            let size = if database.empty {
                format!("{size} · {}", tr("app.mongo.info_dialog.database_empty"))
            } else {
                size
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(26.))
                    .px_2()
                    .rounded(colors.radius)
                    .hover(move |style| style.bg(colors.hover))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(ui_text_size(13.))
                            .child(database.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(colors.muted)
                            .child(size),
                    ),
            );
        }
        list
    };

    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(mongo_info_block(
            tr("app.mongo.info_dialog.section.server").to_string(),
            server_rows,
            colors,
        ))
        .child(mongo_info_block(
            tr("app.mongo.info_dialog.section.topology").to_string(),
            topology_rows,
            colors,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.mongo.info_dialog.section.databases")),
                )
                .child(databases),
        )
}

fn mongo_info_block(title: String, rows: Vec<(String, String)>, colors: UiColors) -> Div {
    let mut block = div().flex().flex_col().gap_2().child(
        div()
            .text_size(ui_text_size(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(colors.muted)
            .child(title),
    );
    for (label, value) in rows {
        block = block.child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .w(px(96.))
                        .flex_shrink_0()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(label),
                )
                .child(div().flex_1().min_w(px(0.)).text_size(ui_text_size(13.)).child(value)),
        );
    }
    block
}

/// 「创建集合」弹框：集合名 + 执行/取消。
fn mongo_create_collection_modal(
    form: MongoCreateCollectionForm,
    name_input: Entity<InputState>,
    running: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_submit = !running && !form.collection.trim().is_empty();
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
                this.cancel_create_collection_modal(cx);
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
                .debug_selector(|| "mongo-create-collection-modal".into())
                .key_context("MongoCreateCollectionModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_create_collection_modal(cx);
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
                                .child(app_icon(AppIcon::List, 16., colors.text))
                                .child(
                                    div()
                                        .text_size(ui_text_size(17.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(tr("app.mongo.create_collection.title")),
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
                                        this.cancel_create_collection_modal(cx);
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
                                .child(format!(
                                    "{} · {}",
                                    tr("app.mongo.create_collection.database"),
                                    form.database
                                )),
                        )
                        .child(
                            div()
                                .debug_selector(|| "mongo-create-collection-input".into())
                                .child(create_database_text_field(
                                    tr("app.mongo.create_collection.field"),
                                    name_input,
                                    running,
                                    colors,
                                )),
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
                            Button::new("mongo-create-collection-cancel")
                                .debug_selector(|| "mongo-create-collection-cancel".into())
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_create_collection_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("mongo-create-collection-confirm")
                                .debug_selector(|| "mongo-create-collection-confirm".into())
                                .label(if running {
                                    tr("app.mongo.create_collection.running")
                                } else {
                                    tr("app.common.execute")
                                })
                                .primary()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_create_collection(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}
