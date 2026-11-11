fn ai_target_summary(target_json: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(target_json).ok()?;
    let mut parts = Vec::new();
    if let Some(object) = value.get("object").and_then(serde_json::Value::as_str) {
        parts.push(object.to_string());
    }
    if let Some(schema) = value.get("schema").and_then(serde_json::Value::as_str) {
        parts.push(schema.to_string());
    }
    if let Some(database) = value.get("database").and_then(serde_json::Value::as_str) {
        parts.push(database.to_string());
    }
    if parts.is_empty() {
        if let Some(redis_db) = value.get("redis_db").and_then(serde_json::Value::as_u64) {
            parts.push(format!("db{redis_db}"));
        }
    }
    if parts.is_empty() {
        value
            .get("connection_id")
            .and_then(serde_json::Value::as_i64)
            .map(|connection_id| {
                tr("app.ai.chat.connection_label")
                    .replace("%{id}", &connection_id.to_string())
            })
    } else {
        Some(parts.join(" / "))
    }
}
fn ai_reference_tag(label: &str, kind: &str, colors: UiColors) -> gpui::AnyElement {
    let icon = match kind {
        "connection" | "database" | "redis_db" => AppIcon::Database,
        "table" | "view" | "schema" => AppIcon::Table,
        "file" => AppIcon::Folder,
        "saved_query" => AppIcon::Query,
        "selection" => AppIcon::Select,
        "result_set" => AppIcon::List,
        _ => AppIcon::FileSearch,
    };
    div()
        .flex_none()
        .max_w(px(180.))
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(3.))
        .rounded(colors.radius)
        .bg(colors.success.alpha(0.09))
        .text_color(colors.success)
        .child(app_icon(icon, 11., colors.success))
        .child(
            div()
                .min_w(px(0.))
                .text_size(ui_text_size(10.))
                .truncate()
                .child(label.to_string()),
        )
        .into_any_element()
}
fn ai_scope_tag(icon: AppIcon, label: &str, tone: gpui::Rgba, colors: UiColors) -> gpui::AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_1p5()
        .py(px(2.))
        .rounded(px(999.))
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .text_color(colors.muted)
        .child(app_icon(icon, 10., tone))
        .child(div().text_size(ui_text_size(10.)).child(label.to_string()))
        .into_any_element()
}
fn ai_reference_chip(
    label: &str,
    kind: &str,
    id: String,
    invalid: bool,
    note: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let icon = match kind {
        "connection" | "database" | "redis_db" => AppIcon::Database,
        "table" | "view" | "schema" => AppIcon::Table,
        "file" => AppIcon::Folder,
        "saved_query" => AppIcon::Query,
        "selection" => AppIcon::Select,
        "result_set" => AppIcon::List,
        _ => AppIcon::FileSearch,
    };
    div()
        .id(SharedString::from(format!("ai-reference-chip:{id}")))
        .tooltip(move |window, cx| Tooltip::new(note.clone()).build(window, cx))
        .debug_selector({
            let id = id.clone();
            move || format!("ai-reference:{id}")
        })
        .flex_none()
        .max_w_full()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(3.))
        .rounded(colors.radius)
        .border_1()
        .border_color(if invalid { colors.danger } else { colors.border_soft })
        .bg(colors.panel_alt)
        .child(app_icon(icon, 12., colors.muted))
        .child(
            div()
                .min_w(px(0.))
                .max_w(px(180.))
                .text_size(ui_text_size(10.))
                .truncate()
                .child(label.to_string()),
        )
        .when(invalid, |chip| {
            chip.child(
                div()
                    .debug_selector({
                        let id = id.clone();
                        move || format!("ai-reference-invalid:{id}")
                    })
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(app_icon(AppIcon::TriangleAlert, 10., colors.danger))
                    .child(
                        div()
                            .text_size(ui_text_size(9.))
                            .text_color(colors.danger)
                            .child(tr("app.ai.chat.part.expired")),
                    ),
            )
        })
        .child(
            Button::new(SharedString::from(format!("ai-reference-remove:{id}")))
                .debug_selector({
                    let id = id.clone();
                    move || format!("ai-reference-remove:{id}")
                })
                .xsmall()
                .ghost()
                .cursor_pointer()
                .child(app_icon(AppIcon::Close, 10., colors.muted))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.ai_chat_request(
                        fluxdb_app::AiConversationCommand::RemoveReference {
                            reference_id: id.clone(),
                        },
                        window,
                        cx,
                    )
                })),
        )
        .into_any_element()
}
/// 校验并提取文本片段里的 `text` 字段；非文本/损坏载荷返回空串而不是 panic。
fn ai_chat_part_text(part: &fluxdb_app::AiConversationPart) -> String {
    serde_json::from_str::<serde_json::Value>(&part.payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// 统一的 Markdown 正文样式：跟随组件主题配色，正文 12px、行高 18px。
///
/// 代码块/表格/链接的配色与边框由 gpui-component 主题（已由宿主
/// `apply_component_theme_colors` 对齐 UiColors）提供，这里只调正文尺寸与前景色。
fn ai_chat_markdown_style(view: TextView, colors: UiColors) -> TextView {
    view.selectable(true)
        .text_size(ui_text_size(12.))
        .line_height(ui_text_size(18.))
        .text_color(colors.text)
}

/// 用稳定 id 渲染一段 Markdown 源文本（用户消息/已完成的助手正文）。
///
/// 用外层 `div` 承载 debug selector：`TextView` 的 `debug_selector` 只影响它自己
/// 的命中测试，把选择器挂在容器上更贴近"这条消息以 Markdown 渲染"的语义。
fn ai_chat_markdown_source(
    id: SharedString,
    source: impl Into<SharedString>,
    colors: UiColors,
) -> gpui::AnyElement {
    let selector = SharedString::from(format!("{id}:markdown"));
    div()
        .debug_selector({
            let selector = selector.clone();
            move || selector.to_string()
        })
        .child(ai_chat_markdown_style(TextView::markdown(id, source), colors))
        .into_any_element()
}

/// 渲染由平滑揭示实体驱动的流式助手正文。
///
/// 同一时刻最多只有一条活跃流，用固定选择器即可被 UI 回归稳定断言。
fn ai_chat_markdown_stream(
    view: &Entity<gpui_component::text::TextViewState>,
    colors: UiColors,
) -> gpui::AnyElement {
    div()
        .debug_selector(|| "ai-chat-stream-text".into())
        .child(ai_chat_markdown_style(TextView::new(view), colors))
        .into_any_element()
}

fn ai_chat_text_only(
    part: &fluxdb_app::AiConversationPart,
    colors: UiColors,
) -> gpui::AnyElement {
    let text = ai_chat_part_text(part);
    ai_chat_markdown_source(SharedString::from(part.id.clone()), text, colors)
}
fn ai_chat_part(
    part: &fluxdb_app::AiConversationPart,
    expanded: bool,
    // 审批卡要用的连接事实；其它片段用不到（target_json 只给工具卡摘要用）。
    approval: &AiApprovalContext<'_>,
    target_json: Option<&str>,
    // 非空表示该正文片段正由平滑揭示实体驱动（流式进行中）。
    stream_view: Option<&Entity<gpui_component::text::TextViewState>>,
    // 该工具调用正在执行时的实时进度；仅未完成调用有值。
    live_progress: Option<&fluxdb_app::AiConversationToolProgress>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let payload: serde_json::Value =
        serde_json::from_str(&part.payload_json).unwrap_or(serde_json::Value::Null);
    let text = payload
        .get("text")
        .or_else(|| payload.get("summary"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let text = if part.kind == "notice" {
        localize_backend_error_text(&text)
    } else {
        text
    };
    let recovery_status = payload.get("recovery_status").and_then(|value| value.as_str());
    let approval_decision = (part.kind == "notice" && payload.get("approval_id").is_some())
        .then(|| payload.get("decision").and_then(serde_json::Value::as_str))
        .flatten();
    if part.kind == "notice" && recovery_status == Some("outcome_unknown") {
        let id = SharedString::from(format!("ai-recovery:{}", part.id));
        return div()
            .debug_selector({ let id = id.to_string(); move || id.clone() })
            .child(gpui_component::alert::Alert::error(id, text).small())
            .into_any_element();
    }
    if part.kind == "artifact_ref" {
        return ai_chat_artifact_card(part, &payload, &text, colors, cx);
    }
    if part.kind == "notice"
        && payload.get("kind").and_then(serde_json::Value::as_str)
            == Some("er_relationship_proposals")
    {
        return ai_chat_er_proposal_card(part, &payload, colors, cx);
    }
    if part.kind == "plan" {
        return ai_chat_plan_card(part, &payload, colors);
    }
    if part.kind == "text" {
        // 流式进行中由平滑揭示实体渲染；其余（含已结束片段）按 Markdown 渲染。
        return match stream_view {
            Some(view) => ai_chat_markdown_stream(view, colors),
            None => ai_chat_markdown_source(SharedString::from(part.id.clone()), text, colors),
        };
    }
    // 审批卡：必须在这里决定（批准/拒绝），决定后由宿主按 plan_hash 复核再执行。
    if part.kind == "notice" {
        if payload.get("approval_id").and_then(|value| value.as_str()).is_some() {
            let revision = payload
                .get("approval_revision")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(1);
            let decided = payload
                .get("decision")
                .and_then(|value| value.as_str())
                .is_some();
            if !decided && part.status == "pending" {
                return ai_chat_approval_card(&part.id, &payload, approval, revision, colors, cx);
            }
            if let Some(decision) = approval_decision {
                return ai_chat_approval_result(part, &payload, decision, expanded, approval, colors, cx);
            }
        }
    }
    let (icon, label) = match part.kind.as_str() {
        "reasoning_summary" => (AppIcon::Workflow, tr("app.ai.chat.part.reasoning_summary")),
        "plan" => (AppIcon::List, tr("app.ai.chat.part.plan")),
        "tool_call_ref" => (AppIcon::Database, tr("app.ai.chat.part.tool_call_ref")),
        "tool_result_ref" => (AppIcon::Table, tr("app.ai.chat.part.tool_result_ref")),
        "artifact_ref" => (AppIcon::Activity, tr("app.ai.chat.part.artifact_ref")),
        "context_ref" => (AppIcon::FileSearch, tr("app.ai.chat.part.context_ref")),
        _ => (AppIcon::Info, tr("app.ai.chat.part.fallback")),
    };
    // 工具卡直接展示真实 tool_id；目标/参数仍在展开区，不凭空拼标签。
    let label = if approval_decision.is_some() {
        tr("app.ai.chat.part.approval_decision").to_string()
    } else if let Some(recovery_status) = recovery_status {
        if recovery_status == "resolved" {
            tr("app.ai.chat.part.recovery_resolved").to_string()
        } else {
            tr("app.ai.chat.part.recovery_paused").to_string()
        }
    } else if part.kind == "tool_call_ref" {
        payload
            .get("tool")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&label)
            .to_string()
    } else if part.kind == "notice" && !text.is_empty() {
        text.clone()
    } else {
        label.to_string()
    };
    let target = if part.kind == "tool_call_ref" {
        target_json.and_then(ai_target_summary)
    } else {
        None
    };
    let duration_ms = payload
        .get("duration_ms")
        .and_then(serde_json::Value::as_i64)
        .filter(|duration_ms| *duration_ms >= 0);
    // 执行中（回执未到）用实时进度里的耗时；回执到达后以落盘 duration 为准。
    let duration_ms = duration_ms.or_else(|| {
        live_progress.map(|progress| progress.elapsed_ms as i64)
    });
    let id = part.id.clone();
    let execution_unknown =
        payload.get("execution_status").and_then(|v| v.as_str()) == Some("unknown");
    // 审计判定行：只在工具调用/结果卡上显示，文案与颜色与审计页一致（绿/黄/红）。
    let audit_line: Option<(String, gpui::Rgba)> =
        match payload.get("audit_decision").and_then(|v| v.as_str()) {
            Some("allow") => Some((
                tr("app.ai.chat.audit.allow").to_string(),
                colors.success,
            )),
            Some("require_approval") => {
                Some((
                    tr("app.ai.chat.audit.require_approval").to_string(),
                    colors.warning,
                ))
            }
            Some("deny") => Some((tr("app.ai.chat.audit.deny").to_string(), colors.danger)),
            _ => None,
        };
    // 状态用稳定 tag 决定，tag 既选文案也选颜色；文案本身不参与任何判断。
    let (status_tag, status) = if let Some(decision) = approval_decision {
        // 批准只是提交决定，不能把仍待复核或恢复失败的工具称为“已完成”。
        match decision {
            "approved" => ("approved", tr("app.ai.chat.status.approved")),
            "denied" => ("denied", tr("app.ai.chat.status.denied")),
            _ => (
                "approval_invalid",
                tr("app.ai.chat.status.approval_invalid"),
            ),
        }
    } else if execution_unknown {
        ("unknown_result", tr("app.ai.chat.status.unknown_result"))
    } else {
        match part.status.as_str() {
            "streaming" => ("streaming", tr("app.ai.chat.status.streaming")),
            "interrupted" => ("interrupted", tr("app.ai.chat.status.interrupted")),
            "error" => ("failed", tr("app.ai.chat.status.failed")),
            "pending" => ("pending", tr("app.ai.chat.status.pending")),
            _ if part.kind == "tool_call_ref" || part.kind == "tool_result_ref" => {
                ("tool_done", tr("app.ai.chat.status.tool_done"))
            }
            _ => ("done", tr("app.ai.chat.status.done")),
        }
    };
    let status_color = match status_tag {
        "approved" | "done" | "tool_done" => colors.success,
        "denied" | "pending" => colors.muted,
        "approval_invalid" | "interrupted" => colors.warning,
        "unknown_result" | "failed" => colors.danger,
        _ => colors.info,
    };
    // 失败的工具行直接带上受控错误码：内联行自己就能说明原因，不必展开或再看末尾汇总。
    // 状态位限宽截断，错误码再长也不会把整行标题挤出面板。
    let status_text = match payload.get("error_code").and_then(|value| value.as_str()) {
        Some(code) if status_tag == "failed" && !code.is_empty() => {
            format!("{status} · {code}")
        }
        _ => status.to_string(),
    };
    div()
        .when(recovery_status.is_some(), |row| row.debug_selector({
            let id = part.id.clone(); move || format!("ai-recovery:{id}")
        }))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            Button::new(SharedString::from(format!("ai-part:{}", part.id)))
                .accessibility_label(format!("{label}，{status_text}"))
                .debug_selector(move || format!("ai-part:{id}"))
                .xsmall()
                .w_full()
                .ghost()
                .cursor_pointer()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(app_icon(icon, 12., colors.muted))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .max_w(px(220.))
                                        .text_size(ui_text_size(if recovery_status.is_some() { 10. } else { 11. }))
                                        .text_color(if recovery_status.is_some() { colors.muted } else { colors.text })
                                        .font_weight(gpui::FontWeight::NORMAL)
                                        .truncate()
                                        .child(label),
                                )
                                .when_some(target, |row, target| {
                                    row.child(
                                        div()
                                            .min_w(px(0.))
                                            .max_w(px(130.))
                                            .text_size(ui_text_size(10.))
                                            .text_color(colors.muted)
                                            .truncate()
                                            .child(target),
                                    )
                                }),
                        )
                        .when_some(duration_ms, |row, duration_ms| {
                            row.child(
                                div()
                                    .flex_none()
                                    .text_size(ui_text_size(10.))
                                    .text_color(colors.muted)
                                    .child(format!("{duration_ms}ms")),
                            )
                        })
                        .when(recovery_status.is_none() && !(part.kind == "notice" && part.status == "complete"), |row| row.child(div().flex_none().max_w(px(160.)).truncate().text_size(ui_text_size(10.)).text_color(status_color).child(status_text)))
                        .child(app_icon(if expanded { AppIcon::ChevronDown } else { AppIcon::ChevronRight }, 10., colors.muted)),
                )
                .on_click(cx.listener({
                    let id = part.id.clone();
                    move |this, _, window, cx| {
                        let mut chat = this.controller.state().ai_conversation.clone();
                        if let Some(index) = chat.expanded_parts.iter().position(|p| p == &id) {
                            chat.expanded_parts.remove(index);
                        } else {
                            chat.expanded_parts.push(id.clone());
                        }
                        this.controller.dispatch(AppCommand::AiConversation(
                            fluxdb_app::AiConversationCommand::Apply { view: chat },
                        ));
                        this.save_ai_chat_ui(window, cx);
                        cx.notify();
                    }
                })),
        )
        // 审计判定单独一行：工具名那行已被名称/目标/耗时占满，判定是用户最该一眼看到的信息。
        .when_some(audit_line, |this, (text, color)| {
            this.child(
                div()
                    .id(SharedString::from(format!("ai-part-audit:{}", part.id)))
                    .debug_selector({
                        let id = part.id.clone();
                        move || format!("ai-part-audit:{id}")
                    })
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(18.))
                    .child(div().size(px(5.)).rounded_full().bg(color))
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(color)
                            .child(text),
                    ),
            )
        })
        // 执行中的实时进度：只有尚未收到回执的调用才有值，展示已返回/影响行数与耗时。
        .when_some(live_progress, |this, progress| {
            let detail = if progress.affected_rows > 0 {
                tr("app.ai.chat.progress.affected")
                    .replace("%{rows}", &progress.affected_rows.to_string())
                    .replace("%{ms}", &progress.elapsed_ms.to_string())
            } else {
                tr("app.ai.chat.progress.returned")
                    .replace("%{rows}", &progress.returned_rows.to_string())
                    .replace("%{ms}", &progress.elapsed_ms.to_string())
            };
            this.child(
                div()
                    .id(SharedString::from(format!("ai-part-progress:{}", part.id)))
                    .debug_selector({
                        let id = part.id.clone();
                        move || format!("ai-part-progress:{id}")
                    })
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(18.))
                    .child(div().size(px(5.)).rounded_full().bg(colors.info))
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.info)
                            .child(detail),
                    ),
            )
        })
        .when(expanded, |this| {
            this.child(
                div()
                    .debug_selector({
                        let id = part.id.clone();
                        move || format!("ai-part-detail:{id}")
                    })
                    .max_h(px(160.))
                    .overflow_y_scrollbar()
                    .text_size(ui_text_size(11.))
                    .line_height(ui_text_size(16.))
                    .child(if text.is_empty() {
                        part.payload_json.clone()
                    } else {
                        text
                    }),
            )
        })
        .into_any_element()
}

