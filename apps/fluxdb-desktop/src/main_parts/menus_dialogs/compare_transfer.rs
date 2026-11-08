// 结构对比 / 同步与跨连接数据传输弹框。
//
// 对比与传输都在后台线程执行：前者只读元数据，后者按源表分页读、目标表分批写。
// 弹框只保存表单与最近一次结果，不把大结果集放进 UI 状态。


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompareTransferTab {
    Structure,
    Data,
}

#[derive(Clone, Debug)]
struct CompareTransferState {
    tab: CompareTransferTab,
    source_connection_id: ConnectionId,
    target_connection_id: Option<ConnectionId>,
    source_database: String,
    source_schema: String,
    target_database: String,
    target_schema: String,
    source_table: String,
    target_table: String,
    compare_running: bool,
    compare: Option<StructureCompareResult>,
    compare_error: Option<String>,
    selected_tables: BTreeSet<String>,
    selected_table: Option<String>,
    sync_added: bool,
    drop_removed: bool,
    recreate_modified: bool,
    sync_running: bool,
    transfer_mode: DataTransferMode,
    transfer_create_target: bool,
    transfer_running: bool,
    transfer_error: Option<String>,
    transfer_log: Vec<String>,
    task_center_id: Option<u64>,
    cancel: Arc<AtomicBool>,
}

struct CompareTransferInputs {
    source_database: Entity<SelectState<SearchableVec<String>>>,
    source_schema: Entity<SelectState<SearchableVec<String>>>,
    target_connection: Entity<SelectState<SearchableVec<String>>>,
    target_database: Entity<SelectState<SearchableVec<String>>>,
    target_schema: Entity<SelectState<SearchableVec<String>>>,
    source_table: Entity<SelectState<SearchableVec<String>>>,
    target_table: Entity<SelectState<SearchableVec<String>>>,
    _subscriptions: Vec<Subscription>,
}

/// 单端下拉候选项。空列表表示连接未就绪或读取失败。
#[derive(Clone, Debug, Default)]
struct CompareTransferEndpointOptions {
    databases: Vec<String>,
    schemas: Vec<String>,
    tables: Vec<String>,
}

