// 恢复数据库弹框：与「数据库备份」同款自绘模态（遮罩 + 面板 + 固定底栏）。
// 渲染与按钮交互在 database_restore_modal 系列函数；预检查/执行逻辑在下方 NavicatMain impl。

/// 恢复弹框运行时共享状态：渲染（self.restore_modal）与后台异步任务（Rc 克隆）两侧共享。
#[derive(Clone)]
struct RestoreRuntime {
    running: bool,
    prepared: Option<(fluxdb_app::RestoreRequest, fluxdb_app::RestorePlan)>,
    logs: Vec<String>,
    cancel: Arc<AtomicBool>,
    /// 从备份列表打开时带入的备份记录 id；执行完成后写入恢复记录做关联。
    backup_id: Option<String>,
}

/// 恢复弹框挂载状态：UI 实体 + 打开时的配置快照（渲染期间不读 controller）。
#[derive(Clone)]
struct RestoreModal {
    kind: DatabaseKind,
    /// 当前页签（常规/对象选择/高级/消息日志）。
    tab: RestoreTab,
    /// 与 kind 同类型、可选为目标连接的配置快照；下拉按此顺序展示，选中行索引即候选索引。
    candidates: Vec<ConnectionConfig>,
    /// 从备份 tab 带入的文件元信息，路径匹配时可附带 manifest 免去重复识别。
    meta: Option<BackupFileMeta>,
    connection: Entity<SelectState<SearchableVec<String>>>,
    source_input: Entity<InputState>,
    target_input: Entity<InputState>,
    /// 「现有库」模式下目标库下拉；选项来自目标连接下所有库。
    target_db: Entity<SelectState<SearchableVec<String>>>,
    /// 已为目标连接加载过库列表的连接 id，用于渲染时判断是否需要重载（连接切换）。
    target_db_loaded_for: Option<ConnectionId>,
    mode: Entity<SelectState<SearchableVec<RestoreModeOption>>>,
    /// 高级页：事务范围下拉（引擎默认/单事务）；SQLite 隐藏。
    transaction: Entity<SelectState<SearchableVec<String>>>,
    /// 高级页：完成验证下拉（基础/逐表行数）；SQLite 隐藏。
    validation: Entity<SelectState<SearchableVec<String>>>,
    /// 逐表模式：每个源表一个动作下拉；候选按探测到的存在性/备份内容过滤。
    table_grid: Vec<TableGridRow>,
    /// 当前 table_grid 的来源指纹（连接id|源路径|目标库）；变化时需重新探测。
    probe_key: Option<String>,
    /// 探测进行中：进入对象页后台检查目标，展示 loading。
    probe_running: bool,
    /// 最近一次探测失败信息；改动表单前不自动重试。
    probe_error: Option<String>,
    /// 破坏性动作二次确认：是否展开确认层。
    confirm_open: bool,
    /// 确认层要求用户原样输入的目标名称（库名或文件名）。
    confirm_target: String,
    /// 确认层展示：会删除目标数据/定义的表清单。
    confirm_destructive: Vec<String>,
    /// 确认层输入框（打开弹框时即创建，避免在监听器里重建实体）。
    confirm_input: Entity<InputState>,
    runtime: std::rc::Rc<std::cell::RefCell<RestoreRuntime>>,
}

/// 恢复目标模式：界面文案可翻译，业务判断始终使用稳定值。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreTargetMode {
    NewFile,
    NewDatabase,
    NewSchema,
    ExistingDatabase,
}

impl RestoreTargetMode {
    fn label(self) -> SharedString {
        match self {
            Self::NewFile => tr("app.database_restore.mode.new_file"),
            Self::NewDatabase => tr("app.database_restore.mode.new_database"),
            Self::NewSchema => tr("app.database_restore.mode.new_schema"),
            Self::ExistingDatabase => tr("app.database_restore.mode.existing_database"),
        }
    }
}

#[derive(Clone)]
struct RestoreModeOption {
    mode: RestoreTargetMode,
    label: SharedString,
}

impl SelectItem for RestoreModeOption {
    type Value = RestoreTargetMode;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.mode
    }
}

fn restore_mode_option(mode: RestoreTargetMode) -> RestoreModeOption {
    RestoreModeOption {
        mode,
        label: mode.label(),
    }
}

/// 逐表动作选择：`Pending` 是强制用户显式选择的占位，绝不静默退化为破坏性动作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreActionChoice {
    Pending,
    Action(fluxdb_core::RestoreTableAction),
}

impl RestoreActionChoice {
    fn label(self) -> SharedString {
        match self {
            Self::Pending => tr("app.database_restore.action.placeholder"),
            Self::Action(action) => restore_action_label(action),
        }
    }
}

#[derive(Clone)]
struct RestoreActionOption {
    choice: RestoreActionChoice,
    label: SharedString,
}

impl SelectItem for RestoreActionOption {
    type Value = RestoreActionChoice;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.choice
    }
}

fn restore_action_option(choice: RestoreActionChoice) -> RestoreActionOption {
    RestoreActionOption {
        choice,
        label: choice.label(),
    }
}

fn restore_action_label(action: fluxdb_core::RestoreTableAction) -> SharedString {
    match action {
        fluxdb_core::RestoreTableAction::Create => tr("app.database_restore.action.create"),
        fluxdb_core::RestoreTableAction::Recreate => tr("app.database_restore.action.recreate"),
        fluxdb_core::RestoreTableAction::TruncateAndLoad => {
            tr("app.database_restore.action.truncate_load")
        }
        fluxdb_core::RestoreTableAction::Append => tr("app.database_restore.action.append"),
        fluxdb_core::RestoreTableAction::Skip => tr("app.database_restore.action.skip"),
    }
}

