// AI 关系建议：聊天摘要卡 + 逐条审阅大弹框。
//
// 工具本身只生成候选，不写目录；这里的选择是唯一写入入口。弹框关闭不写入，
// 已采纳项由宿主更新到持久化卡片 payload，重新打开历史时不会再次当作待审阅。

#[derive(Clone, Default)]
struct ErProposalModalData {
    dialog_open: bool,
    batch: Option<ErProposalBatchView>,
    selected: BTreeSet<String>,
    active_item: Option<String>,
    loading: bool,
    error: Option<String>,
}

impl ErProposalModalData {
    /// 关闭弹框但保留本批勾选草稿；“稍后处理”再次打开时继续审阅。
    fn dismiss(&mut self) {
        self.dialog_open = false;
        self.loading = false;
        self.error = None;
    }

    fn reset(&mut self) {
        self.dialog_open = false;
        self.batch = None;
        self.selected.clear();
        self.active_item = None;
        self.loading = false;
        self.error = None;
    }
}

#[derive(Clone)]
struct ErProposalDialogViews {
    view: Entity<NavicatMain>,
    data: Rc<std::cell::RefCell<ErProposalModalData>>,
    colors: UiColors,
}

#[derive(Clone, Debug)]
struct ErProposalTargetView {
    connection_id: u64,
    database: String,
    schema: Option<String>,
    scope_key: String,
}

