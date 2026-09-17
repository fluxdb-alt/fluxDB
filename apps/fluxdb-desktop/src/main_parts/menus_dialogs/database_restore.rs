// 恢复数据库弹框：与「数据库备份」同款自绘模态（遮罩 + 面板 + 固定底栏）。
// 渲染与按钮交互在 database_restore_modal 系列函数；预检查/执行逻辑在下方 NavicatMain impl。

/// 恢复弹框运行时共享状态：渲染（self.restore_modal）与后台异步任务（Rc 克隆）两侧共享。
#[derive(Clone)]
struct RestoreRuntime {
    running: bool,
    prepared: Option<(fluxdb_app::RestoreRequest, fluxdb_app::RestorePlan)>,
    logs: Vec<String>,
    cancel: Arc<AtomicBool>,
}

/// 恢复弹框挂载状态：UI 实体 + 打开时的配置快照（渲染期间不读 controller）。
#[derive(Clone)]
struct RestoreModal {
    kind: DatabaseKind,
    /// 与 kind 同类型、可选为目标连接的配置快照。
    candidates: Vec<ConnectionConfig>,
    /// 与 candidates 一一对应的下拉展示文本「名称」。
    labels: Vec<String>,
    /// 从备份 tab 带入的文件元信息，路径匹配时可附带 manifest 免去重复识别。
    meta: Option<BackupFileMeta>,
    connection: Entity<SelectState<SearchableVec<String>>>,
    source_input: Entity<InputState>,
    target_input: Entity<InputState>,
    /// 「现有库」模式下目标库下拉；选项来自目标连接下所有库。
    target_db: Entity<SelectState<SearchableVec<String>>>,
    /// 已为目标连接加载过库列表的连接 id，用于渲染时判断是否需要重载（连接切换）。
    target_db_loaded_for: Option<ConnectionId>,
    mode: Entity<SelectState<SearchableVec<String>>>,
    /// 逐表模式：每个源表一个「覆盖/追加/跳过」下拉。
    table_grid: Vec<TableGridRow>,
    runtime: std::rc::Rc<std::cell::RefCell<RestoreRuntime>>,
}

/// 逐表恢复的一行：源表名 + 该表的动作下拉。
#[derive(Clone)]
struct TableGridRow {
    name: String,
    /// 目标库是否已有同名表（预检后回填，用于提示）。
    exists: bool,
    select: Entity<SelectState<SearchableVec<String>>>,
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
                | DatabaseKind::Postgres
                | DatabaseKind::Sqlite
        ) {
            self.show_message("此数据库尚不支持恢复", AppMessageKind::Warning, cx);
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
            vec!["新文件".to_string()]
        } else {
            vec!["新数据库".into(), "现有库".into()]
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
        self.restore_modal = Some(RestoreModal {
            kind,
            candidates,
            labels,
            meta,
            connection,
            source_input,
            target_input,
            target_db,
            target_db_loaded_for: Some(connection_id),
            mode,
            table_grid: Vec::new(),
            runtime: std::rc::Rc::new(std::cell::RefCell::new(RestoreRuntime {
                running: false,
                prepared: None,
                logs: vec!["先检查备份和目标，确认计划后才能恢复。非空目标禁止覆盖。".into()],
                cancel: Arc::new(AtomicBool::new(false)),
            })),
        });
        // 打开即拉取默认目标连接的库列表。
        self.load_restore_target_databases(
            self.restore_modal.as_ref().unwrap().target_db.clone(),
            window,
            cx,
        );
        // 有备份记录的表清单时预填逐表网格（默认覆盖）。
        self.populate_restore_table_grid(window, cx);
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

    /// 从备份记录里的表清单预填逐表网格（每表默认「覆盖」）。
    /// 备份记录缺失/整库（tables 为空或 None）时不预填，走整库恢复。
    fn populate_restore_table_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = &self.restore_modal else {
            return;
        };
        if modal.kind == DatabaseKind::Sqlite || !modal.table_grid.is_empty() {
            return;
        }
        // 源表清单：备份记录 tables 或 manifest.objects（都需非空才逐表）。
        let mut names: Vec<String> = Vec::new();
        if let Some(meta) = &modal.meta {
            if let Some(tables) = &meta.tables {
                if !tables.is_empty() {
                    names = tables.clone();
                }
            }
            if names.is_empty() {
                if let Some(m) = &meta.manifest {
                    if !m.objects.is_empty() {
                        names = m.objects.clone();
                    }
                }
            }
        }
        if names.is_empty() {
            return;
        }
        let actions = vec!["覆盖(重建)".to_string(), "追加数据".to_string(), "跳过".to_string()];
        let grid = names
            .into_iter()
            .map(|name| {
                let select = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(actions.clone()),
                        Some(IndexPath::default().row(0)),
                        window,
                        cx,
                    )
                });
                TableGridRow { name, exists: false, select }
            })
            .collect();
        if let Some(m) = &mut self.restore_modal {
            m.table_grid = grid;
        }
        cx.notify();
    }

    /// 读取逐表网格的决策，供 preflight 组装 RestoreRequest.table_decisions。
    fn restore_table_choices(&self, cx: &App) -> Vec<fluxdb_core::PerTableDecision> {
        let Some(modal) = &self.restore_modal else {
            return Vec::new();
        };
        if modal.mode.read(cx).selected_value().is_none_or(|v| v != "现有库") {
            return Vec::new();
        }
        modal
            .table_grid
            .iter()
            .filter_map(|row| {
                let act = match row.select.read(cx).selected_value()?.as_str() {
                    "覆盖(重建)" => fluxdb_core::RestoreTableAction::Overwrite,
                    "追加数据" => fluxdb_core::RestoreTableAction::Append,
                    _ => fluxdb_core::RestoreTableAction::Skip,
                };
                Some(fluxdb_core::PerTableDecision {
                    table: row.name.clone(),
                    action: act,
                })
            })
            .collect()
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
            data.logs = vec!["正在检查文件、客户端和目标…".into()];
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
                        data.logs.extend(plan.warnings.clone());
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
                .push("开始恢复；关闭弹框不会取消后台任务。".into());
        }
        let controller = self.controller.clone();
        let storage = self.storage.clone();
        let cancel = state.borrow().cancel.clone();
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
                        let message = match result {
                            Ok(outcome) => format!("恢复完成：{}", outcome.verification),
                            Err(e) => format!("恢复失败：{}", e.message),
                        };
                        data.logs.push(message.clone());
                        if let Err(e) = persisted {
                            data.logs.push(format!("恢复记录保存失败：{e}"));
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
}