fn restore_status_label(status: fluxdb_core::RestoreObjectStatus) -> SharedString {
    match status {
        fluxdb_core::RestoreObjectStatus::Succeeded => {
            tr("app.database_restore.status.succeeded")
        }
        fluxdb_core::RestoreObjectStatus::Warning => tr("app.database_restore.status.warning"),
        fluxdb_core::RestoreObjectStatus::Failed => tr("app.database_restore.status.failed"),
        fluxdb_core::RestoreObjectStatus::Skipped => tr("app.database_restore.status.skipped"),
    }
}

fn restore_transaction_label(mode: fluxdb_app::RestoreTransactionMode) -> SharedString {
    match mode {
        fluxdb_app::RestoreTransactionMode::EngineDefault => {
            tr("app.database_restore.transaction.engine_default")
        }
        fluxdb_app::RestoreTransactionMode::SingleTransaction => {
            tr("app.database_restore.transaction.single_transaction")
        }
    }
}

fn restore_validation_label(validation: fluxdb_app::RestoreValidation) -> SharedString {
    match validation {
        fluxdb_app::RestoreValidation::Basic => tr("app.database_restore.validation.basic"),
        fluxdb_app::RestoreValidation::RowCount => {
            tr("app.database_restore.validation.row_count")
        }
    }
}

fn restore_format_label(format: fluxdb_core::BackupFormat) -> SharedString {
    match format {
        fluxdb_core::BackupFormat::Sql => tr("app.database_restore.format.sql"),
        fluxdb_core::BackupFormat::SqliteBinary => {
            tr("app.database_restore.format.sqlite_binary")
        }
    }
}

/// 逐表恢复的一行：探测得到的对象事实 + 该表的动作下拉（候选已按事实过滤）。
#[derive(Clone)]
struct TableGridRow {
    /// 决策匹配键（PG 非 public 为 schema.name），提交给后端的表标识。
    key: String,
    /// 展示名（PG 含 schema 前缀）。
    name: String,
    /// 目标库是否已有同名对象（探测回填，决定默认动作与候选）。
    exists: bool,
    /// 备份是否含可执行结构（决定能否新建/重建）。
    has_ddl: bool,
    /// 备份是否含数据（决定能否清空/追加）。
    has_data: bool,
    select: Entity<SelectState<SearchableVec<RestoreActionOption>>>,
}

