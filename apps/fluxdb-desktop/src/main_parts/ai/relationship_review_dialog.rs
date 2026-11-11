// AI 关系建议弹框渲染；状态与采纳编排见 relationship_review.rs。

fn er_proposal_build_dialog(
    dialog: gpui_component::dialog::Dialog,
    views: ErProposalDialogViews,
) -> gpui_component::dialog::Dialog {
    let (batch, selected, active, loading, error) = {
        let data = views.data.borrow();
        (
            data.batch.clone(),
            data.selected.clone(),
            data.active_item.clone(),
            data.loading,
            data.error.clone(),
        )
    };
    let Some(batch) = batch else {
        return dialog
            .title(tr("app.ai.er_proposal.title"))
            .child(tr("app.ai.er_proposal.unavailable"));
    };
    let colors = views.colors;
    let view_close = views.view.clone();
    let view_adopt = views.view.clone();
    let data_close = views.data.clone();
    let title = tr("app.ai.er_proposal.title_count")
        .replace("%{count}", &batch.counts.total.to_string());
    let body = er_proposal_dialog_body(&batch, &selected, active.as_deref(), error, &views);
    dialog
        .w(px(1040.))
        .max_w(px(1040.))
        .close_button(true)
        .overlay_closable(true)
        .keyboard(true)
        .on_close(move |_, _, _| data_close.borrow_mut().dismiss())
        .title(title)
        .child(body)
        .footer(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.ai.er_proposal.selection_summary")
                                .replace("%{selected}", &selected.len().to_string())
                                .replace("%{ready}", &batch.counts.ready.to_string())
                                .replace(
                                    "%{invalid}",
                                    &(batch.counts.invalid + batch.counts.unavailable).to_string(),
                                ),
                        ),
                )
                .child(div().flex_1())
                .child(
                    Button::new("er-proposal-cancel")
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .child(tr("app.ai.er_proposal.later"))
                        .on_click(move |_, window, cx| {
                            view_close.update(cx, |this, cx| {
                                this.close_er_proposal_review(window, cx);
                            });
                        }),
                )
                .child(
                    Button::new("er-proposal-adopt")
                        .small()
                        .primary()
                        .cursor_pointer()
                        .disabled(selected.is_empty() || loading)
                        .child(if loading {
                            tr("app.ai.er_proposal.adopting").to_string()
                        } else {
                            tr("app.ai.er_proposal.adopt_selected")
                                .replace("%{count}", &selected.len().to_string())
                        })
                        .on_click(move |_, window, cx| {
                            view_adopt.update(cx, |this, cx| {
                                this.adopt_selected_er_proposals(window, cx);
                            });
                        }),
                ),
        )
}

