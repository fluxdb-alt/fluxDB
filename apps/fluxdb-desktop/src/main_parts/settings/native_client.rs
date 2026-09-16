// 设置中的数据库客户端共用 gpui-component 控件、后台检测和下载状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeClientKind {
    Postgres,
    MySql,
}

impl NativeClientKind {
    fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL 客户端",
            Self::MySql => "MySQL 客户端",
        }
    }
    fn directory_id(self) -> &'static str {
        match self {
            Self::Postgres => "settings-backup-pg-client-dir",
            Self::MySql => "settings-backup-mysql-client-dir",
        }
    }
    fn dir<'a>(self, settings: &'a Settings) -> &'a str {
        match self {
            Self::Postgres => &settings.pg_client_dir,
            Self::MySql => &settings.mysql_client_dir,
        }
    }
    fn source<'a>(self, settings: &'a Settings) -> &'a str {
        match self {
            Self::Postgres => &settings.pg_client_download_source,
            Self::MySql => &settings.mysql_client_download_source,
        }
    }
    fn install_hint(self) -> &'static str {
        match self {
            Self::Postgres => fluxdb_app::pg_client_install_hint(),
            Self::MySql => fluxdb_app::mysql_client_install_hint(),
        }
    }
    fn download_supported(self) -> bool {
        match self {
            Self::Postgres => fluxdb_app::pg_client_download_supported(),
            Self::MySql => fluxdb_app::mysql_client_download_supported(),
        }
    }
    fn button_id(self, action: &'static str) -> &'static str {
        match (self, action) {
            (Self::Postgres, "cancel") => "settings-pg-client-cancel",
            (Self::Postgres, "detect") => "settings-pg-client-detect",
            (Self::Postgres, "download") => "settings-pg-client-download",
            (Self::MySql, "cancel") => "settings-mysql-client-cancel",
            (Self::MySql, "detect") => "settings-mysql-client-detect",
            (Self::MySql, "download") => "settings-mysql-client-download",
            _ => unreachable!("unknown native client action"),
        }
    }
}

struct NativeClientState {
    source_input: Entity<InputState>,
    _source_subscription: Subscription,
    status: Option<String>,
    download: Option<PgClientDownloadState>,
    detect_task: Option<Task<()>>,
    download_task: Option<Task<()>>,
    progress_task: Option<Task<()>>,
}

#[derive(Clone)]
struct NativeClientPanelState {
    kind: NativeClientKind,
    source_input: Entity<InputState>,
    status: Option<String>,
    detecting: bool,
    download: Option<PgClientDownloadState>,
}

impl NativeClientState {
    fn snapshot(&self, kind: NativeClientKind) -> NativeClientPanelState {
        NativeClientPanelState {
            kind,
            source_input: self.source_input.clone(),
            status: self.status.clone(),
            detecting: self.detect_task.is_some(),
            download: self.download.clone(),
        }
    }
}

/// 各客户端组内的「单独指定 dump 工具路径」行:从原「备份」分组迁入,
/// 与对应客户端的目录/下载源放在一处,避免同一个库的配置分居两处。
fn dump_path_row(
    kind: NativeClientKind,
    settings: &Settings,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    match kind {
        NativeClientKind::Postgres => settings_path_row(
            "pg_dump 路径",
            "单独指定 pg_dump 可执行文件（优先级最高）；一般留空，使用上方自动发现",
            AppIcon::Database,
            &settings.pg_dump_path,
            "settings-backup-pg-dump",
            false,
            colors,
            cx,
        ),
        NativeClientKind::MySql => settings_path_row(
            "mysqldump 路径",
            "单独指定 mysqldump 可执行文件（旧配置优先级最高）；一般留空，使用上方自动发现",
            AppIcon::Query,
            &settings.mysqldump_path,
            "settings-backup-mysqldump",
            false,
            colors,
            cx,
        ),
    }
}

