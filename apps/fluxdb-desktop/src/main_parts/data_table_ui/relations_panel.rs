// 右侧外键关系面板：当前行的引用（父行）与被引用（子行）关系。
//
// 关系边来自整库外键索引（与 ER 图共享缓存）。这里不读库、不拼 SQL，
// 点击条目只是把关系列值转成目标页的等值筛选条件交给已有数据页打开流程。

/// 面板中的一个可点击关系。
struct DataRelationLink {
    /// 约束名，作为分组/去重身份。
    name: String,
    /// 目标表展示名。
    table: String,
    /// 关系列与当前行值的说明。
    detail: String,
    target: ObjectPath,
    /// 目标页筛选条件；空表示当前行含 NULL，无法定位目标行。
    filters: Vec<(String, CellValue)>,
    /// 复合外键所有列都必须有值；任一列为 NULL 后不再接受后续列。
    filters_complete: bool,
}

fn data_relations_panel(
    tab_id: TabId,
    object: &ObjectPath,
    page: &DataPage,
    data: DataRelationsPanelData<'_>,
    width: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .relative()
        .w(px(width))
        .min_w(px(width))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(data_relations_panel_resize_handle(tab_id, width, colors, cx))
        .child(data_relations_panel_header(tab_id, colors, cx))
        .child(data_relations_panel_body(
            tab_id, object, page, data, colors, cx,
        ))
}

/// 面板渲染所需的数据快照；集中传参，避免继续膨胀位置参数。
struct DataRelationsPanelData<'a> {
    selected_row: Option<usize>,
    edges: Option<&'a [fluxdb_core::ErForeignKeyEdge]>,
    logical_relationships: Option<&'a [fluxdb_core::ErRelationship]>,
    /// 已展开条目的预览结果（键 = 标签页 + 链接身份）。
    previews: &'a BTreeMap<(TabId, String), RelationPreviewState>,
    expanded: &'a BTreeSet<(TabId, String)>,
    loading: bool,
    failed: bool,
}

