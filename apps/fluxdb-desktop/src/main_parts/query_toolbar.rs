fn query_toolbar(
    this: &mut NavicatMain,
    tab_id: TabId,
    editor: &QueryEditorState,
    sql_editor: Entity<editor_component::Editor>,
    connection: String,
    accent: gpui::Rgba,
    output_placement: ResultsPlacement,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let key = QueryPaneKey::new(tab_id, editor.active_pane);
    let has_text = !editor.text.trim().is_empty();
    let soft_wrap = sql_editor.read(cx).soft_wrap();
    let can_explain = !editor.running && this.selected_explain_sql_text(key, cx).is_some();
    // 分屏后单屏可能被拖得很窄；只保留运行/停止/分屏等主操作，其余按钮让位给文本宽度。
    let compact = this.query_pane_compact.get(&key).copied().unwrap_or(false);
    let query_files_loading = this.query_files.loading;
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_scope_context_group(
            this,
            window,
            cx,
            tab_id,
            editor,
            connection,
            accent,
            colors,
        ))
        .child(
            query_toolbar_icon_button(
                if editor.running {
                    tr("app.query.toolbar.running")
                } else {
                    tr("app.query.toolbar.run")
                },
                AppIcon::Play,
                has_text && !editor.running,
                editor.running,
                rgb(0x20c76a),
                colors,
            )
                .debug_selector(|| "query-run".into())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql_editor = sql_editor.clone();
                        move |this, _, window, cx| {
                            let text = sql_editor.read(cx).text();
                            if !text.trim().is_empty() {
                                this.dispatch(AppCommand::UpdateQueryText { tab_id, pane_id: key.pane_id, text }, cx);
                                this.start_query_execution(key, window, cx);
                                cx.stop_propagation();
                            }
                        }
                    }),
                ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.query.toolbar.format"),
                AppIcon::AlignLeft,
                has_text,
                false,
                rgb(0xa45cff),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let sql_editor = sql_editor.clone();
                    move |this, _, _, cx| {
                        if this.format_query_editor_sql(key, sql_editor.clone(), cx) {
                            cx.stop_propagation();
                        }
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.query.toolbar.compact"),
                AppIcon::List,
                has_text,
                false,
                rgb(0x38bdf8),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let sql_editor = sql_editor.clone();
                    move |this, _, _, cx| {
                        if this.compress_query_editor_sql(key, sql_editor.clone(), cx) {
                            cx.stop_propagation();
                        }
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.query.toolbar.stop"),
                AppIcon::Square,
                editor.running,
                false,
                rgb(0xff5c5c),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.request_query_cancel(key, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(query_toolbar_icon_button(
            tr("app.query.toolbar.explain"),
            AppIcon::FileSearch,
            can_explain,
            false,
            rgb(0x45a3ff),
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let sql_editor = sql_editor.clone();
                move |this, _, window, cx| {
                    if this.start_query_explain_execution(key, sql_editor.clone(), window, cx) {
                        cx.stop_propagation();
                    }
                }
            }),
        ))
        .child(
            query_toolbar_icon_button(
                tr("app.query.split_columns"),
                AppIcon::PanelRight,
                true,
                false,
                rgb(0x7b8cff),
                colors,
            )
            .debug_selector(|| "query-split-columns".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        AppCommand::SplitQueryPane {
                            tab_id,
                            pane_id: key.pane_id,
                            direction: QuerySplitDirection::Columns,
                        },
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                tr("app.query.split_rows"),
                AppIcon::PanelBottom,
                true,
                false,
                rgb(0x7b8cff),
                colors,
            )
            .debug_selector(|| "query-split-rows".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        AppCommand::SplitQueryPane {
                            tab_id,
                            pane_id: key.pane_id,
                            direction: QuerySplitDirection::Rows,
                        },
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .child(div().flex_1())
        .when(!compact, |this| {
            this.child(query_toolbar_secondary_actions(
                tab_id,
                sql_editor,
                output_placement,
                soft_wrap,
                query_files_loading,
                colors,
                cx,
            ))
        })
}

/// 次要工具栏动作：空间不足（单屏过窄）时整组隐藏，而不是逐个挤成图标。
#[allow(clippy::too_many_arguments)]
fn query_toolbar_secondary_actions(
    tab_id: TabId,
    sql_editor: Entity<editor_component::Editor>,
    output_placement: ResultsPlacement,
    soft_wrap: bool,
    query_files_loading: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(Button::new("query-open-file")
            .debug_selector(||"query-open-file".into())
            .small().ghost().cursor_pointer().disabled(query_files_loading)
            .loading(query_files_loading)
            .child(app_icon(AppIcon::Folder,16.,colors.muted))
            .tooltip(tr("app.query.toolbar.open_file"))
            .on_click(cx.listener(move|this,_,_,cx|this.choose_local_sql_file(tab_id,cx))))
        .child(query_toolbar_icon_button(
            tr("app.common.save"),
            AppIcon::Save,
            true,
            false,
            rgb(0x4b8dff),
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.save_active_query(window, cx);
                cx.stop_propagation();
            }),
        ))
        .child(query_toolbar_icon_button(
            tr("app.query.toolbar.builder"),
            AppIcon::Workflow,
            false,
            false,
            rgb(0xf0b400),
            colors,
        ))
        .child(query_toolbar_icon_button(
            tr("app.query.toolbar.snippets"),
            AppIcon::FileSql,
            false,
            false,
            rgb(0x28c7d7),
            colors,
        ))
        .child(
            query_toolbar_icon_button(
                tr("app.query.toolbar.history"),
                AppIcon::CalendarClock,
                true,
                false,
                rgb(0xf59e0b),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.show_query_history_quick_search(window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                query_editor_wrap_toggle_label(soft_wrap),
                query_editor_wrap_toggle_icon(soft_wrap),
                true,
                false,
                rgb(0x5f8ff7),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let sql_editor = sql_editor.clone();
                    move |this, _, _, cx| {
                        let mut settings = this.controller.state().settings.clone();
                        settings.editor_word_wrap = !soft_wrap;
                        save_settings_from_ui(
                            this,
                            settings,
                            tr("app.settings.message.word_wrap_updated"),
                            cx,
                        );
                        let line_height = this.controller.state().settings.editor_line_height.clamp(13, 28) as f32;
                        sql_editor.update(cx, |editor, _editor_cx| {
                            editor.apply_settings(editor.font_size, line_height, !soft_wrap, 0);
                        });
                        cx.stop_propagation();
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                query_output_layout_toggle_label(output_placement),
                query_output_layout_toggle_icon(output_placement),
                true,
                false,
                rgb(0x45a3ff),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.results_placement = this.results_placement.toggled();
                    this.query_output_resize_start = None;
                    cx.notify();
                    cx.stop_propagation();
                }),
            ),
        )
}