fn new_compare_transfer_inputs(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> CompareTransferInputs {
    let source_database = new_compare_transfer_select(window, cx);
    let source_schema = new_compare_transfer_select(window, cx);
    let target_connection = new_compare_transfer_select(window, cx);
    let target_database = new_compare_transfer_select(window, cx);
    let target_schema = new_compare_transfer_select(window, cx);
    let source_table = new_compare_transfer_select(window, cx);
    let target_table = new_compare_transfer_select(window, cx);
    let subscriptions = vec![
        subscribe_compare_transfer_select(
            &source_database,
            window,
            cx,
            CompareTransferField::SourceDatabase,
        ),
        subscribe_compare_transfer_select(
            &source_schema,
            window,
            cx,
            CompareTransferField::SourceSchema,
        ),
        subscribe_compare_transfer_target_select(&target_connection, window, cx),
        subscribe_compare_transfer_select(
            &target_database,
            window,
            cx,
            CompareTransferField::TargetDatabase,
        ),
        subscribe_compare_transfer_select(
            &target_schema,
            window,
            cx,
            CompareTransferField::TargetSchema,
        ),
        subscribe_compare_transfer_select(
            &source_table,
            window,
            cx,
            CompareTransferField::SourceTable,
        ),
        subscribe_compare_transfer_select(
            &target_table,
            window,
            cx,
            CompareTransferField::TargetTable,
        ),
    ];
    CompareTransferInputs {
        source_database,
        source_schema,
        target_connection,
        target_database,
        target_schema,
        source_table,
        target_table,
        _subscriptions: subscriptions,
    }
}

fn new_compare_transfer_select(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Entity<SelectState<SearchableVec<String>>> {
    cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompareTransferField {
    SourceDatabase,
    SourceSchema,
    SourceTable,
    TargetDatabase,
    TargetSchema,
    TargetTable,
}

impl CompareTransferField {
    /// 写入表单值，返回值是否真的变化。
    fn set(self, state: &mut CompareTransferState, value: String) -> bool {
        let slot = match self {
            Self::SourceDatabase => &mut state.source_database,
            Self::SourceSchema => &mut state.source_schema,
            Self::SourceTable => &mut state.source_table,
            Self::TargetDatabase => &mut state.target_database,
            Self::TargetSchema => &mut state.target_schema,
            Self::TargetTable => &mut state.target_table,
        };
        if *slot == value {
            return false;
        }
        *slot = value;
        true
    }
}

fn subscribe_compare_transfer_select(
    select: &Entity<SelectState<SearchableVec<String>>>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    field: CompareTransferField,
) -> Subscription {
    cx.subscribe_in(
        select,
        window,
        move |this: &mut NavicatMain,
              _select,
              event: &SelectEvent<SearchableVec<String>>,
              window,
              cx| {
            let SelectEvent::Confirm(value) = event;
            let value = value.clone().unwrap_or_default();
            let changed = this
                .compare_transfer
                .as_mut()
                .map(|state| {
                    let changed = field.set(state, value);
                    if changed {
                        state.compare = None;
                        state.compare_error = None;
                    }
                    changed
                })
                .unwrap_or(false);
            cx.notify();
            if changed {
                // 库/schema 变化会影响下一级候选项；统一走一次后台刷新，保持实现简单。
                this.refresh_compare_transfer_options(window, cx);
            }
        },
    )
}

/// 目标连接下拉：值是连接名，确认后按名字反查连接 id 并重建目标端候选项。
/// 连接数多时用下拉，避免标签行无限变长挤占弹框高度。
fn subscribe_compare_transfer_target_select(
    select: &Entity<SelectState<SearchableVec<String>>>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Subscription {
    cx.subscribe_in(
        select,
        window,
        move |this: &mut NavicatMain,
              _select,
              event: &SelectEvent<SearchableVec<String>>,
              window,
              cx| {
            let SelectEvent::Confirm(Some(name)) = event else {
                return;
            };
            let target = this
                .controller
                .connection_configs()
                .into_iter()
                .find(|connection| connection.name == *name)
                .map(|connection| connection.id);
            if let Some(target) = target
                && this
                    .compare_transfer
                    .as_ref()
                    .and_then(|state| state.target_connection_id)
                    != Some(target)
            {
                this.set_compare_transfer_target(target, window, cx);
            }
        },
    )
}

impl NavicatMain {
    fn show_compare_transfer_modal(
        &mut self,
        connection_id: ConnectionId,
        tab: CompareTransferTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connections = self.controller.connection_configs();
        let Some(source) = connections
            .iter()
            .find(|connection| connection.id == connection_id)
        else {
            self.show_message(
                tr("app.compare_transfer.connection_missing"),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        let target = connections
            .iter()
            .find(|connection| {
                connection.id != connection_id
                    && !matches!(connection.kind, DatabaseKind::Redis | DatabaseKind::MongoDb)
            })
            .map(|connection| connection.id)
            // 只有源连接时允许把源连接作为目标（同库不同 Schema 的场景仍可用）。
            .or(Some(source.id));
        // 目标连接下拉的候选项：SQL 类连接，含源连接（同连接跨库/跨 Schema 复制）。
        let target_connection_names: Vec<String> = connections
            .iter()
            .filter(|connection| {
                !matches!(connection.kind, DatabaseKind::Redis | DatabaseKind::MongoDb)
            })
            .map(|connection| connection.name.clone())
            .collect();
        let target_connection_name = target
            .and_then(|id| connections.iter().find(|connection| connection.id == id))
            .map(|connection| connection.name.clone());
        let source_database = default_compare_database(&self.controller, connection_id);
        // 两端默认库各自取自己的配置；沿用源库名会在跨连接时指向不存在的库。
        let target_database = target
            .map(|connection_id| default_compare_database(&self.controller, connection_id))
            .unwrap_or_else(|| source_database.clone());
        for select in [
            &self.compare_transfer_inputs.source_database,
            &self.compare_transfer_inputs.source_schema,
            &self.compare_transfer_inputs.target_database,
            &self.compare_transfer_inputs.target_schema,
            &self.compare_transfer_inputs.source_table,
            &self.compare_transfer_inputs.target_table,
        ] {
            select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(Vec::new()), window, cx);
                select.set_selected_index(None, window, cx);
            });
        }
        self.compare_transfer_inputs
            .target_connection
            .update(cx, |select, cx| {
                select.set_items(SearchableVec::new(target_connection_names), window, cx);
                match target_connection_name {
                    Some(name) => select.set_selected_value(&name, window, cx),
                    None => select.set_selected_index(None, window, cx),
                }
            });
        self.compare_transfer = Some(CompareTransferState {
            tab,
            source_connection_id: source.id,
            target_connection_id: target,
            source_database: source_database.clone(),
            source_schema: String::new(),
            target_database,
            target_schema: String::new(),
            source_table: String::new(),
            target_table: String::new(),
            compare_running: false,
            compare: None,
            compare_error: None,
            selected_tables: BTreeSet::new(),
            selected_table: None,
            sync_added: true,
            drop_removed: false,
            recreate_modified: false,
            sync_running: false,
            transfer_mode: DataTransferMode::Append,
            transfer_create_target: false,
            transfer_running: false,
            transfer_error: None,
            transfer_log: Vec::new(),
            task_center_id: None,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        // 下拉候选项来自真实对象树，后台读取避免阻塞弹框打开。
        self.refresh_compare_transfer_options(window, cx);
        cx.notify();
    }

    /// 重新读取两端数据库 / Schema / 表候选项。库或 Schema 变化会连带刷新下一级。
    fn refresh_compare_transfer_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_compare_transfer_endpoint(true, window, cx);
        self.refresh_compare_transfer_endpoint(false, window, cx);
    }

    /// 单端刷新。两端独立提交，避免一端连接超时拖住另一端的下拉候选项。
    fn refresh_compare_transfer_endpoint(
        &mut self,
        source: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.compare_transfer.as_ref() else {
            return;
        };
        let snapshot = if source {
            Some((
                state.source_connection_id,
                state.source_database.clone(),
                state.source_schema.clone(),
            ))
        } else {
            state.target_connection_id.map(|connection_id| {
                (
                    connection_id,
                    state.target_database.clone(),
                    state.target_schema.clone(),
                )
            })
        };
        let Some(snapshot) = snapshot else {
            return;
        };
        let controller = self.controller.clone();
        let snapshot_for_read = snapshot.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let options = cx
                .background_spawn(async move {
                    compare_transfer_endpoint_options(&controller, snapshot_for_read)
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                let Some(state) = this.compare_transfer.as_ref() else {
                    return;
                };
                let current = if source {
                    Some((
                        state.source_connection_id,
                        state.source_database.clone(),
                        state.source_schema.clone(),
                    ))
                } else {
                    state.target_connection_id.map(|connection_id| {
                        (
                            connection_id,
                            state.target_database.clone(),
                            state.target_schema.clone(),
                        )
                    })
                };
                // 后台读取期间用户可能已改选端点；丢弃过期结果，避免下拉回跳。
                if current.as_ref() != Some(&snapshot) {
                    return;
                }
                this.apply_compare_transfer_endpoint_options(source, options, window, cx);
            });
        });
        if source {
            self._compare_transfer_source_options_task = Some(task);
        } else {
            self._compare_transfer_target_options_task = Some(task);
        }
    }

    fn apply_compare_transfer_endpoint_options(
        &mut self,
        source: bool,
        options: CompareTransferEndpointOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.compare_transfer.as_ref() else {
            return;
        };
        let (database, schema, table, values) = if source {
            (
                &self.compare_transfer_inputs.source_database,
                &self.compare_transfer_inputs.source_schema,
                &self.compare_transfer_inputs.source_table,
                (
                    state.source_database.clone(),
                    state.source_schema.clone(),
                    state.source_table.clone(),
                ),
            )
        } else {
            (
                &self.compare_transfer_inputs.target_database,
                &self.compare_transfer_inputs.target_schema,
                &self.compare_transfer_inputs.target_table,
                (
                    state.target_database.clone(),
                    state.target_schema.clone(),
                    state.target_table.clone(),
                ),
            )
        };
        let selected = [
            (database, options.databases, values.0),
            (schema, options.schemas, values.1),
            (table, options.tables, values.2),
        ];
        for (select, items, value) in selected {
            select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(items), window, cx);
                select.set_selected_index(None, window, cx);
                if !value.is_empty() {
                    select.set_selected_value(&value, window, cx);
                }
            });
        }
    }

    fn close_compare_transfer_modal(&mut self, cx: &mut Context<Self>) {
        // 关闭只收起弹框；运行中的任务继续在后台执行，取消需显式点「取消任务」。
        self.compare_transfer = None;
        cx.notify();
    }

    fn set_compare_transfer_tab(&mut self, tab: CompareTransferTab, cx: &mut Context<Self>) {
        if let Some(state) = self.compare_transfer.as_mut() {
            state.tab = tab;
            cx.notify();
        }
    }

    fn set_compare_transfer_target(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let database = default_compare_database(&self.controller, connection_id);
        if let Some(state) = self.compare_transfer.as_mut() {
            state.target_connection_id = Some(connection_id);
            // 切换目标连接后沿用旧库名可能在该连接上不存在，直接回到目标自己的默认库。
            state.target_database = database;
            state.target_schema = String::new();
            state.target_table = String::new();
            state.compare = None;
            state.compare_error = None;
        }
        self.refresh_compare_transfer_options(window, cx);
        cx.notify();
    }

    fn start_compare_structure(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.compare_transfer.as_ref() else {
            return;
        };
        if state.compare_running {
            return;
        }
        let source_database = state.source_database.trim().to_string();
        let target_database = state.target_database.trim().to_string();
        let Some(target_connection_id) = state.target_connection_id else {
            self.compare_transfer_error(tr("app.compare_transfer.choose_target").to_string(), cx);
            return;
        };
        if source_database.is_empty() || target_database.is_empty() {
            self.compare_transfer_error(tr("app.compare_transfer.database_required").to_string(), cx);
            return;
        }
        let request = fluxdb_app::StructureCompareRequest {
            source: StructureEndpoint {
                connection_id: state.source_connection_id,
                database: source_database,
                schema: optional_text(&state.source_schema),
            },
            target: StructureEndpoint {
                connection_id: target_connection_id,
                database: target_database,
                schema: optional_text(&state.target_schema),
            },
        };
        if let Some(state) = self.compare_transfer.as_mut() {
            state.compare_running = true;
            state.compare_error = None;
            state.cancel = Arc::new(AtomicBool::new(false));
        }
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { controller.compare_structures(&request) })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let Some(state) = this.compare_transfer.as_mut() else {
                        return;
                    };
                    state.compare_running = false;
                    match result {
                        Ok(result) => {
                            state.selected_tables = result
                                .tables
                                .iter()
                                .filter(|table| table.status != StructureDiffStatus::Same)
                                .map(|table| table.key.clone())
                                .collect();
                            state.selected_table = result
                                .tables
                                .iter()
                                .find(|table| table.status != StructureDiffStatus::Same)
                                .map(|table| table.key.clone());
                            state.compare = Some(result);
                            state.compare_error = None;
                        }
                        Err(error) => {
                            state.compare = None;
                            state.compare_error = Some(error.message);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._compare_transfer_task = Some(task);
        cx.notify();
    }

    fn toggle_compare_transfer_table(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(state) = self.compare_transfer.as_mut() {
            state.selected_table = Some(key.clone());
            if !state.selected_tables.remove(&key) {
                state.selected_tables.insert(key);
            }
            cx.notify();
        }
    }

    fn toggle_compare_transfer_all(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.compare_transfer.as_mut() {
            let all = state
                .compare
                .as_ref()
                .map(|result| {
                    result
                        .tables
                        .iter()
                        .filter(|table| table.status != StructureDiffStatus::Same)
                        .map(|table| table.key.clone())
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            if state.selected_tables.len() == all.len() {
                state.selected_tables.clear();
            } else {
                state.selected_tables = all;
            }
            cx.notify();
        }
    }

    fn start_sync_structure(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.compare_transfer.as_ref() else {
            return;
        };
        if state.sync_running || state.compare_running {
            return;
        }
        let Some(target_connection_id) = state.target_connection_id else {
            self.compare_transfer_error(tr("app.compare_transfer.choose_target").to_string(), cx);
            return;
        };
        if state.selected_tables.is_empty() {
            self.compare_transfer_error(tr("app.compare_transfer.choose_tables").to_string(), cx);
            return;
        }
        let request = fluxdb_app::StructureSyncRequest {
            compare: fluxdb_app::StructureCompareRequest {
                source: StructureEndpoint {
                    connection_id: state.source_connection_id,
                    database: state.source_database.trim().to_string(),
                    schema: optional_text(&state.source_schema),
                },
                target: StructureEndpoint {
                    connection_id: target_connection_id,
                    database: state.target_database.trim().to_string(),
                    schema: optional_text(&state.target_schema),
                },
            },
            tables: state.selected_tables.iter().cloned().collect(),
            options: StructureSyncOptions {
                sync_added: state.sync_added,
                drop_removed: state.drop_removed,
                recreate_modified: state.recreate_modified,
            },
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let task_id = self.task_center.begin(
            TaskCenterKind::StructureSync,
            tr("app.compare_transfer.sync_task").to_string(),
            target_label(&self.controller, target_connection_id),
            Some(cancel.clone()),
        );
        if let Some(state) = self.compare_transfer.as_mut() {
            state.sync_running = true;
            state.task_center_id = Some(task_id);
            state.cancel = cancel.clone();
            state.transfer_log.clear();
        }
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    controller.sync_structures(&request, &cancel, &mut |message| {
                        tracing::info!(target: "gdb_compare_transfer", message, "结构同步");
                    })
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let (status, stage, error) = match &result {
                        Ok(report) if report.errors.is_empty() => (
                            TaskCenterStatus::Success,
                            tr("app.compare_transfer.sync_done").to_string(),
                            None,
                        ),
                        Ok(report) => (
                            TaskCenterStatus::Failed,
                            tr("app.compare_transfer.sync_partial").to_string(),
                            Some(report.errors.join("\n")),
                        ),
                        Err(error) if error.kind == fluxdb_core::ErrorKind::Cancelled => (
                            TaskCenterStatus::Canceled,
                            tr("app.compare_transfer.canceled").to_string(),
                            None,
                        ),
                        Err(error) => (
                            TaskCenterStatus::Failed,
                            tr("app.compare_transfer.sync_failed").to_string(),
                            Some(error.message.clone()),
                        ),
                    };
                    let log = match &result {
                        Ok(report) => vec![
                            tr("app.compare_transfer.sync_created")
                                .replace("%{count}", &report.created.to_string()),
                            tr("app.compare_transfer.sync_dropped")
                                .replace("%{count}", &report.dropped.to_string()),
                            tr("app.compare_transfer.sync_recreated")
                                .replace("%{count}", &report.recreated.to_string()),
                            tr("app.compare_transfer.sync_skipped")
                                .replace("%{count}", &report.skipped.to_string()),
                        ]
                        .into_iter()
                        .chain(report.errors.iter().cloned())
                        .collect(),
                        Err(error) => vec![error.message.clone()],
                    };
                    if let Some(state) = this.compare_transfer.as_mut() {
                        state.sync_running = false;
                        state.task_center_id = None;
                        state.transfer_log = log;
                    }
                    this.finish_task_center_record(task_id, status, stage.clone(), error.clone());
                    this.show_message(stage, if error.is_some() {
                        AppMessageKind::Error
                    } else {
                        AppMessageKind::Success
                    }, cx);
                    cx.notify();
                });
            });
        });
        self._compare_transfer_task = Some(task);
        cx.notify();
    }

    fn start_data_transfer(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.compare_transfer.as_ref() else {
            return;
        };
        if state.transfer_running {
            return;
        }
        let source_table = state.source_table.trim().to_string();
        if source_table.is_empty() {
            self.compare_transfer_error(tr("app.compare_transfer.source_table_required").to_string(), cx);
            return;
        }
        let target_table = {
            let target = state.target_table.trim();
            if target.is_empty() {
                source_table.clone()
            } else {
                target.to_string()
            }
        };
        let source_schema = optional_text(&state.source_schema);
        let target_schema = optional_text(&state.target_schema);
        let source = ObjectPath {
            connection_id: state.source_connection_id,
            database: optional_text(&state.source_database),
            schema: source_schema.clone(),
            name: source_table,
            kind: ObjectKind::Table,
        };
        let target = ObjectPath {
            connection_id: match state.target_connection_id {
                Some(id) => id,
                None => {
                    self.compare_transfer_error(tr("app.compare_transfer.choose_target").to_string(), cx);
                    return;
                }
            },
            database: optional_text(&state.target_database),
            schema: target_schema,
            name: target_table,
            kind: ObjectKind::Table,
        };
        let request = fluxdb_app::DataTransferRequest {
            source,
            target: target.clone(),
            mode: state.transfer_mode,
            create_target: state.transfer_create_target,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let task_id = self.task_center.begin(
            TaskCenterKind::DataTransfer,
            tr("app.compare_transfer.transfer_task").to_string(),
            target_label(&self.controller, target.connection_id),
            Some(cancel.clone()),
        );
        if let Some(state) = self.compare_transfer.as_mut() {
            state.transfer_running = true;
            state.transfer_error = None;
            state.transfer_log.clear();
            state.task_center_id = Some(task_id);
            state.cancel = cancel.clone();
        }
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            // 进度回调在后台线程执行，只能通过通道送回 UI 线程，避免跨线程改状态。
            let (sender, receiver) = std::sync::mpsc::channel::<String>();
            let result = cx
                .background_spawn(async move {
                    controller.transfer_table_data(&request, &cancel, &mut |message| {
                        tracing::info!(target: "gdb_compare_transfer", message, "数据传输");
                        let _ = sender.send(message.to_string());
                    })
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(message) = receiver.try_recv() {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(state) = this.compare_transfer.as_mut()
                            && state.task_center_id == Some(task_id)
                        {
                            state.transfer_log.push(message.clone());
                            // 大表会持续产生进度行，只保留最近 200 条。
                            if state.transfer_log.len() > 200 {
                                state.transfer_log.remove(0);
                            }
                        }
                        this.task_center
                            .update_progress(task_id, None, None, Some(message));
                        cx.notify();
                    });
                }
                let Some(result) = result.as_mut().now_or_never() else {
                    smol::Timer::after(Duration::from_millis(100)).await;
                    continue;
                };
                while let Ok(message) = receiver.try_recv() {
                    let _ = view.update(cx, |this, _| {
                        if let Some(state) = this.compare_transfer.as_mut()
                            && state.task_center_id == Some(task_id)
                        {
                            state.transfer_log.push(message);
                        }
                    });
                }
                let (status, stage, error, log_line) = match &result {
                    Ok(report) => (
                        TaskCenterStatus::Success,
                        tr("app.compare_transfer.transfer_done").to_string(),
                        None,
                        tr("app.compare_transfer.transfer_rows")
                            .replace("%{count}", &report.rows.to_string()),
                    ),
                    Err(error) => (
                        if error.kind == fluxdb_core::ErrorKind::Cancelled {
                            TaskCenterStatus::Canceled
                        } else {
                            TaskCenterStatus::Failed
                        },
                        tr("app.compare_transfer.transfer_failed").to_string(),
                        Some(error.message.clone()),
                        error.message.clone(),
                    ),
                };
                let _ = view.update(cx, |this, cx| {
                    // 弹框已关闭或已重新打开时不再写旧任务的界面状态，任务中心仍正常收尾。
                    if let Some(state) = this.compare_transfer.as_mut()
                        && state.task_center_id == Some(task_id)
                    {
                        state.transfer_running = false;
                        state.task_center_id = None;
                        state.transfer_error = error.clone();
                        // 失败信息用错误块展示，日志区不再重复同一条文本。
                        if error.is_none() {
                            state.transfer_log.push(log_line);
                        }
                    }
                    this.finish_task_center_record(task_id, status, stage, error);
                    cx.notify();
                });
                break;
            }
        });
        self._compare_transfer_task = Some(task);
        cx.notify();
    }

    /// 只取消后台任务，弹框保持打开，等后台返回取消结果。
    fn cancel_compare_transfer_task(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.compare_transfer.as_mut() {
            state.cancel.store(true, Ordering::Relaxed);
            if let Some(id) = state.task_center_id {
                self.task_center.mark_cancel_requested(id);
            }
        }
        cx.notify();
    }

    fn compare_transfer_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.show_message(message, AppMessageKind::Warning, cx);
    }
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn default_compare_database(controller: &AppController, connection_id: ConnectionId) -> String {
    let Some(connection) = controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
    else {
        return String::new();
    };
    if connection.config.kind == DatabaseKind::Sqlite {
        return "main".to_string();
    }
    connection
        .objects
        .iter()
        .find_map(|object| object.path.database.clone())
        .or_else(|| match &connection.config.endpoint {
            Endpoint::Tcp { database, .. } => database.clone(),
            Endpoint::SqliteFile { .. } => Some("main".to_string()),
            Endpoint::Uri { .. } => None,
        })
        .unwrap_or_default()
}