/// 「计划与执行进度」卡片：展示模型公开的步骤计划与各步状态。
///
/// 只呈现模型声明的状态，不按工具名/关键词自动推进；状态用颜色图标区分，
/// 文案随明暗主题走 `UiColors`。
fn ai_chat_plan_card(
    part: &fluxdb_app::AiConversationPart,
    payload: &serde_json::Value,
    colors: UiColors,
) -> gpui::AnyElement {
    let title = payload
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| tr("app.ai.chat.plan.default_title").to_string());
    let steps = payload
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let done = steps
        .iter()
        .filter(|step| step.get("status").and_then(serde_json::Value::as_str) == Some("done"))
        .count();
    let total = steps.len();
    let mut rows = Vec::with_capacity(total);
    for (index, step) in steps.iter().enumerate() {
        let step_title = step
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let status = step
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("pending");
        // 状态 → (图标, 文案, 颜色)：颜色/图标都要与审计卡一致的明暗语义。
        let (icon, status_text, tone) = match status {
            "done" => (
                AppIcon::CircleCheck,
                tr("app.ai.chat.plan.step_done"),
                colors.success,
            ),
            "in_progress" => (
                AppIcon::Loader,
                tr("app.ai.chat.plan.step_in_progress"),
                colors.info,
            ),
            "blocked" => (
                AppIcon::TriangleAlert,
                tr("app.ai.chat.plan.step_blocked"),
                colors.warning,
            ),
            "skipped" => (
                AppIcon::CircleSlash,
                tr("app.ai.chat.plan.step_skipped"),
                colors.muted,
            ),
            _ => (
                AppIcon::Circle,
                tr("app.ai.chat.plan.step_pending"),
                colors.muted,
            ),
        };
        let note = step
            .get("note")
            .and_then(serde_json::Value::as_str)
            .filter(|note| !note.is_empty());
        rows.push(
            div()
                .id(SharedString::from(format!("ai-plan-step:{}:{index}", part.id)))
                .debug_selector({
                    let id = part.id.clone();
                    move || format!("ai-plan-step:{id}:{index}")
                })
                .flex()
                .items_start()
                .gap_2()
                .child(app_icon(icon, 12., tone))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.text)
                                .child(step_title),
                        )
                        .when_some(note, |this, note| {
                            this.child(
                                div()
                                    .text_size(ui_text_size(10.))
                                    .text_color(colors.muted)
                                    .child(note.to_string()),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(10.))
                        .text_color(tone)
                        .child(status_text),
                ),
        );
    }
    div()
        .id(SharedString::from(format!("ai-plan:{}", part.id)))
        .debug_selector({
            let id = part.id.clone();
            move || format!("ai-plan:{id}")
        })
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .rounded(px(6.))
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .p_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(app_icon(AppIcon::List, 12., colors.muted))
                        .child(
                            div()
                                .min_w(px(0.))
                                .text_size(ui_text_size(11.))
                                .text_color(colors.text)
                                .truncate()
                                .child(title.to_string()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(format!("{done}/{total}")),
                ),
        )
        .child(div().flex().flex_col().gap_1p5().children(rows))
        .into_any_element()
}

