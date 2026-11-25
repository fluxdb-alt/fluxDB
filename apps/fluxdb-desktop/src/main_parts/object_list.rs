// 数据表概览：双击侧边栏「表」分组打开，列出当前库/schema 下的表。
// 数据来自连接器只读 list_objects；搜索只在本地过滤，不重复查询。

const OBJECT_LIST_ROW_HEIGHT: f32 = 32.;
const OBJECT_LIST_NAME_WIDTH: f32 = 240.;
const OBJECT_LIST_ROWS_WIDTH: f32 = 100.;
const OBJECT_LIST_SIZE_WIDTH: f32 = 110.;
const OBJECT_LIST_MODIFIED_WIDTH: f32 = 160.;

impl NavicatMain {
    fn active_object_list_tab_id(&self) -> Option<TabId> {
        self.controller.state().active_tab().and_then(|tab| {
            matches!(tab.kind, TabKind::ObjectList(_)).then_some(tab.id)
        })
    }

    /// 双击「表」分组：打开或激活概览页签，并后台拉取表清单。
    fn open_object_list(&mut self, parent: ObjectPath, cx: &mut Context<Self>) {
        match self.dispatch(AppCommand::OpenObjectList(Some(parent)), cx) {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => {
                self.start_object_list_load(tab_id, cx);
            }
            _ => {}
        }
    }

    /// 同一页签不并发读取；迟到的旧结果只写入仍存在的页签。
    fn start_object_list_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._object_list_load_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        self.dispatch(AppCommand::StartObjectListLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadObjectList(tab_id)) {
                        AppEvent::ObjectListLoaded { result, .. } => result,
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.object_list.load_failed").to_string(),
                            message: tr("app.object_list.load_failed").to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._object_list_load_tasks.remove(&tab_id);
                    // 页签可能在读取期间被关闭；迟到结果不再写回，避免全局报错。
                    if this.controller.state().tabs.iter().any(|tab| tab.id == tab_id) {
                        this.dispatch(AppCommand::FinishObjectListLoad { tab_id, result }, cx);
                    }
                    cx.notify();
                });
            });
        });
        self._object_list_load_tasks.insert(tab_id, task);
    }
}