fn target_label(controller: &AppController, connection_id: ConnectionId) -> String {
    controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
        .map(|connection| connection.config.name.clone())
        .unwrap_or_default()
}

/// 读取单端下拉候选项。某个层级读取失败时保留空列表，不阻塞其它层级；
/// 具体错误只记录日志，避免弹框一打开就弹错误提示。
fn compare_transfer_endpoint_options(
    controller: &AppController,
    (connection_id, database, schema): (ConnectionId, String, String),
) -> CompareTransferEndpointOptions {
    let databases =
        compare_transfer_list_objects(controller, connection_id, None, ObjectKind::Database);
    let mut options = CompareTransferEndpointOptions {
        databases,
        ..Default::default()
    };
    // 库列表为空通常意味着连接不可用；继续查 schema/表只会再等两轮超时。
    if options.databases.is_empty() {
        return options;
    }
    let db = database.trim();
    if db.is_empty() {
        return options;
    }
    if let Some(path) = compare_transfer_parent_path(connection_id, db, "") {
        options.schemas =
            compare_transfer_list_objects(controller, connection_id, Some(&path), ObjectKind::Schema);
    }
    if let Some(path) = compare_transfer_parent_path(connection_id, db, schema.trim()) {
        options.tables =
            compare_transfer_list_objects(controller, connection_id, Some(&path), ObjectKind::Table);
    }
    options
}