// 按真实调用外键合并，不能按工具名匹配：同一个工具可能被并发调用多次。
fn ai_compact_parts(parts: &[fluxdb_app::AiConversationPart]) -> Vec<fluxdb_app::AiConversationPart> {
    let call_key = |part: &fluxdb_app::AiConversationPart| {
        serde_json::from_str::<serde_json::Value>(&part.payload_json).ok()
            .and_then(|p| p.get("tool_call_id").and_then(|v| v.as_str()).map(str::to_owned))
    };
    let mut output = Vec::new();
    for part in parts {
        // 已被原审批结果替代的等待提示只保留审计，不重复显示。
        let superseded = part.kind == "notice" && serde_json::from_str::<serde_json::Value>(&part.payload_json).ok()
            .is_some_and(|payload| {
                payload["superseded_by"].as_str().is_some_and(|id| {
                    parts.iter().any(|other| other.id == id && other.kind == "notice"
                        && serde_json::from_str::<serde_json::Value>(&other.payload_json).ok()
                            .is_some_and(|approval| {
                                approval["approval_id"].as_str().is_some()
                                    && approval["approval_id"] == payload["approval_id"]
                                    && approval["decision"].as_str().is_some()
                            }))
                })
            });
        if superseded {
            continue;
        }
        if part.kind == "tool_call_ref" {
            if let Some(key) = call_key(part) {
                if let Some(result) = parts.iter().find(|p| p.kind == "tool_result_ref" && call_key(p).as_ref() == Some(&key)) {
                    let mut merged = part.clone();
                    merged.status = result.status.clone();
                    let mut payload: serde_json::Value = serde_json::from_str(&result.payload_json).unwrap_or_default();
                    if let Some(object) = payload.as_object_mut() {
                        object.insert("call".into(), serde_json::from_str::<serde_json::Value>(&part.payload_json).unwrap_or_default());
                        // 合并回执后仍以调用开始位置展示，不把工具挪到结果返回的位置。
                        if let Some(offset) = ai_part_text_offset(part) {
                            object.insert("text_offset".into(), offset.into());
                        }
                    }
                    merged.payload_json = payload.to_string();
                    output.push(merged);
                    continue;
                }
            }
        } else if part.kind == "tool_result_ref" {
            if let Some(key) = call_key(part) {
                if parts.iter().any(|p| p.kind == "tool_call_ref" && call_key(p).as_ref() == Some(&key)) {
                    continue;
                }
            }
        }
        output.push(part.clone());
    }
    output
}