fn object_list_content(
    tab_id: TabId,
    list: &ObjectListState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    sync_input_value(
        &this.object_list_search_input,
        &list.search,
        window,
        cx,
    );

    let query = list.search.trim().to_lowercase();
    let objects = object_list_filter(&list.objects, &query);

    let mut content = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(object_list_toolbar(tab_id, list.loading, this, colors, cx))
        .child(object_list_header(colors));

    if let Some(error) = &list.error {
        content = content.child(object_list_error(tab_id, error, colors, cx));
    } else if list.loading && list.objects.is_empty() {
        content = content.child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_size(ui_text_size(13.))
                .text_color(colors.muted)
                .child(loading_spinner(18.))
                .child(tr("app.object_list.loading")),
        );
    } else if objects.is_empty() {
        let message = if query.is_empty() {
            tr("app.object_list.empty")
        } else {
            tr("app.object_list.empty_search")
        };
        content = content.child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(ui_text_size(13.))
                .text_color(colors.muted)
                .child(message),
        );
    } else {
        let item_count = objects.len();
        let rows = objects.clone();
        content = content.child(
            uniform_list(
                ("object-list-rows", tab_id.0),
                item_count,
                cx.processor(move |_this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| {
                            let object = &rows[index];
                            let path = object.path.clone();
                            let row_name = object.path.name.clone();
                            object_list_row(object, colors)
                                .debug_selector(move || format!("object-list-row:{row_name}"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        if event.click_count >= 2 {
                                            this.dispatch(
                                                AppCommand::OpenDataEditor(path.clone()),
                                                cx,
                                            );
                                            cx.stop_propagation();
                                        }
                                    }),
                                )
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h(px(0.)),
        );
    }

    content.child(object_list_status(objects.len(), colors))
}

/// 本地过滤：表名或注释包含搜索词（大小写不敏感）；空词返回全部。
fn object_list_filter(objects: &[ObjectSummary], query: &str) -> Vec<ObjectSummary> {
    objects
        .iter()
        .filter(|object| {
            query.is_empty()
                || object.path.name.to_lowercase().contains(query)
                || object
                    .comment
                    .as_deref()
                    .is_some_and(|comment| comment.to_lowercase().contains(query))
        })
        .cloned()
        .collect()
}

fn object_list_toolbar(
    tab_id: TabId,
    loading: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(40.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .px_3()
        .child(
            div()
                .w(px(240.))
                .child(user_admin_input_box(
                    this.object_list_search_input.clone(),
                    colors,
                )),
        )
        .child(
            div()
                .id(("object-list-refresh", tab_id.0))
                .size(px(28.))
                .rounded(colors.radius)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(if loading {
                    loading_spinner(14.).into_any_element()
                } else {
                    app_icon(AppIcon::Refresh, 14., colors.muted)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.start_object_list_load(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn object_list_header(colors: UiColors) -> Div {
    div()
        .debug_selector(|| "object-list-header".to_string())
        .h(px(30.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(object_list_cell(
            tr("app.object_list.columns.name").to_string(),
            Some(OBJECT_LIST_NAME_WIDTH),
            colors,
        ))
        .child(object_list_cell(
            tr("app.object_list.columns.comment").to_string(),
            None,
            colors,
        ))
        .child(object_list_cell(
            tr("app.object_list.columns.rows").to_string(),
            Some(OBJECT_LIST_ROWS_WIDTH),
            colors,
        ))
        .child(object_list_cell(
            tr("app.object_list.columns.size").to_string(),
            Some(OBJECT_LIST_SIZE_WIDTH),
            colors,
        ))
        .child(object_list_cell(
            tr("app.object_list.columns.modified").to_string(),
            Some(OBJECT_LIST_MODIFIED_WIDTH),
            colors,
        ))
}

fn object_list_row(object: &ObjectSummary, colors: UiColors) -> Div {
    div()
        .w_full()
        .h(px(OBJECT_LIST_ROW_HEIGHT))
        .flex_none()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .text_size(ui_text_size(13.))
        .text_color(colors.text)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(OBJECT_LIST_NAME_WIDTH))
                .flex_none()
                .h_full()
                .px_4()
                .flex()
                .items_center()
                .gap_2()
                .overflow_hidden()
                .border_r_1()
                .border_color(colors.border_soft)
                .child(app_icon(AppIcon::Table, 14., colors.muted))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(object.path.name.clone()),
                ),
        )
        .child(object_list_cell(
            object.comment.clone().unwrap_or_default(),
            None,
            colors,
        ))
        .child(object_list_cell(
            object
                .rows
                .map(|rows| rows.to_string())
                .unwrap_or_else(|| "—".to_string()),
            Some(OBJECT_LIST_ROWS_WIDTH),
            colors,
        ))
        .child(object_list_cell(
            object
                .size_bytes
                .map(fluxdb_core::format_byte_length)
                .unwrap_or_else(|| "—".to_string()),
            Some(OBJECT_LIST_SIZE_WIDTH),
            colors,
        ))
        .child(object_list_cell(
            object.modified_at.clone().unwrap_or_else(|| "—".to_string()),
            Some(OBJECT_LIST_MODIFIED_WIDTH),
            colors,
        ))
}

fn object_list_cell(text: String, width: Option<f32>, colors: UiColors) -> Div {
    let cell = div()
        .h_full()
        .px_4()
        .flex()
        .items_center()
        .overflow_hidden()
        .border_r_1()
        .border_color(colors.border_soft)
        .child(div().min_w(px(0.)).overflow_hidden().text_ellipsis().child(text));
    match width {
        // 固定列不参与 flex 收缩，否则窄窗口或长内容会把列宽挤乱、表头与行对不上。
        Some(width) => cell.w(px(width)).flex_none(),
        None => cell.flex_1(),
    }
}

fn object_list_error(
    tab_id: TabId,
    error: &fluxdb_core::UserFacingError,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .text_center()
        .child(
            div()
                .text_size(ui_text_size(14.))
                .text_color(colors.text)
                .child(error.message.clone()),
        )
        .child(
            Button::new(("object-list-retry", tab_id.0))
                .label(tr("app.common.retry"))
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.start_object_list_load(tab_id, cx);
                }))
                .into_any_element(),
        )
}

fn object_list_status(count: usize, colors: UiColors) -> Div {
    div()
        .debug_selector(|| "object-list-status".to_string())
        .h(px(26.))
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.status_bg)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(tr("app.object_list.status_count").replace("%{count}", &count.to_string()))
        .child("·")
        .child(tr("app.object_list.status_hint"))
}

#[cfg(test)]
mod object_list_filter_tests {
    use super::*;

    fn table(name: &str, comment: Option<&str>) -> ObjectSummary {
        ObjectSummary {
            path: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".into()),
                schema: None,
                name: name.into(),
                kind: ObjectKind::Table,
            },
            rows: None,
            size_bytes: None,
            modified_at: None,
            comment: comment.map(str::to_string),
            stable: None,
        }
    }

    #[test]
    fn filters_by_name_or_comment_case_insensitively() {
        let objects = vec![
            table("Orders", None),
            table("users", Some("Customer Accounts")),
            table("audit", Some("write log")),
        ];
        let names = |query: &str| {
            object_list_filter(&objects, query)
                .into_iter()
                .map(|object| object.path.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(""), vec!["Orders", "users", "audit"]);
        assert_eq!(names("order"), vec!["Orders"]);
        assert_eq!(names("accounts"), vec!["users"]);
        assert_eq!(names("missing"), Vec::<String>::new());
    }
}