fn compare_transfer_list_objects(
    controller: &AppController,
    connection_id: ConnectionId,
    parent: Option<&ObjectPath>,
    kind: ObjectKind,
) -> Vec<String> {
    match controller.list_objects(connection_id, parent) {
        Ok(objects) => {
            let mut names = objects
                .into_iter()
                .filter(|object| object.path.kind == kind)
                .map(|object| object.path.name)
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names
        }
        Err(error) => {
            tracing::warn!(
                connection_id = connection_id.0,
                ?kind,
                %error,
                "结构对比候选项读取失败"
            );
            Vec::new()
        }
    }
}

fn compare_transfer_parent_path(
    connection_id: ConnectionId,
    database: &str,
    schema: &str,
) -> Option<ObjectPath> {
    let database = database.trim();
    if database.is_empty() {
        return None;
    }
    let schema = schema.trim();
    Some(ObjectPath {
        connection_id,
        database: Some(database.to_string()),
        schema: (!schema.is_empty()).then(|| schema.to_string()),
        name: if schema.is_empty() {
            database.to_string()
        } else {
            schema.to_string()
        },
        kind: if schema.is_empty() {
            ObjectKind::Database
        } else {
            ObjectKind::Schema
        },
    })
}

#[cfg(test)]
mod compare_transfer_tests {
    use crate::optional_text;

    #[test]
    fn optional_text_trims_and_drops_empty() {
        assert_eq!(optional_text(" public "), Some("public".to_string()));
        assert_eq!(optional_text("   "), None);
    }
}
