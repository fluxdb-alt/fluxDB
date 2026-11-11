#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateUiStatus {
    Idle,
    Checking,
    Available,
    UpToDate,
    Downloading,
    ReadyToInstall,
    Installing,
    Error,
}

#[derive(Clone, Debug)]
struct UpdateUiState {
    open: bool,
    status: UpdateUiStatus,
    update: Option<fluxdb_app::AvailableUpdate>,
    error: Option<fluxdb_app::UpdateError>,
    downloaded_path: Option<PathBuf>,
    downloaded_bytes: u64,
    total_bytes: u64,
    locale: String,
}

impl Default for UpdateUiState {
    fn default() -> Self {
        Self {
            open: false,
            status: UpdateUiStatus::Idle,
            update: None,
            error: None,
            downloaded_path: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            locale: "en".to_string(),
        }
    }
}

impl NavicatMain {
    fn start_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if matches!(
            self.update_ui.status,
            UpdateUiStatus::Checking
                | UpdateUiStatus::Downloading
                | UpdateUiStatus::Installing
        ) {
            return;
        }
        self.update_ui.status = UpdateUiStatus::Checking;
        self.update_ui.error = None;
        self.update_ui.downloaded_path = None;
        self.update_ui.downloaded_bytes = 0;
        self.update_ui.total_bytes = 0;
        self.update_ui.locale =
            fluxdb_i18n::effective_locale(&self.controller.state().settings.locale);
        if manual {
            self.update_ui.open = true;
        }
        cx.notify();

        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    fluxdb_app::check_for_updates(env!("CARGO_PKG_VERSION"))
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    match result {
                        Ok(fluxdb_app::UpdateCheck::Available(update)) => {
                            this.update_ui.status = UpdateUiStatus::Available;
                            this.update_ui.update = Some(update);
                            this.update_ui.open = true;
                        }
                        Ok(fluxdb_app::UpdateCheck::UpToDate { .. }) => {
                            if manual {
                                this.update_ui.status = UpdateUiStatus::UpToDate;
                                this.update_ui.update = None;
                                this.update_ui.open = true;
                            } else {
                                this.update_ui.status = UpdateUiStatus::Idle;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: "fluxdb_desktop",
                                kind = ?error.kind,
                                detail = %error.detail,
                                "检查应用更新失败"
                            );
                            if manual {
                                this.update_ui.status = UpdateUiStatus::Error;
                                this.update_ui.error = Some(error);
                                this.update_ui.open = true;
                            } else {
                                this.update_ui.status = UpdateUiStatus::Idle;
                            }
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._update_check_task = Some(task);
    }

    fn start_update_download(&mut self, cx: &mut Context<Self>) {
        if self.update_ui.status != UpdateUiStatus::Available {
            return;
        }
        let Some(update) = self.update_ui.update.clone() else {
            return;
        };
        self.update_ui.status = UpdateUiStatus::Downloading;
        self.update_ui.error = None;
        self.update_ui.downloaded_path = None;
        self.update_ui.downloaded_bytes = 0;
        self.update_ui.total_bytes = update.asset.size;
        // 每次下载使用独立取消令牌：旧任务（可能仍阻塞在网络读取里）不会被新任务重置。
        self.update_cancel = Arc::new(AtomicBool::new(false));
        self.update_downloaded_bytes.store(0, Ordering::Relaxed);
        self.update_total_bytes
            .store(update.asset.size, Ordering::Relaxed);
        cx.notify();

        let cache_dir = self.storage.root_dir().join("updates");
        let cancel = self.update_cancel.clone();
        let downloaded = self.update_downloaded_bytes.clone();
        let total = self.update_total_bytes.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut progress = |value: fluxdb_app::UpdateDownloadProgress| {
                        downloaded.store(value.downloaded, Ordering::Relaxed);
                        total.store(value.total, Ordering::Relaxed);
                    };
                    fluxdb_app::download_update(
                        &update.asset,
                        &cache_dir,
                        cancel.as_ref(),
                        &mut progress,
                    )
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    match result {
                        Ok(path) => {
                            this.update_ui.status = UpdateUiStatus::ReadyToInstall;
                            this.update_ui.downloaded_path = Some(path);
                            this.update_ui.downloaded_bytes = this.update_ui.total_bytes;
                        }
                        Err(error) if error.kind == fluxdb_app::UpdateErrorKind::Cancelled => {
                            // 用户点击取消时已立即关闭弹框并回到可下载状态，这里不重复提示。
                            if this.update_ui.status == UpdateUiStatus::Downloading {
                                this.update_ui.status = UpdateUiStatus::Available;
                                this.update_ui.downloaded_bytes = 0;
                                this.show_message(
                                    tr("app.settings.update.cancelled"),
                                    AppMessageKind::Info,
                                    cx,
                                );
                            }
                        }
                        Err(error) => {
                            this.update_ui.status = UpdateUiStatus::Error;
                            this.update_ui.error = Some(error);
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._update_download_task = Some(task);

        let downloaded = self.update_downloaded_bytes.clone();
        let total = self.update_total_bytes.clone();
        self._update_progress_task = Some(cx.spawn(async move |view, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let mut keep_running = false;
            let _ = view.update(cx, |this, cx| {
                if this.update_ui.status != UpdateUiStatus::Downloading {
                    return;
                }
                this.update_ui.downloaded_bytes = downloaded.load(Ordering::Relaxed);
                this.update_ui.total_bytes = total.load(Ordering::Relaxed);
                keep_running = true;
                cx.notify();
            });
            if !keep_running {
                break;
            }
        }));
    }

    fn cancel_update_download(&mut self, cx: &mut Context<Self>) {
        if self.update_ui.status != UpdateUiStatus::Downloading {
            return;
        }
        // 后台读取可能阻塞到下一次网络返回，取消不能等它：立即关闭弹框并回到可下载状态，
        // 下载线程随后观察到取消标志时自行清理临时文件。
        self.update_cancel.store(true, Ordering::Relaxed);
        self.update_ui.status = UpdateUiStatus::Available;
        self.update_ui.downloaded_bytes = 0;
        self.update_ui.open = false;
        cx.notify();
    }

    fn start_update_install(&mut self, cx: &mut Context<Self>) {
        if self.update_ui.status != UpdateUiStatus::ReadyToInstall {
            return;
        }
        if self
            .controller
            .state()
            .tabs
            .iter()
            .any(|tab| tab.dirty)
        {
            self.show_message(
                tr("app.settings.update.save_before_install"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let Some(update) = self.update_ui.update.clone() else {
            return;
        };
        let Some(path) = self.update_ui.downloaded_path.clone() else {
            return;
        };
        let cache_dir = self.storage.root_dir().join("updates");
        self.update_ui.status = UpdateUiStatus::Installing;
        self.update_ui.error = None;
        cx.notify();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    fluxdb_app::install_update(&update.asset, &path, &cache_dir)
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                match result {
                    Ok(()) => {
                        tracing::info!(
                            target: "fluxdb_desktop",
                            version = %update.version,
                            "更新安装器已启动，应用即将退出"
                        );
                        cx.quit();
                    }
                    Err(error) => {
                        view.update(cx, |this, cx| {
                            this.update_ui.status = UpdateUiStatus::Error;
                            this.update_ui.error = Some(error);
                            cx.notify();
                        });
                    }
                }
            });
        });
        self._update_install_task = Some(task);
    }

    fn close_update_dialog(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_ui.status,
            UpdateUiStatus::Downloading | UpdateUiStatus::Installing
        ) {
            return;
        }
        self.update_ui.open = false;
        cx.notify();
    }