/// 面板左缘的宽度拖拽条：只改当前标签页宽度，不影响表格列的既有宽度记忆。
fn data_relations_panel_resize_handle(
    tab_id: TabId,
    width: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("data-relations-resize", tab_id.0))
        .absolute()
        .left(px(-3.))
        .top_0()
        .bottom_0()
        .w(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_ew_resize()
        .hover(|style| style.bg(hsla(212. / 360., 0.92, 0.58, 0.16)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.data_relation_panel_resize_start = Some(DataRelationPanelResizeStart {
                    tab_id,
                    x: f32::from(event.position.x),
                    width,
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(DataRelationPanelResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &gpui::DragMoveEvent<DataRelationPanelResizeDrag>, _, cx| {
                if let Some(start) = this.data_relation_panel_resize_start
                    && start.tab_id == tab_id
                {
                    // 面板在右侧：指针左移（delta 为负）= 面板变宽。
                    let delta = f32::from(event.event.position.x) - start.x;
                    let next = (start.width - delta).clamp(
                        DATA_RELATION_PANEL_MIN_WIDTH,
                        DATA_RELATION_PANEL_MAX_WIDTH,
                    );
                    this.data_relation_panel_widths.insert(tab_id, next);
                    cx.notify();
                }
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.data_relation_panel_resize_start = None;
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(1.))
                .h_full()
                .bg(colors.border_soft)
                .hover(|style| style.bg(rgb(0x3478f6))),
        )
}

fn data_relations_panel_header(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(40.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(app_icon(AppIcon::Link, 14., colors.muted))
                .child(tr("app.data.relations.title")),
        )
        .child(
            div()
                .size(px(22.))
                .rounded(colors.radius)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 13., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_data_relation_panel(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn data_relations_panel_body(
    tab_id: TabId,
    object: &ObjectPath,
    page: &DataPage,
    data: DataRelationsPanelData<'_>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let DataRelationsPanelData {
        selected_row,
        edges,
        logical_relationships,
        previews,
        expanded,
        loading,
        failed,
    } = data;
    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_2()
        .p_3();
    let Some(row) = selected_row.and_then(|row| page.rows.get(row)) else {
        return body.child(data_relations_hint(
            tr("app.data.relations.select_row"),
            colors,
        ));
    };
    let foreign_key_columns = local_foreign_key_columns(object, edges, logical_relationships);
    body = body.child(data_relations_row_summary(
        &object.name,
        page,
        row,
        &foreign_key_columns,
        colors,
    ));
    let logical_links = logical_relationships
        .map(|relationships| logical_relation_links(object, page, row, relationships))
        .unwrap_or_default();
    let mut parent_links = Vec::new();
    let mut child_links = Vec::new();
    let mut has_physical = false;
    match edges {
        Some(edges) => {
            let (parents, children) = data_relation_links(object, page, row, edges);
            has_physical = !parents.is_empty() || !children.is_empty();
            parent_links = parents;
            child_links = children;
        }
        None if logical_links.is_empty() => {
            return body.child(data_relations_hint(
                if failed {
                    tr("app.data.relations.load_failed")
                } else if loading {
                    tr("app.data.relations.loading")
                } else {
                    tr("app.data.relations.load_failed")
                },
                colors,
            ));
        }
        None => {}
    }
    let has_logical = !logical_links.is_empty();
    if !has_physical && !has_logical {
        return body.child(data_relations_hint(
            tr("app.data.relations.empty"),
            colors,
        ));
    }
    let panel = RelationsPanelRender {
        tab_id,
        object,
        previews,
        expanded,
        colors,
    };
    if !parent_links.is_empty() {
        let count = tr("app.data.relations.parent_count")
            .replace("%{count}", &parent_links.len().to_string());
        body = body.child(panel.section(tr("app.data.relations.parents"), count, parent_links, cx));
    }
    if !child_links.is_empty() {
        let count = tr("app.data.relations.child_count")
            .replace("%{count}", &child_links.len().to_string());
        body = body.child(panel.section(tr("app.data.relations.children"), count, child_links, cx));
    }
    if has_logical {
        let count = tr("app.data.relations.logical_count")
            .replace("%{count}", &logical_links.len().to_string());
        body = body.child(panel.section(
            tr("app.data.relations.logical"),
            count,
            logical_links,
            cx,
        ));
    }
    body = body.child(panel.er_mini_section(edges, logical_relationships));
    body
}


fn data_relations_hint(text: SharedString, colors: UiColors) -> Div {
    div()
        .px_2()
        .py_3()
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(text)
}

/// 当前表自身的外键列名：摘要里跳过这些列，避免与外键入口重复。
fn local_foreign_key_columns(
    object: &ObjectPath,
    edges: Option<&[fluxdb_core::ErForeignKeyEdge]>,
    relationships: Option<&[fluxdb_core::ErRelationship]>,
) -> Vec<String> {
    let mut columns = Vec::new();
    if let Some(edges) = edges {
        for edge in edges {
            if er_reference_matches_object(&edge.from_reference, object) {
                columns.push(edge.from_column.clone());
            }
        }
    }
    if let Some(relationships) = relationships {
        for relationship in relationships {
            if matches!(
                relationship.validity.state,
                fluxdb_core::ErValidityState::Invalid
            ) {
                continue;
            }
            let Some(left) = er_reference_from_entity(&relationship.left_entity) else {
                continue;
            };
            let Some(right) = er_reference_from_entity(&relationship.right_entity) else {
                continue;
            };
            for pair in &relationship.column_pairs {
                if er_reference_matches_object(&left, object) {
                    columns.push(fluxdb_core::er_column_id_name(&pair.left_column).to_string());
                } else if er_reference_matches_object(&right, object) {
                    columns.push(fluxdb_core::er_column_id_name(&pair.right_column).to_string());
                }
            }
        }
    }
    columns
}

/// 当前行摘要：表名 + 主键 + 最多 3 个普通字段，和原型的信息密度一致。
fn data_relations_row_summary(
    table: &str,
    page: &DataPage,
    row: &Row,
    foreign_key_columns: &[String],
    colors: UiColors,
) -> Div {
    let primary_key = page
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .filter_map(|(index, _)| row.values.get(index).map(CellValue::display_label))
        .collect::<Vec<_>>();
    // 主键与外键已在别处展示，摘要只挑普通字段，避免重复信息。
    let meta = page
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| !column.primary_key)
        .filter(|(_, column)| !foreign_key_columns.iter().any(|name| name == &column.name))
        .filter(|(index, _)| {
            row.values
                .get(*index)
                .is_some_and(|value| !matches!(value, CellValue::Null))
        })
        .take(3)
        .filter_map(|(index, column)| {
            row.values
                .get(index)
                .map(|value| (column.name.clone(), value.display_label()))
        })
        .collect::<Vec<_>>();
    let head = div()
        .flex()
        .items_center()
        .gap_2()
        .min_w(px(0.))
        .child(
            div()
                .text_size(ui_text_size(12.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(table.to_string()),
        )
        .when(!primary_key.is_empty(), |this| {
            this.child(
                div()
                    .text_size(ui_text_size(11.5))
                    .text_color(colors.muted)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!("#{}", primary_key.join(" · "))),
            )
        });
    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .px_2()
        .py_1p5()
        .flex()
        .flex_col()
        .gap_1()
        .child(head)
        .when(!meta.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_3()
                    .gap_y_1()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .children(meta.into_iter().map(|(name, value)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w(px(0.))
                            .child(name)
                            .child(
                                div()
                                    .text_color(colors.text)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(value),
                            )
                    })),
            )
        })
}

/// 面板渲染上下文：统一持有当前行、预览缓存与主题，避免逐函数透传。
struct RelationsPanelRender<'a> {
    tab_id: TabId,
    object: &'a ObjectPath,
    previews: &'a BTreeMap<(TabId, String), RelationPreviewState>,
    expanded: &'a BTreeSet<(TabId, String)>,
    colors: UiColors,
}

impl RelationsPanelRender<'_> {
    fn section(
        &self,
        title: SharedString,
        count: impl Into<SharedString>,
        links: Vec<DataRelationLink>,
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        let colors = self.colors;
        let count = count.into();
        let mut section = div().flex().flex_col().gap_1().child(
            div()
                .px_1()
                .flex()
                .items_center()
                .justify_between()
                .text_size(ui_text_size(10.5))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(colors.muted)
                .child(title)
                .child(
                    div()
                        .font_weight(gpui::FontWeight::NORMAL)
                        .child(count),
                ),
        );
        for link in links {
            section = section.child(self.link_row(link, cx));
        }
        section
    }

    fn link_row(&self, link: DataRelationLink, cx: &mut Context<NavicatMain>) -> Div {
        let colors = self.colors;
        let navigable = !link.filters.is_empty();
        // 行身份参与 key：翻页或换行后不会命中上一条行的预览结果。
        let entry = (self.tab_id, format!("{}|{}", link.name, link.detail));
        let is_open = self.expanded.contains(&entry);
        let toggle_target = link.target.clone();
        let toggle_filters = link.filters.clone();
        let toggle_entry = entry.clone();
        let open_target = link.target.clone();
        let open_filters = link.filters.clone();
        let preview = is_open.then(|| self.preview_block(&link, &entry, cx));
    // 与原型一致：预览加载完成后在标题右侧显示行数，尚未加载时不占位。
    let count_pill = match self.previews.get(&entry) {
        Some(RelationPreviewState::Loaded(page)) => Some(
            page.rows.len().to_string()
                + if page.has_more { "+" } else { "" },
        ),
        _ => None,
    };
        div()
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel_alt)
            .px_2()
            .py_1p5()
            .flex()
            .flex_col()
            .gap_1()
            .when(navigable, |this| {
                this.cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.open_related_table(open_target.clone(), open_filters.clone(), cx);
                            cx.stop_propagation();
                        }),
                    )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w(px(0.))
                            // NULL 行没有筛选条件，展开只会拉到无关数据，不显示开关。
                            .when(navigable, |this| this.child(
                                    div()
                                        .size(px(14.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .child(app_icon(
                                            if is_open {
                                                AppIcon::ChevronDown
                                            } else {
                                                AppIcon::ChevronRight
                                            },
                                            11.,
                                            colors.muted,
                                        ))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                this.toggle_data_relation_preview(
                                                    toggle_entry.0,
                                                    toggle_entry.1.clone(),
                                                    toggle_target.clone(),
                                                    toggle_filters.clone(),
                                                    cx,
                                                );
                                                cx.stop_propagation();
                                            }),
                                        ),
                            ))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(ui_text_size(12.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if navigable { colors.text } else { colors.muted })
                                    .child(link.table),
                            ),
                    )
                    .when_some(count_pill, |this, count| {
                        this.child(
                            div()
                                .px_1p5()
                                .rounded(px(999.))
                                .border_1()
                                .border_color(colors.border_soft)
                                .bg(colors.panel_bg)
                                .text_size(ui_text_size(10.))
                                .text_color(colors.muted)
                                .child(tr("app.data.relations.row_count").replace("%{count}", &count)),
                        )
                    })
                    .child(app_icon(
                        AppIcon::Link,
                        12.,
                        if navigable { colors.muted } else { colors.border },
                    )),
            )
            .child(
                div()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(link.detail),
            )
            .when(!navigable, |this| {
                this.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(tr("app.data.relations.null_value")),
                )
            })
            .when_some(preview, |this, preview| this.child(preview))
    }

    /// 展开条目：懒加载目标表前 50 行并显示前三行。
    fn preview_block(
        &self,
        link: &DataRelationLink,
        entry: &(TabId, String),
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        let colors = self.colors;
        let mut block = div()
            .mt_1()
            .pt_1()
            .border_t_1()
            .border_color(colors.border_soft)
            .flex()
            .flex_col()
            .gap_1()
            .text_size(ui_text_size(10.5));
        let Some(state) = self.previews.get(entry) else {
            return block.child(
                div()
                    .text_color(colors.muted)
                    .child(tr("app.data.relations.preview_loading")),
            );
        };
        match state {
            RelationPreviewState::Loading => {
                block = block.child(
                    div()
                        .text_color(colors.muted)
                        .child(tr("app.data.relations.preview_loading")),
                );
            }
            RelationPreviewState::Failed => {
                block = block.child(
                    div()
                        .text_color(colors.muted)
                        .child(tr("app.data.relations.preview_failed")),
                );
            }
            RelationPreviewState::Loaded(page) => {
                if page.rows.is_empty() {
                    block = block.child(
                        div()
                            .text_color(colors.muted)
                            .child(tr("app.data.relations.preview_empty")),
                    );
                } else {
                    block = block.child(data_relation_preview_table(page, colors));
                }
                let count = if page.has_more {
                    format!("{}+", page.rows.len())
                } else {
                    page.rows.len().to_string()
                };
                let target = link.target.clone();
                let filters = link.filters.clone();
                block = block.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_color(colors.muted)
                                .child(tr("app.data.relations.preview_count").replace("%{count}", &count)),
                        )
                        .child(
                            div()
                                .text_color(colors.muted)
                                .cursor_pointer()
                                .hover(move |style| style.text_color(colors.text))
                                .child(
                                    tr("app.data.relations.open_table")
                                        .replace("%{table}", &link.table),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.open_related_table(target.clone(), filters.clone(), cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                );
            }
        }
        block
    }

    /// ER 迷你图：父表在上、当前表居中、子表在下；实线为物理外键，虚线为 ER 设计关系。
    fn er_mini_section(
        &self,
        edges: Option<&[fluxdb_core::ErForeignKeyEdge]>,
        logical_relationships: Option<&[fluxdb_core::ErRelationship]>,
    ) -> Div {
        let colors = self.colors;
        let mut parents: Vec<ErNeighbor> = Vec::new();
        let mut children: Vec<ErNeighbor> = Vec::new();
        if let Some(edges) = edges {
            for edge in edges {
                if er_reference_matches_object(&edge.from_reference, self.object) {
                    push_er_neighbor(
                        &mut parents,
                        edge.to_table.clone(),
                        format!("{} → {}", edge.from_column, edge.to_column),
                        false,
                    );
                }
                if er_reference_matches_object(&edge.to_reference, self.object) {
                    push_er_neighbor(
                        &mut children,
                        edge.from_table.clone(),
                        format!("{} → {}", edge.from_column, edge.to_column),
                        false,
                    );
                }
            }
        }
        if let Some(relationships) = logical_relationships {
            for relationship in relationships {
                if matches!(
                    relationship.validity.state,
                    fluxdb_core::ErValidityState::Invalid
                ) {
                    continue;
                }
                let Some(left) = er_reference_from_entity(&relationship.left_entity) else {
                    continue;
                };
                let Some(right) = er_reference_from_entity(&relationship.right_entity) else {
                    continue;
                };
                let (left_columns, right_columns) = relationship
                    .column_pairs
                    .iter()
                    .map(|pair| {
                        (
                            fluxdb_core::er_column_id_name(&pair.left_column).to_string(),
                            fluxdb_core::er_column_id_name(&pair.right_column).to_string(),
                        )
                    })
                    .unzip::<_, _, Vec<_>, Vec<_>>();
                if er_reference_matches_object(&left, self.object) {
                    push_er_neighbor(
                        &mut parents,
                        right.name.clone(),
                        format!("{} → {}", left_columns.join("+"), right_columns.join("+")),
                        true,
                    );
                }
                if er_reference_matches_object(&right, self.object) {
                    push_er_neighbor(
                        &mut children,
                        left.name.clone(),
                        format!("{} → {}", right_columns.join("+"), left_columns.join("+")),
                        true,
                    );
                }
            }
        }
        if parents.is_empty() && children.is_empty() {
            return div();
        }
        let has_parent = !parents.is_empty();
        let has_child = !children.is_empty();
        let mut boxed = div()
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel_alt)
            .px_2()
            .py_3()
            .flex()
            .flex_col()
            .items_center()
            .gap_1();
        if has_parent {
            let columns = parents
                .iter()
                .map(|neighbor| neighbor.columns.clone())
                .collect::<Vec<_>>()
                .join(" · ");
            boxed = boxed
                .child(data_er_node_row(
                    parents,
                    tr("app.data.relations.er_parent_tag"),
                    colors,
                ))
                .child(data_er_connector(columns.into(), colors));
        }
        boxed = boxed.child(data_er_node(
            self.object.name.clone(),
            tr("app.data.relations.er_current_tag"),
            true,
            false,
            colors,
        ));
        if has_child {
            let columns = children
                .iter()
                .map(|neighbor| neighbor.columns.clone())
                .collect::<Vec<_>>()
                .join(" · ");
            boxed = boxed
                .child(data_er_connector(columns.into(), colors))
                .child(data_er_node_row(
                    children,
                    tr("app.data.relations.er_child_tag"),
                    colors,
                ));
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(ui_text_size(10.5))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(colors.muted)
                    .child(tr("app.data.relations.er"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .child(data_er_legend_item(
                                tr("app.data.relations.er_physical"),
                                false,
                                colors,
                            ))
                            .child(data_er_legend_item(
                                tr("app.data.relations.er_logical"),
                                true,
                                colors,
                            )),
                    ),
            )
            .child(boxed)
    }
}