fn er_proposal_dialog_body(
    batch: &ErProposalBatchView,
    selected: &BTreeSet<String>,
    active: Option<&str>,
    error: Option<String>,
    views: &ErProposalDialogViews,
) -> gpui::AnyElement {
    let colors = views.colors;
    let list = div()
        .w(px(535.))
        .h(px(530.))
        .flex_none()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(tr("app.ai.er_proposal.select_hint")),
        )
        .child(
            div()
                .id("er-proposal-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .children(batch.items.iter().enumerate().map(|(index, item)| {
                    let item_id = item.item_id.clone();
                    let is_active = active == Some(item_id.as_str());
                    let is_selected = selected.contains(&item_id);
                    let selectable = item.selectable();
                    let view = views.view.clone();
                    let click_id = item_id.clone();
                    div()
                        .id(SharedString::from(format!("er-proposal-row:{item_id}")))
                        .aria_label(format!(
                            "{} → {} · {} · {}",
                            item.left,
                            item.right,
                            item.role,
                            status_label(item)
                        ))
                        .debug_selector({
                            let id = item_id.clone();
                            move || format!("er-proposal-row:{id}")
                        })
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1p5()
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(if is_active {
                            colors.info.alpha(0.55)
                        } else {
                            colors.border_soft
                        })
                        .bg(if is_active {
                            colors.tree_selected
                        } else if is_selected {
                            colors.success.alpha(0.08)
                        } else {
                            colors.panel_bg
                        })
                        .when(!selectable, |row| row.opacity(0.62))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    let mut data = this.er_proposal_modal.borrow_mut();
                                    data.active_item = Some(click_id.clone());
                                    let selectable = data
                                        .batch
                                        .as_ref()
                                        .and_then(|batch| batch.items.iter().find(|i| i.item_id == click_id))
                                        .is_some_and(ErProposalItemView::selectable);
                                    if selectable && !data.selected.remove(&click_id) {
                                        data.selected.insert(click_id.clone());
                                    }
                                    cx.notify();
                                });
                                cx.stop_propagation();
                            },
                        )
                        .child(
                            div()
                                .flex_none()
                                .size(px(17.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(if is_selected {
                                    colors.success
                                } else {
                                    colors.border
                                })
                                .bg(if is_selected {
                                    colors.success
                                } else {
                                    colors.panel_bg
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(is_selected, |check| {
                                    check.child(app_icon(AppIcon::Check, 11., colors.panel_bg))
                                })
                                .when(!selectable, |check| {
                                    check.child(
                                        div()
                                            .text_size(ui_text_size(10.))
                                            .text_color(colors.muted)
                                            .child("—"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(1.))
                                .child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .text_color(colors.text)
                                        .truncate()
                                        .child(format!(
                                            "{} → {}",
                                            item.left, item.right
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_size(ui_text_size(10.))
                                        .text_color(colors.muted)
                                        .child(item.role.clone())
                                        .child("·")
                                        .child(status_label(item)),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(ui_text_size(9.))
                .text_color(status_color(item))
                                .child(format!("#{}", index + 1)),
                        )
                })),
        );
    let detail = er_proposal_detail(batch, active, colors);
    div()
        .w_full()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .flex()
                .min_h(px(530.))
                .child(list)
                .child(detail),
        )
        .when_some(error, |body, error| {
            body.child(
                div()
                    .mx_3()
                    .mb_2()
                    .rounded(colors.radius)
                    .bg(colors.danger.alpha(0.08))
                    .px_3()
                    .py_2()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.danger)
                    .child(error),
            )
        })
        .into_any_element()
}

fn status_color(item: &ErProposalItemView) -> gpui::Rgba {
    match item.status.as_str() {
        "ready" => item.warning_color(),
        "adopted" => item.good_color(),
        "duplicate_existing" | "duplicate_batch" | "rejected_existing" => item.muted_color(),
        "invalid" => item.bad_color(),
        _ => item.muted_color(),
    }
}

fn status_label(item: &ErProposalItemView) -> SharedString {
    match item.status.as_str() {
        "ready" => tr("app.ai.er_proposal.status.ready"),
        "adopted" => tr("app.ai.er_proposal.status.adopted"),
        "duplicate_existing" | "duplicate_batch" => {
            tr("app.ai.er_proposal.status.duplicate")
        }
        "rejected_existing" => tr("app.ai.er_proposal.status.rejected"),
        "invalid" => tr("app.ai.er_proposal.status.invalid"),
        _ => tr("app.ai.er_proposal.status.pending"),
    }
}

impl ErProposalItemView {
    fn warning_color(&self) -> gpui::Rgba {
        if self.warnings.is_empty() {
            rgb(0x2f8f66)
        } else {
            rgb(0xc27b1e)
        }
    }
    fn good_color(&self) -> gpui::Rgba {
        rgb(0x2f8f66)
    }
    fn muted_color(&self) -> gpui::Rgba {
        rgb(0x7b8781)
    }
    fn bad_color(&self) -> gpui::Rgba {
        rgb(0xb64f43)
    }
}

fn er_proposal_detail(
    batch: &ErProposalBatchView,
    active: Option<&str>,
    colors: UiColors,
) -> gpui::AnyElement {
    let Some(item) = batch.active(active) else {
        return div()
            .flex_1()
            .min_w(px(0.))
            .p_4()
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(tr("app.ai.er_proposal.select_item_hint"))
            .into_any_element();
    };
    let source_status = if item.selectable() {
        tr("app.ai.er_proposal.not_written")
    } else {
        tr("app.ai.er_proposal.status.invalid")
    };
    let mut body = div()
        .flex_1()
        .min_w(px(0.))
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .truncate()
                        .child(format!("{} → {}", item.left, item.right)),
                )
                .child(
                    div()
                        .flex_none()
                        .px_1p5()
                        .py(px(2.))
                        .rounded(px(999.))
                        .bg(if item.status == "ready" {
                            colors.success.alpha(0.12)
                        } else {
                            colors.panel_alt
                        })
                        .text_size(ui_text_size(9.))
                        .text_color(status_color(item))
                        .child(status_label(item).to_string()),
                ),
        )
        .child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(
                    tr("app.ai.er_proposal.source_status")
                        .replace("%{role}", &item.role)
                        .replace("%{status}", source_status.as_ref()),
                ),
        );
    if let Some(relationship) = &item.relationship {
        body = body
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.muted)
                            .child(tr("app.ai.er_proposal.column_pairs")),
                    )
                    .children(relationship.column_pairs.iter().map(|pair| {
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.text)
                            .child(format!(
                                "{} = {}",
                                er_column_label(&pair.left_column),
                                er_column_label(&pair.right_column)
                            ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.muted)
                            .child(tr("app.ai.er_proposal.cardinality")),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.text)
                            .child(format!(
                                "{} : {}（{}）",
                                er_review_cardinality_label(
                                    &relationship.match_cardinality.left_to_right
                                ),
                                er_review_cardinality_label(
                                    &relationship.match_cardinality.right_to_left
                                ),
                                er_review_cardinality_basis_label(
                                    relationship.match_cardinality.basis
                                )
                            )),
                    ),
            )
            .when(!relationship.required_filters.is_empty(), |body| {
                body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(ui_text_size(10.))
                                .text_color(colors.muted)
                                .child(tr("app.ai.er_proposal.required_filters")),
                        )
                        .children(relationship.required_filters.iter().map(|filter| {
                            div()
                                .text_size(ui_text_size(10.))
                                .text_color(colors.text)
                                .child(format!(
                                    "{} {} {}",
                                    er_column_label(&filter.column_id),
                                    er_review_filter_op_label(&filter.op),
                                    er_review_literal_label(&filter.literal)
                                ))
                        })),
                )
            })
            .when_some(relationship.description.clone(), |body, description| {
                body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(ui_text_size(10.))
                                .text_color(colors.muted)
                                .child(tr("app.ai.er_proposal.rationale")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(10.))
                                .text_color(colors.text)
                                .child(description),
                        ),
                )
            });
    } else if let Some(message) = &item.message {
        body = body.child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(message.clone()),
        );
    }
    if !item.warnings.is_empty() {
        body = body.child(
            div()
                .rounded(colors.radius)
                .bg(colors.warning.alpha(0.08))
                .px_2()
                .py_2()
                .text_size(ui_text_size(10.))
                .text_color(colors.warning)
                .child(
                    tr("app.ai.er_proposal.warning")
                        .replace("%{items}", &item.warnings.join(", ")),
                ),
        );
    }
    body.child(
        div()
            .mt_auto()
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border_soft)
            .bg(colors.panel_alt)
            .px_2()
            .py_2()
            .text_size(ui_text_size(9.))
            .text_color(colors.muted)
            .child(tr("app.ai.er_proposal.adopt_note")),
    )
    .into_any_element()
}