impl NavicatMain {
    fn open_restore_dialog(
        &mut self,
        connection_id: ConnectionId,
        source: Option<PathBuf>,
        meta: Option<BackupFileMeta>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configs = self.controller.connection_configs();
        let Some(initial) = configs.iter().find(|c| c.id == connection_id) else {
            return;
        };
        if !matches!(
            initial.kind,
            DatabaseKind::MySql
                | DatabaseKind::TiDb
                | DatabaseKind::MariaDb
                | DatabaseKind::Postgres
                | DatabaseKind::Sqlite
                | DatabaseKind::Dameng
                | DatabaseKind::Oracle
        ) {
            self.show_message(
                tr("app.database_restore.unsupported"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let kind = initial.kind;
        let candidates: Vec<_> = configs.into_iter().filter(|c| c.kind == kind).collect();
        let labels: Vec<String> = candidates.iter().map(|c| c.name.clone()).collect();
        let selected = candidates
            .iter()
            .position(|c| c.id == connection_id)
            .unwrap_or(0);
        let connection = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(labels.clone()),
                Some(IndexPath::default().row(selected)),
                window,
                cx,
            )
        });
        let source_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                source
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        });
        let target_default = if kind == DatabaseKind::Sqlite {
            source
                .as_ref()
                .map(|p| {
                    p.with_file_name(format!(
                        "restored_{}.db",
                        chrono::Local::now().format("%Y%m%d_%H%M%S")
                    ))
                    .to_string_lossy()
                    .into_owned()
                })
                .unwrap_or_default()
        } else {
            format!("restore_{}", chrono::Local::now().format("%Y%m%d_%H%M%S"))
        };
        let target_input = cx.new(|cx| InputState::new(window, cx).default_value(target_default));
        let modes = if kind == DatabaseKind::Sqlite {
            vec![restore_mode_option(RestoreTargetMode::NewFile)]
        } else if matches!(kind, DatabaseKind::Dameng | DatabaseKind::Oracle) {
            // Oracle 的 schema 就是账号，且导入只写空 schema：只提供「新建 schema」。 
            vec![restore_mode_option(RestoreTargetMode::NewSchema)]
        } else {
            vec![
                restore_mode_option(RestoreTargetMode::NewDatabase),
                restore_mode_option(RestoreTargetMode::ExistingDatabase),
            ]
        };
        let mode = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(modes),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        // 「现有库」目标下拉，初始为空，弹框打开与目标连接变化时异步填充。
        let target_db = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx)
        });
        // 高级页事务/验证下拉：选项取自枚举 label，默认选中第一项（引擎默认 / 基础）。
        let transaction = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    [
                        tr("app.database_restore.transaction.engine_default"),
                        tr("app.database_restore.transaction.single_transaction"),
                    ]
                    .into_iter()
                    .map(|label| label.to_string())
                    .collect::<Vec<String>>(),
                ),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let validation = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    [
                        tr("app.database_restore.validation.basic"),
                        tr("app.database_restore.validation.row_count"),
                    ]
                    .into_iter()
                    .map(|label| label.to_string())
                    .collect::<Vec<String>>(),
                ),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let confirm_input = cx.new(|cx| InputState::new(window, cx));
        // 老备份记录可能没有 id（空串），视为无关联，不写恢复记录。
        let backup_record_id = meta
            .as_ref()
            .map(|m| m.id.clone())
            .filter(|id| !id.is_empty());
        self.restore_modal = Some(RestoreModal {
            kind,
            tab: RestoreTab::General,
            candidates,
            meta,
            connection,
            source_input,
            target_input,
            target_db,
            target_db_loaded_for: Some(connection_id),
            mode,
            transaction,
            validation,
            table_grid: Vec::new(),
            probe_key: None,
            probe_running: false,
            probe_error: None,
            confirm_open: false,
            confirm_target: String::new(),
            confirm_destructive: Vec::new(),
            confirm_input,
            runtime: std::rc::Rc::new(std::cell::RefCell::new(RestoreRuntime {
                running: false,
                prepared: None,
                logs: vec![tr("app.database_restore.initial_log").to_string()],
                cancel: Arc::new(AtomicBool::new(false)),
                backup_id: backup_record_id,
            })),
        });
        // 打开即拉取默认目标连接的库列表。
        self.load_restore_target_databases(
            self.restore_modal.as_ref().unwrap().target_db.clone(),
            window,
            cx,
        );
        cx.notify();
    }

    /// 拉取当前「目标连接」下所有库，就地填充 target_db 下拉（异步，连接按需打开）。
    fn load_restore_target_databases(
        &mut self,
        target_db: Entity<SelectState<SearchableVec<String>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.restore_modal.as_ref().and_then(|m| {
            restore_selected_connection_id(m, cx)
        }) else {
            return;
        };
        let mut controller = self.controller.clone();
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let (controller, _event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::OpenConnection(connection_id));
                    (controller, event)
                })
                .await;
            let _ = cx.update(|window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.controller.merge_last_error_from(&controller);
                    // 库列表：OpenConnection 已把目标连接所有库写入克隆 state，直接读取。
                    let names = controller
                        .state()
                        .connections
                        .iter()
                        .find(|c| c.config.id == connection_id)
                        .map(all_connection_database_names)
                        .unwrap_or_default();
                    target_db.update(cx, |select, cx| {
                        select.set_items(SearchableVec::new(names), window, cx);
                    });
                    cx.notify();
                    cx.refresh_windows();
                });
            });
        }));
    }

    /// 渲染期消费：现有库模式下，源文件与目标库就绪且指纹变化时后台探测目标对象。
    /// 非现有库模式（新建/SQLite）网格无意义，清空并复位探测状态。
    fn sync_restore_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = &self.restore_modal else {
            return;
        };
        let existing_mode = modal.kind != DatabaseKind::Sqlite
            && modal
                .mode
                .read(cx)
                .selected_value()
                .is_some_and(|v| *v == RestoreTargetMode::ExistingDatabase);
        if !existing_mode {
            if modal.probe_key.is_some() || modal.probe_running || !modal.table_grid.is_empty() {
                if let Some(m) = &mut self.restore_modal {
                    m.probe_key = None;
                    m.probe_running = false;
                    m.probe_error = None;
                    m.table_grid.clear();
                }
                cx.notify();
            }
            return;
        }
        let Some(connection_id) = restore_selected_connection_id(modal, cx) else {
            return;
        };
        let source = modal.source_input.read(cx).value().trim().to_string();
        let target = modal
            .target_db
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        // 文件/目标未就绪：等待，不清已有网格以免闪烁。
        if source.is_empty() || target.is_empty() {
            return;
        }
        let key = format!("{connection_id:?}|{source}|{target}");
        if modal.probe_running || modal.probe_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.start_restore_probe(connection_id, source, target, key, window, cx);
    }

    /// 后台探测目标：只读解析备份内容 + 枚举目标对象存在性，回填逐表网格。
    /// 进入时把 probe_key 置为本次指纹，完成时再比对以丢弃陈旧结果（切连接/文件/目标）。
    fn start_restore_probe(
        &mut self,
        connection_id: ConnectionId,
        source: String,
        target: String,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self
            .restore_modal
            .as_ref()
            .and_then(|m| m.candidates.iter().find(|c| c.id == connection_id).cloned())
        else {
            return;
        };
        let request = fluxdb_app::RestoreRequest {
            config,
            source: PathBuf::from(source),
            target,
            create_target: false,
            tool: PathBuf::new(),
            manifest: None,
            table_decisions: Vec::new(),
            options: Default::default(),
        };
        if let Some(m) = &mut self.restore_modal {
            m.probe_key = Some(key.clone());
            m.probe_running = true;
            m.probe_error = None;
        }
        cx.notify();
        let controller = self.controller.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch_database_task(
                        AppCommand::ProbeRestore(request),
                        &cancel,
                        &mut |_| {},
                    ) {
                        AppEvent::RestoreObjectsProbed(objects) => Ok(objects),
                        AppEvent::Failed(error) => Err(error.message),
                        _ => Err(tr("app.database_restore.probe_error").to_string()),
                    }
                })
                .await;
            let _ = cx.update(|window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    // 陈旧结果：指纹已变，交给下一次探测覆盖，这里不动状态。
                    if this
                        .restore_modal
                        .as_ref()
                        .and_then(|m| m.probe_key.as_deref())
                        != Some(key.as_str())
                    {
                        return;
                    }
                    if let Some(m) = &mut this.restore_modal {
                        m.probe_running = false;
                    }
                    match result {
                        Ok(objects) => {
                            this.rebuild_restore_grid(objects, window, cx);
                            if let Some(m) = &mut this.restore_modal {
                                m.probe_error = None;
                            }
                        }
                        Err(message) => {
                            if let Some(m) = &mut this.restore_modal {
                                m.table_grid.clear();
                                m.probe_error = Some(message.clone());
                            }
                            this.show_message(
                                format!(
                                    "{}: {message}",
                                    tr("app.database_restore.target_check_failed")
                                ),
                                AppMessageKind::Warning,
                                cx,
                            );
                        }
                    }
                    cx.notify();
                    cx.refresh_windows();
                });
            });
        }));
    }

    /// 用探测结果重建逐表网格：每行按存在性/备份内容生成合法动作候选与默认选择。
    fn rebuild_restore_grid(
        &mut self,
        objects: Vec<fluxdb_app::RestoreObjectProbe>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let grid = objects
            .into_iter()
            .map(|o| {
                let (options, default_idx) =
                    restore_row_action_plan(o.exists_in_target, o.has_ddl, o.has_data);
                let select = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(options),
                        Some(IndexPath::default().row(default_idx)),
                        window,
                        cx,
                    )
                });
                TableGridRow {
                    key: o.key,
                    name: o.name,
                    exists: o.exists_in_target,
                    has_ddl: o.has_ddl,
                    has_data: o.has_data,
                    select,
                }
            })
            .collect();
        if let Some(m) = &mut self.restore_modal {
            m.table_grid = grid;
        }
    }

    /// 读取逐表网格的决策，供 preflight 组装 RestoreRequest.table_decisions。
    /// 用 key（PG=schema.name）匹配；占位「待选择策略」不提交（预检查按钮此时已禁用，双保险）。
    fn restore_table_choices(&self, cx: &App) -> Vec<fluxdb_core::PerTableDecision> {
        let Some(modal) = &self.restore_modal else {
            return Vec::new();
        };
        if modal
            .mode
            .read(cx)
            .selected_value()
            .is_none_or(|v| *v != RestoreTargetMode::ExistingDatabase)
        {
            return Vec::new();
        }
        modal
            .table_grid
            .iter()
            .filter_map(|row| {
                let act = match row.select.read(cx).selected_value()? {
                    RestoreActionChoice::Pending => return None,
                    RestoreActionChoice::Action(action) => *action,
                };
                Some(fluxdb_core::PerTableDecision {
                    table: row.key.clone(),
                    action: act,
                })
            })
            .collect()
    }

    /// 仍处于「待选择策略」占位的已存在对象数：>0 时禁止预检查与确认恢复（设计文档 §6.3/§14.7）。
    fn restore_pending_count(&self, cx: &App) -> usize {
        let Some(modal) = &self.restore_modal else {
            return 0;
        };
        modal
            .table_grid
            .iter()
            .filter(|row| {
                row.select
                    .read(cx)
                    .selected_value()
                    .is_some_and(|v| *v == RestoreActionChoice::Pending)
            })
            .count()
    }

    fn start_restore_preflight(
        &mut self,
        request: fluxdb_app::RestoreRequest,
        state: std::rc::Rc<std::cell::RefCell<RestoreRuntime>>,
        cx: &mut Context<Self>,
    ) {
        if state.borrow().running {
            return;
        }
        {
            let mut data = state.borrow_mut();
            data.running = true;
            data.logs = vec![tr("app.database_restore.checking").to_string()];
            data.cancel.store(false, Ordering::Relaxed);
        }
        let controller = self.controller.clone();
        let cancel = state.borrow().cancel.clone();
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch_database_task(
                        AppCommand::PrepareRestore(request),
                        &cancel,
                        &mut |_| {},
                    ) {
                        AppEvent::RestorePrepared { request, plan } => Ok((request, plan)),
                        AppEvent::Failed(error) => Err(error),
                        _ => unreachable!("恢复预检查返回了错误事件"),
                    }
                })
                .await;
            let _ = view.update(cx, |_, cx| {
                let mut data = state.borrow_mut();
                data.running = false;
                match result {
                    Ok((request, plan)) => {
                        data.logs = vec![plan.summary.clone()];
                        data.logs.extend(
                            plan.warnings
                                .iter()
                                .map(|warning| crate::localize_backend_error_text(warning)),
                        );
                        // 逐表回显：解析后的动作 + 预检查风险；破坏性动作显式告警。
                        for info in &plan.tables {
                            let action = info.default_action;
                            let mut line = format!(
                                "{}{}",
                                tr("app.database_restore.plan.table_line")
                                    .replace("%{name}", &info.name)
                                    .replace("%{action}", restore_action_label(action).as_str()),
                                if info.exists_in_target {
                                    tr("app.database_restore.target_exists")
                                } else {
                                    tr("app.database_restore.target_new")
                                }
                            );
                            if action.destroys_target_data() {
                                line.push_str(&tr("app.database_restore.destructive_warning"));
                            }
                            data.logs.push(line);
                            for risk in &info.risks {
                                data.logs.push(tr("app.database_restore.risk").replace("%{risk}", risk));
                            }
                        }
                        data.prepared = Some((request, plan));
                    }
                    Err(e) => data.logs = vec![e.message.clone()],
                }
                cx.notify();
                cx.refresh_windows();
            });
        })
        .detach();
        cx.notify();
    }
    fn start_restore_execution(
        &mut self,
        state: std::rc::Rc<std::cell::RefCell<RestoreRuntime>>,
        cx: &mut Context<Self>,
    ) {
        if state.borrow().running {
            return;
        }
        let Some((request, plan)) = state.borrow().prepared.clone() else {
            return;
        };
        {
            let mut data = state.borrow_mut();
            data.running = true;
            data.cancel.store(false, Ordering::Relaxed);
            data.logs
                .push(tr("app.database_restore.started_log").to_string());
        }
        let controller = self.controller.clone();
        let storage = self.storage.clone();
        let cancel = state.borrow().cancel.clone();
        let backup_id = state.borrow().backup_id.clone();
        cx.spawn(async move |view, cx| {
            let (sender, receiver) = mpsc::channel();
            let result = cx
                .background_spawn(async move {
                    let result = match controller.dispatch_database_task(
                        AppCommand::RunRestore {
                            request: request.clone(),
                            plan,
                        },
                        &cancel,
                        &mut |event| {
                            let _ = sender.send(event);
                        },
                    ) {
                        AppEvent::RestoreCompleted(outcome) => Ok(outcome),
                        AppEvent::Failed(error) => Err(error),
                        _ => unreachable!("恢复返回了错误事件"),
                    };
                    let message = match &result {
                        Ok(outcome) => outcome.verification.clone(),
                        Err(error) => error.message.clone(),
                    };
                    let record = fluxdb_storage::RestoreRecord {
                        source: request.source.to_string_lossy().into_owned(),
                        connection_id: request.config.id,
                        target: request.target.clone(),
                        finished_unix: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        success: result.is_ok(),
                        canceled: cancel.load(Ordering::Relaxed),
                        result: message,
                        outcome: result.as_ref().ok().cloned(),
                        backup_id: backup_id.unwrap_or_default(),
                    };
                    let persisted = controller.record_restore(&storage, record);
                    (result, persisted)
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(event) = receiver.try_recv() {
                    let mut data = state.borrow_mut();
                    data.logs
                        .push(format!("{}：{}", event.stage, event.message));
                    if data.logs.len() > 300 {
                        data.logs.remove(0);
                    }
                }
                if let Some((result, persisted)) = result.as_mut().now_or_never() {
                    let _ = view.update(cx, |this, cx| {
                        let mut data = state.borrow_mut();
                        data.running = false;
                        data.prepared = None;
                        let success = result.is_ok();
                        let message = match &result {
                            Ok(outcome) => tr("app.database_restore.completed")
                                .replace("%{verification}", &outcome.verification),
                            Err(e) => tr("app.database_restore.failed")
                                .replace("%{error}", &e.message),
                        };
                        data.logs.push(message.clone());
                        // 逐对象结果（设计文档 §15.3）：状态 + 动作 + 明细。
                        if let Ok(outcome) = &result {
                            if outcome.total_objects > 0 {
                                data.logs.push(
                                    tr("app.database_restore.object_results")
                                        .replace(
                                            "%{count}",
                                            &outcome.total_objects.to_string(),
                                        ),
                                );
                            }
                            for obj in &outcome.objects {
                                let action = obj
                                    .action
                                    .map(|a| {
                                        tr("app.database_restore.result.action_suffix")
                                            .replace("%{action}", &restore_action_label(a))
                                    })
                                    .unwrap_or_default();
                                let detail = if obj.detail.is_empty() {
                                    String::new()
                                } else {
                                    tr("app.database_restore.result.detail_suffix")
                                        .replace("%{detail}", &obj.detail)
                                };
                                data.logs.push(format!(
                                    "{} {}{}{}",
                                    tr("app.database_restore.result.object")
                                        .replace("%{name}", &obj.name),
                                    action,
                                    tr("app.database_restore.result.status")
                                        .replace("%{status}", &restore_status_label(obj.status)),
                                    detail
                                ));
                            }
                        }
                        if let Err(e) = persisted {
                            data.logs.push(
                                tr("app.database_restore.record_save_failed")
                                    .replace("%{error}", &e.to_string()),
                            );
                        }
                        this.show_message(
                            message,
                            if success {
                                AppMessageKind::Success
                            } else {
                                AppMessageKind::Error
                            },
                            cx,
                        );
                        if success {
                            this.dispatch(AppCommand::RefreshConnectionTree, cx);
                        }
                        cx.notify();
                        cx.refresh_windows();
                    });
                    break;
                }
                let _ = cx.update(|cx| cx.refresh_windows());
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        })
        .detach();
        cx.notify();
    }

    /// 关闭恢复弹框:仅移除挂载状态,后台任务通过 Rc 继续运行不受影响。
    fn close_restore_dialog(&mut self, cx: &mut Context<Self>) {
        if self.restore_modal.take().is_some() {
            cx.notify();
        }
    }

    /// 「确认恢复」入口：计划含破坏性动作（重建/清空后导入）时先要求原样输入目标名，
    /// 否则直接执行。设计文档 §6.4/§14：破坏性动作必须二次确认，避免误删目标数据。
    fn confirm_or_run_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = &self.restore_modal else {
            return;
        };
        if modal.runtime.borrow().running {
            return;
        }
        let prepared = modal.runtime.borrow().prepared.clone();
        let Some((request, plan)) = prepared else {
            return;
        };
        let destructive: Vec<String> = plan
            .tables
            .iter()
            .filter(|t| t.default_action.destroys_target_data())
            .map(|t| {
                tr("app.database_restore.plan.object_with_action")
                    .replace("%{name}", &t.name)
                    .replace("%{action}", restore_action_label(t.default_action).as_str())
            })
            .collect();
        // 整库覆盖（非逐表）到现有库同样具有破坏性：create_target=false 且非逐表时提示。
        let whole_overwrite = plan.tables.is_empty() && !request.create_target;
        if destructive.is_empty() && !whole_overwrite {
            let runtime = modal.runtime.clone();
            self.start_restore_execution(runtime, cx);
            return;
        }
        if let Some(m) = &mut self.restore_modal {
            m.confirm_open = true;
            m.confirm_target = request.target.clone();
            m.confirm_destructive = if destructive.is_empty() {
                vec![tr("app.database_restore.whole_db_warning").to_string()]
            } else {
                destructive
            };
            m.confirm_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
        }
        cx.notify();
        cx.refresh_windows();
    }

    /// 取消破坏性操作确认层（不执行恢复，回到计划查看）。
    fn cancel_restore_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = &mut self.restore_modal {
            m.confirm_open = false;
        }
        cx.notify();
        cx.refresh_windows();
    }

    /// 确认层「确认执行」：输入必须与目标名完全一致，否则拒绝并提示。
    fn run_restore_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = &self.restore_modal else {
            return;
        };
        let typed = modal.confirm_input.read(cx).value().trim().to_string();
        if typed != modal.confirm_target {
            self.show_message(
                tr("app.database_restore.confirm_target")
                    .replace("%{target}", &modal.confirm_target),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let runtime = modal.runtime.clone();
        if let Some(m) = &mut self.restore_modal {
            m.confirm_open = false;
        }
        self.start_restore_execution(runtime, cx);
    }
}

