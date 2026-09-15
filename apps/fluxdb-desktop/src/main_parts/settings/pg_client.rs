// 设置 → 数据 → PostgreSQL 客户端：展示自动发现结果，并提供「下载并安装 / 指定目录 / 自定义下载源」。
//
// 对齐 DBeaver 的客户端管理体验：不等到点了备份才报错，而是在设置里就能看到检测状态与补齐入口。
// 真正的发现/下载逻辑都在 `fluxdb_app::pg_client_tools`，这里只负责渲染与任务编排。

/// 设置面板渲染 PostgreSQL 客户端分组所需的运行时状态快照。
#[derive(Clone)]
struct PgClientPanelState {
    /// 下载源输入框（空值 = 使用内置官方源）。
    source_input: Entity<InputState>,
    /// 最近一次检测得到的文案；None 表示尚未检测。
    status: Option<String>,
    /// 正在进行的下载任务状态；Some 时按钮切换为进度 + 取消。
    download: Option<PgClientDownloadState>,
}

fn settings_pg_client_group(
    settings: &Settings,
    pg_client: PgClientPanelState,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    let downloading = pg_client.download.is_some();
    let status = pg_client
        .status
        .clone()
        .unwrap_or_else(|| "点击「检测」查看本机可用的 pg_dump / pg_restore / psql".to_string());
    settings_panel_group("PostgreSQL 客户端", colors)
        .child(settings_pg_client_status_row(
            &status,
            pg_client.download.clone(),
            colors,
            cx,
        ))
        .child(settings_path_row(
            "客户端目录",
            "pg_dump / pg_restore / psql 所在目录（可选安装根目录或 bin 目录）；留空时自动发现，下载也会安装到这里",
            AppIcon::Folder,
            &settings.pg_client_dir,
            "settings-backup-pg-client-dir",
            true,
            colors,
            cx,
        ))
        .child(settings_pg_client_source_row(
            settings,
            &pg_client.source_input,
            downloading,
            colors,
            window,
            cx,
        ))
}

/// 状态行：左侧显示检测结论，右侧是「检测」+「下载并安装」（下载中切换为进度与取消）。
fn settings_pg_client_status_row(
    status: &str,
    download: Option<PgClientDownloadState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let downloading = download.is_some();
    let text = download
        .as_ref()
        .map(|download| download.message.clone())
        .unwrap_or_else(|| status.to_string());
    let actions = if let Some(download) = download {
        let cancel = download.cancel.clone();
        h_flex().gap_2().child(
            Button::new("settings-pg-client-cancel")
                .label("取消下载")
                .small()
                .rounded(colors.radius)
                .on_click(cx.listener(move |this, _, _window, cx| {
                    // 只置标志，后台线程在下一个分片处退出并清理临时文件。
                    cancel.store(true, Ordering::Relaxed);
                    this.show_message("正在取消下载…", AppMessageKind::Info, cx);
                    cx.notify();
                })),
        )
    } else {
        h_flex()
            .gap_2()
            .child(
                Button::new("settings-pg-client-detect")
                    .label("检测")
                    .small()
                    .rounded(colors.radius)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.detect_pg_client_tools(cx);
                    })),
            )
            .when(fluxdb_app::pg_client_download_supported(), |this| {
                this.child(
                    Button::new("settings-pg-client-download")
                        .label("下载并安装")
                        .small()
                        .rounded(colors.radius)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.download_pg_client_tools(cx);
                        })),
                )
            })
    };

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
                    .text_color(if downloading { colors.text } else { colors.muted })
                    .truncate()
                    .child(text),
            )
            .child(actions),
    )
}

/// 下载源输入行：留空使用内置 EDB 官方源，企业内网可改成自建镜像。
fn settings_pg_client_source_row(
    settings: &Settings,
    input: &Entity<InputState>,
    downloading: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    // 未获得焦点时与草稿保持一致（例如「恢复默认」之后），避免输入框显示过期值。
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value() != settings.pg_client_download_source.as_str() {
        let value = settings.pg_client_download_source.clone();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
    let focus_border = if colors.is_dark {
        rgb(0x8ab4ff)
    } else {
        rgb(0x111111)
    };

    settings_action_row(
        "下载源",
        "留空使用 PostgreSQL 官方二进制包；支持 {version} 与 {platform} 占位符，可改为内网镜像",
        AppIcon::Query,
        colors,
    )
    .child(
        div()
            .w(px(380.))
            .h(px(34.))
            .rounded(colors.radius)
            .border_1()
            .border_color(if focused { focus_border } else { colors.border })
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
                    .text_size(px(12.))
                    .text_color(colors.text),
            ),
    )
}

