// AI 面板「诊断」只读弹框（设计 §23.2 `fluxdb.lifecycle.telemetry`）。
//
// 与模型读取面同源：展示本会话最近几轮的脱敏失败事实（轮终态、失败阶段与归属方、
// 受控错误码、可重试性、阶段计数、失败工具与用户已可见的说明），供人核验"模型说的
// 原因是否属实"。只读、脱敏、仅限当前会话：不含提示词、消息正文、工具参数与结果、
// 执行目标或任何凭据。
//
// 数据在后台命令 `LoadAiTurnDiagnostics` 里读库，弹框只渲染 `AppState.ai_turn_diagnostics`
// 快照（render 期间不查库、不阻塞 UI）；加载态与错误态分开显示，不把"读取失败"伪装成
// "没有失败"。

/// 「诊断」弹框的共享状态：dialog builder 在 render 期间同步执行，实时数据放
/// `Rc<RefCell<..>>`（与产物报告弹框同一先例）。
#[derive(Default)]
struct AiDiagnosticsModalData {
    dialog_open: bool,
    loading: bool,
    error: Option<String>,
    /// 后台读库得到的诊断行快照（弹框关闭后清空）。
    rows: Vec<fluxdb_app::AiTurnDiagnostic>,
    /// 展开查看阶段/工具明细的行（Turn ordinal）。
    expanded: Option<i64>,
}

impl AiDiagnosticsModalData {
    fn reset(&mut self) {
        self.dialog_open = false;
        self.loading = false;
        self.error = None;
        self.rows.clear();
        self.expanded = None;
    }
}

#[derive(Clone)]
struct AiDiagnosticsDialogViews {
    view: Entity<NavicatMain>,
    data: Rc<std::cell::RefCell<AiDiagnosticsModalData>>,
    colors: UiColors,
}