fn query_editor_wrap_toggle_icon(soft_wrap: bool) -> AppIcon {
    if soft_wrap {
        AppIcon::Text
    } else {
        AppIcon::WrapText
    }
}

fn query_editor_wrap_toggle_label(soft_wrap: bool) -> SharedString {
    if soft_wrap {
        tr("app.query.toolbar.wrap_off")
    } else {
        tr("app.query.toolbar.wrap_auto")
    }
}

fn query_output_layout_toggle_icon(placement: ResultsPlacement) -> AppIcon {
    match placement {
        ResultsPlacement::Bottom => AppIcon::PanelBottom,
        ResultsPlacement::Right => AppIcon::PanelRight,
    }
}

fn query_output_layout_toggle_label(placement: ResultsPlacement) -> SharedString {
    match placement {
        ResultsPlacement::Bottom => tr("app.query.toolbar.results_bottom"),
        ResultsPlacement::Right => tr("app.query.toolbar.results_right"),
    }
}

fn query_toolbar_icon_button(
    label: impl Into<SharedString>,
    icon: AppIcon,
    enabled: bool,
    loading: bool,
    accent: gpui::Rgba,
    colors: UiColors,
) -> Stateful<Div> {
    let label = label.into();
    let label_for_id = label.clone();
    let bg = colors.panel_bg;
    let icon_color = if enabled { accent } else { colors.muted };
    div()
        .id(label_for_id)
        .size(px(30.))
        .rounded(colors.radius)
        .border_1()
        .border_color(if enabled {
            if colors.is_dark {
                rgb(0x3a414d)
            } else {
                rgb(0xd7dde8)
            }
        } else {
            colors.border
        })
        .bg(if enabled { bg } else { colors.panel_alt })
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| {
                style.bg(if colors.is_dark {
                    rgb(0x2a3039)
                } else {
                    rgb(0xf4f7fb)
                })
            })
        })
        .child(if loading {
            loading_spinner_with_color(17., icon_color).into_any_element()
        } else {
            app_icon_box(icon, 20., 17., icon_color).into_any_element()
        })
}

