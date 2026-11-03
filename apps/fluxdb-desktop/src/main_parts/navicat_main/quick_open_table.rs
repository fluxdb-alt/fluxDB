impl NavicatMain {
    /// 快速打开表的作用域：当前工作区的连接/库；没有工作区时仅接受唯一的 SQL 连接。
    ///
    /// 返回 `None` 表示无法确定目标库，弹层直接提示而不是展示一份可能来自别的库的清单。
    fn quick_open_table_scope(&self) -> Option<(ConnectionId, Option<String>)> {
        let state = self.controller.state();
        if let Some(tab) = state.active_tab() {
            let scope = match &tab.kind {
                TabKind::DataEditor(editor) => Some((
                    editor.object.connection_id,
                    editor.object.database.clone(),
                )),
                TabKind::QueryEditor(editor) => {
                    Some((editor.connection_id, editor.database.clone()))
                }
                TabKind::CreateTable(create) => {
                    Some((create.connection_id, create.database.clone()))
                }
                TabKind::ErDiagram(er) => {
                    Some((er.connection_id, Some(er.database.clone())))
                }
                TabKind::BackupList(list) => {
                    Some((list.connection_id, Some(list.database.clone())))
                }
                TabKind::ObjectList(list) => list
                    .parent
                    .as_ref()
                    .map(|parent| (parent.connection_id, parent.database.clone())),
                _ => None,
            };
            if let Some(scope) = scope {
                return self
                    .is_sql_connection(scope.0)
                    .then_some(scope);
            }
        }

        // 没有活动页签时不存在“当前库”，优先选侧边栏里已展开且已加载表的 SQL 连接，
        // 避免多连接场景直接提示“请先打开一个数据库”而打不开弹层。
        let connection = state
            .connections
            .iter()
            .find(|connection| {
                connection.expanded
                    && self.is_sql_connection(connection.config.id)
                    && quick_open_table_connection_has_tables(connection)
            })
            .or_else(|| {
                state.connections.iter().find(|connection| {
                    self.is_sql_connection(connection.config.id)
                        && quick_open_table_connection_has_tables(connection)
                })
            })
            .or_else(|| {
                state
                    .connections
                    .iter()
                    .find(|connection| self.is_sql_connection(connection.config.id))
            })?;
        // 无活动页签就没有“当前库”语义，直接搜索该连接所有已加载库。
        Some((connection.config.id, None))
    }

    fn is_sql_connection(&self, connection_id: ConnectionId) -> bool {
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .is_some_and(|connection| {
                !matches!(
                    connection.config.kind,
                    DatabaseKind::MongoDb | DatabaseKind::Redis
                )
            })
    }

    fn open_quick_open_table_action(
        &mut self,
        _: &QuickOpenTable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_open_table(window, cx);
    }

    /// 打开快速打开表弹层。
    ///
    /// 除视图内快捷键外，应用级监听也会调用这里：窗口没有焦点（刚启动、点击侧边栏）
    /// 时视图的 `on_action` 不在焦点路径上，不能只依赖视图分发。
    fn open_quick_open_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection_id, database)) = self.quick_open_table_scope() else {
            self.show_message(
                tr("app.quick_open_table.no_scope").as_ref(),
                AppMessageKind::Info,
                cx,
            );
            return;
        };
        // 只读侧边栏对象树已加载节点，不额外查询数据库 catalog。
        let tables = quick_open_table_loaded_entries(self, connection_id, database.as_deref());
        if tables.is_empty() {
            self.show_message(
                tr("app.quick_open_table.not_loaded").as_ref(),
                AppMessageKind::Info,
                cx,
            );
            return;
        }

        // 避免和历史/标签快搜叠层：同一时刻只保留一个键盘驱动弹层。
        self.query_history_quick_open = false;
        self.query_history_open = false;
        self.tab_switcher = None;
        self.quick_open_table_entries = tables;
        self.quick_open_table_search.clear();
        self.quick_open_table_selected = 0;
        self.quick_open_table_open = true;
        self.quick_open_table_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn close_quick_open_table(&mut self, cx: &mut Context<Self>) {
        if self.quick_open_table_open {
            self.quick_open_table_open = false;
            cx.notify();
        }
    }

    fn quick_open_table_previous(
        &mut self,
        _: &QuickOpenTablePrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.quick_open_table_open {
            return;
        }
        let len = quick_open_table_filtered(self).len();
        if len == 0 {
            self.quick_open_table_selected = 0;
        } else if self.quick_open_table_selected == 0 {
            self.quick_open_table_selected = len - 1;
        } else {
            self.quick_open_table_selected -= 1;
        }
        cx.notify();
    }

    fn quick_open_table_next(
        &mut self,
        _: &QuickOpenTableNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.quick_open_table_open {
            return;
        }
        let len = quick_open_table_filtered(self).len();
        if len == 0 {
            self.quick_open_table_selected = 0;
        } else {
            self.quick_open_table_selected = (self.quick_open_table_selected + 1) % len;
        }
        cx.notify();
    }

    fn quick_open_table_confirm(
        &mut self,
        _: &QuickOpenTableConfirm,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_quick_open_table(cx);
    }

    fn confirm_quick_open_table(&mut self, cx: &mut Context<Self>) {
        if !self.quick_open_table_open {
            return;
        }
        let entries = quick_open_table_filtered(self);
        let Some(entry) = entries
            .get(self.quick_open_table_selected.min(entries.len().saturating_sub(1)))
            .cloned()
        else {
            return;
        };
        self.open_quick_open_table_entry(entry, cx);
    }

    fn open_quick_open_table_entry(&mut self, entry: ObjectPath, cx: &mut Context<Self>) {
        if !matches!(entry.kind, ObjectKind::Table | ObjectKind::View) {
            return;
        }
        self.quick_open_table_open = false;
        self.dispatch(AppCommand::OpenDataEditor(entry), cx);
    }
}