/// ER 迷你图邻接节点：表名、关系列说明与来源（物理/设计）。
struct ErNeighbor {
    name: String,
    columns: String,
    logical: bool,
}

fn push_er_neighbor(
    neighbors: &mut Vec<ErNeighbor>,
    name: String,
    columns: String,
    logical: bool,
) {
    if let Some(existing) = neighbors.iter_mut().find(|neighbor| neighbor.name == name) {
        if !existing.columns.split(" · ").any(|part| part == columns) {
            existing.columns.push_str(" · ");
            existing.columns.push_str(&columns);
        }
        return;
    }
    neighbors.push(ErNeighbor {
        name,
        columns,
        logical,
    });
}

fn data_er_node_row(nodes: Vec<ErNeighbor>, side: SharedString, colors: UiColors) -> Div {
    nodes.into_iter().fold(
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap_1(),
        |row, node| {
            row.child(data_er_node(
                node.name,
                side.clone(),
                false,
                node.logical,
                colors,
            ))
        },
    )
}

fn data_er_node(
    name: String,
    sub: impl Into<SharedString>,
    current: bool,
    logical: bool,
    colors: UiColors,
) -> Div {
    let sub = sub.into();
    div()
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .items_center()
        .rounded(colors.radius)
        .border_1()
        .border_color(if current { colors.info } else { colors.border })
        .bg(if current { colors.panel_bg } else { colors.panel_alt })
        .when(logical, |this| this.border_dashed())
        .child(
            div()
                .text_size(ui_text_size(10.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if current { colors.text } else { colors.muted })
                .child(name),
        )
        .child(
            div()
                .text_size(ui_text_size(8.5))
                .text_color(colors.muted)
                .child(sub),
        )
}

/// 当前表与一侧邻居之间的连接说明：虚线表示设计关系。
fn data_er_connector(label: SharedString, colors: UiColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(1.))
                .h(px(14.))
                .border_l_1()
                .border_dashed()
                .border_color(colors.border),
        )
        .child(
            div()
                .text_size(ui_text_size(8.5))
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .w(px(1.))
                .h(px(14.))
                .border_l_1()
                .border_dashed()
                .border_color(colors.border),
        )
}