/// 恢复弹框入口：遮罩 + 面板 + 头部/表单/日志/底栏，复用备份弹框的视觉规范。
fn database_restore_modal(
    modal: &RestoreModal,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let runtime = modal.runtime.borrow().clone();
    let busy = runtime.running;
    let has_plan = runtime.prepared.is_some();
    let locked = busy || has_plan;
    let panel = restore_modal_panel(colors, cx)
        .child(restore_modal_header(colors, cx))
        .child(restore_modal_body(modal, &runtime, locked, colors, cx))
        .child(restore_modal_footer(busy, has_plan, colors, cx));
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

fn restore_modal_header(colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .h(px(56.))
        .flex_none()
        .px_5()
        .flex()
        .items_center()
        .justify_between()
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
                        .child("恢复数据库"),
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

/// 表单 + 消息日志。行布局与备份常规页一致：120px 标签列 + 弹性内容列。
fn restore_modal_body(
    modal: &RestoreModal,
    runtime: &RestoreRuntime,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let target_label = if modal.kind == DatabaseKind::Sqlite {
        "目标新文件"
    } else {
        "目标数据库名称"
    };
    // 「现有库」模式：目标库走下拉，且显示逐表动作。
    let existing_mode = modal.kind != DatabaseKind::Sqlite
        && modal.mode.read(cx).selected_value().is_some_and(|v| v == "现有库");
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .px_5()
        .py_4()
        .flex()
        .flex_col()
        .gap_3()
        // 目标连接
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    "目标连接",
                    colors,
                )))
                .child(Select::new(&modal.connection).disabled(locked).w_full()),
        )
        // 备份文件完整路径：输入框 + 「选择」按钮（原生打开设置备份目录）
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    "备份文件",
                    colors,
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(restore_input_frame(modal.source_input.clone(), colors))
                        .child(
                            Button::new("restore-pick-backup-file")
                                .label("选择")
                                .small()
                                .rounded(colors.radius)
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose_restore_backup_file(window, cx);
                                })),
                        ),
                ),
        )
        .child(
            div()
                .pl(px(132.))
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("按文件内容识别格式；SQLite 备份恢复为 .db 新文件。"),
        )
        // 目标库名称/文件 + 恢复方式
        // 「现有库」模式 -> 目标库下拉（目标连接下的库）；「新数据库」/Sqlite -> 文本框。
        .child({
            let label = if existing_mode {
                "目标数据库"
            } else {
                target_label
            };
            let content = if existing_mode {
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(Select::new(&modal.target_db).disabled(locked).w_full())
            } else {
                restore_input_frame(modal.target_input.clone(), colors)
            };
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    label,
                    colors,
                )))
                .child(content)
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                    "恢复方式",
                    colors,
                )))
                .child(Select::new(&modal.mode).disabled(locked).w_full()),
        )
        // 逐表模式：目标库已有同名表/追加数据等，列出源表 + 每表动作。
        .when(existing_mode && !modal.table_grid.is_empty(), |this| {
            this.child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(div().w(px(120.)).flex_none().child(sql_file_section_label(
                        "表动作",
                        colors,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .h(px(150.))
                                    .overflow_y_scrollbar()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .children(modal.table_grid.iter().map(|row| {
                                        div().flex().items_center().gap_2().child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .text_size(px(13.))
                                                .child(
                                                    if row.exists {
                                                        format!("{}（目标已有）", row.name)
                                                    } else {
                                                        row.name.clone()
                                                    },
                                                ),
                                        ).child(
                                            div().w(px(150.)).flex_none().child(
                                                Select::new(&row.select).disabled(locked).w_full(),
                                            ),
                                        )
                                    })),
                            ),
                    ),
            )
        })
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("先「预检查」生成恢复计划并确认，才能「确认恢复」；非空目标禁止覆盖。"),
        )
        // 消息日志：与备份任务日志同款边框区域，预检查计划/警告与执行进度都在此追加。
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .gap_2()
                .child(sql_file_section_label("消息日志", colors))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .overflow_y_scrollbar()
                        .child(restore_log_list(&runtime.logs, colors)),
                ),
        )
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
                .child("暂无消息。"),
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
    busy: bool,
    has_plan: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(54.))
        .flex_none()
        .px_5()
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            Button::new("restore-cancel-task")
                .label(if busy { "取消任务" } else { "关闭" })
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
                .label("修改目标")
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
                .label(if busy { "处理中…" } else { "预检查" })
                .small()
                .w(px(92.))
                .disabled(busy || has_plan)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.request_restore_preflight(cx);
                    cx.stop_propagation();
                })),
        )
        .child(
            Button::new("restore-run")
                .label("确认恢复")
                .primary()
                .small()
                .w(px(92.))
                .disabled(busy || !has_plan)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(modal) = &this.restore_modal {
                        let runtime = modal.runtime.clone();
                        this.start_restore_execution(runtime, cx);
                    }
                    cx.stop_propagation();
                })),
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
                prompt: Some("选择备份文件".into()),
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
        let Some(modal) = &self.restore_modal else {
            return;
        };
        if modal.runtime.borrow().running {
            return;
        }
        let source = PathBuf::from(modal.source_input.read(cx).value().to_string());
        let kind = modal.kind;
        let mode_value = modal
            .mode
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        // 非 Sqlite 且选「现有库」→ 目标是目标连接下的已存在库（下拉选择，不建库）。
        let existing_mode = kind != DatabaseKind::Sqlite && mode_value == "现有库";
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
        let label = modal
            .connection
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let Some(index) = modal.labels.iter().position(|l| l == &label) else {
            return;
        };
        // Sqlite 走「新文件」，恒建新目标；非 Sqlite 仅「现有库」不建。
        let create_target = !existing_mode;
        let manifest = modal
            .meta
            .as_ref()
            .filter(|m| Path::new(&m.output_path) == source)
            .and_then(|m| m.manifest.clone());
        let request = fluxdb_app::RestoreRequest {
            config: modal.candidates[index].clone(),
            source,
            target,
            create_target,
            tool: PathBuf::new(),
            manifest,
            table_decisions: self.restore_table_choices(cx),
        };
        let runtime = modal.runtime.clone();
        self.start_restore_preflight(request, runtime, cx);
    }
}