/// 恢复弹框入口：遮罩 + 面板 + 头部/页签/分页正文/底栏，复用备份弹框的视觉规范。
fn database_restore_modal(
    modal: &RestoreModal,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let runtime = modal.runtime.borrow().clone();
    let busy = runtime.running;
    let has_plan = runtime.prepared.is_some();
    let locked = busy || has_plan;
    // 已存在对象未选策略时禁止预检查/确认（设计文档 §6.3/§14.7）。
    let pending = modal
        .table_grid
        .iter()
        .filter(|row| {
            row.select
                .read(cx)
                .selected_value()
                    .is_some_and(|v| *v == RestoreActionChoice::Pending)
        })
        .count();
    let panel = restore_modal_panel(colors, cx)
        .child(restore_modal_header(modal, colors, cx))
        .child(restore_modal_tabs(modal.tab, colors, cx))
        .child(restore_modal_body(modal, &runtime, locked, colors, cx))
        .child(restore_modal_footer(
            &runtime, busy, has_plan, pending, colors, cx,
        ))
        .when(modal.confirm_open, |p| {
            p.child(restore_confirm_overlay(modal, colors, cx))
        });
    restore_modal_shell(panel, colors, cx)
}

/// 遮罩层：点击空白处关闭（运行中关闭不取消后台任务，与备份弹框行为一致）。
fn restore_modal_shell(panel: Div, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.58)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_restore_dialog(cx);
                cx.stop_propagation();
            }),
        )
        .child(panel)
}