impl ErProposalTargetView {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            connection_id: value.get("connection_id")?.as_u64()?,
            database: value.get("database")?.as_str()?.to_string(),
            schema: value
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            scope_key: value.get("relationship_scope_key")?.as_str()?.to_string(),
        })
    }

    fn label(&self) -> String {
        match self.schema.as_deref().filter(|schema| !schema.is_empty()) {
            Some(schema) => format!("{} / {}", self.database, schema),
            None => self.database.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ErProposalCounts {
    total: usize,
    ready: usize,
    unavailable: usize,
    invalid: usize,
}

#[derive(Clone, Debug)]
struct ErProposalItemView {
    item_id: String,
    role: String,
    left: String,
    right: String,
    status: String,
    message: Option<String>,
    warnings: Vec<String>,
    relationship: Option<fluxdb_core::ErRelationship>,
}

impl ErProposalItemView {
    fn from_json(index: usize, value: &serde_json::Value) -> Self {
        let relationship: Option<fluxdb_core::ErRelationship> = value
            .get("relationship")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let item_id = value
            .get("item_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| relationship.as_ref().map(|relationship| relationship.id.clone()))
            .unwrap_or_else(|| format!("proposal-{index}"));
        Self {
            item_id,
            role: value
                .get("role")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            left: value
                .get("left")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            right: value
                .get("right")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            status: value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("invalid")
                .to_string(),
            message: value
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            warnings: value
                .get("warnings")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            relationship,
        }
    }

    fn selectable(&self) -> bool {
        self.status == "ready" && self.relationship.is_some()
    }

}

#[derive(Clone, Debug)]
struct ErProposalBatchView {
    part_id: String,
    batch_id: String,
    target: ErProposalTargetView,
    counts: ErProposalCounts,
    items: Vec<ErProposalItemView>,
    review_status: Option<String>,
    adopted: usize,
}

impl ErProposalBatchView {
    fn from_payload(part_id: String, payload: &serde_json::Value) -> Option<Self> {
        let batch_id = payload.get("batch_id")?.as_str()?.to_string();
        let target = ErProposalTargetView::from_json(payload.get("target")?)?;
        let items = payload
            .get("items")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| ErProposalItemView::from_json(index, item))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let counts = payload
            .get("counts")
            .map(|counts| ErProposalCounts {
                total: counts
                    .get("total")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(items.len() as u64) as usize,
                ready: counts
                    .get("ready")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize,
                unavailable: counts
                    .get("unavailable")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize,
                invalid: counts
                    .get("invalid")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize,
            })
            .unwrap_or_default();
        let review_status = payload
            .get("review")
            .and_then(|review| review.get("status"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let adopted = payload
            .get("review")
            .and_then(|review| review.get("adopted"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        Some(Self {
            part_id,
            batch_id,
            target,
            counts,
            items,
            review_status,
            adopted,
        })
    }

    fn active(&self, active: Option<&str>) -> Option<&ErProposalItemView> {
        active
            .and_then(|id| self.items.iter().find(|item| item.item_id == id))
            .or_else(|| self.items.first())
    }
}

impl NavicatMain {
    fn open_er_proposal_review(
        &mut self,
        part_id: String,
        payload: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.er_proposal_modal.borrow().dialog_open {
            return;
        }
        let Some(batch) = ErProposalBatchView::from_payload(part_id.clone(), &payload) else {
            self.show_message(
                tr("app.ai.er_proposal.card_unreadable"),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        {
            let mut data = self.er_proposal_modal.borrow_mut();
            let previous_batch_id = data
                .batch
                .as_ref()
                .map(|previous| previous.batch_id.clone());
            let same_batch = previous_batch_id.as_deref() == Some(batch.batch_id.as_str());
            data.dialog_open = true;
            data.error = None;
            data.loading = false;
            if !same_batch {
                data.selected.clear();
                data.active_item = batch
                    .items
                    .iter()
                    .find(|item| item.selectable())
                    .map(|item| item.item_id.clone())
                    .or_else(|| batch.items.first().map(|item| item.item_id.clone()));
            }
            data.batch = Some(batch);
        }
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        let views = ErProposalDialogViews {
            view: cx.entity(),
            data: self.er_proposal_modal.clone(),
            colors,
        };
        self.focus_handle.focus(window, cx);
        window.open_dialog(cx, move |dialog, _window, _cx| {
            er_proposal_build_dialog(dialog, views.clone())
        });
    }

    fn close_er_proposal_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = {
            let mut data = self.er_proposal_modal.borrow_mut();
            let was_open = data.dialog_open;
            data.dismiss();
            was_open
        };
        if was_open {
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn discard_er_proposal_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = self.er_proposal_modal.borrow().dialog_open;
        self.er_proposal_modal.borrow_mut().reset();
        if was_open {
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn adopt_selected_er_proposals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (batch, proposals) = {
            let data = self.er_proposal_modal.borrow();
            let Some(batch) = data.batch.clone() else {
                return;
            };
            let proposals = batch
                .items
                .iter()
                .filter(|item| {
                    item.selectable()
                        && data.selected.contains(&item.item_id)
                })
                .filter_map(|item| item.relationship.clone())
                .collect::<Vec<_>>();
            (batch, proposals)
        };
        if proposals.is_empty() {
            return;
        }
        {
            let mut data = self.er_proposal_modal.borrow_mut();
            data.loading = true;
            data.error = None;
        }
        let controller = self.controller.clone();
        let part_id = batch.part_id.clone();
        let batch_id = batch.batch_id.clone();
        let target = batch.target.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    let mut controller = controller;
                    controller.dispatch(fluxdb_app::AppCommand::AdoptErRelationshipProposals {
                        scope_key: target.scope_key,
                        connection_id: fluxdb_core::ConnectionId(target.connection_id),
                        database: target.database,
                        schema: target.schema,
                        proposals,
                        message_part_id: part_id,
                        batch_id,
                    })
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                let still_open = this.er_proposal_modal.borrow().dialog_open;
                match event {
                    fluxdb_app::AppEvent::ErRelationshipProposalsAdopted {
                        relationships,
                        ..
                    } => {
                        let session_id = this
                            .controller
                            .state()
                            .ai_conversation
                            .session_id
                            .clone();
                        if still_open {
                            this.discard_er_proposal_review(window, cx);
                        }
                        this.refresh_open_er_relationships(cx);
                        this.show_message(
                            tr("app.ai.er_proposal.adopted_count")
                                .replace("%{count}", &relationships.len().to_string()),
                            AppMessageKind::Success,
                            cx,
                        );
                        if let Some(session_id) = session_id {
                            this.ai_chat_request(
                                fluxdb_app::AiConversationCommand::Load {
                                    session_id: Some(session_id),
                                },
                                window,
                                cx,
                            );
                        }
                    }
                    fluxdb_app::AppEvent::Failed(error) => {
                        if still_open {
                            let mut data = this.er_proposal_modal.borrow_mut();
                            data.loading = false;
                            data.error = Some(error.message);
                            cx.notify();
                        } else {
                            this.show_message(
                                tr("app.ai.er_proposal.adopt_failed")
                                    .replace("%{error}", &error.message),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    _ => {
                        if still_open {
                            let mut data = this.er_proposal_modal.borrow_mut();
                            data.loading = false;
                            data.error =
                                Some(tr("app.ai.er_proposal.unknown_result").to_string());
                            cx.notify();
                        } else {
                            this.show_message(
                                tr("app.ai.er_proposal.unknown_result"),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                }
            });
        });
        self._er_proposal_task = Some(task);
    }
}

/// 聊天中的紧凑摘要卡：20 条详情不进聊天正文。
fn ai_chat_er_proposal_card(
    part: &fluxdb_app::AiConversationPart,
    payload: &serde_json::Value,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let batch = ErProposalBatchView::from_payload(part.id.clone(), payload);
    let Some(batch) = batch else {
        return div()
            .w_full()
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border_soft)
            .bg(colors.panel_alt)
            .p_3()
            .child(tr("app.ai.er_proposal.card_unreadable"))
            .into_any_element();
    };
    let ready = batch.counts.ready;
    let review_complete = batch.review_status.as_deref() == Some("complete");
    let action_label = if review_complete {
        tr("app.ai.er_proposal.view_adopted").to_string()
    } else if batch.adopted > 0 {
        tr("app.ai.er_proposal.continue_review")
            .replace("%{count}", &ready.to_string())
    } else {
        tr("app.ai.er_proposal.review_count")
            .replace("%{count}", &batch.counts.total.to_string())
    };
    let active_label = batch
        .items
        .iter()
        .filter(|item| item.selectable())
        .take(2)
        .map(|item| format!("{} → {} · {}", item.left, item.right, item.role))
        .collect::<Vec<_>>();
    let click_part = part.id.clone();
    let click_payload = payload.clone();
    let card = div()
        .w_full()
        .min_w(px(0.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.warning.alpha(0.40))
        .bg(colors.warning.alpha(0.06))
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.warning.alpha(0.22))
                .child(app_icon(AppIcon::Workflow, 14., colors.warning))
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(tr("app.ai.er_proposal.card_title")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.))
                                .text_color(colors.muted)
                                .truncate()
                                .child(
                                    tr("app.ai.er_proposal.target_not_written")
                                        .replace("%{target}", &batch.target.label()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(if review_complete {
                            tr("app.ai.er_proposal.adopted")
                                .replace("%{count}", &batch.adopted.to_string())
                        } else {
                            tr("app.ai.er_proposal.ready")
                                .replace("%{count}", &ready.to_string())
                        }),
                ),
        )
        .child(
            div()
                .px_3()
                .pt_2()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .children(active_label.into_iter().map(|label| {
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.text)
                        .truncate()
                        .child(label)
                }))
                .when(batch.counts.total > 2, |list| {
                    list.child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.muted)
                            .child(
                                tr("app.ai.er_proposal.more_items").replace(
                                    "%{count}",
                                    &(batch.counts.total - 2).to_string(),
                                ),
                            ),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .pb_3()
                .pt_1()
                .child(
                    Button::new(SharedString::from(format!(
                        "er-proposal-open:{}",
                        part.id
                    )))
                    .debug_selector({
                        let id = part.id.clone();
                        move || format!("er-proposal-open:{id}")
                    })
                    .small()
                    .primary()
                    .cursor_pointer()
                    .child(action_label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_er_proposal_review(
                            click_part.clone(),
                            click_payload.clone(),
                            window,
                            cx,
                        );
                    })),
                ),
        );
    card.into_any_element()
}


#[cfg(test)]
mod er_proposal_review_tests {
    use super::*;

    fn relationship() -> serde_json::Value {
        serde_json::to_value(fluxdb_core::ErRelationship {
            id: "agent-review".into(),
            revision: 1,
            left_entity: "sales::orders".into(),
            right_entity: "sales::customers".into(),
            role: "buyer".into(),
            column_pairs: vec![fluxdb_core::ErColumnPair {
                left_column: "sales::orders::customer_id".into(),
                right_column: "sales::customers::id".into(),
            }],
            required_filters: Vec::new(),
            match_cardinality: fluxdb_core::ErMatchCardinality {
                left_to_right: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Many,
                },
                right_to_left: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::One,
                },
                basis: fluxdb_core::ErCardinalityBasis::Unknown,
            },
            origin: fluxdb_core::ErRelationshipOrigin::Agent,
            review: fluxdb_core::ErRelationshipReview {
                state: fluxdb_core::ErReviewState::Proposed,
                confirmed_revision: None,
                confirmed_by: None,
            },
            enforcement: fluxdb_core::ErRelationshipEnforcement {
                kind: fluxdb_core::ErEnforcementKind::None,
                constraint_ref: None,
                enforced: None,
            },
            validity: fluxdb_core::ErValidity {
                state: fluxdb_core::ErValidityState::Current,
                reason: None,
            },
            description: Some("列名匹配".into()),
            evidence_refs: Vec::new(),
        })
        .expect("relationship serializes")
    }

    #[test]
    fn proposal_card_keeps_ready_and_unavailable_items_distinct() {
        let payload = serde_json::json!({
            "batch_id": "batch-1",
            "target": {
                "connection_id": 1,
                "database": "sales",
                "schema": null,
                "relationship_scope_key": "1:sales:"
            },
            "counts": {"total": 2, "ready": 1, "unavailable": 1, "invalid": 0},
            "items": [
                {
                    "item_id": "agent-review",
                    "left": "orders",
                    "right": "customers",
                    "role": "buyer",
                    "status": "ready",
                    "warnings": ["cardinality_unknown"],
                    "relationship": relationship()
                },
                {
                    "item_id": "duplicate",
                    "left": "orders",
                    "right": "regions",
                    "role": "region",
                    "status": "duplicate_existing",
                    "message": "已存在同一条关系"
                }
            ]
        });

        let batch = ErProposalBatchView::from_payload("part-1".into(), &payload)
            .expect("proposal payload");

        assert_eq!(batch.target.label(), "sales");
        assert_eq!(batch.counts.ready, 1);
        assert_eq!(batch.items.len(), 2);
        assert!(batch.items[0].selectable());
        assert!(!batch.items[1].selectable());
        assert_eq!(batch.items[0].warnings, vec!["cardinality_unknown"]);
    }
}