impl NavicatMain {
    /// 检测本机可用的 PostgreSQL 客户端，把结论写进设置面板状态行。
    /// 用草稿（未保存的设置）检测，方便用户选完目录立刻验证，不必先保存。
    fn detect_pg_client_tools(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings_editor_draft.clone();
        let locations = fluxdb_app::discover_pg_clients(&settings);
        let Some(best) = locations.first() else {
            self.pg_client_status = Some(fluxdb_app::pg_client_install_hint().to_string());
            tracing::warn!("未发现可用的 PostgreSQL 客户端工具");
            cx.notify();
            return;
        };
        let version = best
            .major_version
            .map(|major| format!("{major}"))
            .unwrap_or_else(|| "未知".to_string());
        let missing: Vec<&str> = [
            fluxdb_app::PgClientTool::Restore,
            fluxdb_app::PgClientTool::Psql,
        ]
        .into_iter()
        .filter(|tool| !best.bin_dir.join(tool.binary_name()).is_file())
        .map(|tool| tool.display_name())
        .collect();
        let mut status = format!(
            "已找到客户端（主版本 {version}，共 {} 处）：{}",
            locations.len(),
            best.bin_dir.display()
        );
        if !missing.is_empty() {
            // 只有 pg_dump 时仍可备份，但恢复/脚本执行会缺工具，提前说明。
            status.push_str(&format!("；该目录缺少 {}", missing.join("、")));
        }
        tracing::info!(bin_dir = %best.bin_dir.display(), "PostgreSQL 客户端检测完成");
        self.pg_client_status = Some(status);
        cx.notify();
    }

    /// 下载并安装官方 PostgreSQL 客户端：装到设置的客户端目录，未设置则装到应用托管目录。
    /// 下载在后台线程进行，进度回流到状态行；完成后把安装目录写进草稿并提示保存。
    fn download_pg_client_tools(&mut self, cx: &mut Context<Self>) {
        if self.pg_client_download.is_some() {
            return;
        }
        let settings = self.settings_editor_draft.clone();
        let Some(url) = fluxdb_app::pg_client_download_url(&settings.pg_client_download_source, None)
        else {
            self.show_message(
                fluxdb_app::pg_client_install_hint(),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        let version = fluxdb_app::pg_client_version_for_server(None);
        let install_dir = fluxdb_app::pg_client_install_dir(&settings, version);
        let cancel = Arc::new(AtomicBool::new(false));
        self.pg_client_download = Some(PgClientDownloadState {
            message: format!("准备下载 PostgreSQL {version} 客户端…"),
            cancel: cancel.clone(),
        });
        cx.notify();

        let (progress_sender, progress_receiver) = mpsc::channel::<String>();
        let cancel_for_task = cancel.clone();
        let install_dir_for_task = install_dir.clone();
        tracing::info!(url, install_dir = %install_dir.display(), "开始下载 PostgreSQL 客户端");
        self._pg_client_download_task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut last_percent = u64::MAX;
                    fluxdb_app::download_pg_client(
                        &url,
                        &install_dir_for_task,
                        &cancel_for_task,
                        &mut |progress| {
                            let text = pg_client_progress_text(progress, &mut last_percent);
                            if let Some(text) = text {
                                let _ = progress_sender.send(text);
                            }
                        },
                    )
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.pg_client_download = None;
                    this._pg_client_download_task = None;
                    match result {
                        Ok(bin_dir) => {
                            // 安装目录写回草稿：后续解析直接命中这里，保存后长期生效。
                            this.settings_editor_draft.pg_client_dir =
                                bin_dir.display().to_string();
                            this.detect_pg_client_tools(cx);
                            this.show_message(
                                "PostgreSQL 客户端已安装，点击保存后生效",
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        Err(error) => {
                            tracing::warn!(?error, "PostgreSQL 客户端下载失败");
                            this.pg_client_status = Some(format!("下载失败：{error}"));
                            this.show_message(
                                format!("PostgreSQL 客户端下载失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            });
        }));

        // 进度回流：后台线程只发文案，UI 侧定时取最新一条刷新状态行，避免每个分片都唤醒渲染。
        self._pg_client_progress_task = Some(cx.spawn(async move |view, cx| {
            loop {
                let latest = progress_receiver.try_iter().last();
                let alive = cx
                    .update(|cx| -> bool {
                        let Some(view) = view.upgrade() else {
                            return false;
                        };
                        view.update(cx, |this, cx| {
                            let Some(download) = this.pg_client_download.as_mut() else {
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
    }
}

/// 把下载进度折算成用户可读文案；下载阶段只在百分比变化时更新，避免刷屏。
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