fn restore_modal_panel(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .relative()
        .w(px(640.))
        .max_w(px(640.))
        .h(px(520.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .shadow(vec![box_shadow(
            px(0.),
            px(18.),
            px(42.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .text_size(px(13.))
        .text_color(colors.text)
        .key_context("DatabaseRestoreModal")
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.close_restore_dialog(cx);
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn restore_modal_header(
    modal: &RestoreModal,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 上下文行（设计文档 §14.1 A 区）：连接名 · 引擎 · 目标，切页签不消失。
    let conn_label = modal
        .connection
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_default();
    let context = if conn_label.is_empty() {
        tr("app.database_restore.no_target_connection")
            .replace("%{kind}", database_kind_name(modal.kind))
    } else {
        format!("{conn_label} · {}", database_kind_name(modal.kind))
    };
    div()
        .flex_none()
        .px_5()
        .pt_4()
        .pb_3()
        .flex()
        .items_start()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(app_icon(AppIcon::Refresh, 18., colors.text))
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.database_restore.title")),
                        ),
                )
                .child(
                    div()
                        .pl(px(30.))
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(context),
                ),
        )
        .child(
            div()
                .size(px(30.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 16., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_restore_dialog(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

/// 分页正文：按当前页签渲染常规/对象选择/高级/消息日志（设计文档 §14.1）。
/// 各页渲染函数在 database_restore/ui.rs，切页只改 modal.tab，不销毁表单实体。
fn restore_modal_body(
    modal: &RestoreModal,
    runtime: &RestoreRuntime,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        // 必须是 flex 列：否则子页的 flex_1 拿不到受约束高度，
        // 对象页的滚动框会塌成 0 高、把表头与数据行整块裁掉。
        .flex()
        .flex_col()
        .child(match modal.tab {
            RestoreTab::General => restore_general_page(modal, locked, colors, cx).into_any_element(),
            RestoreTab::Objects => {
                restore_objects_page(modal, runtime, locked, colors, cx).into_any_element()
            }
            RestoreTab::Advanced => {
                restore_advanced_page(modal, runtime, locked, colors, cx).into_any_element()
            }
            RestoreTab::Log => restore_log_page(runtime, colors).into_any_element(),
        })
}

/// 表单输入框：34px 外框 + 无边框内嵌 Input（ui-style 规范，与备份弹框一致）。
fn restore_input_frame(input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .h(px(34.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(px(13.)),
        )
}

fn restore_log_list(logs: &[String], colors: UiColors) -> Div {
    let mut list = div().flex().flex_col().gap_1().p_3();
    if logs.is_empty() {
        list = list.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(tr("app.database_restore.no_messages")),
        );
    }
    for line in logs {
        list = list.child(
            div()
                .w_full()
                .min_w(px(0.))
                .text_size(px(12.))
                .text_color(colors.muted)
                .whitespace_normal()
                .child(line.clone()),
        );
    }
    list
}

fn restore_modal_footer(
    runtime: &RestoreRuntime,
    busy: bool,
    has_plan: bool,
    pending: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 底栏左侧状态（设计文档 §14.1 D 区）：任务状态 / 计划摘要 / 待选择策略提示 / 草稿提示。
    let status = if busy {
        tr("app.database_restore.processing").to_string()
    } else if let Some((_, plan)) = &runtime.prepared {
        plan.summary.clone()
    } else if pending > 0 {
        tr("app.database_restore.pending_objects")
            .replace("%{count}", &pending.to_string())
    } else {
        tr("app.database_restore.draft_status").to_string()
    };
    div()
        .h(px(54.))
        .flex_none()
        .px_5()
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.))
                .text_color(colors.muted)
                .truncate()
                .child(status),
        )
        .child(
            Button::new("restore-cancel-task")
                .label(if busy {
                    tr("app.database_restore.cancel_task")
                } else {
                    tr("app.common.close")
                })
                .small()
                .w(px(92.))
                .on_click(cx.listener(|this, _, _, cx| {
                    // 运行中：仅置取消标记，后台任务自行收尾；空闲：直接关闭。
                    if let Some(modal) = &this.restore_modal {
                        if modal.runtime.borrow().running {
                            modal
                                .runtime
                                .borrow()
                                .cancel
                                .store(true, Ordering::Relaxed);
                            cx.stop_propagation();
                            return;
                        }
                    }
                    this.close_restore_dialog(cx);
                    cx.stop_propagation();
                })),
        )
        .child(
            Button::new("restore-edit")
                .label(tr("app.database_restore.change_target"))
                .small()
                .w(px(92.))
                .disabled(busy || !has_plan)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(modal) = &this.restore_modal {
                        modal.runtime.borrow_mut().prepared = None;
                        cx.refresh_windows();
                    }
                    cx.stop_propagation();
                })),
        )
        .child(
            Button::new("restore-check")
                .label(if busy {
                    tr("app.database_restore.processing")
                } else {
                    tr("app.database_restore.preflight")
                })
                .small()
                .w(px(92.))
                .disabled(busy || has_plan || pending > 0)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.request_restore_preflight(cx);
                    cx.stop_propagation();
                })),
        )
        .child(
            Button::new("restore-run")
                .label(tr("app.database_restore.confirm_restore"))
                .primary()
                .small()
                .w(px(92.))
                .disabled(busy || !has_plan || pending > 0)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.confirm_or_run_restore(window, cx);
                    cx.stop_propagation();
                })),
        )
}