/// 该连接是否已经有可搜索的表/视图节点。
fn quick_open_table_connection_has_tables(connection: &ConnectionState) -> bool {
    connection
        .objects
        .iter()
        .any(|object| matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
}

/// 从侧边栏已加载对象树中取当前库的表/视图。
fn quick_open_table_loaded_entries(
    this: &NavicatMain,
    connection_id: ConnectionId,
    database: Option<&str>,
) -> Vec<ObjectPath> {
    let Some(connection) = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
    else {
        return Vec::new();
    };
    let mut paths = connection
        .objects
        .iter()
        .filter(|object| matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
        // 与侧边栏分组一致：缺省库名按 `main` 处理（SQLite / 单库连接）。
        .filter(|object| {
            database.is_none_or(|database| {
                object.path.database.as_deref().unwrap_or("main") == database
            })
        })
        .map(|object| object.path.clone())
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.schema.cmp(&right.schema))
    });
    paths.dedup_by(|left, right| {
        left.name == right.name && left.schema == right.schema && left.database == right.database
    });
    paths
}

/// 按当前搜索串过滤并排序候选；空串保留全部，按名称字典序。
fn quick_open_table_filtered(this: &NavicatMain) -> Vec<ObjectPath> {
    quick_open_table_matches(&this.quick_open_table_entries, &this.quick_open_table_search)
}

fn quick_open_table_matches(entries: &[ObjectPath], search: &str) -> Vec<ObjectPath> {
    let query = search.trim();
    let mut entries = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let name_score = fluxdb_completion::fuzzy_score(&entry.name, query);
            let schema_score = entry
                .schema
                .as_deref()
                .and_then(|schema| fluxdb_completion::fuzzy_score(schema, query));
            if name_score.is_none() && schema_score.is_none() {
                return None;
            }
            Some((
                name_score.unwrap_or(0) + schema_score.unwrap_or(0).saturating_sub(4),
                index,
                entry.clone(),
            ))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.2.name.cmp(&right.2.name))
            .then_with(|| left.1.cmp(&right.1))
    });
    entries.into_iter().take(50).map(|(_, _, entry)| entry).collect()
}