impl NavicatMain {
    /// 打开「诊断」弹框：后台读取最近几轮诊断事实，成功后渲染快照。
    fn open_ai_diagnostics_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_diagnostics_modal.borrow().dialog_open {
            return;
        }
        let Some(session_id) = self.controller.state().ai_conversation.session_id.clone() else {
            self.show_message(
                tr("app.ai.chat.diagnostics.no_session").as_ref(),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        let Ok(storage) = self.controller.ai_storage() else {
            self.show_message(
                tr("app.ai.chat.diagnostics.storage_missing").as_ref(),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        {
            let mut data = self.ai_diagnostics_modal.borrow_mut();
            data.dialog_open = true;
            data.loading = true;
            data.error = None;
            data.rows.clear();
            data.expanded = None;
        }
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        let views = AiDiagnosticsDialogViews {
            view: cx.entity(),
            data: self.ai_diagnostics_modal.clone(),
            colors,
        };
        // 焦点先落到主视图：Dialog 关闭后会还原到这里。
        self.focus_handle.focus(window, cx);
        window.open_dialog(cx, move |dialog, _window, _cx| {
            ai_diagnostics_build_dialog(dialog, views.clone())
        });
        // 读库放后台：先给出 loading，再回填结果或错误；失败不伪装成"没有失败"。
        let task = cx.spawn_in(window, async move |view, cx| {
            let loaded = cx
                .background_spawn(async move {
                    fluxdb_app::load_turn_diagnostics_rows(
                        &storage,
                        &session_id,
                        AI_DIAGNOSTICS_TURNS,
                    )
                })
                .await;
            let _ = view.update_in(cx, |this, _window, cx| {
                let mut data = this.ai_diagnostics_modal.borrow_mut();
                data.loading = false;
                match loaded {
                    Ok(rows) => {
                        data.error = None;
                        data.rows = rows;
                    }
                    Err(error) => {
                        data.error = Some(error.message);
                        data.rows.clear();
                    }
                }
                drop(data);
                cx.notify();
            });
        });
        self._ai_diagnostics_task = Some(task);
    }

    fn close_ai_diagnostics_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = {
            let mut data = self.ai_diagnostics_modal.borrow_mut();
            let was_open = data.dialog_open;
            data.reset();
            was_open
        };
        if was_open {
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn toggle_ai_diagnostics_row(&mut self, ordinal: i64, cx: &mut Context<Self>) {
        let mut data = self.ai_diagnostics_modal.borrow_mut();
        data.expanded = if data.expanded == Some(ordinal) {
            None
        } else {
            Some(ordinal)
        };
        drop(data);
        cx.notify();
    }
}

/// 诊断弹框读取的轮数：足够覆盖"上一轮/用户说的第 N 轮"，又不会把上下文撑爆。
const AI_DIAGNOSTICS_TURNS: usize = 5;

fn ai_diagnostics_build_dialog(
    dialog: gpui_component::dialog::Dialog,
    views: AiDiagnosticsDialogViews,
) -> gpui_component::dialog::Dialog {
    let (loading, error, expanded, rows) = {
        let data = views.data.borrow();
        (
            data.loading,
            data.error.clone(),
            data.expanded,
            data.rows.clone(),
        )
    };
    let colors = views.colors;
    let view_close = views.view.clone();
    let data_close = views.data.clone();
    let body = ai_diagnostics_body(&rows, loading, error.as_deref(), expanded, &views);
    dialog
        .w(px(760.))
        .max_w(px(760.))
        .close_button(true)
        .overlay_closable(true)
        .keyboard(true)
        .on_close(move |_, _, _| data_close.borrow_mut().reset())
        .title(tr("app.ai.chat.diagnostics.title"))
        .child(body)
        .footer(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(tr("app.ai.chat.diagnostics.scope")),
                )
                .child(div().flex_1())
                .child(
                    Button::new("ai-diagnostics-close")
                        .debug_selector(|| "ai-diagnostics-close".into())
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .child(tr("app.ai.chat.diagnostics.close"))
                        .on_click(move |_, window, cx| {
                            let _ = view_close.update(cx, |this, cx| {
                                this.close_ai_diagnostics_dialog(window, cx);
                            });
                        }),
                ),
        )
}

fn ai_diagnostics_body(
    rows: &[fluxdb_app::AiTurnDiagnostic],
    loading: bool,
    error: Option<&str>,
    expanded: Option<i64>,
    views: &AiDiagnosticsDialogViews,
) -> gpui::AnyElement {
    let colors = views.colors;
    if loading {
        return div()
            .flex()
            .items_center()
            .gap_2()
            .py_4()
            .text_size(ui_text_size(12.))
            .text_color(colors.muted)
            .child(app_icon(AppIcon::Activity, 14., colors.muted))
            .child(tr("app.ai.chat.diagnostics.loading"))
            .into_any_element();
    }
    if let Some(message) = error {
        return div()
            .py_4()
            .text_size(ui_text_size(12.))
            .text_color(colors.danger)
            .child(tr("app.ai.chat.diagnostics.load_failed").replace("%{message}", message))
            .into_any_element();
    }
    if rows.is_empty() {
        return div()
            .py_4()
            .text_size(ui_text_size(12.))
            .text_color(colors.muted)
            .child(tr("app.ai.chat.diagnostics.empty"))
            .into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(
            rows.iter()
                .rev()
                .map(|row| ai_diagnostics_row(row, expanded == Some(row.ordinal), views)),
        )
        .into_any_element()
}

fn ai_diagnostics_row(
    row: &fluxdb_app::AiTurnDiagnostic,
    expanded: bool,
    views: &AiDiagnosticsDialogViews,
) -> gpui::AnyElement {
    let colors = views.colors;
    let (status_label, status_color) = ai_diagnostics_status(row, colors);
    let view_toggle = views.view.clone();
    let ordinal = row.ordinal;
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(format!("#{}", row.ordinal)),
        )
        .child(ai_pill(&status_label, status_color, colors))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(ui_text_size(12.))
                .child(ai_diagnostics_summary(row)),
        )
        .child(
            Button::new(SharedString::from(format!(
                "ai-diagnostics-toggle-{ordinal}"
            )))
            .debug_selector({
                let ordinal = ordinal;
                move || format!("ai-diagnostics-toggle-{ordinal}")
            })
            .small()
            .ghost()
            .cursor_pointer()
            .child(if expanded {
                tr("app.ai.chat.diagnostics.collapse")
            } else {
                tr("app.ai.chat.diagnostics.expand")
            })
            .on_click(move |_, _, cx| {
                let _ = view_toggle.update(cx, |this, cx| {
                    this.toggle_ai_diagnostics_row(ordinal, cx);
                });
            }),
        );
    let mut card = div()
        .id(SharedString::from(format!("ai-diagnostics-row-{ordinal}")))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border_soft)
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(header);
    if expanded {
        // 稳定业务 ID（Turn ordinal）便于交互测试定位，不依赖下标。
        card = card.child(
            div()
                .id(SharedString::from(format!("ai-diagnostics-detail-{ordinal}")))
                .debug_selector({
                    let ordinal = ordinal;
                    move || format!("ai-diagnostics-detail-{ordinal}")
                })
                .child(ai_diagnostics_detail(row, colors)),
        );
    }
    card.into_any_element()
}

/// 状态标签与颜色：失败/中断用危险或警告色，完成用成功色。
fn ai_diagnostics_status(
    row: &fluxdb_app::AiTurnDiagnostic,
    colors: UiColors,
) -> (String, gpui::Rgba) {
    match row.status.as_str() {
        "completed" => (
            tr("app.ai.chat.diagnostics.status.completed").to_string(),
            colors.success,
        ),
        "failed" => (
            tr("app.ai.chat.diagnostics.status.failed").to_string(),
            colors.danger,
        ),
        "cancelled" => (
            tr("app.ai.chat.diagnostics.status.cancelled").to_string(),
            colors.muted,
        ),
        "interrupted" => (
            tr("app.ai.chat.diagnostics.status.interrupted").to_string(),
            colors.warning,
        ),
        "outcome_unknown" => (
            tr("app.ai.chat.diagnostics.status.outcome_unknown").to_string(),
            colors.warning,
        ),
        "waiting_approval" | "waiting_user" => (
            tr("app.ai.chat.diagnostics.status.waiting").to_string(),
            colors.warning,
        ),
        "cancel_requested" => (
            tr("app.ai.chat.diagnostics.status.cancel_requested").to_string(),
            colors.muted,
        ),
        other => (other.to_string(), colors.muted),
    }
}

/// 一行摘要：错误码 + 归属方 + 可重试性 + 阶段计数（只给结论，不编造原因）。
fn ai_diagnostics_summary(row: &fluxdb_app::AiTurnDiagnostic) -> String {
    let mut parts = Vec::new();
    if let Some(code) = row.error_code.as_deref() {
        parts.push(
            tr("app.ai.chat.diagnostics.summary.error_code").replace("%{code}", code),
        );
    }
    if let Some(stage) = row.failure_stage.as_deref() {
        let owner = row.failure_owner.as_deref().unwrap_or("-");
        parts.push(
            tr("app.ai.chat.diagnostics.summary.stage")
                .replace("%{stage}", stage)
                .replace("%{owner}", owner),
        );
    }
    match row.retryable {
        Some(true) => parts.push(tr("app.ai.chat.diagnostics.summary.retryable").to_string()),
        Some(false) => parts.push(
            tr("app.ai.chat.diagnostics.summary.not_retryable").to_string(),
        ),
        None => {}
    }
    if let (Some(requests), Some(failures)) = (row.model_requests, row.model_failures) {
        if failures > 0 {
            parts.push(
                tr("app.ai.chat.diagnostics.summary.model_requests_failed")
                    .replace("%{requests}", &requests.to_string())
                    .replace("%{failures}", &failures.to_string()),
            );
        } else {
            parts.push(
                tr("app.ai.chat.diagnostics.summary.model_requests")
                    .replace("%{requests}", &requests.to_string()),
            );
        }
    }
    if let (Some(calls), Some(failures)) = (row.tool_calls, row.tool_failures) {
        if failures > 0 {
            parts.push(
                tr("app.ai.chat.diagnostics.summary.tool_calls_failed")
                    .replace("%{calls}", &calls.to_string())
                    .replace("%{failures}", &failures.to_string()),
            );
        }
    }
    if parts.is_empty() {
        parts.push(
            tr("app.ai.chat.diagnostics.summary.iterations")
                .replace("%{count}", &row.iterations.to_string()),
        );
    }
    parts.join(" · ")
}

fn ai_diagnostics_detail(row: &fluxdb_app::AiTurnDiagnostic, colors: UiColors) -> Div {
    let mut detail = div().flex().flex_col().gap_1().text_size(ui_text_size(11.));
    detail = detail.child(ai_diagnostics_kv(
        &tr("app.ai.chat.diagnostics.field.turn_status"),
        &row.status,
        colors,
    ));
    if let Some(code) = row.error_code.as_deref() {
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.error_code"),
            code,
            colors,
        ));
    }
    if let Some(codes) = (!row.error_codes.is_empty()).then(|| row.error_codes.join("、")) {
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.seen_codes"),
            &codes,
            colors,
        ));
    }
    if let Some(requests) = row.model_requests {
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.model_requests"),
            &tr("app.ai.chat.diagnostics.count_failed")
                .replace("%{total}", &requests.to_string())
                .replace("%{failures}", &row.model_failures.unwrap_or(0).to_string()),
            colors,
        ));
    } else {
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.stage_telemetry"),
            &tr("app.ai.chat.diagnostics.field.stage_telemetry_off"),
            colors,
        ));
    }
    if let Some(calls) = row.tool_calls {
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.tool_calls"),
            &tr("app.ai.chat.diagnostics.count_failed")
                .replace("%{total}", &calls.to_string())
                .replace("%{failures}", &row.tool_failures.unwrap_or(0).to_string()),
            colors,
        ));
    }
    for tool in &row.failed_tools {
        let code = tool.code.as_deref().unwrap_or("-");
        let duration = tool
            .duration_ms
            .map(|ms| format!("，{ms}ms"))
            .unwrap_or_default();
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.failed_tools"),
            &tr("app.ai.chat.diagnostics.failed_tool_row")
                .replace("%{iteration}", &tool.iteration.to_string())
                .replace("%{tool}", &tool.tool_id)
                .replace("%{status}", &tool.status)
                .replace("%{code}", code)
                .replace("%{duration}", &duration),
            colors,
        ));
    }
    for notice in &row.notices {
        detail = detail.child(ai_diagnostics_kv(
            &tr("app.ai.chat.diagnostics.field.note"),
            notice,
            colors,
        ));
    }
    detail
}

fn ai_diagnostics_kv(label: &str, value: &str, colors: UiColors) -> Div {
    div()
        .flex()
        .gap_2()
        .child(
            div()
                .flex_none()
                .w(px(84.))
                .text_color(colors.muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_color(colors.text)
                .child(value.to_string()),
        )
}