fn settings_native_client_group(
    settings: &Settings,
    client: NativeClientPanelState,
    collapsed: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    let kind = client.kind;
    let downloading = client.download.is_some();
    // 折叠时标题右侧的状态摘要:优先级 下载中 > 检测中 > 已检测/未找到 > 未检测。
    // 检测失败时 status 会是安装引导文案(以「未找到」开头),摘要区分开,
    // 避免没找到客户端还显示「已检测」误导。
    let status_hint = if downloading {
        "下载中..."
    } else if client.detecting {
        "检测中..."
    } else if client
        .status
        .as_deref()
        .is_some_and(|s| s.starts_with("未找到"))
    {
        "未找到"
    } else if client.status.is_some() {
        "已检测"
    } else {
        "未检测"
    };
    let group_enum = match kind {
        NativeClientKind::Postgres => SettingsDataGroup::PgClient,
        NativeClientKind::MySql => SettingsDataGroup::MySql,
    };
    let mut group =
        settings_collapsible_group(group_enum, Some(status_hint), collapsed, colors, cx);
    // 折叠时不渲染内部行（含下载源输入框同步），展开时恢复。
    if collapsed {
        return group;
    }
    let status = client
        .download
        .as_ref()
        .map(|download| download.message.clone())
        .or(client.status)
        .unwrap_or_else(|| "点击「检测」查看本机客户端版本与位置".into());
    let mut actions = h_flex().gap_2();
    if let Some(download) = client.download {
        actions = actions.child(
            Button::new(kind.button_id("cancel"))
                .label("取消下载")
                .small()
                .rounded_md()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    download.cancel.store(true, Ordering::Relaxed);
                    this.show_message("正在取消下载…", AppMessageKind::Info, cx);
                })),
        );
    } else {
        actions = actions.child(
            Button::new(kind.button_id("detect"))
                .label("检测")
                .small()
                .rounded_md()
                .cursor_pointer()
                .loading(client.detecting)
                .disabled(client.detecting)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.detect_native_client_tools(kind, cx)),
                ),
        );
        if kind.download_supported() {
            actions = actions.child(
                Button::new(kind.button_id("download"))
                    .label("下载并安装")
                    .small()
                    .rounded_md()
                    .cursor_pointer()
                    .disabled(client.detecting)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.download_native_client_tools(kind, cx)
                    })),
            );
        }
    }
    group = group
        .child(
            settings_action_row(
                "客户端状态",
                "自动发现顺序：客户端目录 → 应用下载目录 → 系统安装路径 → PATH",
                AppIcon::Database,
                colors,
            )
            .child(
                h_flex()
                    .min_w(px(0.))
                    .max_w(px(460.))
                    .gap_2()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .whitespace_normal()
                            .child(status),
                    )
                    .child(actions),
            ),
        )
        .child(dump_path_row(kind, settings, colors, cx))
.child({
            // 未设置客户端目录时,占位直接展示本机默认搜索路径,让用户知道留空会发生什么。
            let placeholder = match kind {
                NativeClientKind::Postgres => fluxdb_app::pg_client_default_dirs_summary(),
                NativeClientKind::MySql => fluxdb_app::mysql_client_default_dirs_summary(),
            };
            settings_path_row_with_placeholder(
                "客户端目录",
                "可选安装根目录或 bin 目录;留空自动发现,下载也会安装到这里",
                AppIcon::Folder,
                kind.dir(settings),
                &placeholder,
                kind.directory_id(),
                true,
                colors,
                cx,
            )
        });
    // 不再在组底部常驻安装引导文案:检测失败时状态行已显示同一句 install_hint,
    // 常驻版本属于重复提示。
    if !kind.download_supported() {
        return group;
    }
    let input = &client.source_input;
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value() != kind.source(settings) {
        input.update(cx, |input, cx| {
            input.set_value(kind.source(settings).to_string(), window, cx)
        });
    }
    let focus_border = if colors.is_dark {
        rgb(0x8ab4ff)
    } else {
        rgb(0x111111)
    };
    group = group.child(
        settings_action_row(
            "下载源",
            "留空使用官方二进制包；支持 {version} 与 {platform}，可改为内网镜像",
            AppIcon::Query,
            colors,
        )
        .child(
            div()
                .w(px(380.))
                .h(px(34.))
                .rounded_md()
                .border_1()
                .border_color(if focused { focus_border } else { colors.border })
                .when(!focused, |this| {
                    this.hover(|this| this.border_color(colors.muted))
                })
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .overflow_hidden()
                .cursor_text()
                .child(
                    Input::new(input)
                        .appearance(false)
                        .focus_bordered(false)
                        .disabled(downloading)
                        .w_full()
                        .h_full()
                        .px_2()
                        .text_size(px(13.))
                        .text_color(colors.text),
                ),
        ),
    );
    group
}

impl NavicatMain {
    fn native_client_state(&mut self, kind: NativeClientKind) -> &mut NativeClientState {
        match kind {
            NativeClientKind::Postgres => &mut self.pg_client,
            NativeClientKind::MySql => &mut self.mysql_client,
        }
    }