fn quick_open_table_modal(
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let entries = quick_open_table_filtered(this);
    let selected = if entries.is_empty() {
        0
    } else {
        this.quick_open_table_selected.min(entries.len() - 1)
    };
    let selected_bg = if colors.is_dark {
        rgb(0x263244)
    } else {
        rgb(0xe8f2ff)
    };
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.48)
        } else {
            opaque_grey(0.75, 0.20)
        })
        .flex()
        .items_start()
        .justify_center()
        .pt(px(72.))
        .px_4()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_quick_open_table(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w_full()
                .max_w(px(620.))
                .max_h(px(480.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., 0.26),
                )])
                .flex()
                .flex_col()
                .overflow_hidden()
                .key_context("QuickOpenTable")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(54.))
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon(AppIcon::Search, 16., colors.muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(Input::new(&this.quick_open_table_input).small()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .whitespace_nowrap()
                                .child("Esc"),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius_lg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |style| style.bg(colors.hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.close_quick_open_table(cx);
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(app_icon(AppIcon::Close, 16., colors.muted)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p_2()
                        .when(entries.is_empty(), |this| {
                            this.child(
                                div()
                                    .h(px(118.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(13.))
                                    .text_color(colors.muted)
                                    .child(tr("app.quick_open_table.empty")),
                            )
                        })
                        .children(entries.into_iter().enumerate().map(|(index, entry)| {
                            quick_open_table_row(
                                entry,
                                &this.quick_open_table_search,
                                selected == index,
                                selected_bg,
                                colors,
                                cx,
                            )
                        })),
                ),
        )
}

fn quick_open_table_row(
    entry: ObjectPath,
    search: &str,
    selected: bool,
    selected_bg: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let subtitle = match (&entry.database, &entry.schema) {
        (Some(database), Some(schema)) => format!("{database} / {schema}"),
        (Some(database), None) => database.clone(),
        (None, Some(schema)) => schema.clone(),
        (None, None) => String::new(),
    };
    let name = entry.name.clone();
    let row_selector = format!("quick-open-table-row:{name}");
    div()
        .h(px(52.))
        .rounded(colors.radius_lg)
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .bg(if selected { selected_bg } else { colors.panel_bg })
        .hover(move |style| style.bg(if selected { selected_bg } else { colors.hover }))
        .debug_selector({
            let row_selector = row_selector.clone();
            move || row_selector.clone()
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.open_quick_open_table_entry(entry.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .size(px(28.))
                .flex_none()
                .rounded(colors.radius_lg)
                .bg(if colors.is_dark {
                    rgb(0x202a36)
                } else {
                    rgb(0xf0f4f8)
                })
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon(AppIcon::Table, 15., colors.muted)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(quick_open_table_highlighted_text(
                            &name,
                            search,
                            colors.text,
                            colors,
                        )),
                )
                .when(!subtitle.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .line_height(px(15.))
                            .text_color(colors.muted)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(quick_open_table_highlighted_text(
                                &subtitle,
                                search,
                                colors.muted,
                                colors,
                            )),
                    )
                }),
        )
}

/// 按模糊搜索命中的字符逐个高亮；与 `fuzzy_score` 的子序列规则保持一致。
///
/// 命中字符可能不连续，因此不能复用历史搜索的连续子串高亮。
fn quick_open_table_highlighted_text(
    text: &str,
    query: &str,
    base_color: gpui::Rgba,
    colors: UiColors,
) -> Div {
    let mut label = div()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(base_color);
    let query_chars = query
        .trim()
        .chars()
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if query_chars.is_empty() {
        return label.child(text.to_string());
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut matched = vec![false; chars.len()];
    let mut query_index = 0;
    for (index, ch) in chars.iter().enumerate() {
        if query_index < query_chars.len() && ch.to_ascii_lowercase() == query_chars[query_index] {
            matched[index] = true;
            query_index += 1;
        }
    }
    if query_index < query_chars.len() {
        return label.child(text.to_string());
    }

    let highlight_bg = if colors.is_dark {
        rgb(0x5b4513)
    } else {
        rgb(0xffe7a3)
    };
    let highlight_text = if colors.is_dark {
        rgb(0xffd166)
    } else {
        rgb(0x8a4b00)
    };
    let mut cursor = 0;
    while cursor < chars.len() {
        let is_match = matched[cursor];
        let mut end = cursor + 1;
        while end < chars.len() && matched[end] == is_match {
            end += 1;
        }
        let segment = chars[cursor..end].iter().collect::<String>();
        if is_match {
            label = label.child(
                div()
                    .rounded(colors.radius * 0.5)
                    .px(px(1.5))
                    .bg(highlight_bg)
                    .text_color(highlight_text)
                    .child(segment),
            );
        } else {
            label = label.child(segment);
        }
        cursor = end;
    }
    label
}