fn data_er_legend_item(label: SharedString, dashed: bool, colors: UiColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .w(px(14.))
                .h(px(0.))
                .border_t_1()
                .border_color(colors.muted)
                .when(dashed, |this| this.border_dashed()),
        )
        .child(label)
}

/// 预览前三行、前四列：足以确认方向，不把整页塞进窄面板。
fn data_relation_preview_table(page: &DataPage, colors: UiColors) -> Div {
    let column_limit = page.columns.len().min(4);
    let header = (0..column_limit).fold(
        div().flex().items_center().gap_1().pb_1(),
        |row, index| {
            row.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(colors.muted)
                    .child(page.columns[index].name.clone()),
            )
        },
    );
    let mut table = div().flex().flex_col().gap_1().child(header);
    for row in page.rows.iter().take(3) {
        let cells = (0..column_limit).fold(
            div()
                .flex()
                .items_center()
                .gap_1()
                .border_t_1()
                .border_color(colors.border_soft)
                .pt_1(),
            |cells, index| {
                let text = row
                    .values
                    .get(index)
                    .map(CellValue::display_label)
                    .unwrap_or_else(|| "NULL".to_string());
                cells.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(colors.text)
                        .child(text),
                )
            },
        );
        table = table.child(cells);
    }
    table
}

