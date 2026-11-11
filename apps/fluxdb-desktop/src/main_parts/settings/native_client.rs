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
    fn install_hint(self) -> SharedString {
        let key = match self {
            Self::Postgres => {
                if cfg!(target_os = "linux") {
                    "app.settings.native_client.hint.pg_linux"
                } else if cfg!(target_os = "macos") {
                    "app.settings.native_client.hint.pg_macos"
                } else {
                    "app.settings.native_client.hint.pg_windows"
                }
            }
            Self::MySql => {
                if cfg!(target_os = "linux") {
                    "app.settings.native_client.hint.mysql_linux"
                } else if cfg!(target_os = "macos") {
                    "app.settings.native_client.hint.mysql_macos"
                } else {
                    "app.settings.native_client.hint.mysql_windows"
                }
            }
        };
        tr(key)
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
    status_missing: bool,
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
    status_missing: bool,
    detecting: bool,
    download: Option<PgClientDownloadState>,
}

impl NativeClientState {
    fn snapshot(&self, kind: NativeClientKind) -> NativeClientPanelState {
        NativeClientPanelState {
            kind,
            source_input: self.source_input.clone(),
            status: self.status.clone(),
            status_missing: self.status_missing,
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
            tr("app.settings.native_client.dump_pg"),
            tr("app.settings.native_client.dump_pg_detail"),
            AppIcon::Database,
            &settings.pg_dump_path,
            "settings-backup-pg-dump",
            false,
            colors,
            cx,
        ),
        NativeClientKind::MySql => settings_path_row(
            tr("app.settings.native_client.dump_mysql"),
            tr("app.settings.native_client.dump_mysql_detail"),
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
        tr("app.settings.native_client.status.downloading")
    } else if client.detecting {
        tr("app.settings.native_client.status.detecting")
    } else if client.status_missing {
        tr("app.settings.native_client.status.not_found")
    } else if client.status.is_some() {
        tr("app.settings.native_client.status.detected")
    } else {
        tr("app.settings.native_client.status.not_detected")
    };
    let group_enum = match kind {
        NativeClientKind::Postgres => SettingsDataGroup::PgClient,
        NativeClientKind::MySql => SettingsDataGroup::MySql,
    };
    let mut group =
        settings_collapsible_group(group_enum, Some(&status_hint), collapsed, colors, cx);
    // 折叠时不渲染内部行（含下载源输入框同步），展开时恢复。
    if collapsed {
        return group;
    }
    let status = client
        .download
        .as_ref()
        .map(|download| download.message.clone())
        .or(client.status)
        .unwrap_or_else(|| tr("app.settings.native_client.status_hint").to_string());
    let mut actions = h_flex().gap_2();
    if let Some(download) = client.download {
        actions = actions.child(
            Button::new(kind.button_id("cancel"))
                .label(tr("app.settings.native_client.cancel_download"))
                .small()
                .rounded_md()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    download.cancel.store(true, Ordering::Relaxed);
                    this.show_message(
                        tr("app.settings.native_client.canceling"),
                        AppMessageKind::Info,
                        cx,
                    );
                })),
        );
    } else {
        actions = actions.child(
            Button::new(kind.button_id("detect"))
                .label(tr("app.settings.native_client.detect"))
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
                    .label(tr("app.settings.native_client.download_install"))
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
            settings_action_row_with_id(
                "settings-native-client-status",
                tr("app.settings.native_client.status_title"),
                tr("app.settings.native_client.status_detail"),
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
                            .text_size(ui_text_size(11.))
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
                tr("app.settings.native_client.directory_title"),
                tr("app.settings.native_client.directory_detail"),
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
        settings_action_row_with_id(
            "settings-native-client-source",
            tr("app.settings.native_client.source_title"),
            tr("app.settings.native_client.source_detail"),
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
                        .text_size(ui_text_size(13.))
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
        state.status = Some(tr("app.settings.native_client.status.detecting_text").to_string());
        state.status_missing = false;
        state.detect_task = Some(cx.spawn(async move |view, cx| {
            let (status, status_missing) = cx
                .background_spawn(async move {
                    match kind {
                        NativeClientKind::Postgres => {
                            let found = fluxdb_app::discover_pg_clients(&settings);
                            let Some(best) = found
                                .iter()
                                .find(|location| location.major_version.is_some())
                            else {
                                return (kind.install_hint().to_string(), true);
                            };
                            let missing: Vec<_> = [
                                fluxdb_app::PgClientTool::Restore,
                                fluxdb_app::PgClientTool::Psql,
                            ]
                            .into_iter()
                            .filter(|tool| !best.bin_dir.join(tool.binary_name()).is_file())
                            .map(|tool| tool.display_name())
                            .collect();
                            (tr("app.settings.native_client.status.version")
                                .replace("%{version}", &best.major_version.unwrap_or_default().to_string())
                                .replace("%{path}", &best.bin_dir.display().to_string())
                                .replace(
                                    "%{missing}",
                                    &if missing.is_empty() {
                                        String::new()
                                    } else {
                                        tr("app.settings.native_client.status.missing")
                                            .replace("%{tools}", &missing.join(", "))
                                    },
                                ), false)
                        }
                        NativeClientKind::MySql => {
                            let Some(best) = fluxdb_app::resolve_mysql_client_tool(
                                &settings,
                                fluxdb_app::MySqlClientTool::Dump,
                            ) else {
                                return (kind.install_hint().to_string(), true);
                            };
                            (
                                tr("app.settings.native_client.status.version_plain")
                                    .replace(
                                        "%{name}",
                                        if best.version.mariadb {
                                            "MariaDB"
                                        } else {
                                            "MySQL"
                                        },
                                    )
                                    .replace("%{major}", &best.version.major.to_string())
                                    .replace("%{minor}", &best.version.minor.to_string())
                                    .replace("%{path}", &best.program.display().to_string()),
                                false,
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
                        state.status_missing = status_missing;
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
            message: tr("app.settings.native_client.status.preparing_download").into(),
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
                                    tr("app.settings.native_client.toast.installed").as_ref(),
                                    AppMessageKind::Success,
                                    cx,
                                );
                            }
                            Err(error) => {
                                tracing::warn!(?error, client = kind.label(), "客户端下载失败");
                                this.native_client_state(kind).status =
                                    Some(tr("app.settings.native_client.error.download").replace("%{error}", &error.to_string()));
                                this.show_message(
                                    tr("app.settings.native_client.error.download_client").replace("%{error}", &error.to_string()),
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
                return Some(tr("app.settings.native_client.progress.downloading_mb").replace(
                    "%{mb}",
                    &format!("{:.1}", progress.downloaded as f64 / 1024.0 / 1024.0),
                ));
            }
            let percent = progress.downloaded * 100 / progress.total;
            if percent == *last_percent {
                return None;
            }
            *last_percent = percent;
            Some(tr("app.settings.native_client.progress.downloading_percent")
                .replace("%{percent}", &percent.to_string())
                .replace("%{done}", &format!("{:.1}", progress.downloaded as f64 / 1024.0 / 1024.0))
                .replace("%{total}", &format!("{:.1}", progress.total as f64 / 1024.0 / 1024.0)))
        }
        "extract" => Some(tr("app.settings.native_client.progress.extracting").to_string()),
        _ => Some(tr("app.settings.native_client.progress.verifying").to_string()),
    }
}
