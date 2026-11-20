// 数据同步 / 结构同步向导（四步：选择对象 → 表配对 → 同步选项 → 差异预览）。
//
// 状态只保存表单、表清单和最近一次预览；预览与执行都在后台线程完成，
// 进度通过通道送回 UI 线程。执行前后台会重新比对，界面上的预览不作为写入依据。

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncMode {
    Structure,
    Data,
}

impl SyncMode {
    fn context_key(self) -> &'static str {
        match self {
            Self::Structure => "StructureSyncWizard",
            Self::Data => "DataSyncWizard",
        }
    }
}

/// 消息日志一行：入队时的本地时间 + 文本，便于与后台日志对齐。
#[derive(Clone, Debug)]
struct SyncLogEntry {
    at: String,
    text: String,
}

impl SyncLogEntry {
    fn now(text: impl Into<String>) -> Self {
        Self {
            at: chrono::Local::now().format("%H:%M:%S").to_string(),
            text: text.into(),
        }
    }
}

/// 一张源表的「目标表」下拉。选项首项是「自动配对」，其余为目标端表键。
#[derive(Clone)]
struct SyncPairRow {
    source_key: String,
    auto_label: String,
    select: Entity<SelectState<SearchableVec<String>>>,
}

/// 单表「字段设置」弹框：比对键、跳过字段、字段映射三块，保存后写回三份文本。
struct SyncFieldMapState {
    source_key: String,
    source_name: String,
    target_name: String,
    skip_label: String,
    loading: bool,
    error: Option<String>,
    /// 目标端列（名称, 是否主键）；比对键/跳过字段的勾选项来源。
    target_columns: Vec<(String, bool)>,
    /// 当前勾选的比对键（目标端列名）；为空表示沿用全局或目标主键。
    key_columns: BTreeSet<String>,
    /// 打开时「不写表级项」应得到的比对键，用于判断用户是否真的改过。
    key_default: BTreeSet<String>,
    /// 当前勾选的跳过字段。
    skip_columns: BTreeSet<String>,
    /// 打开时全局跳过字段，用于判断是否真的改过。
    skip_default: BTreeSet<String>,
    rows: Vec<SyncFieldMapRow>,
    /// 字段列表滚动位置；内容超过弹框上限时滚动。
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

struct SyncFieldMapRow {
    source: String,
    select: Entity<SelectState<SearchableVec<String>>>,
}

/// 单表「类型映射」弹框（仅结构同步）：跨类型同步时逐列覆盖目标端类型。
struct SyncTypeMapState {
    source_key: String,
    source_name: String,
    target_name: String,
    loading: bool,
    error: Option<String>,
    rows: Vec<SyncTypeMapRow>,
    scroll: ScrollHandle,
}

struct SyncTypeMapRow {
    column: String,
    /// 源端类型，仅作对照展示；目标类型以输入框为准。
    source_type: String,
    input: Entity<InputState>,
}

#[derive(Clone, Debug)]
struct SyncWizardState {
    mode: SyncMode,
    step: u8,
    source_connection_id: ConnectionId,
    target_connection_id: Option<ConnectionId>,
    source_database: String,
    source_schema: String,
    target_database: String,
    target_schema: String,
    tables: Vec<SyncTableInfo>,
    /// 目标端表清单，仅用于「表配对」下拉；加载失败时保持空列表，不影响自动配对。
    target_tables: Vec<SyncTableInfo>,
    tables_loading: bool,
    /// 表清单搜索词，仅影响列表展示与「全选」范围，不参与同步请求。
    table_filter: String,
    selected_tables: BTreeSet<String>,
    data_options: DataSyncOptions,
    key_columns_text: String,
    column_map_text: String,
    type_overrides_text: String,
    skip_columns_text: String,
    batch_rows_text: String,
    /// 「高级」卡是否展开；比对键/字段映射/跳过字段等批量文本默认收起。
    advanced_expanded: bool,
    /// 结构差异是否左右并排展示；默认统一视图（git 风格上下对照）。
    structure_diff_split: bool,
    structure_options: StructureSyncOptions,
    data_preview: Option<DataSyncPreview>,
    structure_preview: Option<StructureSyncPreview>,
    preview_running: bool,
    /// 正在加载当前表的下一页差异；只锁「下一页」按钮，不遮整个预览。
    page_running: bool,
    /// 连续比对的取消标志；`None` 表示当前没有进行中的分页链路。
    page_cancel: Option<Arc<AtomicBool>>,
    preview_error: Option<String>,
    selected_table: Option<String>,
    /// 数据预览样本的动作筛选；`None` 表示显示全部差异。
    data_sample_filter: Option<RowDiffAction>,
    running: bool,
    /// 向导是否显示。运行中关窗只置 false，状态保留，任务中心可重新打开。
    visible: bool,
    error: Option<String>,
    log: Vec<SyncLogEntry>,
    /// 消息日志滚动位置；追加新行时贴底，回看历史时不打扰。
    log_scroll: ScrollHandle,
    task_center_id: Option<u64>,
    cancel: Arc<AtomicBool>,
    /// 最近一次执行写入的本地记录 ID；供「查看执行记录」打开回看弹框。
    last_run_id: Option<i64>,
}

impl SyncWizardState {
    fn endpoint(&self, source: bool) -> Option<SyncEndpoint> {
        if source {
            return Some(SyncEndpoint::new(
                self.source_connection_id,
                self.source_database.trim(),
                optional_text(&self.source_schema),
            ));
        }
        self.target_connection_id.map(|connection_id| {
            SyncEndpoint::new(
                connection_id,
                self.target_database.trim(),
                optional_text(&self.target_schema),
            )
        })
    }

    /// 选中的表与全部表一致时返回空列表，让后台把目标端多出的表也纳入结构同步。
    fn request_tables(&self) -> Vec<String> {
        if self.selected_tables.len() == self.tables.len() {
            return Vec::new();
        }
        self.selected_tables.iter().cloned().collect()
    }

    fn clear_preview(&mut self) {
        self.data_preview = None;
        self.structure_preview = None;
        self.preview_error = None;
        self.selected_table = None;
    }
}

struct SyncInputs {
    source_database: Entity<SelectState<SearchableVec<String>>>,
    source_schema: Entity<SelectState<SearchableVec<String>>>,
    target_connection: Entity<SelectState<SearchableVec<String>>>,
    target_database: Entity<SelectState<SearchableVec<String>>>,
    target_schema: Entity<SelectState<SearchableVec<String>>>,
    key_columns: Entity<InputState>,
    column_map: Entity<InputState>,
    type_overrides: Entity<InputState>,
    skip_columns: Entity<InputState>,
    batch_rows: Entity<InputState>,
    table_filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug, Default)]