/// 查询工具栏作用域项：`value = None` 表示「连接默认」（schema 下拉专用）。
#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryScopeItem {
    label: SharedString,
    value: Option<String>,
}

impl SelectItem for QueryScopeItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

impl NavicatMain {
    /// 当前活动标签若是 SQL 查询编辑器则返回其 id，供作用域下拉回写命令。
    fn active_query_tab_id(&self) -> Option<TabId> {
        let tab = self.controller.state().active_tab()?;
        matches!(tab.kind, TabKind::QueryEditor(_)).then_some(tab.id)
    }

    /// 同步作用域下拉的候选项；仅在内容变化时重建，避免每帧重置弹出层状态。
    fn sync_query_scope_items(
        &mut self,
        database_items: Vec<QueryScopeItem>,
        schema_items: Vec<QueryScopeItem>,
        expected_database: Option<&str>,
        expected_schema: Option<&str>,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        if self.query_database_items != database_items {
            self.query_database_items = database_items.clone();
            self.query_database_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(database_items), window, cx);
            });
        }
        if self.query_schema_items != schema_items {
            self.query_schema_items = schema_items.clone();
            self.query_schema_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(schema_items), window, cx);
            });
            // 新列表默认选中「默认」项，避免触发器停在占位符上。
            if expected_schema.is_none()
                && !self.query_schema_items.is_empty()
                && self.query_schema_select.read(cx).selected_index(cx).is_none()
            {
                self.query_schema_select.update(cx, |select, cx| {
                    select.set_selected_index(Some(IndexPath::new(0)), window, cx);
                });
            }
        }
        sync_query_scope_value(
            &self.query_database_select,
            expected_database.map(str::to_string),
            window,
            cx,
        );
        sync_query_scope_value(
            &self.query_schema_select,
            expected_schema.map(str::to_string),
            window,
            cx,
        );
    }
}

/// 把下拉选中值对齐到编辑器作用域；值相同就不动，避免打断正在展开的菜单。
fn sync_query_scope_value(
    select: &Entity<SelectState<SearchableVec<QueryScopeItem>>>,
    expected: Option<String>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    if select.read(cx).selected_value().cloned() == Some(expected.clone()) {
        return;
    }
    let Some(expected) = expected else {
        // 「未选择」不需要反复写值：每次 set_selected_value 都会清空选择器里的搜索框，
        // 用户正在筛选数据库时会被下一帧打断。只有确实残留了选择才清一次。
        if select.read(cx).selected_index(cx).is_some() {
            select.update(cx, |select, cx| select.set_selected_index(None, window, cx));
        }
        return;
    };
    select.update(cx, |select, cx| {
        select.set_selected_value(&Some(expected), window, cx);
    });
}