/// 把外键边按约束分组，生成当前行的父/子关系链接。
///
/// 复合外键会拆成多条边，必须按约束名 + 两端表重新聚合，否则会显示成
/// 多条互相矛盾的单列关系。
fn data_relation_links(
    object: &ObjectPath,
    page: &DataPage,
    row: &Row,
    edges: &[fluxdb_core::ErForeignKeyEdge],
) -> (Vec<DataRelationLink>, Vec<DataRelationLink>) {
    let mut parents: Vec<DataRelationLink> = Vec::new();
    let mut children: Vec<DataRelationLink> = Vec::new();
    for edge in edges {
        if er_reference_matches_object(&edge.from_reference, object) {
            push_relation_link(&mut parents, edge, page, row, true, object);
        }
        if er_reference_matches_object(&edge.to_reference, object) {
            push_relation_link(&mut children, edge, page, row, false, object);
        }
    }
    (parents, children)
}

fn push_relation_link(
    links: &mut Vec<DataRelationLink>,
    edge: &fluxdb_core::ErForeignKeyEdge,
    page: &DataPage,
    row: &Row,
    outgoing: bool,
    object: &ObjectPath,
) {
    let key = format!(
        "{}\u{1}{}\u{1}{}",
        edge.name,
        edge.from_reference.display(),
        edge.to_reference.display()
    );
    let existing = links.iter().position(|link| link.name == key);
    let target_reference = if outgoing {
        &edge.to_reference
    } else {
        &edge.from_reference
    };
    let (local_column, target_column, detail_column) = if outgoing {
        (&edge.from_column, &edge.to_column, &edge.from_column)
    } else {
        (&edge.to_column, &edge.from_column, &edge.from_column)
    };
    let value = data_page_cell_value(page, row, local_column);
    let detail = format!(
        "{detail_column} = {}",
        value
            .map(CellValue::display_label)
            .unwrap_or_else(|| "NULL".to_string())
    );
    let filter = value
        .filter(|value| !matches!(value, CellValue::Null))
        .map(|value| (target_column.clone(), value.clone()));
    let target = ObjectPath {
        connection_id: object.connection_id,
        database: (!target_reference.database.is_empty())
            .then(|| target_reference.database.clone())
            .or_else(|| object.database.clone()),
        schema: target_reference.schema.clone(),
        name: target_reference.name.clone(),
        kind: ObjectKind::Table,
    };
    if let Some(index) = existing {
        let link = &mut links[index];
        link.detail.push_str(" · ");
        link.detail.push_str(&detail);
        match filter {
            Some(filter) if link.filters_complete => link.filters.push(filter),
            Some(_) => {}
            None => {
                link.filters.clear();
                link.filters_complete = false;
            }
        }
        return;
    }
    let filters_complete = filter.is_some();
    links.push(DataRelationLink {
        name: key,
        table: target_reference.display(),
        detail,
        target,
        filters: filter.into_iter().collect(),
        filters_complete,
    });
}