struct SyncEndpointOptions {
    databases: Vec<String>,
    schemas: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncField {
    SourceDatabase,
    SourceSchema,
    TargetDatabase,
    TargetSchema,
}

fn new_sync_inputs(window: &mut Window, cx: &mut Context<NavicatMain>) -> SyncInputs {
    let source_database = new_sync_select(window, cx);
    let source_schema = new_sync_select(window, cx);
    let target_connection = new_sync_select(window, cx);
    let target_database = new_sync_select(window, cx);
    let target_schema = new_sync_select(window, cx);
    let key_columns = cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.sync.key_columns_placeholder")));
    let column_map = cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.sync.column_map_placeholder")));
    let type_overrides = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.sync.type_overrides_placeholder"))
    });
    let skip_columns = cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.sync.skip_columns_placeholder")));
    let batch_rows = cx.new(|cx| {
        InputState::new(window, cx).default_value(DEFAULT_BATCH_ROWS.to_string())
    });
    let table_filter = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.sync.table_search_placeholder"))
    });
    let subscriptions = vec![
        subscribe_sync_select(&source_database, window, cx, SyncField::SourceDatabase),
        subscribe_sync_select(&source_schema, window, cx, SyncField::SourceSchema),
        subscribe_sync_target(&target_connection, window, cx),
        subscribe_sync_select(&target_database, window, cx, SyncField::TargetDatabase),
        subscribe_sync_select(&target_schema, window, cx, SyncField::TargetSchema),
        subscribe_sync_input(&key_columns, window, cx, SyncText::KeyColumns),
        subscribe_sync_input(&column_map, window, cx, SyncText::ColumnMap),
        subscribe_sync_input(&type_overrides, window, cx, SyncText::TypeOverrides),
        subscribe_sync_input(&skip_columns, window, cx, SyncText::SkipColumns),
        subscribe_sync_input(&batch_rows, window, cx, SyncText::BatchRows),
        subscribe_sync_table_filter(&table_filter, window, cx),
    ];
    SyncInputs {
        source_database,
        source_schema,
        target_connection,
        target_database,
        target_schema,
        key_columns,
        column_map,
        type_overrides,
        skip_columns,
        batch_rows,
        table_filter,
        _subscriptions: subscriptions,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncText {
    KeyColumns,
    ColumnMap,
    TypeOverrides,
    SkipColumns,
    BatchRows,
}

fn subscribe_sync_input(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    field: SyncText,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        move |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let value = input.read(cx).value().to_string();
            if let Some(state) = this.sync_wizard.as_mut() {
                match field {
                    SyncText::KeyColumns => state.key_columns_text = value,
                    SyncText::ColumnMap => state.column_map_text = value,
                    SyncText::TypeOverrides => state.type_overrides_text = value,
                    SyncText::SkipColumns => state.skip_columns_text = value,
                    SyncText::BatchRows => state.batch_rows_text = value,
                }
                state.clear_preview();
            }
            cx.notify();
        },
    )
}

/// 表清单搜索只改变列表展示，不触发重新比对，因此不清空已有预览。
fn subscribe_sync_table_filter(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        move |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let value = input.read(cx).value().to_string();
            if let Some(state) = this.sync_wizard.as_mut() {
                state.table_filter = value;
            }
            cx.notify();
        },
    )
}

fn new_sync_select(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Entity<SelectState<SearchableVec<String>>> {
    cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    })
}

fn set_sync_select(
    select: &Entity<SelectState<SearchableVec<String>>>,
    values: Vec<String>,
    selected: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    select.update(cx, |select, cx| {
        select.set_items(SearchableVec::new(values), window, cx);
        if selected.is_empty() {
            select.set_selected_index(None, window, cx);
        } else {
            select.set_selected_value(&selected.to_string(), window, cx);
        }
    });
}

impl SyncField {
    fn set(self, state: &mut SyncWizardState, value: String) -> bool {
        let slot = match self {
            Self::SourceDatabase => &mut state.source_database,
            Self::SourceSchema => &mut state.source_schema,
            Self::TargetDatabase => &mut state.target_database,
            Self::TargetSchema => &mut state.target_schema,
        };
        if *slot == value {
            return false;
        }
        *slot = value;
        true
    }
}

fn subscribe_sync_select(
    select: &Entity<SelectState<SearchableVec<String>>>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    field: SyncField,
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
                .sync_wizard
                .as_mut()
                .map(|state| {
                    let changed = field.set(state, value);
                    if changed {
                        state.clear_preview();
                        state.tables.clear();
                        state.selected_tables.clear();
                        // 换库后旧搜索词会把新表清单筛空，一并清掉。
                        state.table_filter.clear();
                    }
                    changed
                })
                .unwrap_or(false);
            cx.notify();
            if changed {
                this.sync_inputs
                    .table_filter
                    .update(cx, |input, cx| input.set_value("", window, cx));
                this.refresh_sync_options(window, cx);
                // 换目标库/模式后可选目标表变了，配对下拉要跟着刷新。
                if matches!(field, SyncField::TargetDatabase | SyncField::TargetSchema) {
                    this.refresh_sync_target_tables(window, cx);
                }
            }
        },
    )
}

fn subscribe_sync_target(
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
            let Some(target) = target else {
                return;
            };
            let changed = this
                .sync_wizard
                .as_ref()
                .is_some_and(|state| state.target_connection_id != Some(target));
            if !changed {
                return;
            }
            if let Some(state) = this.sync_wizard.as_mut() {
                state.target_connection_id = Some(target);
                state.target_database = default_sync_database(&this.controller, target);
                state.target_schema.clear();
                state.clear_preview();
            }
            this.refresh_sync_options(window, cx);
            this.refresh_sync_target_tables(window, cx);
            cx.notify();
        },
    )
}

impl NavicatMain {
    fn show_sync_wizard(
        &mut self,
        connection_id: ConnectionId,
        mode: SyncMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 已有运行中的同步时不再新建向导：新向导会覆盖唯一的一份运行状态，
        // 进度只能推到被顶掉的那份 state 上。直接把它拉回前台继续看。
        if let Some(state) = self.sync_wizard.as_mut()
            && state.running
        {
            state.visible = true;
            state.step = 4;
            cx.notify();
            return;
        }
        let connections = self.controller.connection_configs();
        let Some(source) = connections
            .iter()
            .find(|connection| connection.id == connection_id)
        else {
            self.show_message(tr("app.sync.connection_missing"), AppMessageKind::Error, cx);
            return;
        };
        let target = connections
            .iter()
            .find(|connection| {
                connection.id != connection_id
                    && !matches!(connection.kind, DatabaseKind::Redis | DatabaseKind::MongoDb)
            })
            .map(|connection| connection.id)
            .or(Some(source.id));
        let target_names = connections
            .iter()
            .filter(|connection| {
                !matches!(connection.kind, DatabaseKind::Redis | DatabaseKind::MongoDb)
            })
            .map(|connection| connection.name.clone())
            .collect::<Vec<_>>();
        let target_name = target
            .and_then(|id| connections.iter().find(|connection| connection.id == id))
            .map(|connection| connection.name.clone());
        let source_database = default_sync_database(&self.controller, connection_id);
        let target_database = target
            .map(|id| default_sync_database(&self.controller, id))
            .unwrap_or_else(|| source_database.clone());

        set_sync_select(
            &self.sync_inputs.source_database,
            Vec::new(),
            "",
            window,
            cx,
        );
        set_sync_select(
            &self.sync_inputs.source_schema,
            Vec::new(),
            "",
            window,
            cx,
        );
        set_sync_select(
            &self.sync_inputs.target_database,
            Vec::new(),
            "",
            window,
            cx,
        );
        set_sync_select(
            &self.sync_inputs.target_schema,
            Vec::new(),
            "",
            window,
            cx,
        );
        self.sync_inputs
            .target_connection
            .update(cx, |select, cx| {
                select.set_items(SearchableVec::new(target_names), window, cx);
                match target_name {
                    Some(name) => select.set_selected_value(&name, window, cx),
                    None => select.set_selected_index(None, window, cx),
                }
            });

        self.sync_wizard = Some(SyncWizardState {
            mode,
            step: 1,
            source_connection_id: source.id,
            target_connection_id: target,
            source_database,
            source_schema: String::new(),
            target_database,
            target_schema: String::new(),
            tables: Vec::new(),
            target_tables: Vec::new(),
            tables_loading: true,
            table_filter: String::new(),
            selected_tables: BTreeSet::new(),
            data_options: DataSyncOptions::default(),
            key_columns_text: String::new(),
            column_map_text: String::new(),
            type_overrides_text: String::new(),
            skip_columns_text: String::new(),
            batch_rows_text: DEFAULT_BATCH_ROWS.to_string(),
            advanced_expanded: false,
            structure_diff_split: false,
            structure_options: StructureSyncOptions::default(),
            data_preview: None,
            structure_preview: None,
            preview_running: false,
            page_running: false,
            page_cancel: None,
            preview_error: None,
            selected_table: None,
            data_sample_filter: None,
            running: false,
            visible: true,
            error: None,
            log: Vec::new(),
            log_scroll: ScrollHandle::new(),
            task_center_id: None,
            cancel: Arc::new(AtomicBool::new(false)),
            last_run_id: None,
        });
        // 重新打开向导时表单回到默认值，避免上次的比对键/批量行数残留在界面上。
        for input in [
            &self.sync_inputs.key_columns,
            &self.sync_inputs.column_map,
            &self.sync_inputs.type_overrides,
            &self.sync_inputs.skip_columns,
            &self.sync_inputs.table_filter,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.sync_inputs.batch_rows.update(cx, |input, cx| {
            input.set_value(DEFAULT_BATCH_ROWS.to_string(), window, cx)
        });
        self.refresh_sync_options(window, cx);
        self.refresh_sync_tables(window, cx);
        self.refresh_sync_target_tables(window, cx);
        self.sync_pair_rows.clear();
        self._sync_pair_subscriptions.clear();
        cx.notify();
    }

    fn close_sync_wizard(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.sync_wizard.as_mut() {
            if state.running {
                // 运行中关窗 = 收起界面，同步继续在后台跑；要停请点「停止」。
                // 状态留着，任务中心可凭 task_center_id 重新打开进度视图。
                state.visible = false;
            } else {
                // 关闭弹框时连分页链路一起停，避免关掉界面后还在后台空跑大表。
                if let Some(cancel) = state.page_cancel.as_ref() {
                    cancel.store(true, Ordering::Relaxed);
                }
                self.sync_wizard = None;
            }
        }
        self.sync_pair_rows.clear();
        self._sync_pair_subscriptions.clear();
        self.sync_field_map = None;
        self.sync_type_map = None;
        cx.notify();
    }

    /// 从任务中心回到同步向导：只有本次会话仍在运行的那次同步能回来。
    fn reopen_sync_wizard(&mut self, task_id: u64, cx: &mut Context<Self>) {
        if let Some(state) = self.sync_wizard.as_mut()
            && state.task_center_id == Some(task_id)
        {
            state.visible = true;
            state.step = 4;
            self.task_center.open = false;
            self.task_center.selected = None;
            cx.notify();
            return;
        }
        self.show_message(
            tr("app.task_center.open_unavailable"),
            AppMessageKind::Warning,
            cx,
        );
    }

    /// 重新读取两端数据库/Schema 候选项。两端独立提交，一端超时不拖住另一端。
    fn refresh_sync_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_sync_endpoint(true, window, cx);
        self.refresh_sync_endpoint(false, window, cx);
    }