/// 组装下拉候选项：加载到的对象列表 + 当前值（对象树尚未加载时也要能显示当前作用域）。
fn query_database_items(
    connection: Option<&ConnectionState>,
    current: Option<&str>,
) -> Vec<QueryScopeItem> {
    let mut names = connection
        .map(|connection| {
            connection_databases_unfiltered(connection)
                .into_iter()
                .map(|database| database_display_name(&database))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(current) = current
        && !names.iter().any(|name| name == current)
    {
        names.push(current.to_string());
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| QueryScopeItem {
            label: name.clone().into(),
            value: Some(name),
        })
        .collect()
}

fn query_schema_items(
    connection: Option<&ConnectionState>,
    database: Option<&str>,
    current: Option<&str>,
) -> Vec<QueryScopeItem> {
    let mut schemas = connection
        .map(|connection| database_schemas(connection, database.unwrap_or("main")))
        .unwrap_or_default();
    if let Some(current) = current
        && !schemas.iter().any(|schema| schema == current)
    {
        schemas.push(current.to_string());
    }
    schemas.sort();
    schemas.dedup();
    let mut items = vec![QueryScopeItem {
        label: tr("app.query.toolbar.default_schema"),
        value: None,
    }];
    items.extend(schemas.into_iter().map(|schema| QueryScopeItem {
        label: schema.clone().into(),
        value: Some(schema),
    }));
    items
}

/// 只读作用域胶囊（Redis / MongoDB 等没有可切换库上下文的工具栏复用）。
fn query_context_group(
    connection: String,
    database: String,
    accent: gpui::Rgba,
    colors: UiColors,
) -> Div {
    div()
        .h(px(30.))
        .min_w(px(0.))
        .max_w(px(360.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .overflow_hidden()
        .child(query_context_item(connection, Some(accent), colors))
        .child(div().w(px(1.)).h(px(14.)).flex_none().bg(colors.border_soft))
        .child(query_context_item(database, None, colors))
}

/// 作用域下拉：数据库/ schema 都只在能力允许的库型上出现。
///
/// 连接名保持只读；SQLite / 达梦没有可切换的库上下文，继续显示静态文本。
fn query_scope_context_group(
    this: &mut NavicatMain,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    _tab_id: TabId,
    editor: &QueryEditorState,
    connection: String,
    accent: gpui::Rgba,
    colors: UiColors,
) -> Div {
    let connection_state = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == editor.connection_id);
    let kind = connection_state.map(|connection| connection.config.kind);
    let can_switch_database = kind.is_some_and(|kind| {
        kind.is_mysql_family()
            || kind.is_postgres_family()
            || kind == DatabaseKind::SqlServer
    });
    let can_switch_schema =
        kind.is_some_and(|kind| kind.is_postgres_family() || kind == DatabaseKind::Oracle);
    let database_items = query_database_items(connection_state, editor.database.as_deref());
    let schema_items = query_schema_items(
        connection_state,
        editor.database.as_deref(),
        editor.schema.as_deref(),
    );
    // 该连接不需要某个下拉时传空列表，避免残留其它连接的同名项被误选。
    this.sync_query_scope_items(
        if can_switch_database {
            database_items
        } else {
            Vec::new()
        },
        if can_switch_schema {
            schema_items
        } else {
            Vec::new()
        },
        editor.database.as_deref(),
        editor.schema.as_deref(),
        window,
        cx,
    );

    let database_label = editor
        .database
        .clone()
        .unwrap_or_else(|| tr("app.history.default_database").to_string());
    let mut group = div()
        .h(px(30.))
        .min_w(px(0.))
        .max_w(px(460.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .overflow_hidden()
        .child(query_context_item(connection, Some(accent), colors))
        .child(div().w(px(1.)).h(px(14.)).flex_none().bg(colors.border_soft));
    group = group.child(if can_switch_database {
        query_scope_select(
            &this.query_database_select,
            AppIcon::Database,
            tr("app.query.toolbar.choose_database"),
            "query-database-select",
            colors,
        )
    } else {
        query_context_item(database_label, None, colors)
    });
    if can_switch_schema {
        group = group
            .child(div().w(px(1.)).h(px(14.)).flex_none().bg(colors.border_soft))
            .child(query_scope_select(
                &this.query_schema_select,
                AppIcon::Schema,
                tr("app.query.toolbar.choose_schema"),
                "query-schema-select",
                colors,
            ));
    }
    group
}

fn query_scope_select(
    select: &Entity<SelectState<SearchableVec<QueryScopeItem>>>,
    icon: AppIcon,
    placeholder: SharedString,
    selector: &'static str,
    colors: UiColors,
) -> Div {
    // Select 自带边框/底色，嵌进作用域胶囊时关掉外观，只保留文字与下拉箭头。
    div()
        .h_full()
        .min_w(px(0.))
        .max_w(px(160.))
        .px_2()
        .flex()
        .items_center()
        .gap_1p5()
        .debug_selector(move || selector.to_string())
        .child(app_icon(icon, 13., colors.muted))
        .child(
            Select::new(select)
                .appearance(false)
                .placeholder(placeholder)
                .menu_width(px(240.))
                .text_color(colors.text)
                .h(px(28.))
                .flex_1(),
        )
}

fn query_context_item(
    label: String,
    identity_color: Option<gpui::Rgba>,
    colors: UiColors,
) -> Div {
    let leading: gpui::AnyElement = match identity_color {
        Some(color) => div()
            .size(px(6.))
            .flex_none()
            .rounded_full()
            .bg(color)
            .into_any_element(),
        None => app_icon(AppIcon::Database, 13., colors.muted).into_any_element(),
    };
    div()
        .h_full()
        .min_w(px(0.))
        .max_w(px(170.))
        .px_2()
        .flex()
        .items_center()
        .gap_1p5()
        .child(leading)
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .child(label),
        )
}