/// 恢复弹框当前选中的「目标连接」id（按标签匹配候选配置）。
fn restore_selected_connection_id(
    modal: &RestoreModal,
    cx: &Context<NavicatMain>,
) -> Option<ConnectionId> {
    let label = modal.connection.read(cx).selected_value()?.clone();
    modal
        .labels
        .iter()
        .position(|l| l == &label)
        .map(|i| modal.candidates[i].id)
}

#[cfg(target_os = "macos")]
/// 原生打开文件选择框（NSOpenPanel），起始目录设为指定目录，返回所选文件路径。
/// gpui 的 prompt_for_paths 无法指定起始目录，这里直接走 AppKit 以满足「打开到设置备份目录」。
fn native_open_backup_panel(start_dir: &str) -> Option<std::path::PathBuf> {
    use std::ffi::CStr;

    use cocoa::{
        appkit::{NSModalResponse, NSOpenPanel, NSSavePanel},
        base::{id, nil, NO, YES},
        foundation::{NSURL, NSString},
    };

    unsafe {
        let panel: id = NSOpenPanel::openPanel(nil);
        panel.setCanChooseFiles_(YES);
        panel.setCanChooseDirectories_(NO);
        panel.setAllowsMultipleSelection_(NO);
        panel.setResolvesAliases_(NO);
        if !start_dir.is_empty() {
            let dir = NSString::alloc(nil).init_str(start_dir);
            let url: id = NSURL::fileURLWithPath_(NSURL::alloc(nil), dir);
            panel.setDirectoryURL(url);
        }
        if panel.runModal() != NSModalResponse::NSModalResponseOk {
            return None;
        }
        let url: id = panel.URL();
        if url == nil {
            return None;
        }
        let path: id = url.path(); // NSString
        let cstr = CStr::from_ptr(path.UTF8String() as *const std::ffi::c_char);
        Some(std::path::PathBuf::from(cstr.to_string_lossy().into_owned()))
    }
}