/// ER 逻辑关系没有物理外键方向，当前行两侧都可作为起点。
///
/// 每条关系只渲染一条链接：当前表在左端就跳到右表，在右端就跳到左表；
/// 自关联（左右同名表）两侧各生成一条，避免丢方向。
fn logical_relation_links(
    object: &ObjectPath,
    page: &DataPage,
    row: &Row,
    relationships: &[fluxdb_core::ErRelationship],
) -> Vec<DataRelationLink> {
    let mut links = Vec::new();
    for relationship in relationships {
        if matches!(
            relationship.validity.state,
            fluxdb_core::ErValidityState::Invalid
        ) {
            continue;
        }
        let Some(left) = er_reference_from_entity(&relationship.left_entity) else {
            continue;
        };
        let Some(right) = er_reference_from_entity(&relationship.right_entity) else {
            continue;
        };
        if er_reference_matches_object(&left, object) {
            push_logical_relation_link(
                &mut links, relationship, &right, true, object, page, row, "left",
            );
        }
        if er_reference_matches_object(&right, object) {
            push_logical_relation_link(
                &mut links, relationship, &left, false, object, page, row, "right",
            );
        }
    }
    links
}

fn push_logical_relation_link(
    links: &mut Vec<DataRelationLink>,
    relationship: &fluxdb_core::ErRelationship,
    target: &fluxdb_core::ErTableRef,
    local_is_left: bool,
    object: &ObjectPath,
    page: &DataPage,
    row: &Row,
    side_tag: &str,
) {
    let mut details = Vec::new();
    let mut filters = Vec::new();
    let mut filters_complete = true;
    for pair in &relationship.column_pairs {
        let (local_id, target_id) = if local_is_left {
            (&pair.left_column, &pair.right_column)
        } else {
            (&pair.right_column, &pair.left_column)
        };
        let local_column = fluxdb_core::er_column_id_name(local_id);
        let target_column = fluxdb_core::er_column_id_name(target_id);
        let value = data_page_cell_value(page, row, local_column);
        details.push(format!(
            "{local_column} = {}",
            value
                .map(CellValue::display_label)
                .unwrap_or_else(|| "NULL".to_string())
        ));
        match value.filter(|value| !matches!(value, CellValue::Null)) {
            Some(value) if filters_complete => {
                filters.push((target_column.to_string(), value.clone()));
            }
            Some(_) => {}
            None => {
                // 复合关系任一列为 NULL 就无法唯一定位目标行，整条降级为不可跳转。
                filters.clear();
                filters_complete = false;
            }
        }
    }
    links.push(DataRelationLink {
        name: format!("logic:{}:{side_tag}", relationship.id),
        table: target.display(),
        detail: details.join(" · "),
        target: object_path_for_reference(object, target),
        filters,
        filters_complete,
    });
}