    fn detect_native_client_tools(&mut self, kind: NativeClientKind, cx: &mut Context<Self>) {
        let settings = self.settings_editor_draft.clone();
        let state = self.native_client_state(kind);
        if state.detect_task.is_some() || state.download.is_some() {
            return;
        }
        state.status = Some("正在检测客户端…".into());
        state.detect_task = Some(cx.spawn(async move |view, cx| {
            let status = cx
                .background_spawn(async move {
                    match kind {
                        NativeClientKind::Postgres => {
                            let found = fluxdb_app::discover_pg_clients(&settings);
                            let Some(best) = found
                                .iter()
                                .find(|location| location.major_version.is_some())
                            else {
                                return kind.install_hint().to_string();
                            };
                            let missing: Vec<_> = [
                                fluxdb_app::PgClientTool::Restore,
                                fluxdb_app::PgClientTool::Psql,
                            ]
                            .into_iter()
                            .filter(|tool| !best.bin_dir.join(tool.binary_name()).is_file())
                            .map(|tool| tool.display_name())
                            .collect();
                            format!(
                                "主版本 {}：{}{}",
                                best.major_version.unwrap_or_default(),
                                best.bin_dir.display(),
                                if missing.is_empty() {
                                    String::new()
                                } else {
                                    format!("；缺少 {}", missing.join("、"))
                                }
                            )
                        }
                        NativeClientKind::MySql => {
                            let Some(best) = fluxdb_app::resolve_mysql_client_tool(
                                &settings,
                                fluxdb_app::MySqlClientTool::Dump,
                            ) else {
                                return kind.install_hint().to_string();
                            };
                            format!(
                                "{} {}.{}：{}",
                                if best.version.mariadb {
                                    "MariaDB"
                                } else {
                                    "MySQL"
                                },
                                best.version.major,
                                best.version.minor,
                                best.program.display()
                            )
                        }
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |this, cx| {
                        tracing::info!(client = kind.label(), "客户端检测完成");
                        let state = this.native_client_state(kind);
                        state.detect_task = None;
                        state.status = Some(status);
                        cx.notify();
                    });
                }
            });
        }));
        cx.notify();
    }

    fn download_native_client_tools(&mut self, kind: NativeClientKind, cx: &mut Context<Self>) {
        let state = self.native_client_state(kind);
        if state.download.is_some() || state.detect_task.is_some() {
            return;
        }
        let settings = self.settings_editor_draft.clone();
        let (url, install_dir) = match kind {
            NativeClientKind::Postgres => (
                fluxdb_app::pg_client_download_url(kind.source(&settings), None),
                fluxdb_app::pg_client_install_dir(
                    &settings,
                    fluxdb_app::pg_client_version_for_server(None),
                ),
            ),
            NativeClientKind::MySql => (
                fluxdb_app::mysql_client_download_url(kind.source(&settings)),
                fluxdb_app::mysql_client_install_dir(&settings),
            ),
        };
        let Some(url) = url else {
            self.show_message(kind.install_hint(), AppMessageKind::Info, cx);
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.native_client_state(kind).download = Some(PgClientDownloadState {
            message: "准备下载客户端…".into(),
            cancel: cancel.clone(),
        });
        let (sender, receiver) = mpsc::channel::<String>();
        self.native_client_state(kind).download_task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut last_percent = u64::MAX;
                    let mut progress = |progress| {
                        if let Some(text) = pg_client_progress_text(progress, &mut last_percent) {
                            let _ = sender.send(text);
                        }
                    };
                    match kind {
                        NativeClientKind::Postgres => fluxdb_app::download_pg_client(
                            &url,
                            &install_dir,
                            &cancel,
                            &mut progress,
                        ),
                        NativeClientKind::MySql => fluxdb_app::download_mysql_client(
                            &url,
                            &install_dir,
                            &cancel,
                            &mut progress,
                        ),
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |this, cx| {
                        let state = this.native_client_state(kind);
                        state.download = None;
                        state.download_task = None;
                        state.progress_task = None;
                        match result {
                            Ok(bin) => {
                                match kind {
                                    NativeClientKind::Postgres => {
                                        this.settings_editor_draft.pg_client_dir =
                                            bin.display().to_string()
                                    }
                                    NativeClientKind::MySql => {
                                        this.settings_editor_draft.mysql_client_dir =
                                            bin.display().to_string()
                                    }
                                }
                                this.detect_native_client_tools(kind, cx);
                                this.show_message(
                                    "客户端已安装，点击保存后生效",
                                    AppMessageKind::Success,
                                    cx,
                                );
                            }
                            Err(error) => {
                                tracing::warn!(?error, client = kind.label(), "客户端下载失败");
                                this.native_client_state(kind).status =
                                    Some(format!("下载失败：{error}"));
                                this.show_message(
                                    format!("客户端下载失败：{error}"),
                                    AppMessageKind::Error,
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }
            });
        }));
        self.native_client_state(kind).progress_task = Some(cx.spawn(async move |view, cx| {
            loop {
                let latest = receiver.try_iter().last();
                let alive = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return false;
                    };
                    view.update(cx, |this, cx| {
                        let Some(download) = this.native_client_state(kind).download.as_mut()
                        else {
                            return false;
                        };
                        if let Some(text) = latest {
                            download.message = text;
                            cx.notify();
                        }
                        true
                    })
                });
                if !alive {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
            }
        }));
        cx.notify();
    }
}

fn pg_client_progress_text(
    progress: fluxdb_app::PgClientDownloadProgress,
    last_percent: &mut u64,
) -> Option<String> {
    match progress.stage {
        "download" => {
            if progress.total == 0 {
                return Some(format!(
                    "下载中（{:.1} MB）",
                    progress.downloaded as f64 / 1024.0 / 1024.0
                ));
            }
            let percent = progress.downloaded * 100 / progress.total;
            if percent == *last_percent {
                return None;
            }
            *last_percent = percent;
            Some(format!(
                "下载中 {percent}%（{:.1} MB / {:.1} MB）",
                progress.downloaded as f64 / 1024.0 / 1024.0,
                progress.total as f64 / 1024.0 / 1024.0
            ))
        }
        "extract" => Some("正在解压客户端文件…".to_string()),
        _ => Some("正在校验客户端…".to_string()),
    }
}