    fn update_status_label(&self) -> SharedString {
        match self.update_ui.status {
            UpdateUiStatus::Idle => tr("app.settings.update.status.idle"),
            UpdateUiStatus::Checking => tr("app.settings.update.status.checking"),
            UpdateUiStatus::Available => tr("app.settings.update.status.available"),
            UpdateUiStatus::UpToDate => tr("app.settings.update.status.up_to_date"),
            UpdateUiStatus::Downloading => tr("app.settings.update.status.downloading"),
            UpdateUiStatus::ReadyToInstall => tr("app.settings.update.status.ready"),
            UpdateUiStatus::Installing => tr("app.settings.update.status.installing"),
            UpdateUiStatus::Error => tr("app.settings.update.status.error"),
        }
    }

    fn update_error_message(error: &fluxdb_app::UpdateError) -> SharedString {
        let key = match error.kind {
            fluxdb_app::UpdateErrorKind::Disabled => "app.settings.update.error.disabled",
            fluxdb_app::UpdateErrorKind::Network => "app.settings.update.error.network",
            fluxdb_app::UpdateErrorKind::InvalidManifest => {
                "app.settings.update.error.invalid_manifest"
            }
            fluxdb_app::UpdateErrorKind::InvalidSignature => {
                "app.settings.update.error.invalid_signature"
            }
            fluxdb_app::UpdateErrorKind::UnsupportedPlatform => {
                "app.settings.update.error.unsupported_platform"
            }
            fluxdb_app::UpdateErrorKind::Download => "app.settings.update.error.download",
            fluxdb_app::UpdateErrorKind::Checksum => "app.settings.update.error.checksum",
            fluxdb_app::UpdateErrorKind::Cancelled => "app.settings.update.cancelled",
            fluxdb_app::UpdateErrorKind::Install => "app.settings.update.error.install",
        };
        tr(key)
    }

