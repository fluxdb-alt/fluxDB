// ER 本地逻辑关系表单状态、选项与提交校验。沿用 main.rs 的 include! 同作用域。

fn er_table_options(
    tables: &[fluxdb_core::ErTableNode],
) -> Vec<ErRelationshipSelectOption> {
    tables
        .iter()
        .map(|table| ErRelationshipSelectOption {
            id: er_entity_id(&table.reference),
            label: table.reference.display(),
        })
        .collect()
}

fn er_column_options(
    table: Option<&fluxdb_core::ErTableNode>,
) -> Vec<ErRelationshipSelectOption> {
    table
        .into_iter()
        .flat_map(|table| {
            table.columns.iter().map(|column| ErRelationshipSelectOption {
                id: er_column_id(&table.reference, &column.name),
                label: column.name.clone(),
            })
        })
        .collect()
}

fn er_filter_op_id(op: &fluxdb_core::ErFilterOp) -> &'static str {
    match op {
        fluxdb_core::ErFilterOp::Eq => "eq",
        fluxdb_core::ErFilterOp::Ne => "ne",
        fluxdb_core::ErFilterOp::IsNull => "is_null",
        fluxdb_core::ErFilterOp::IsNotNull => "is_not_null",
        fluxdb_core::ErFilterOp::In => "in",
    }
}

fn er_literal_text(literal: &fluxdb_core::ErLiteral) -> String {
    match literal {
        fluxdb_core::ErLiteral::Text(value) => value.clone(),
        fluxdb_core::ErLiteral::Int(value) => value.to_string(),
        fluxdb_core::ErLiteral::Float(value) => value.to_string(),
        fluxdb_core::ErLiteral::Bool(value) => value.to_string(),
        fluxdb_core::ErLiteral::Null => String::new(),
    }
}

/// 基数选择选项（1:1 / 1:N / N:1 / N:N / 未知）。
fn er_cardinality_options() -> Vec<ErRelationshipSelectOption> {
    vec![
        ErRelationshipSelectOption { id: "1_1".into(), label: "1 对 1".into() },
        ErRelationshipSelectOption { id: "1_n".into(), label: "1 对多（1:N）".into() },
        ErRelationshipSelectOption { id: "n_1".into(), label: "多对 1（N:1）".into() },
        ErRelationshipSelectOption { id: "n_n".into(), label: "多对多（N:N）".into() },
        ErRelationshipSelectOption { id: "unknown".into(), label: "未知（不声明）".into() },
    ]
}

/// 由匹配基数推导选择框 id（未知任一向 → unknown；其余按 max 映射）。
fn er_cardinality_to_option_id(
    card: &fluxdb_core::ErMatchCardinality,
) -> &'static str {
    use fluxdb_core::ErCardinalityBound::{Many, One, Unknown, Zero};
    // max 为 One 或 Zero（0..1）都按「单条」归到 One；Unknown 视为未声明。
    let (l2r, r2l) = (card.left_to_right.max, card.right_to_left.max);
    match (l2r, r2l) {
        (One, One) | (Zero, One) | (One, Zero) | (Zero, Zero) => "1_1",
        (Many, One) | (Many, Zero) => "1_n",
        (One, Many) | (Zero, Many) => "n_1",
        (Many, Many) => "n_n",
        (Unknown, _) | (_, Unknown) => "unknown",
    }
}

/// 基数选项的中文展示（方向文案用）。
fn er_cardinality_label(id: &str) -> &'static str {
    match id {
        "1_1" => "一对一",
        "1_n" => "一对多",
        "n_1" => "多对一",
        "n_n" => "多对多",
        _ => "未声明匹配数量",
    }
}

/// 由选择框 id 生成匹配基数（新增/编辑表单；basis=UserAssertion，未知不声明）。
fn er_option_id_to_cardinality(
    id: &str,
) -> fluxdb_core::ErMatchCardinality {
    use fluxdb_core::ErCardinalityBound::{Many, One, Unknown};
    let (l2r, r2l) = match id {
        "1_1" => (One, One),
        "1_n" => (Many, One),
        "n_1" => (One, Many),
        "n_n" => (Many, Many),
        _ => (Unknown, Unknown),
    };
    fluxdb_core::ErMatchCardinality {
        left_to_right: fluxdb_core::ErCardinality { min: Unknown, max: l2r },
        right_to_left: fluxdb_core::ErCardinality { min: Unknown, max: r2l },
        basis: if id == "unknown" {
            fluxdb_core::ErCardinalityBasis::Unknown
        } else {
            fluxdb_core::ErCardinalityBasis::UserAssertion
        },
    }
}