fn ai_execution_needs_attention(part: &fluxdb_app::AiConversationPart) -> bool {
    matches!(part.status.as_str(), "error" | "pending" | "streaming" | "interrupted")
        || serde_json::from_str::<serde_json::Value>(&part.payload_json).ok()
            .is_some_and(|p| p.get("execution_status").and_then(|v| v.as_str()) == Some("unknown"))
}

fn ai_chat_message_parts(
    message: &fluxdb_app::AiConversationMessage,
    chat: &fluxdb_app::AiConversationView,
    approval: &AiApprovalContext<'_>,
    // 该消息若正是活跃流式正文，用揭示实体渲染；否则 None。
    stream_view: Option<&Entity<gpui_component::text::TextViewState>>,
    revealed: Option<&str>,
    // 正在执行的工具实时进度（按 call_id）；终态回执到达后由消费端清理。
    tool_progress: &std::collections::BTreeMap<String, fluxdb_app::AiConversationToolProgress>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Vec<gpui::AnyElement> {
    // 从工具卡的 payload 里取 `call_id`，命中正在执行的调用才有实时进度。
    let live_progress = |part: &fluxdb_app::AiConversationPart| {
        serde_json::from_str::<serde_json::Value>(&part.payload_json)
            .ok()
            .and_then(|payload| {
                payload
                    .get("call_id")
                    .or_else(|| payload.get("tool_call_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .and_then(|call_id| tool_progress.get(&call_id))
    };
    let compact = ai_compact_parts(&message.parts);
    let segmented = compact.iter().any(|part| ai_part_text_offset(part).is_some());
    let parts = ai_timeline_parts(&compact, revealed);
    let mut output = Vec::new();
    let mut index = 0;
    let mut group_count = 0;
    while index < parts.len() {
        let part = &parts[index];
        if ai_completed_execution(part) {
            let end = index + parts[index..].iter().take_while(|part| ai_completed_execution(part)).count();
            let tools = &parts[index..end];
            let group_id = if group_count == 0 {
                format!("execution:{}", message.id)
            } else {
                format!("execution:{}:{}", message.id, part.id)
            };
            let open = chat.expanded_parts.contains(&group_id);
            group_count += 1;
            index = end;
            let toggle_id = group_id.clone();
            output.push(div().w_full().pl_3().flex().flex_col().items_start().gap_1().min_w(px(0.))
                .child(Button::new(SharedString::from(group_id.clone()))
                    .debug_selector({ let id = group_id.clone(); move || id.clone() })
                    .xsmall().ghost().rounded_md().self_start().cursor_pointer()
                    .child(div().flex().items_center().gap_1p5().text_size(ui_text_size(10.)).text_color(colors.muted)
                        .child(app_icon(AppIcon::Check, 11., colors.muted))
                        .child(tr("app.ai.chat.plan.summary").replace("%{count}", &tools.len().to_string()))
                        .child(app_icon(if open { AppIcon::ChevronDown } else { AppIcon::ChevronRight }, 10., colors.muted)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let mut view = this.controller.state().ai_conversation.clone();
                        if let Some(index) = view.expanded_parts.iter().position(|id| id == &toggle_id) {
                            view.expanded_parts.remove(index);
                        } else { view.expanded_parts.push(toggle_id.clone()); }
                        this.controller.dispatch(AppCommand::AiConversation(fluxdb_app::AiConversationCommand::Apply { view }));
                        this.save_ai_chat_ui(window, cx);
                        cx.notify();
                    })))
                .when(open, |column| column.child(div().w_full().min_w(px(0.)).pl_2().border_l_1().border_color(colors.border_soft)
                    .flex().flex_col().gap_1().children(tools.iter().map(|p| ai_chat_part(p, chat.expanded_parts.contains(&p.id), approval, chat.target_json.as_deref(), None, live_progress(p), colors, cx)))))
                .into_any_element());
        } else {
            let part_stream = (part.kind == "text" && !segmented).then_some(stream_view).flatten();
            output.push(ai_chat_part(part, chat.expanded_parts.contains(&part.id), approval, chat.target_json.as_deref(), part_stream, live_progress(part), colors, cx));
            index += 1;
        }
    }
    output
}