/// 破坏性动作二次确认层（设计文档 §6.4/§14）：覆盖在恢复面板之上，
/// 列出将删除目标数据/定义的对象，要求原样输入目标名后才能执行。
fn restore_confirm_overlay(
    modal: &RestoreModal,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let target = modal.confirm_target.clone();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.58)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(430.))
                .max_w(px(430.))
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
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_size(px(13.))
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // 头部：危险图标 + 标题。
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon_box(AppIcon::CircleSlash, 22., 15., rgb(0xe5484d)))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_size(px(15.))
                                .child(tr("app.database_restore.destructive_confirm")),
                        ),
                )
                // 主体：说明 + 破坏性对象清单 + 原样输入目标名。
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(colors.muted)
                                .child(tr("app.database_restore.destructive_objects")),
                        )
                        .child(
                            div()
                                .max_h(px(150.))
                                .overflow_y_scrollbar()
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.input_bg)
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .children(modal.confirm_destructive.iter().map(|item| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(app_icon(AppIcon::Trash, 13., rgb(0xe5484d)))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(colors.text)
                                                .child(item.clone()),
                                        )
                                })),
                        )
                        .child(div().text_size(px(12.)).child(
                            tr("app.database_restore.enter_target")
                                .replace("%{target}", &target),
                        ))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .child(restore_input_frame(modal.confirm_input.clone(), colors)),
                        ),
                )
                .child(div().h(px(1.)).flex_none().bg(colors.border))
                // 底栏：取消 / 确认执行（危险样式）。
                .child(
                    div()
                        .h(px(54.))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("restore-confirm-cancel")
                                .label(tr("app.common.cancel"))
                                .small()
                                .w(px(88.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_restore_confirm(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("restore-confirm-run")
                                .label(tr("app.database_restore.confirm_execute"))
                                .danger()
                                .small()
                                .w(px(100.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run_restore_confirmed(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

impl NavicatMain {
    /// 「选择」备份文件按钮：原生打开设置里配置的备份目录，选中的文件回填输入框。
    /// macOS 下 NSOpenPanel 可指定起始目录；其他平台回退 gpui 选择框（记住上次位置）。
    fn choose_restore_backup_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = &self.restore_modal else {
            return;
        };
        let input = modal.source_input.clone();
        let source_value = modal.source_input.read(cx).value().to_string();
        let backup_dir = self.controller.state().settings.backup_dir.trim().to_string();
        // 起始目录优先级：设置的备份目录 > 当前已填文件所在目录。
        let start_dir = if !backup_dir.is_empty() {
            backup_dir
        } else {
            Path::new(&source_value)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        #[cfg(target_os = "macos")]
        {
            self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
                let selected = native_open_backup_panel(&start_dir);
                let _ = cx.update(|window, cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |_, cx| {
                        if let Some(path) = selected {
                            input.update(cx, |input, cx| {
                                input.set_value(path.display().to_string(), window, cx);
                            });
                            cx.notify();
                        }
                    });
                });
            }));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some(tr("app.database_restore.choose_backup")),
            });
            self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
                let selected = match receiver.await {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    _ => None,
                };
                let _ = cx.update(|window, cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |_, cx| {
                        if let Some(path) = selected {
                            input.update(cx, |input, cx| {
                                input.set_value(path.display().to_string(), window, cx);
                            });
                            cx.notify();
                        }
                    });
                });
            }));
        }
    }

    /// 从挂载的弹框实体读取表单值,构造 RestoreRequest 并启动预检查。
    fn request_restore_preflight(&mut self, cx: &mut Context<Self>) {
        // 有已存在对象未选策略：不生成计划（按钮已禁用，此处兜底防止陈旧计划）。
        if self.restore_pending_count(cx) > 0 {
            return;
        }
        let Some(modal) = &self.restore_modal else {
            return;
        };
        if modal.runtime.borrow().running {
            return;
        }
        let source = PathBuf::from(modal.source_input.read(cx).value().to_string());
        let kind = modal.kind;
        let mode = modal
            .mode
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(RestoreTargetMode::NewDatabase);
        // 非 Sqlite 且选「现有库」→ 目标是目标连接下的已存在库（下拉选择，不建库）。
        let existing_mode =
            kind != DatabaseKind::Sqlite && mode == RestoreTargetMode::ExistingDatabase;
        let target = if existing_mode {
            modal
                .target_db
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
        } else {
            modal.target_input.read(cx).value().to_string()
        };
        // 按选中行索引取候选连接（不用名称匹配：同名连接会错配）。
        let Some(row) = modal.connection.read(cx).selected_index(cx).map(|p| p.row) else {
            return;
        };
        let Some(config) = modal.candidates.get(row).cloned() else {
            return;
        };
        // Sqlite 走「新文件」，恒建新目标；非 Sqlite 仅「现有库」不建。
        let create_target = !existing_mode;
        let manifest = modal
            .meta
            .as_ref()
            .filter(|m| Path::new(&m.output_path) == source)
            .and_then(|m| m.manifest.clone());
        // 高级页选项：按选中行索引映射回枚举（越界回退默认）。
        let transaction = match modal.transaction.read(cx).selected_index(cx).map(|p| p.row) {
            Some(1) => fluxdb_app::RestoreTransactionMode::SingleTransaction,
            _ => fluxdb_app::RestoreTransactionMode::EngineDefault,
        };
        let validation = match modal.validation.read(cx).selected_index(cx).map(|p| p.row) {
            Some(1) => fluxdb_app::RestoreValidation::RowCount,
            _ => fluxdb_app::RestoreValidation::Basic,
        };
        let request = fluxdb_app::RestoreRequest {
            config,
            source,
            target,
            create_target,
            tool: PathBuf::new(),
            manifest,
            table_decisions: self.restore_table_choices(cx),
            options: fluxdb_app::RestoreOptions {
                transaction,
                validation,
            },
        };
        let runtime = modal.runtime.clone();
        self.start_restore_preflight(request, runtime, cx);
    }
}