fn er_parse_literal(text: &str) -> Option<fluxdb_core::ErLiteral> {
    if text.is_empty() {
        return None;
    }
    if text.eq_ignore_ascii_case("true") {
        return Some(fluxdb_core::ErLiteral::Bool(true));
    }
    if text.eq_ignore_ascii_case("false") {
        return Some(fluxdb_core::ErLiteral::Bool(false));
    }
    if let Ok(value) = text.parse::<i64>() {
        return Some(fluxdb_core::ErLiteral::Int(value));
    }
    if let Ok(value) = text.parse::<f64>() {
        return Some(fluxdb_core::ErLiteral::Float(value));
    }
    Some(fluxdb_core::ErLiteral::Text(text.to_string()))
}

impl NavicatMain {
    fn er_form_table_by_id(
        &self,
        tab_id: TabId,
        entity_id: Option<&String>,
    ) -> Option<fluxdb_core::ErTableNode> {
        let entity_id = entity_id?;
        self.er_full_tables
            .get(&tab_id)?
            .iter()
            .find(|table| er_entity_id(&table.reference) == *entity_id)
            .cloned()
    }

    fn er_refresh_select_options(
        select: &Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
        options: Vec<ErRelationshipSelectOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = select.read(cx).selected_value().cloned();
        let selected_is_valid = selected
            .as_ref()
            .is_some_and(|id| options.iter().any(|option| &option.id == id));
        select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(options), window, cx);
            if selected_is_valid {
                if let Some(selected) = &selected {
                    state.set_selected_value(selected, window, cx);
                }
            } else {
                state.set_selected_index(None, window, cx);
            }
        });
    }

    fn ensure_er_relationship_form_controls(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tables = self.er_full_tables.get(&tab_id).cloned().unwrap_or_default();
        let table_options = er_table_options(&tables);
        if !self.er_relationship_form_role_inputs.contains_key(&tab_id) {
            self.er_relationship_form_role_inputs.insert(
                tab_id,
                cx.new(|cx| InputState::new(window, cx).placeholder("业务角色")),
            );
        }
        if !self
            .er_relationship_form_description_inputs
            .contains_key(&tab_id)
        {
            self.er_relationship_form_description_inputs.insert(
                tab_id,
                cx.new(|cx| InputState::new(window, cx).placeholder("说明（可选）")),
            );
        }
        if let Some(select) = self.er_relationship_form_left_tables.get(&tab_id) {
            Self::er_refresh_select_options(select, table_options.clone(), window, cx);
        } else {
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(table_options.clone()), None, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<ErRelationshipSelectOption>>,
                      window,
                      cx| {
                    if matches!(event, SelectEvent::Confirm(_)) {
                        this.er_clear_relationship_pair_selections(tab_id, window, cx);
                        this.er_clear_relationship_filter_columns(tab_id, window, cx);
                        cx.notify();
                    }
                },
            );
            self.er_relationship_form_left_tables.insert(tab_id, select);
            self.er_relationship_form_subscriptions
                .entry(tab_id)
                .or_default()
                .push(subscription);
        }
        if let Some(select) = self.er_relationship_form_right_tables.get(&tab_id) {
            Self::er_refresh_select_options(select, table_options, window, cx);
        } else {
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(table_options), None, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<ErRelationshipSelectOption>>,
                      window,
                      cx| {
                    if matches!(event, SelectEvent::Confirm(_)) {
                        this.er_clear_relationship_pair_selections(tab_id, window, cx);
                        this.er_clear_relationship_filter_columns(tab_id, window, cx);
                        cx.notify();
                    }
                },
            );
            self.er_relationship_form_right_tables.insert(tab_id, select);
            self.er_relationship_form_subscriptions
                .entry(tab_id)
                .or_default()
                .push(subscription);
        }
        if !self.er_relationship_form_pairs.contains_key(&tab_id) {
            self.er_relationship_form_pairs.insert(tab_id, Vec::new());
        }
        if self
            .er_relationship_form_pairs
            .get(&tab_id)
            .is_some_and(Vec::is_empty)
        {
            self.er_add_relationship_pair(tab_id, window, cx);
        }

        let left_id = self
            .er_relationship_form_left_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned());
        let right_id = self
            .er_relationship_form_right_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned());
        let left_table = self.er_form_table_by_id(tab_id, left_id.as_ref());
        let right_table = self.er_form_table_by_id(tab_id, right_id.as_ref());
        let pair_options = self
            .er_relationship_form_pairs
            .get_mut(&tab_id)
            .expect("ER relationship form pairs initialized");
        for pair in pair_options {
            let left_options = er_column_options(left_table.as_ref());
            let right_options = er_column_options(right_table.as_ref());
            let left_key = left_options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            let right_key = right_options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            if pair.left_options_key != left_key {
                Self::er_refresh_select_options(&pair.left, left_options, window, cx);
                pair.left_options_key = left_key;
            }
            if pair.right_options_key != right_key {
                Self::er_refresh_select_options(&pair.right, right_options, window, cx);
                pair.right_options_key = right_key;
            }
        }
        let filter_controls = self.er_relationship_form_filters.entry(tab_id).or_default();
        for filter in filter_controls {
            let side = filter
                .side
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_else(|| "left".into());
            let table = if side == "right" {
                right_table.as_ref()
            } else {
                left_table.as_ref()
            };
            let options = er_column_options(table);
            let key = options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            if filter.column_options_key != key {
                Self::er_refresh_select_options(&filter.column, options, window, cx);
                filter.column_options_key = key;
            }
        }
        // 基数选择：缺失时创建，默认「未知」。
        if let Some(select) = self.er_relationship_form_cardinality.get(&tab_id) {
            Self::er_refresh_select_options(select, er_cardinality_options(), window, cx);
        } else {
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(er_cardinality_options()),
                    Some(IndexPath::default().row(4)), // 默认选中「未知」
                    window,
                    cx,
                )
                .searchable(false)
            });
            self.er_relationship_form_cardinality.insert(tab_id, select);
        }
    }

    fn er_add_relationship_pair(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let left = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<ErRelationshipSelectOption>::new()), None, window, cx)
                .searchable(true)
        });
        let right = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<ErRelationshipSelectOption>::new()), None, window, cx)
                .searchable(true)
        });
        self.er_relationship_form_pairs
            .entry(tab_id)
            .or_default()
            .push(ErRelationshipPairControls {
                left,
                right,
                left_options_key: String::new(),
                right_options_key: String::new(),
            });
    }

    /// 删除一组字段配对；至少保留一组（沿用 HTML 参考「至少保留一组」约定）。
    fn er_remove_relationship_pair(
        &mut self,
        tab_id: TabId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(pairs) = self.er_relationship_form_pairs.get_mut(&tab_id)
            && pairs.len() > 1
            && index < pairs.len()
        {
            pairs.remove(index);
            cx.notify();
        }
    }

    fn er_add_relationship_filter(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ErRelationshipSelectOption {
                        id: "left".into(),
                        label: "左表".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "right".into(),
                        label: "右表".into(),
                    },
                ]),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
            .searchable(false)
        });
        let column = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<ErRelationshipSelectOption>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let op = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ErRelationshipSelectOption {
                        id: "eq".into(),
                        label: "等于".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "ne".into(),
                        label: "不等于".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "is_null".into(),
                        label: "为空".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "is_not_null".into(),
                        label: "不为空".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "in".into(),
                        label: "包含".into(),
                    },
                ]),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
            .searchable(false)
        });
        let literal = cx.new(|cx| InputState::new(window, cx).placeholder("常量值"));
        let subscription = cx.subscribe_in(
            &side,
            window,
            move |_this: &mut NavicatMain,
                  _select,
                  _event: &SelectEvent<SearchableVec<ErRelationshipSelectOption>>,
                  _window,
                  cx| {
                cx.notify();
            },
        );
        self.er_relationship_form_subscriptions
            .entry(tab_id)
            .or_default()
            .push(subscription);
        self.er_relationship_form_filters
            .entry(tab_id)
            .or_default()
            .push(ErRelationshipFilterControls {
                side,
                column,
                op,
                literal,
                column_options_key: String::new(),
            });
    }

    fn er_clear_relationship_filter_columns(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(filters) = self.er_relationship_form_filters.get(&tab_id) {
            for filter in filters {
                filter.column.update(cx, |state, cx| {
                    state.set_selected_index(None, window, cx);
                });
            }
        }
    }

    fn er_clear_relationship_pair_selections(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pairs) = self.er_relationship_form_pairs.get(&tab_id) {
            for pair in pairs {
                pair.left.update(cx, |state, cx| {
                    state.set_selected_index(None, window, cx);
                });
                pair.right.update(cx, |state, cx| {
                    state.set_selected_index(None, window, cx);
                });
            }
        }
    }

    fn er_prepare_new_relationship_form(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        self.er_relationship_form_editing.insert(tab_id, None);
        if let Some(input) = self.er_relationship_form_role_inputs.get(&tab_id) {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        if let Some(input) = self.er_relationship_form_description_inputs.get(&tab_id) {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        for select in [
            self.er_relationship_form_left_tables.get(&tab_id),
            self.er_relationship_form_right_tables.get(&tab_id),
        ]
        .into_iter()
        .flatten()
        {
            select.update(cx, |state, cx| state.set_selected_index(None, window, cx));
        }
        self.er_relationship_form_pairs.insert(tab_id, Vec::new());
        self.er_add_relationship_pair(tab_id, window, cx);
        self.er_relationship_form_filters.insert(tab_id, Vec::new());
        // 新建时基数复位为「未知」。
        if let Some(select) = self.er_relationship_form_cardinality.get(&tab_id) {
            select.update(cx, |state, cx| {
                state.set_selected_value(&"unknown".to_string(), window, cx)
            });
        }
    }

    /// 按表名取表节点：优先全库目录（关系表单数据源），graph 兜底。
    fn er_table_node_by_name(
        &self,
        tab_id: TabId,
        name: &str,
    ) -> Option<&fluxdb_core::ErTableNode> {
        self.er_full_tables
            .get(&tab_id)
            .and_then(|tables| tables.iter().find(|t| t.name == name))
            .or_else(|| {
                self.er_graphs
                    .get(&tab_id)
                    .and_then(|g| g.tables.iter().find(|t| t.name == name))
            })
    }

    /// 由字段端口拖拽发起「新建关系」：打开面板与表单，并预填左右表与第一组字段配对。
    /// 只在源/目标表都能在目录中找到时预填（不伪造字段选项）；其余信息由用户在表单内补全。
    fn er_prepare_new_relationship_from_ports(
        &mut self,
        tab_id: TabId,
        from_table: &str,
        from_column: &str,
        to_table: &str,
        to_column: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(from_node), Some(to_node)) = (
            self.er_table_node_by_name(tab_id, from_table),
            self.er_table_node_by_name(tab_id, to_table),
        ) else {
            return;
        };
        let from_id = er_entity_id(&from_node.reference);
        let to_id = er_entity_id(&to_node.reference);
        // 列名为空 = 从表头/非字段区拖入：只锁左右表，字段留给表单选择（不伪造列 id）。
        let from_col_id = (!from_column.is_empty())
            .then(|| er_column_id(&from_node.reference, from_column));
        let to_col_id = (!to_column.is_empty()).then(|| er_column_id(&to_node.reference, to_column));

        // 关系面板若未打开，表单无处渲染：拖拽发起时一并打开。
        self.er_relationship_panel_open.insert(tab_id);
        self.er_relationship_form_open.insert(tab_id);
        self.er_prepare_new_relationship_form(tab_id, window, cx);

        // 先定左右表（沿用原型「左右表已锁定」语义，仍可在表单内改），
        // 再刷新字段选项，最后写入首组配对——顺序颠倒会因选项未就绪而丢值。
        if let Some(select) = self.er_relationship_form_left_tables.get(&tab_id) {
            select.update(cx, |state, cx| state.set_selected_value(&from_id, window, cx));
        }
        if let Some(select) = self.er_relationship_form_right_tables.get(&tab_id) {
            select.update(cx, |state, cx| state.set_selected_value(&to_id, window, cx));
        }
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        if let Some(pairs) = self.er_relationship_form_pairs.get(&tab_id)
            && let Some(first) = pairs.first()
        {
            if let Some(cid) = &from_col_id {
                first.left
                    .update(cx, |state, cx| state.set_selected_value(cid, window, cx));
            }
            if let Some(cid) = &to_col_id {
                first.right
                    .update(cx, |state, cx| state.set_selected_value(cid, window, cx));
            }
        }
        cx.notify();
    }

    fn er_begin_edit_relationship(
        &mut self,
        tab_id: TabId,
        relationship: &fluxdb_core::ErRelationship,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        self.er_relationship_form_open.insert(tab_id);
        self.er_relationship_form_editing
            .insert(tab_id, Some(relationship.id.clone()));
        if let Some(input) = self.er_relationship_form_role_inputs.get(&tab_id) {
            input.update(cx, |input, cx| {
                input.set_value(relationship.role.clone(), window, cx)
            });
        }
        if let Some(input) = self.er_relationship_form_description_inputs.get(&tab_id) {
            input.update(cx, |input, cx| {
                input.set_value(relationship.description.clone().unwrap_or_default(), window, cx)
            });
        }
        if let Some(select) = self.er_relationship_form_left_tables.get(&tab_id) {
            select.update(cx, |state, cx| {
                state.set_selected_value(&relationship.left_entity, window, cx)
            });
        }
        if let Some(select) = self.er_relationship_form_right_tables.get(&tab_id) {
            select.update(cx, |state, cx| {
                state.set_selected_value(&relationship.right_entity, window, cx)
            });
        }
        self.er_relationship_form_pairs.insert(tab_id, Vec::new());
        for _ in &relationship.column_pairs {
            self.er_add_relationship_pair(tab_id, window, cx);
        }
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        if let Some(pairs) = self.er_relationship_form_pairs.get(&tab_id) {
            for (controls, pair) in pairs.iter().zip(&relationship.column_pairs) {
                controls.left.update(cx, |state, cx| {
                    state.set_selected_value(&pair.left_column, window, cx)
                });
                controls.right.update(cx, |state, cx| {
                    state.set_selected_value(&pair.right_column, window, cx)
                });
            }
        }
        self.er_relationship_form_filters.insert(tab_id, Vec::new());
        for _ in &relationship.required_filters {
            self.er_add_relationship_filter(tab_id, window, cx);
        }
        if let Some(filters) = self.er_relationship_form_filters.get(&tab_id) {
            for (controls, filter) in filters.iter().zip(&relationship.required_filters) {
                let side = match filter.side {
                    fluxdb_core::ErRelationSide::Left => "left",
                    fluxdb_core::ErRelationSide::Right => "right",
                };
                controls.side.update(cx, |state, cx| {
                    state.set_selected_value(&side.to_string(), window, cx)
                });
                let op = er_filter_op_id(&filter.op).to_string();
                controls.op.update(cx, |state, cx| {
                    state.set_selected_value(&op, window, cx)
                });
                controls.literal.update(cx, |input, cx| {
                    input.set_value(er_literal_text(&filter.literal), window, cx)
                });
            }
        }
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        if let Some(filters) = self.er_relationship_form_filters.get(&tab_id) {
            for (controls, filter) in filters.iter().zip(&relationship.required_filters) {
                controls.column.update(cx, |state, cx| {
                    state.set_selected_value(&filter.column_id, window, cx)
                });
            }
        }
        // 预填当前基数（编辑时可改；若不改则沿用）。
        if let Some(select) = self.er_relationship_form_cardinality.get(&tab_id) {
            let card_id = er_cardinality_to_option_id(&relationship.match_cardinality).to_string();
            select.update(cx, |state, cx| {
                state.set_selected_value(&card_id, window, cx)
            });
        }
        cx.notify();
    }

    fn submit_er_relationship_form(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if self.er_relationship_tasks.contains_key(&tab_id) {
            return;
        }
        let Some(scope_key) = self.er_relationship_scope_keys.get(&tab_id).cloned() else {
            return;
        };
        let Some(role_input) = self.er_relationship_form_role_inputs.get(&tab_id) else {
            return;
        };
        let role = role_input.read(cx).value().trim().to_string();
        let Some(left_entity) = self
            .er_relationship_form_left_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned())
        else {
            self.er_relationship_errors.insert(tab_id, "请选择左表".into());
            return;
        };
        let Some(right_entity) = self
            .er_relationship_form_right_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned())
        else {
            self.er_relationship_errors.insert(tab_id, "请选择右表".into());
            return;
        };
        if role.is_empty() {
            self.er_relationship_errors.insert(tab_id, "业务角色不能为空".into());
            return;
        }
        let mut column_pairs = Vec::new();
        for pair in self.er_relationship_form_pairs.get(&tab_id).into_iter().flatten() {
            let Some(left_column) = pair.left.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请完成全部字段配对".into());
                return;
            };
            let Some(right_column) = pair.right.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请完成全部字段配对".into());
                return;
            };
            column_pairs.push(fluxdb_core::ErColumnPair {
                left_column,
                right_column,
            });
        }
        if column_pairs.is_empty() {
            self.er_relationship_errors.insert(tab_id, "至少需要一组字段配对".into());
            return;
        }
        let mut required_filters = Vec::new();
        for filter in self.er_relationship_form_filters.get(&tab_id).into_iter().flatten() {
            let Some(side) = filter.side.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请选择过滤条件端点".into());
                return;
            };
            let Some(column_id) = filter.column.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请选择过滤条件字段".into());
                return;
            };
            let Some(op_id) = filter.op.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请选择过滤条件操作".into());
                return;
            };
            let op = match op_id.as_str() {
                "eq" => fluxdb_core::ErFilterOp::Eq,
                "ne" => fluxdb_core::ErFilterOp::Ne,
                "is_null" => fluxdb_core::ErFilterOp::IsNull,
                "is_not_null" => fluxdb_core::ErFilterOp::IsNotNull,
                "in" => fluxdb_core::ErFilterOp::In,
                _ => {
                    self.er_relationship_errors.insert(tab_id, "未知的过滤条件操作".into());
                    return;
                }
            };
            let literal_text = filter.literal.read(cx).value().trim().to_string();
            let literal = if matches!(op, fluxdb_core::ErFilterOp::IsNull | fluxdb_core::ErFilterOp::IsNotNull) {
                fluxdb_core::ErLiteral::Null
            } else if let Some(literal) = er_parse_literal(&literal_text) {
                literal
            } else {
                self.er_relationship_errors.insert(tab_id, "过滤条件字面量不能为空".into());
                return;
            };
            required_filters.push(fluxdb_core::ErRequiredFilter {
                side: if side == "right" {
                    fluxdb_core::ErRelationSide::Right
                } else {
                    fluxdb_core::ErRelationSide::Left
                },
                column_id,
                op,
                literal,
            });
        }
        // 不能靠旧选择值提交已删字段，更不能仅编辑角色就把 unresolved 清成 current。
        let valid_column = |entity: &str, id: &str| {
            self.er_full_tables.get(&tab_id).into_iter().flatten()
                .find(|table| er_entity_id(&table.reference) == entity && table.status == ErLoadStatus::Loaded)
                .is_some_and(|table| table.columns.iter().any(|column| er_column_id(&table.reference, &column.name) == id))
        };
        if column_pairs.iter().any(|pair| !valid_column(&left_entity, &pair.left_column)
            || !valid_column(&right_entity, &pair.right_column))
            || required_filters.iter().any(|filter| {
                let entity = if filter.side == fluxdb_core::ErRelationSide::Left { &left_entity } else { &right_entity };
                !valid_column(entity, &filter.column_id)
            })
        {
            self.er_relationship_errors.insert(tab_id, "关系字段已不存在，请选择当前表中的字段".into());
            return;
        }
        let description = self
            .er_relationship_form_description_inputs
            .get(&tab_id)
            .map(|input| input.read(cx).value().trim().to_string())
            .filter(|value| !value.is_empty());
        let mut relationship = fluxdb_core::ErRelationship {
            id: format!(
                "user-{}-{}",
                tab_id.0,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default()
            ),
            revision: 1,
            left_entity,
            right_entity,
            role,
            column_pairs,
            required_filters,
            match_cardinality: {
                let card_id = self
                    .er_relationship_form_cardinality
                    .get(&tab_id)
                    .and_then(|select| select.read(cx).selected_value().cloned())
                    .unwrap_or_else(|| "unknown".to_string());
                er_option_id_to_cardinality(&card_id)
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
            description,
            evidence_refs: Vec::new(),
        };
        let command = if let Some(editing_id) = self
            .er_relationship_form_editing
            .get(&tab_id)
            .cloned()
            .flatten()
        {
            let Some(current) = self
                .er_relationships
                .get(&tab_id)
                .into_iter()
                .flatten()
                .find(|relationship| relationship.id == editing_id)
                .cloned()
            else {
                self.er_relationship_errors.insert(tab_id, "关系已不存在，请刷新列表".into());
                return;
            };
            relationship.id = current.id.clone();
            relationship.revision = current.revision;
            // match_cardinality 取表单选择（编辑时已预填当前值，可改）。
            relationship.origin = current.origin;
            relationship.review = current.review;
            relationship.enforcement = current.enforcement;
            relationship.validity = fluxdb_core::ErValidity {
                state: fluxdb_core::ErValidityState::Current,
                reason: None,
            };
            relationship.evidence_refs = current.evidence_refs;
            AppCommand::UpdateErRelationship {
                scope_key,
                relationship,
                expected_revision: current.revision,
            }
        } else {
            AppCommand::CreateErRelationship {
                scope_key,
                relationship,
            }
        };
        self.run_er_relationship_command(
            tab_id,
            command,
            cx,
        );
    }
}