/// 由 ER 关系端点 ID（`database:schema:table`）还原表身份。
fn er_reference_from_entity(entity_id: &str) -> Option<fluxdb_core::ErTableRef> {
    let (database, schema, name) = fluxdb_core::er_entity_id_parts(entity_id)?;
    Some(fluxdb_core::ErTableRef {
        database,
        schema,
        name,
    })
}

/// 关系目标端的表路径：沿用当前连接的库，目标端带库/schema 时以其为准。
fn object_path_for_reference(
    object: &ObjectPath,
    reference: &fluxdb_core::ErTableRef,
) -> ObjectPath {
    ObjectPath {
        connection_id: object.connection_id,
        database: (!reference.database.is_empty())
            .then(|| reference.database.clone())
            .or_else(|| object.database.clone()),
        schema: reference.schema.clone(),
        name: reference.name.clone(),
        kind: ObjectKind::Table,
    }
}

fn data_page_cell_value<'a>(
    page: &'a DataPage,
    row: &'a Row,
    column: &str,
) -> Option<&'a CellValue> {
    page.columns
        .iter()
        .position(|candidate| candidate.name == column)
        .and_then(|index| row.values.get(index))
}

fn er_reference_matches_object(
    reference: &fluxdb_core::ErTableRef,
    object: &ObjectPath,
) -> bool {
    reference.name == object.name
        && object
            .schema
            .as_ref()
            .is_none_or(|schema| reference.schema.as_ref() == Some(schema))
        && object
            .database
            .as_ref()
            .is_none_or(|database| &reference.database == database)
}

#[cfg(test)]
mod data_relation_link_tests {
    use super::*;
    use fluxdb_core::{ErForeignKeyEdge, ErTableRef};

    fn object() -> ObjectPath {
        object_named("orders")
    }