fn er_column_label(column_id: &str) -> String {
    let (entity, column) = column_id.rsplit_once("::").unwrap_or((column_id, ""));
    if column.is_empty() {
        column_id.to_string()
    } else {
        let table = fluxdb_core::er_entity_id_parts(entity)
            .map(|(_, schema, name)| {
                schema
                    .map(|schema| format!("{schema}.{name}"))
                    .unwrap_or(name)
            })
            .unwrap_or_else(|| entity.to_string());
        format!("{table}.{column}")
    }
}

fn er_review_cardinality_label(cardinality: &fluxdb_core::ErCardinality) -> &'static str {
    match cardinality.max {
        fluxdb_core::ErCardinalityBound::Zero => "0",
        fluxdb_core::ErCardinalityBound::One => "1",
        fluxdb_core::ErCardinalityBound::Many => "N",
        fluxdb_core::ErCardinalityBound::Unknown => "?",
    }
}

fn er_review_cardinality_basis_label(basis: fluxdb_core::ErCardinalityBasis) -> SharedString {
    match basis {
        fluxdb_core::ErCardinalityBasis::DatabaseConstraint => {
            tr("app.ai.er_proposal.basis.database_constraint")
        }
        fluxdb_core::ErCardinalityBasis::UserAssertion => {
            tr("app.ai.er_proposal.basis.user_assertion")
        }
        fluxdb_core::ErCardinalityBasis::ProfileObservation => {
            tr("app.ai.er_proposal.basis.profile_observation")
        }
        fluxdb_core::ErCardinalityBasis::Unknown => tr("app.ai.er_proposal.basis.unknown"),
    }
}

fn er_review_filter_op_label(op: &fluxdb_core::ErFilterOp) -> &'static str {
    match op {
        fluxdb_core::ErFilterOp::Eq => "=",
        fluxdb_core::ErFilterOp::Ne => "!=",
        fluxdb_core::ErFilterOp::IsNull => "IS NULL",
        fluxdb_core::ErFilterOp::IsNotNull => "IS NOT NULL",
        fluxdb_core::ErFilterOp::In => "IN",
    }
}

fn er_review_literal_label(literal: &fluxdb_core::ErLiteral) -> String {
    match literal {
        fluxdb_core::ErLiteral::Text(value) => format!("'{value}'"),
        fluxdb_core::ErLiteral::Int(value) => value.to_string(),
        fluxdb_core::ErLiteral::Float(value) => value.to_string(),
        fluxdb_core::ErLiteral::Bool(value) => value.to_string(),
        fluxdb_core::ErLiteral::Null => "NULL".to_string(),
    }
}