    fn update_notes_for_dialog(&self) -> Vec<(String, String)> {
        let Some(update) = &self.update_ui.update else {
            return Vec::new();
        };
        update
            .releases
            .iter()
            .filter_map(|release| {
                fluxdb_app::localized_note(&release.notes, &self.update_ui.locale)
                    .map(|notes| (release.version.clone(), notes.to_string()))
            })
            .collect()
    }

    fn update_dialog(&self, colors: UiColors, cx: &mut Context<Self>) -> Div {
        let status = self.update_ui.status;
        let notes = self.update_notes_for_dialog();
        let version = self
            .update_ui
            .update
            .as_ref()
            .map(|update| update.version.clone())
            .unwrap_or_default();
        let current_version = self
            .update_ui
            .update
            .as_ref()
            .map(|update| update.current_version.clone())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        let progress = if self.update_ui.total_bytes == 0 {
            0.
        } else {
            (self.update_ui.downloaded_bytes as f32 / self.update_ui.total_bytes as f32 * 100.)
                .clamp(0., 100.)
        };
        let progress_percent = progress.round() as u32;
        let notes_empty = notes.is_empty();
        let mut notes_view = div()
            .debug_selector(|| "update-dialog-notes".into())
            .flex()
            .flex_col()
            .gap_2();
        for (release_version, note) in notes {
            notes_view = notes_view
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(format!("v{release_version}")),
                )
                .child(
                    TextView::markdown(
                        SharedString::from(format!("update-notes-{release_version}")),
                        note,
                    )
                    .selectable(true)
                    .text_size(ui_text_size(12.))
                    .line_height(ui_text_size(18.))
                    .text_color(colors.text),
                );
        }
        if notes_empty {
            notes_view = notes_view.child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(colors.muted)
                    .child(tr("app.settings.update.no_notes")),
            );
        }

        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(if version.is_empty() {
                        format!("{}: v{current_version}", tr("app.settings.update.current_version"))
                    } else {
                        format!(
                            "{}: v{current_version}  →  v{version}",
                            tr("app.settings.update.current_version")
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(status != UpdateUiStatus::Downloading, |this| {
                        this.child(
                            Button::new("update-dialog-later")
                                .label(tr("app.settings.update.later"))
                                .small()
                                .debug_selector(|| "update-dialog-later".into())
                                .disabled(status == UpdateUiStatus::Installing)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_update_dialog(cx);
                                })),
                        )
                    })
                    .when(status == UpdateUiStatus::Available, |this| {
                        this.child(
                            Button::new("update-dialog-download")
                                .label(tr("app.settings.update.download"))
                                .small()
                                .primary()
                                .debug_selector(|| "update-dialog-download".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_update_download(cx);
                                })),
                        )
                    })
                    .when(status == UpdateUiStatus::Downloading, |this| {
                        this.child(
                            Button::new("update-dialog-cancel")
                                .label(tr("app.common.cancel"))
                                .small()
                                .debug_selector(|| "update-dialog-cancel".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_update_download(cx);
                                })),
                        )
                    })
                    .when(status == UpdateUiStatus::ReadyToInstall, |this| {
                        this.child(
                            Button::new("update-dialog-install")
                                .label(tr("app.settings.update.install_restart"))
                                .small()
                                .primary()
                                .debug_selector(|| "update-dialog-install".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_update_install(cx);
                                })),
                        )
                    })
                    .when(status == UpdateUiStatus::UpToDate, |this| {
                        this.child(
                            Button::new("update-dialog-close")
                                .label(tr("app.common.close"))
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_update_dialog(cx);
                                })),
                        )
                    })
                    .when(
                        matches!(status, UpdateUiStatus::Error | UpdateUiStatus::Installing),
                        |this| {
                            this.child(
                                Button::new("update-dialog-close")
                                    .label(tr("app.common.close"))
                                    .small()
                                    .disabled(status == UpdateUiStatus::Installing)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_update_dialog(cx);
                                    })),
                            )
                        },
                    ),
            );

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .debug_selector(|| "update-dialog".into())
            .bg(if colors.is_dark {
                opaque_grey(0.08, 0.62)
            } else {
                opaque_grey(0.6, 0.36)
            })
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.close_update_dialog(cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .w(px(720.))
                    .max_h(px(680.))
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
                    .flex()
                    .flex_col()
                    .text_size(ui_text_size(13.))
                    .text_color(colors.text)
                    .key_context("UpdateDialog")
                    .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                        this.close_update_dialog(cx);
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(52.))
                            .flex_none()
                            .px_4()
                            .border_b_1()
                            .border_color(colors.border_soft)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(tr("app.settings.update.title")),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(colors.muted)
                                    .child(self.update_status_label()),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            // 弹窗高度由内容决定；正文不能再被 flex 收缩到 0，否则说明和进度都不可见。
                            .min_h(px(220.))
                            .overflow_y_scrollbar()
                            .p_4()
                            .child(match status {
                                UpdateUiStatus::Checking => div()
                                    .h(px(120.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(colors.muted)
                                    .child(tr("app.settings.update.checking")),
                                UpdateUiStatus::UpToDate => div()
                                    .h(px(100.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(colors.muted)
                                    .child(tr("app.settings.update.up_to_date")),
                                UpdateUiStatus::Error => div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .when_some(self.update_ui.error.as_ref(), |this, error| {
                                        this.child(
                                            div()
                                                .text_size(ui_text_size(13.))
                                                .text_color(colors.danger)
                                                .child(Self::update_error_message(error)),
                                        )
                                        .child(
                                            div()
                                                .text_size(ui_text_size(11.))
                                                .text_color(colors.muted)
                                                .child(error.detail.clone()),
                                        )
                                    })
                                    .child(
                                        Button::new("update-dialog-retry")
                                            .label(tr("app.settings.update.retry"))
                                            .small()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if this.update_ui.downloaded_path.is_some() {
                                                    this.start_update_install(cx);
                                                } else {
                                                    this.start_update_check(true, cx);
                                                }
                                            })),
                                    ),
                                UpdateUiStatus::Downloading => div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .debug_selector(|| "update-download-progress".into())
                                            .child(
                                                Progress::new("update-download-progress")
                                                    .value(progress)
                                                    .color(colors.info)
                                                    .accessibility_label(
                                                        tr("app.settings.update.downloading_percent")
                                                            .replace(
                                                                "%{percent}",
                                                                &progress_percent.to_string(),
                                                            ),
                                                    )
                                                    .w_full(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .debug_selector(|| "update-download-status".into())
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_3()
                                            .text_size(ui_text_size(11.))
                                            .text_color(colors.muted)
                                            .child(tr("app.settings.update.downloading_percent")
                                                .replace("%{percent}", &progress_percent.to_string()))
                                            .child(format!(
                                                "{} / {}",
                                                format_bytes(self.update_ui.downloaded_bytes),
                                                format_bytes(self.update_ui.total_bytes)
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(ui_text_size(11.))
                                            .text_color(colors.muted)
                                            .child(tr("app.settings.update.downloading_detail")),
                                    ),
                                UpdateUiStatus::ReadyToInstall
                                | UpdateUiStatus::Installing => div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_size(ui_text_size(13.))
                                            .child(if status == UpdateUiStatus::Installing {
                                                tr("app.settings.update.installing_detail")
                                            } else {
                                                tr("app.settings.update.ready_detail")
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(ui_text_size(11.))
                                            .text_color(colors.muted)
                                            .child(tr("app.settings.update.unsigned_notice")),
                                    ),
                                UpdateUiStatus::Available | UpdateUiStatus::Idle => notes_view,
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(colors.border_soft)
                            .child(footer),
                    ),
            )
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.;
    const MIB: f64 = KIB * 1024.;
    if bytes >= MIB as u64 {
        format!("{:.1} MiB", bytes as f64 / MIB)
    } else if bytes >= KIB as u64 {
        format!("{:.1} KiB", bytes as f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}