    fn object_named(name: &str) -> ObjectPath {
        ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("shop".to_string()),
            schema: None,
            name: name.to_string(),
            kind: ObjectKind::Table,
        }
    }

    fn reference(name: &str) -> ErTableRef {
        ErTableRef {
            database: "shop".to_string(),
            schema: None,
            name: name.to_string(),
        }
    }

    fn page() -> DataPage {
        DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: false,
                    primary_key: true,
                    comment: None,
                },
                GdbColumn {
                    name: "user_id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "coupon_id".to_string(),
                    type_name: Some("int".to_string()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "country_id".to_string(),
                    type_name: Some("varchar".to_string()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![Row {
                values: vec![
                    CellValue::I64(1024),
                    CellValue::I64(3),
                    CellValue::Null,
                    CellValue::Text("AG".to_string()),
                ],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
            incomplete: None,
        }
    }

    fn edge(name: &str, from: &str, from_column: &str, to: &str, to_column: &str) -> ErForeignKeyEdge {
        ErForeignKeyEdge {
            name: name.to_string(),
            from_table: from.to_string(),
            from_column: from_column.to_string(),
            to_table: to.to_string(),
            to_column: to_column.to_string(),
            from_reference: reference(from),
            to_reference: reference(to),
        }
    }

    #[test]
    fn outgoing_foreign_key_becomes_parent_link() {
        let page = page();
        let row = &page.rows[0];
        let edges = vec![edge("fk_user", "orders", "user_id", "users", "id")];

        let (parents, children) = data_relation_links(&object(), &page, row, &edges);

        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].table, "users");
        assert_eq!(parents[0].filters, vec![("id".to_string(), CellValue::I64(3))]);
        assert!(children.is_empty());
    }

    #[test]
    fn incoming_foreign_key_becomes_child_link() {
        let page = page();
        let row = &page.rows[0];
        let edges = vec![edge("fk_order", "payments", "order_id", "orders", "id")];

        let (parents, children) = data_relation_links(&object(), &page, row, &edges);

        assert!(parents.is_empty());
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].table, "payments");
        assert_eq!(
            children[0].filters,
            vec![("order_id".to_string(), CellValue::I64(1024))]
        );
    }

    #[test]
    fn composite_foreign_key_groups_and_drops_null_pairs() {
        let page = page();
        let row = &page.rows[0];
        // 两列同属一个约束：user_id 有值，coupon_id 为 NULL，整组不可跳转。
        let edges = vec![
            edge("fk_pair", "orders", "user_id", "users", "id"),
            edge("fk_pair", "orders", "coupon_id", "users", "coupon_id"),
        ];

        let (parents, _) = data_relation_links(&object(), &page, row, &edges);

        assert_eq!(parents.len(), 1);
        assert!(parents[0].filters.is_empty());
    }

    fn logical_relationship(
        id: &str,
        left: &str,
        left_column: &str,
        right: &str,
        right_column: &str,
    ) -> fluxdb_core::ErRelationship {
        let left_reference = reference(left);
        let right_reference = reference(right);
        fluxdb_core::ErRelationship {
            id: id.to_string(),
            revision: 1,
            left_entity: fluxdb_core::er_entity_id(&left_reference),
            right_entity: fluxdb_core::er_entity_id(&right_reference),
            role: "rel".to_string(),
            column_pairs: vec![fluxdb_core::ErColumnPair {
                left_column: fluxdb_core::er_column_id(&left_reference, left_column),
                right_column: fluxdb_core::er_column_id(&right_reference, right_column),
            }],
            required_filters: Vec::new(),
            match_cardinality: fluxdb_core::ErMatchCardinality {
                left_to_right: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Unknown,
                },
                right_to_left: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Unknown,
                },
                basis: fluxdb_core::ErCardinalityBasis::Unknown,
            },
            origin: fluxdb_core::ErRelationshipOrigin::User,
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
            description: None,
            evidence_refs: Vec::new(),
        }
    }

    #[test]
    fn logical_relationship_left_side_targets_right_table() {
        let page = page();
        let relationships = vec![logical_relationship(
            "rel_country",
            "orders",
            "country_id",
            "countries",
            "id",
        )];

        let links = logical_relation_links(&object(), &page, &page.rows[0], &relationships);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].table, "countries");
        assert_eq!(
            links[0].filters,
            vec![("id".to_string(), CellValue::Text("AG".to_string()))]
        );
    }

    #[test]
    fn logical_relationship_right_side_targets_left_table() {
        let page = page();
        let relationships = vec![logical_relationship(
            "rel_country",
            "orders",
            "country_id",
            "countries",
            "id",
        )];

        let links = logical_relation_links(
            &object_named("countries"),
            &page,
            &page.rows[0],
            &relationships,
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].table, "orders");
        assert_eq!(
            links[0].filters,
            vec![("country_id".to_string(), CellValue::I64(1024))]
        );
    }
}