/// 依据探测事实（存在性 + 备份内容）给出某行的合法动作候选与默认选择索引（设计文档 §6.4）。
/// - 目标不存在：有结构默认「新建表」；无结构无处建表，只能「不处理」。
/// - 目标已存在：默认「待选择策略」占位强制显式选择；重建需结构，清空/追加需数据；恒含「不处理」。
fn restore_row_action_plan(
    exists: bool,
    has_ddl: bool,
    has_data: bool,
) -> (Vec<RestoreActionOption>, usize) {
    use fluxdb_core::RestoreTableAction as Action;
    let create = || restore_action_option(RestoreActionChoice::Action(Action::Create));
    let recreate = || restore_action_option(RestoreActionChoice::Action(Action::Recreate));
    let truncate = || restore_action_option(RestoreActionChoice::Action(Action::TruncateAndLoad));
    let append = || restore_action_option(RestoreActionChoice::Action(Action::Append));
    let skip = || restore_action_option(RestoreActionChoice::Action(Action::Skip));
    let pending = || restore_action_option(RestoreActionChoice::Pending);
    if !exists {
        if has_ddl {
            (vec![create(), skip()], 0)
        } else {
            (vec![skip()], 0)
        }
    } else {
        let mut real: Vec<RestoreActionOption> = Vec::new();
        if has_ddl {
            real.push(recreate());
        }
        if has_data {
            real.push(truncate());
            real.push(append());
        }
        if real.is_empty() {
            (vec![skip()], 0)
        } else {
            let mut options = vec![pending()];
            options.extend(real);
            options.push(skip());
            (options, 0)
        }
    }
}

/// 恢复弹框当前选中的「目标连接」id（按选中行索引取候选，避免同名连接错配）。
fn restore_selected_connection_id(
    modal: &RestoreModal,
    cx: &Context<NavicatMain>,
) -> Option<ConnectionId> {
    let row = modal.connection.read(cx).selected_index(cx)?.row;
    modal.candidates.get(row).map(|c| c.id)
}

#[cfg(target_os = "macos")]
/// 原生打开文件选择框（NSOpenPanel），起始目录设为指定目录，返回所选文件路径。
/// gpui 的 prompt_for_paths 无法指定起始目录，这里直接走 AppKit 以满足「打开到设置备份目录」。
fn native_open_backup_panel(start_dir: &str) -> Option<std::path::PathBuf> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    // 原生面板必须跑在主线程；非主线程调用（例如后台触发的恢复流程）直接放弃。
    let mtm = MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(false);
    if !start_dir.is_empty() {
        let dir = NSString::from_str(start_dir);
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&dir)));
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URL()?;
    let path = url.path()?;
    Some(std::path::PathBuf::from(path.to_string()))
}