    fn refresh_sync_endpoint(
        &mut self,
        source: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.sync_wizard.as_ref() else {
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
                .background_spawn(async move { sync_endpoint_options(&controller, snapshot_for_read) })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                let Some(state) = this.sync_wizard.as_ref() else {
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
                if current.as_ref() != Some(&snapshot) {
                    return;
                }
                this.apply_sync_endpoint_options(source, options, window, cx);
            });
        });
        if source {
            self._sync_source_options_task = Some(task);
        } else {
            self._sync_target_options_task = Some(task);
        }
    }

    fn apply_sync_endpoint_options(
        &mut self,
        source: bool,
        options: SyncEndpointOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.sync_wizard.as_ref() else {
            return;
        };
        let (database, schema) = if source {
            (state.source_database.clone(), state.source_schema.clone())
        } else {
            (state.target_database.clone(), state.target_schema.clone())
        };
        let (database_select, schema_select) = if source {
            (
                &self.sync_inputs.source_database,
                &self.sync_inputs.source_schema,
            )
        } else {
            (
                &self.sync_inputs.target_database,
                &self.sync_inputs.target_schema,
            )
        };
        set_sync_select(
            database_select,
            options.databases,
            &database,
            window,
            cx,
        );
        set_sync_select(schema_select, options.schemas, &schema, window, cx);
        if source {
            // 源端库/Schema 变化后表清单会变；这里统一刷新，避免两级状态不同步。
            self.refresh_sync_tables(window, cx);
        }
    }

    /// 读取源端表清单，默认全选。
    fn refresh_sync_tables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.sync_wizard.as_mut() else {
            return;
        };
        let Some(endpoint) = state.endpoint(true) else {
            return;
        };
        if endpoint.database.trim().is_empty() {
            state.tables_loading = false;
            return;
        }
        state.tables_loading = true;
        state.clear_preview();
        let controller = self.controller.clone();
        let endpoint_for_read = endpoint.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let tables = cx
                .background_spawn(async move { controller.list_sync_tables(&endpoint_for_read) })
                .await;
            let _ = view.update_in(cx, |this, _window, cx| {
                let Some(state) = this.sync_wizard.as_mut() else {
                    return;
                };
                if state.endpoint(true).as_ref() != Some(&endpoint) {
                    return;
                }
                state.tables_loading = false;
                match tables {
                    Ok(tables) => {
                        state.selected_tables = tables.iter().map(|table| table.key.clone()).collect();
                        state.tables = tables;
                        state.error = None;
                    }
                    Err(error) => {
                        state.tables.clear();
                        state.selected_tables.clear();
                        state.error = Some(error.message);
                    }
                }
                cx.notify();
            });
        });
        self._sync_tables_task = Some(task);
        cx.notify();
    }

    /// 读取目标端表清单，供「表配对」下拉使用；失败时保持空列表，自动配对仍可用。
    fn refresh_sync_target_tables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.sync_wizard.as_mut() else {
            return;
        };
        let Some(endpoint) = state.endpoint(false) else {
            state.target_tables.clear();
            return;
        };
        if endpoint.database.trim().is_empty() {
            state.target_tables.clear();
            return;
        }
        let controller = self.controller.clone();
        let endpoint_for_read = endpoint.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let tables = cx
                .background_spawn(async move { controller.list_sync_tables(&endpoint_for_read) })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                let Some(state) = this.sync_wizard.as_mut() else {
                    return;
                };
                if state.endpoint(false).as_ref() != Some(&endpoint) {
                    return;
                }
                state.target_tables = tables.unwrap_or_default();
                // 已进入选项页时立刻补上目标表选项，避免清单一到就被丢弃。
                if state.step >= 2 && !this.sync_pair_rows.is_empty() {
                    this.rebuild_sync_pair_rows(window, cx);
                }
                cx.notify();
            });
        });
        self._sync_target_tables_task = Some(task);
    }

    /// 按当前选中表重建「表配对」下拉；保留上一轮已选的显式目标。
    fn rebuild_sync_pair_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous = table_map_from_rows(&self.sync_pair_rows, cx);
        let Some(state) = self.sync_wizard.as_ref() else {
            self.sync_pair_rows.clear();
            self._sync_pair_subscriptions.clear();
            return;
        };
        let auto_label = tr("app.sync.pair_auto").to_string();
        let mut items = Vec::with_capacity(state.target_tables.len() + 1);
        items.push(auto_label.clone());
        items.extend(state.target_tables.iter().map(|table| table.key.clone()));

        let mut rows = Vec::new();
        let mut subscriptions = Vec::new();
        for table in &state.tables {
            if !state.selected_tables.contains(&table.key) {
                continue;
            }
            // 默认选中自动配对的实际目标表，让「自动匹配到了谁」一眼可见；
            // 找不到同名/近似表时仍选中「自动配对」，避免显示空白的「请选择」。
            let auto_index = auto_target_table(state, table)
                .and_then(|target| items.iter().position(|item| item == &target.key));
            let selected = previous
                .get(&table.key)
                .and_then(|target| items.iter().position(|item| item == target))
                .or(auto_index)
                .or(Some(0));
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items.clone()),
                    selected.map(IndexPath::new),
                    window,
                    cx,
                )
                .searchable(true)
            });
            subscriptions.push(cx.subscribe_in(
                &select,
                window,
                |this: &mut NavicatMain,
                 _select,
                 event: &SelectEvent<SearchableVec<String>>,
                 _window,
                 cx| {
                    if !matches!(event, SelectEvent::Confirm(_)) {
                        return;
                    }
                    if let Some(state) = this.sync_wizard.as_mut() {
                        state.clear_preview();
                    }
                    cx.notify();
                },
            ));
            rows.push(SyncPairRow {
                source_key: table.key.clone(),
                auto_label: auto_label.clone(),
                select,
            });
        }
        self.sync_pair_rows = rows;
        self._sync_pair_subscriptions = subscriptions;
    }

    /// 打开单表「字段设置」弹框：读取两端字段后预填比对键、跳过字段与逐列映射。
    fn open_sync_field_map(
        &mut self,
        source_key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let skip_label = tr("app.sync.field_map_skip").to_string();
        let Some((
            source_endpoint,
            target_endpoint,
            source_table,
            target_table,
            existing,
            key_global,
            key_table,
            skip_global,
            skip_table,
        )) = (|| {
            let state = self.sync_wizard.as_ref()?;
            let source_table = state.tables.iter().find(|t| t.key == source_key)?.clone();
            let explicit_target = self
                .sync_pair_rows
                .iter()
                .find(|row| row.source_key == source_key)
                .and_then(|row| {
                    let value = row.select.read(cx).selected_value()?.clone();
                    (value != row.auto_label).then_some(value)
                });
            let target_table = match explicit_target {
                Some(key) => state.target_tables.iter().find(|t| t.key == key).cloned(),
                None => auto_target_table(state, &source_table).cloned(),
            }?;
            let existing = parse_column_maps(&state.column_map_text)
                .1
                .remove(&source_key)
                .unwrap_or_default();
            let (key_global, mut key_per_table) = parse_key_columns(&state.key_columns_text);
            let (skip_global, mut skip_per_table) = parse_key_columns(&state.skip_columns_text);
            Some((
                state.endpoint(true)?,
                state.endpoint(false)?,
                source_table,
                target_table,
                existing,
                key_global,
                key_per_table.remove(&source_key),
                skip_global,
                skip_per_table.remove(&source_key),
            ))
        })()
        else {
            return;
        };
        let controller = self.controller.clone();
        self.sync_field_map = Some(SyncFieldMapState {
            source_key,
            source_name: source_table.name.clone(),
            target_name: target_table.name.clone(),
            skip_label: skip_label.clone(),
            loading: true,
            error: None,
            target_columns: Vec::new(),
            key_columns: BTreeSet::new(),
            key_default: BTreeSet::new(),
            skip_columns: BTreeSet::new(),
            skip_default: BTreeSet::new(),
            rows: Vec::new(),
            scroll: ScrollHandle::new(),
            _subscriptions: Vec::new(),
        });
        let source_name = source_table.name;
        let source_schema = source_table.schema;
        let target_name = target_table.name;
        let target_schema = target_table.schema;
        let task = cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let source = controller.list_sync_columns(
                        &source_endpoint,
                        source_schema.as_deref(),
                        &source_name,
                    )?;
                    let target = controller.list_sync_columns(
                        &target_endpoint,
                        target_schema.as_deref(),
                        &target_name,
                    )?;
                    Ok::<_, fluxdb_core::Error>((source, target))
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                let Some(dialog) = this.sync_field_map.as_mut() else {
                    return;
                };
                dialog.loading = false;
                match result {
                    Ok((source_columns, target_columns)) => {
                        let target_names: Vec<String> =
                            target_columns.iter().map(|c| c.name.clone()).collect();
                        // 未单独设置时：全局比对键优先，否则目标主键。
                        let primary_key: BTreeSet<String> = target_columns
                            .iter()
                            .filter(|column| column.primary_key)
                            .map(|column| column.name.clone())
                            .collect();
                        let key_default: BTreeSet<String> = if key_global.is_empty() {
                            primary_key
                        } else {
                            key_global.iter().cloned().collect()
                        };
                        let key_columns = key_table
                            .clone()
                            .map(|columns| columns.into_iter().collect::<BTreeSet<_>>())
                            .filter(|columns| !columns.is_empty())
                            .unwrap_or_else(|| key_default.clone());
                        let skip_default: BTreeSet<String> =
                            skip_global.iter().cloned().collect();
                        let skip_columns = skip_table
                            .clone()
                            .map(|columns| columns.into_iter().collect::<BTreeSet<_>>())
                            .filter(|columns| !columns.is_empty())
                            .unwrap_or_else(|| skip_default.clone());
                        let mut items = target_names.clone();
                        items.push(skip_label.clone());
                        let mut rows = Vec::new();
                        let mut subscriptions = Vec::new();
                        for column in source_columns {
                            let mapped = existing
                                .get(&column.name)
                                .filter(|mapped| items.contains(mapped))
                                .cloned()
                                .or_else(|| {
                                    target_names
                                        .iter()
                                        .find(|target| target.eq_ignore_ascii_case(&column.name))
                                        .cloned()
                                })
                                .unwrap_or_else(|| skip_label.clone());
                            let index = items.iter().position(|item| item == &mapped);
                            let select = cx.new(|cx| {
                                SelectState::new(
                                    SearchableVec::new(items.clone()),
                                    index.map(IndexPath::new),
                                    window,
                                    cx,
                                )
                                .searchable(true)
                            });
                            subscriptions.push(cx.subscribe_in(
                                &select,
                                window,
                                |_this: &mut NavicatMain,
                                 _select,
                                 _event: &SelectEvent<SearchableVec<String>>,
                                 _window,
                                 cx| cx.notify(),
                            ));
                            rows.push(SyncFieldMapRow {
                                source: column.name,
                                select,
                            });
                        }
                        dialog.target_columns = target_columns
                            .into_iter()
                            .map(|column| (column.name, column.primary_key))
                            .collect();
                        dialog.key_columns = key_columns;
                        dialog.key_default = key_default;
                        dialog.skip_columns = skip_columns;
                        dialog.skip_default = skip_default;
                        dialog.rows = rows;
                        dialog._subscriptions = subscriptions;
                    }
                    Err(error) => dialog.error = Some(error.message),
                }
                cx.notify();
            });
        });
        self._sync_field_map_task = Some(task);
        cx.notify();
    }

    /// 保存字段设置：比对键/跳过字段写表级（与默认一致时删掉表级项，
    /// 继续跟随全局或主键），字段映射只保留真正改名的项；三份文本各自回写。
    fn save_sync_field_map(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.sync_field_map.take() else {
            return;
        };
        let mut entries = BTreeMap::new();
        for row in &dialog.rows {
            let Some(value) = row.select.read(cx).selected_value().cloned() else {
                continue;
            };
            if value == dialog.skip_label || value.eq_ignore_ascii_case(&row.source) {
                continue;
            }
            entries.insert(row.source.clone(), value);
        }
        let keys: Vec<String> = dialog.key_columns.iter().cloned().collect();
        let key_entry = (dialog.key_columns != dialog.key_default).then_some(keys);
        let skipped: Vec<String> = dialog.skip_columns.iter().cloned().collect();
        let skip_entry = (dialog.skip_columns != dialog.skip_default).then_some(skipped);
        let (column_text, key_text, skip_text) = match self.sync_wizard.as_mut() {
            Some(state) => {
                let (global, mut per_table) = parse_column_maps(&state.column_map_text);
                if entries.is_empty() {
                    per_table.remove(&dialog.source_key);
                } else {
                    per_table.insert(dialog.source_key.clone(), entries);
                }
                let mut parts: Vec<String> =
                    global.iter().map(|(source, target)| format!("{source}={target}")).collect();
                for (table, columns) in &per_table {
                    for (source, target) in columns {
                        parts.push(format!("{table}.{source}={target}"));
                    }
                }
                let column_text = parts.join(", ");
                let key_text =
                    write_table_column_list(&state.key_columns_text, &dialog.source_key, key_entry);
                let skip_text = write_table_column_list(
                    &state.skip_columns_text,
                    &dialog.source_key,
                    skip_entry,
                );
                state.column_map_text = column_text.clone();
                state.key_columns_text = key_text.clone();
                state.skip_columns_text = skip_text.clone();
                state.clear_preview();
                (column_text, key_text, skip_text)
            }
            None => return,
        };
        self.sync_inputs
            .column_map
            .update(cx, |input, cx| input.set_value(&column_text, window, cx));
        self.sync_inputs
            .key_columns
            .update(cx, |input, cx| input.set_value(&key_text, window, cx));
        self.sync_inputs
            .skip_columns
            .update(cx, |input, cx| input.set_value(&skip_text, window, cx));
        cx.notify();
    }

    /// 勾选/取消某列的比对键；全不选时保存即回落全局或主键。
    fn toggle_sync_field_key(&mut self, column: String, cx: &mut Context<Self>) {
        let Some(dialog) = self.sync_field_map.as_mut() else {
            return;
        };
        if !dialog.key_columns.remove(&column) {
            dialog.key_columns.insert(column);
        }
        cx.notify();
    }

    /// 勾选/取消某列的跳过字段。
    fn toggle_sync_field_skip(&mut self, column: String, cx: &mut Context<Self>) {
        let Some(dialog) = self.sync_field_map.as_mut() else {
            return;
        };
        if !dialog.skip_columns.remove(&column) {
            dialog.skip_columns.insert(column);
        }
        cx.notify();
    }

    fn close_sync_field_map(&mut self, cx: &mut Context<Self>) {
        self.sync_field_map = None;
        cx.notify();
    }

    /// 打开单表「类型映射」弹框：读源表字段与已有覆盖，逐列给出目标类型输入框。
    fn open_sync_type_map(
        &mut self,
        source_key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((source_endpoint, source_table, target_name, existing)) = (|| {
            let state = self.sync_wizard.as_ref()?;
            let source_table = state.tables.iter().find(|t| t.key == source_key)?.clone();
            let explicit_target = self
                .sync_pair_rows
                .iter()
                .find(|row| row.source_key == source_key)
                .and_then(|row| {
                    let value = row.select.read(cx).selected_value()?.clone();
                    (value != row.auto_label).then_some(value)
                });
            let target_table = match explicit_target {
                Some(key) => state.target_tables.iter().find(|t| t.key == key).cloned(),
                None => auto_target_table(state, &source_table).cloned(),
            };
            let existing = parse_type_overrides(&state.type_overrides_text)
                .remove(&source_key)
                .unwrap_or_default();
            Some((
                state.endpoint(true)?,
                source_table,
                target_table.map(|table| table.name).unwrap_or_default(),
                existing,
            ))
        })()
        else {
            return;
        };
        let controller = self.controller.clone();
        self.sync_type_map = Some(SyncTypeMapState {
            source_key,
            source_name: source_table.name.clone(),
            target_name,
            loading: true,
            error: None,
            rows: Vec::new(),
            scroll: ScrollHandle::new(),
        });
        let source_name = source_table.name;
        let source_schema = source_table.schema;
        let task = cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    controller.list_sync_columns(
                        &source_endpoint,
                        source_schema.as_deref(),
                        &source_name,
                    )
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                let Some(dialog) = this.sync_type_map.as_mut() else {
                    return;
                };
                dialog.loading = false;
                match result {
                    Ok(columns) => {
                        dialog.rows = columns
                            .into_iter()
                            .map(|column| {
                                let preset = existing
                                    .get(&column.name)
                                    .cloned()
                                    .unwrap_or_default();
                                let input = cx.new(|cx| {
                                    InputState::new(window, cx)
                                        .default_value(preset)
                                        .placeholder(tr("app.sync.type_map_placeholder"))
                                });
                                SyncTypeMapRow {
                                    column: column.name,
                                    source_type: column.type_name.unwrap_or_default(),
                                    input,
                                }
                            })
                            .collect();
                    }
                    Err(error) => dialog.error = Some(error.message),
                }
                cx.notify();
            });
        });
        self._sync_type_map_task = Some(task);
        cx.notify();
    }

    /// 保存类型映射：非空项写成 `表.字段=类型`，清空即删除该列的覆盖。
    fn save_sync_type_map(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.sync_type_map.take() else {
            return;
        };
        let mut overrides = BTreeMap::new();
        for row in &dialog.rows {
            let value = row.input.read(cx).value().trim().to_string();
            if !value.is_empty() {
                overrides.insert(row.column.clone(), value);
            }
        }
        if let Some(state) = self.sync_wizard.as_mut() {
            state.type_overrides_text =
                write_table_type_overrides(&state.type_overrides_text, &dialog.source_key, &overrides);
            state.clear_preview();
        }
        cx.notify();
    }

    fn close_sync_type_map(&mut self, cx: &mut Context<Self>) {
        self.sync_type_map = None;
        cx.notify();
    }

    fn toggle_sync_table(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(state) = self.sync_wizard.as_mut() {
            state.selected_table = Some(key.clone());
            if !state.selected_tables.remove(&key) {
                state.selected_tables.insert(key);
            }
            state.clear_preview();
            cx.notify();
        }
    }

    /// 全选/取消全选传入的表（当前筛选结果）；筛选为空时即全部表。
    fn toggle_sync_tables(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        let Some(state) = self.sync_wizard.as_mut() else {
            return;
        };
        if keys.is_empty() {
            return;
        }
        if keys.iter().all(|key| state.selected_tables.contains(key)) {
            for key in &keys {
                state.selected_tables.remove(key);
            }
        } else {
            state.selected_tables.extend(keys);
        }
        state.clear_preview();
        cx.notify();
    }

    /// 仅切换预览详情里的选中表，不动同步选择。
    fn select_sync_preview_table(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(state) = self.sync_wizard.as_mut() {
            state.selected_table = Some(key);
            // 换表时回到「全部差异」，避免上一张表的筛选条件把新表样本全隐藏。
            state.data_sample_filter = None;
            cx.notify();
        }
    }

    /// 切换数据预览样本的动作筛选；同一筛选项再次点击回到「全部差异」。
    fn set_sync_sample_filter(&mut self, filter: Option<RowDiffAction>, cx: &mut Context<Self>) {
        if let Some(state) = self.sync_wizard.as_mut() {
            state.data_sample_filter = if state.data_sample_filter == filter {
                None
            } else {
                filter
            };
            cx.notify();
        }
    }

    fn go_sync_step(&mut self, step: u8, window: &mut Window, cx: &mut Context<Self>) {
        let (needs_preview, build_rows) = {
            let Some(state) = self.sync_wizard.as_mut() else {
                return;
            };
            if state.running {
                return;
            }
            if step > 1 && state.selected_tables.is_empty() {
                state.error = Some(tr("app.sync.choose_tables").to_string());
                cx.notify();
                return;
            }
            state.error = None;
            state.step = step.clamp(1, 4);
            let needs_preview = state.step == 4
                && match state.mode {
                    SyncMode::Data => state.data_preview.is_none(),
                    SyncMode::Structure => state.structure_preview.is_none(),
                };
            (needs_preview, state.step == 2)
        };
        if build_rows {
            self.rebuild_sync_pair_rows(window, cx);
        }
        if needs_preview {
            self.start_sync_preview(cx);
        }
        cx.notify();
    }

    fn start_sync_preview(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.sync_wizard.as_mut() else {
            return;
        };
        if state.preview_running || state.page_running || state.running {
            return;
        }
        let (Some(source), Some(target)) = (state.endpoint(true), state.endpoint(false)) else {
            state.error = Some(tr("app.sync.choose_target").to_string());
            cx.notify();
            return;
        };
        let mode = state.mode;
        let tables = state.request_tables();
        let data_request = (mode == SyncMode::Data).then(|| DataSyncRequest {
            source: source.clone(),
            target: target.clone(),
            tables: tables.clone(),
            options: data_options_from_state(state, &self.sync_pair_rows, cx),
        });
        let structure_request = (mode == SyncMode::Structure).then(|| StructureSyncRequest {
            source,
            target,
            tables,
            options: structure_options_from_state(state, &self.sync_pair_rows, cx),
        });
        state.preview_running = true;
        state.preview_error = None;
        state.error = None;
        let controller = self.controller.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(request) = structure_request {
                        return SyncPreviewOutcome::Structure(
                            controller.preview_structure_sync(&request, &cancel, &mut |_| {}),
                        );
                    }
                    let Some(request) = data_request else {
                        return SyncPreviewOutcome::None;
                    };
                    SyncPreviewOutcome::Data(
                        controller.preview_data_sync(&request, &cancel, &mut |_| {}),
                    )
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if let Some(state) = this.sync_wizard.as_mut() {
                        state.preview_running = false;
                        match result {
                            SyncPreviewOutcome::Structure(Ok(preview)) => {
                                state.selected_table = default_structure_selection(&preview);
                                state.structure_preview = Some(preview);
                            }
                            SyncPreviewOutcome::Data(Ok(preview)) => {
                                state.selected_table = default_data_selection(&preview);
                                state.data_preview = Some(preview);
                            }
                            SyncPreviewOutcome::Structure(Err(error))
                            | SyncPreviewOutcome::Data(Err(error)) => {
                                state.preview_error = Some(error.message);
                            }
                            SyncPreviewOutcome::None => {}
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._sync_preview_task = Some(task);
        cx.notify();
    }

    /// 继续比对当前选中表的下一页差异：只推进一块键区间，不重新比对整表。
    fn start_sync_next_page(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.sync_wizard.as_mut() else {
            return;
        };
        if state.page_running || state.preview_running || state.running {
            return;
        }
        let Some(key) = state.selected_table.clone() else {
            return;
        };
        let Some((cursor, chunk_rows)) = state
            .data_preview
            .as_ref()
            .and_then(|preview| preview.tables.iter().find(|table| table.key == key))
            .and_then(|table| table.cursor.clone().map(|cursor| (cursor, table.chunk_rows)))
        else {
            return;
        };
        let (Some(source), Some(target)) = (state.endpoint(true), state.endpoint(false)) else {
            return;
        };
        let request = DataSyncRequest {
            source,
            target,
            tables: state.request_tables(),
            options: data_options_from_state(state, &self.sync_pair_rows, cx),
        };
        state.page_running = true;
        state.preview_error = None;
        let cancel = state
            .page_cancel
            .clone()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        state.page_cancel = Some(cancel.clone());
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let page_cancel = cancel.clone();
            let result = cx
                .background_spawn(async move {
                    controller.preview_data_sync_page(
                        &request,
                        &key,
                        Some(&cursor),
                        chunk_rows,
                        &page_cancel,
                        &mut |_| {},
                    )
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    // 一次点击只比对一块：大表预览是抽样，整表统计留给执行阶段。
                    if let Some(state) = this.sync_wizard.as_mut() {
                        state.page_running = false;
                        state.page_cancel = None;
                        match result {
                            Ok(page) => {
                                if let Some(table) = state
                                    .data_preview
                                    .as_mut()
                                    .and_then(|preview| {
                                        preview
                                            .tables
                                            .iter_mut()
                                            .find(|table| table.key == page.key)
                                    })
                                {
                                    table.absorb_page(page);
                                }
                            }
                            Err(error) => state.preview_error = Some(error.message),
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._sync_preview_task = Some(task);
        cx.notify();
    }

    fn start_sync_execute(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.sync_wizard.as_ref() else {
            return;
        };
        if state.running || state.preview_running || state.page_running {
            return;
        }
        let (Some(source), Some(target)) = (state.endpoint(true), state.endpoint(false)) else {
            if let Some(state) = self.sync_wizard.as_mut() {
                state.error = Some(tr("app.sync.choose_target").to_string());
            }
            cx.notify();
            return;
        };
        let mode = state.mode;
        let tables = state.request_tables();
        let data_request = (mode == SyncMode::Data).then(|| DataSyncRequest {
            source: source.clone(),
            target: target.clone(),
            tables: tables.clone(),
            options: data_options_from_state(state, &self.sync_pair_rows, cx),
        });
        let structure_request = (mode == SyncMode::Structure).then(|| StructureSyncRequest {
            source,
            target: target.clone(),
            tables,
            options: structure_options_from_state(state, &self.sync_pair_rows, cx),
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let kind = match mode {
            SyncMode::Data => TaskCenterKind::DataSync,
            SyncMode::Structure => TaskCenterKind::StructureSync,
        };
        let title = match mode {
            SyncMode::Data => tr("app.sync.kind_data"),
            SyncMode::Structure => tr("app.sync.kind_structure"),
        };
        let task_id = self.task_center.begin(
            kind,
            title.to_string(),
            target_label(&self.controller, target.connection_id),
            Some(cancel.clone()),
        );
        if let Some(state) = self.sync_wizard.as_mut() {
            state.running = true;
            state.task_center_id = Some(task_id);
            state.cancel = cancel.clone();
            state.error = None;
            state.log.clear();
        }
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (sender, receiver) = mpsc::channel::<String>();
            let result = cx
                .background_spawn(async move {
                    let progress = &mut |message: &str| {
                        tracing::info!(target: "gdb_sync", message, "同步进度");
                        let _ = sender.send(message.to_string());
                    };
                    if let Some(request) = structure_request {
                        return SyncExecuteOutcome::Structure(
                            controller.execute_structure_sync(&request, &cancel, progress),
                        );
                    }
                    let Some(request) = data_request else {
                        return SyncExecuteOutcome::None;
                    };
                    SyncExecuteOutcome::Data(
                        controller.execute_data_sync(&request, &cancel, progress),
                    )
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(message) = receiver.try_recv() {
                    let _ = view.update(cx, |this, cx| {
                        if let Some(state) = this.sync_wizard.as_mut()
                            && state.task_center_id == Some(task_id)
                        {
                            push_sync_log(state, message);
                            // 只在原本贴底时跟随新行：用户往回看历史时不被强行拉回。
                            let offset = state.log_scroll.offset();
                            if state.log_scroll.max_offset().y - offset.y <= gpui::px(8.) {
                                state.log_scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        }
                    });
                }
                futures::select! {
                    outcome = result => {
                        let _ = view.update(cx, |this, cx| {
                            this.finish_sync_execute(task_id, outcome, cx);
                        });
                        break;
                    }
                    _ = cx.background_executor().timer(std::time::Duration::from_millis(80)).fuse() => {}
                }
            }
        });
        self._sync_execute_task = Some(task);
        cx.notify();
    }

    fn finish_sync_execute(
        &mut self,
        task_id: u64,
        outcome: SyncExecuteOutcome,
        cx: &mut Context<Self>,
    ) {
        let mut data_run_id = None;
        let (status, stage, error, lines) = match outcome {
            SyncExecuteOutcome::Data(Ok(report)) => {
                data_run_id = report.run_id;
                let failed = report.failed_tables();
                let mut lines = data_report_lines(&report);
                // 执行账本落盘后给出可追溯的记录号，便于回看块明细与样本。
                if let Some(run_id) = report.run_id {
                    lines.push(
                        tr("app.sync.run_recorded")
                            .replace("%{id}", &run_id.to_string()),
                    );
                }
                if failed.is_empty() {
                    (
                        TaskCenterStatus::Success,
                        tr("app.sync.data_done").to_string(),
                        None,
                        lines,
                    )
                } else {
                    (
                        TaskCenterStatus::Failed,
                        tr("app.sync.data_partial").to_string(),
                        Some(data_failures(&failed)),
                        lines,
                    )
                }
            }
            SyncExecuteOutcome::Structure(Ok(report)) => {
                let failed = report.failed_tables();
                let lines = structure_report_lines(&report);
                if failed.is_empty() {
                    (
                        TaskCenterStatus::Success,
                        tr("app.sync.structure_done").to_string(),
                        None,
                        lines,
                    )
                } else {
                    (
                        TaskCenterStatus::Failed,
                        tr("app.sync.structure_partial").to_string(),
                        Some(structure_failures(&failed)),
                        lines,
                    )
                }
            }
            SyncExecuteOutcome::Data(Err(error)) | SyncExecuteOutcome::Structure(Err(error)) => {
                if error.kind == fluxdb_core::ErrorKind::Cancelled {
                    (
                        TaskCenterStatus::Canceled,
                        tr("app.sync.canceled").to_string(),
                        None,
                        vec![error.message],
                    )
                } else {
                    (
                        TaskCenterStatus::Failed,
                        tr("app.sync.failed").to_string(),
                        Some(error.message.clone()),
                        vec![error.message],
                    )
                }
            }
            SyncExecuteOutcome::None => (
                TaskCenterStatus::Failed,
                tr("app.sync.failed").to_string(),
                Some(tr("app.sync.choose_tables").to_string()),
                Vec::new(),
            ),
        };
        // 报告摘要同时写进任务中心：向导被收起（后台运行）时也能在记录里看到逐表结果。
        if let Some(record) = self
            .task_center
            .records
            .iter_mut()
            .find(|record| record.id == task_id)
        {
            for line in &lines {
                record.push_log(line.clone());
            }
        }
        let mut close_after_finish = false;
        if let Some(state) = self.sync_wizard.as_mut()
            && state.task_center_id == Some(task_id)
        {
            state.running = false;
            state.task_center_id = None;
            state.log = lines.into_iter().map(SyncLogEntry::now).collect();
            state.error = error.clone();
            if data_run_id.is_some() {
                state.last_run_id = data_run_id;
            }
            // 后台跑完且界面仍是收起状态：结果已在任务中心，没必要再留一份向导状态。
            close_after_finish = !state.visible;
        }
        if close_after_finish {
            self.sync_wizard = None;
            self.sync_pair_rows.clear();
            self._sync_pair_subscriptions.clear();
        }
        self.finish_task_center_record(task_id, status, stage.clone(), error.clone());
        self.show_message(
            stage,
            if error.is_some() {
                AppMessageKind::Error
            } else {
                AppMessageKind::Success
            },
            cx,
        );
        cx.notify();
    }
}

enum SyncPreviewOutcome {
    Data(fluxdb_core::Result<DataSyncPreview>),
    Structure(fluxdb_core::Result<StructureSyncPreview>),
    None,
}

enum SyncExecuteOutcome {
    Data(fluxdb_core::Result<DataSyncReport>),
    Structure(fluxdb_core::Result<StructureSyncReport>),
    None,
}

fn data_options_from_state(
    state: &SyncWizardState,
    pair_rows: &[SyncPairRow],
    cx: &App,
) -> DataSyncOptions {
    let mut options = state.data_options.clone();
    let (key_columns, table_key_columns) = parse_key_columns(&state.key_columns_text);
    options.key_columns = key_columns;
    options.table_key_columns = table_key_columns;
    let (skip_columns, table_skip_columns) = parse_key_columns(&state.skip_columns_text);
    options.skip_columns = skip_columns;
    options.table_skip_columns = table_skip_columns;
    let (column_map, table_column_map) = parse_column_maps(&state.column_map_text);
    options.column_map = column_map;
    options.table_column_map = table_column_map;
    options.table_map = table_map_from_rows(pair_rows, cx);
    if let Ok(rows) = state.batch_rows_text.trim().parse::<usize>() {
        options.batch_rows = rows;
    }
    options
}

fn structure_options_from_state(
    state: &SyncWizardState,
    pair_rows: &[SyncPairRow],
    cx: &App,
) -> StructureSyncOptions {
    let mut options = state.structure_options.clone();
    options.type_overrides = parse_type_overrides(&state.type_overrides_text);
    options.table_map = table_map_from_rows(pair_rows, cx);
    options
}

/// 读取「表配对」下拉：首项「自动配对」不写入映射，其余为显式目标表键。
/// 自动配对的目标表：完整配对键优先，跨 schema 维度时按唯一表名回退，
/// 再无同名表时取「名称 + 数字后缀」里序号最小的近似表。
fn auto_target_table<'a>(
    state: &'a SyncWizardState,
    source: &SyncTableInfo,
) -> Option<&'a SyncTableInfo> {
    auto_target_in(&state.target_tables, source)
}

fn auto_target_in<'a>(
    targets: &'a [SyncTableInfo],
    source: &SyncTableInfo,
) -> Option<&'a SyncTableInfo> {
    if let Some(table) = targets.iter().find(|table| table.key == source.key) {
        return Some(table);
    }
    let same: Vec<&SyncTableInfo> = targets
        .iter()
        .filter(|table| table.name == source.name)
        .collect();
    match same.len() {
        0 => {}
        1 => return Some(same[0]),
        // 同名多张表（跨 schema）时不猜。
        _ => return None,
    }
    // 近似回退只认 `3d_ip_region` → `3d_ip_region_1` / `3d_ip_region1`，
    // 不认 `3d_ip_region_log` 这类表，避免把无关表配进来。
    targets
        .iter()
        .filter_map(|table| numbered_suffix(&source.name, &table.name).map(|n| (n, table)))
        .min_by_key(|(suffix, table)| (*suffix, table.key.clone()))
        .map(|(_, table)| table)
}

/// 候选名是否是「源名 + 数字后缀」（大小写不敏感，允许 `_` / `-` / 空格分隔）。
fn numbered_suffix(source: &str, candidate: &str) -> Option<u32> {
    if source.is_empty() || candidate.len() <= source.len() {
        return None;
    }
    let prefix = candidate.get(..source.len())?;
    if !prefix.eq_ignore_ascii_case(source) {
        return None;
    }
    let rest = &candidate[source.len()..];
    let digits = rest.trim_start_matches(['_', '-', ' ']);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn table_map_from_rows(rows: &[SyncPairRow], cx: &App) -> BTreeMap<String, String> {
    rows.iter()
        .filter_map(|row| {
            let value = row.select.read(cx).selected_value()?;
            (value != &row.auto_label).then(|| (row.source_key.clone(), value.clone()))
        })
        .collect()
}

/// 比对键：`表键:列, 列` 为表级，其余为全局；组之间用分号或换行分隔。
///
/// 例：`id; orders:id, tenant_id` → 全局 `id`，orders 用 `id, tenant_id`。
/// 列本身不含逗号，因此组内仍按逗号切分；没有 `表键:` 前缀的组并入全局。
fn parse_key_columns(text: &str) -> (Vec<String>, BTreeMap<String, Vec<String>>) {
    let mut global = Vec::new();
    let mut per_table: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for group in text.split([';', '\n']) {
        let group = group.trim();
        if group.is_empty() {
            continue;
        }
        match group.split_once(':') {
            // 带 `表键:` 前缀的组只作用于该表；表名为空的组视为误输入，忽略。
            Some((table, columns)) => {
                let table = table.trim();
                let columns = split_column_list(columns);
                if !table.is_empty() && !columns.is_empty() {
                    per_table
                        .entry(table.to_string())
                        .or_default()
                        .extend(columns);
                }
            }
            None => global.extend(split_column_list(group)),
        }
    }
    (global, per_table)
}

/// 把单张表的比对键/跳过字段写回文本：`Some(columns)` 覆盖或新增表级项，
/// `None` 或空列表删除该项（继续跟随全局）；全局部分原样保留。
fn write_table_column_list(text: &str, table: &str, columns: Option<Vec<String>>) -> String {
    let (global, mut per_table) = parse_key_columns(text);
    match columns.filter(|columns| !columns.is_empty()) {
        Some(columns) => {
            per_table.insert(table.to_string(), columns);
        }
        None => {
            per_table.remove(table);
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !global.is_empty() {
        parts.push(global.join(", "));
    }
    for (table, columns) in &per_table {
        parts.push(format!("{table}:{}", columns.join(", ")));
    }
    parts.join("; ")
}

/// 字段清单：逗号/分号/换行分隔，空项忽略。
fn split_column_list(text: &str) -> Vec<String> {
    text.split([',', ';', '\n'])
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .map(str::to_string)
        .collect()
}

/// 字段映射：每项 `源字段=目标字段` 或 `表.源字段=目标字段`，逗号/分号/换行分隔。
///
/// 不带表前缀的项对所有表生效；带前缀的项只作用于该表（完整配对键，含 schema）。
/// 缺少 `=`、任一侧为空、或表前缀为空的项直接忽略，避免把误输入当成映射。
fn parse_column_maps(
    text: &str,
) -> (
    BTreeMap<String, String>,
    BTreeMap<String, BTreeMap<String, String>>,
) {
    let mut global = BTreeMap::new();
    let mut per_table: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for pair in split_column_list(text) {
        let Some((left, right)) = pair.split_once('=') else {
            continue;
        };
        let left = left.trim();
        let right = right.trim();
        if left.is_empty() || right.is_empty() {
            continue;
        }
        // 表键本身可能带 schema（`public.orders`），按最后一个 `.` 切分。
        match left.rsplit_once('.') {
            Some((table, column)) if !table.trim().is_empty() && !column.trim().is_empty() => {
                per_table
                    .entry(table.trim().to_string())
                    .or_default()
                    .insert(column.trim().to_string(), right.to_string());
            }
            _ => {
                global.insert(left.to_string(), right.to_string());
            }
        }
    }
    (global, per_table)
}

/// 目标类型覆盖：每项 `表.列=目标类型`，按换行/分号分隔（不能用逗号，
/// 类型本身可能带 `numeric(12,3)` 这类参数）。表前缀必填，缺少的项忽略。
fn parse_type_overrides(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut overrides: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for pair in text.split([';', '\n']) {
        let Some((left, right)) = pair.split_once('=') else {
            continue;
        };
        let left = left.trim();
        let right = right.trim();
        let Some((table, column)) = left.rsplit_once('.') else {
            continue;
        };
        let (table, column) = (table.trim(), column.trim());
        if table.is_empty() || column.is_empty() || right.is_empty() {
            continue;
        }
        overrides
            .entry(table.to_string())
            .or_default()
            .insert(column.to_string(), right.to_string());
    }
    overrides
}

/// 写回单表类型覆盖：保留其它表项；该表没有覆盖时整表删除。
fn write_table_type_overrides(
    text: &str,
    table: &str,
    entries: &BTreeMap<String, String>,
) -> String {
    let mut overrides = parse_type_overrides(text);
    if entries.is_empty() {
        overrides.remove(table);
    } else {
        overrides.insert(table.to_string(), entries.clone());
    }
    let mut parts = Vec::new();
    for (table, columns) in &overrides {
        for (column, target) in columns {
            parts.push(format!("{table}.{column}={target}"));
        }
    }
    parts.join("\n")
}

fn default_data_selection(preview: &DataSyncPreview) -> Option<String> {
    preview
        .tables
        .iter()
        .find(|table| table.changed_rows() > 0)
        .or_else(|| preview.tables.first())
        .map(|table| table.key.clone())
}

fn default_structure_selection(preview: &StructureSyncPreview) -> Option<String> {
    preview
        .tables
        .iter()
        .find(|table| table.selected > 0)
        .or_else(|| preview.tables.first())
        .map(|table| table.key.clone())
}

fn push_sync_log(state: &mut SyncWizardState, message: String) {
    // 逐表的进度消息会滚动上千条，只保留最近一屏多一点，避免无限增长。
    const LOG_LIMIT: usize = 200;
    if state.log.len() >= LOG_LIMIT {
        state.log.remove(0);
    }
    state.log.push(SyncLogEntry::now(message));
}

fn data_report_lines(report: &DataSyncReport) -> Vec<String> {
    report
        .tables
        .iter()
        .flat_map(|table| {
            let mut summary = format!(
                "{}：+{} ~{} -{}",
                table.name, table.inserted, table.updated, table.deleted
            );
            if table.failed_rows > 0 {
                summary.push_str(" · ");
                summary.push_str(&tr("app.sync.skipped_rows").replace("%{n}", &table.failed_rows.to_string()));
            }
            let headline = match &table.error {
                Some(error) if !table.skipped => format!("{summary} · {error}"),
                Some(reason) if table.skipped => format!("{} · {reason}", table.name),
                _ => summary,
            };
            // 失败行逐条展示，方便按键回查；条数已在报告层限流。
            std::iter::once(headline).chain(
                table
                    .failures
                    .iter()
                    .map(|failure| format!("{} · {} · {}", table.name, failure.key, failure.message)),
            )
        })
        .collect()
}

fn structure_report_lines(report: &StructureSyncReport) -> Vec<String> {
    report
        .tables
        .iter()
        .map(|table| {
            let summary = format!("{}：{} 项变更", table.name, table.applied);
            match &table.error {
                Some(error) => format!("{summary} · {error}"),
                None => summary,
            }
        })
        .collect()
}

fn data_failures(failed: &[&TableSyncReport]) -> String {
    failed
        .iter()
        .filter_map(|table| table.error.as_ref().map(|error| format!("{}：{error}", table.name)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn structure_failures(failed: &[&TableStructureReport]) -> String {
    failed
        .iter()
        .filter_map(|table| table.error.as_ref().map(|error| format!("{}：{error}", table.name)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn default_sync_database(controller: &AppController, connection_id: ConnectionId) -> String {
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

/// 读取单端数据库/Schema 候选项；单层失败只记日志，不阻塞弹框。
fn sync_endpoint_options(
    controller: &AppController,
    (connection_id, database, _schema): (ConnectionId, String, String),
) -> SyncEndpointOptions {
    let databases = sync_list_objects(controller, connection_id, None, ObjectKind::Database);
    let mut options = SyncEndpointOptions {
        databases,
        ..Default::default()
    };
    if options.databases.is_empty() {
        return options;
    }
    // MySQL 家族/SQLite 没有独立 schema 概念，界面也不展示 Schema 选择器；
    // 这里不再多查一层，避免把「database 下的同名对象」误当 schema。
    let uses_schemas = controller
        .connection_configs()
        .into_iter()
        .find(|config| config.id == connection_id)
        .is_some_and(|config| config.kind.uses_schemas());
    if !uses_schemas {
        return options;
    }
    let database = database.trim();
    if database.is_empty() {
        return options;
    }
    if let Some(path) = sync_parent_path(connection_id, database, "") {
        options.schemas =
            sync_list_objects(controller, connection_id, Some(&path), ObjectKind::Schema);
    }
    options
}

fn sync_list_objects(
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
            tracing::warn!(connection_id = connection_id.0, ?kind, %error, "同步候选项读取失败");
            Vec::new()
        }
    }
}

fn sync_parent_path(connection_id: ConnectionId, database: &str, schema: &str) -> Option<ObjectPath> {
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
