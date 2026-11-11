const CREATE_DATABASE_CHARSETS: &[(&str, &[&str])] = &[
    ("utf8mb4", &["utf8mb4_unicode_ci", "utf8mb4_0900_ai_ci", "utf8mb4_general_ci", "utf8mb4_bin"]),
    ("utf8", &["utf8_general_ci", "utf8_unicode_ci", "utf8_bin"]),
    ("latin1", &["latin1_swedish_ci", "latin1_general_ci", "latin1_bin"]),
    ("gbk", &["gbk_chinese_ci", "gbk_bin"]),
    ("big5", &["big5_chinese_ci", "big5_bin"]),
    ("ascii", &["ascii_general_ci", "ascii_bin"]),
    ("ucs2", &["ucs2_unicode_ci", "ucs2_general_ci", "ucs2_bin"]),
    ("utf16", &["utf16_unicode_ci", "utf16_general_ci", "utf16_bin"]),
    ("utf32", &["utf32_unicode_ci", "utf32_general_ci", "utf32_bin"]),
];

fn default_collation_for_charset(charset: &str) -> &'static str {
    CREATE_DATABASE_CHARSETS
        .iter()
        .find(|(name, _)| *name == charset)
        .and_then(|(_, collations)| collations.first().copied())
        .unwrap_or("utf8mb4_unicode_ci")
}

fn collations_for_charset(charset: &str) -> &'static [&'static str] {
    CREATE_DATABASE_CHARSETS
        .iter()
        .find(|(name, _)| *name == charset)
        .map(|(_, collations)| *collations)
        .unwrap_or_else(|| {
            CREATE_DATABASE_CHARSETS
                .first()
                .map(|(_, collations)| *collations)
                .unwrap_or(&[])
        })
}

fn create_database_charset_options() -> Vec<String> {
    CREATE_DATABASE_CHARSETS
        .iter()
        .map(|(charset, _)| (*charset).to_string())
        .collect()
}

/// PostgreSQL server 可用的 ENCODING 名。
fn pg_create_database_encoding_options() -> Vec<String> {
    ["UTF8", "SQL_ASCII", "LATIN1", "LATIN2", "LATIN5", "LATIN9", "EUC_JP", "EUC_KR", "GB18030", "GBK", "WIN1251", "WIN1252", "WIN1256"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// PostgreSQL locale（LC_COLLATE/LC_CTYPE）候选。
fn pg_locale_options() -> &'static [&'static str] {
    &["C", "POSIX", "en_US.UTF-8", "zh_CN.UTF-8", "zh_CN.GB18030", "de_DE.UTF-8", "ja_JP.UTF-8", "ko_KR.UTF-8"]
}

fn create_database_collation_options(charset: &str) -> Vec<String> {
    collations_for_charset(charset)
        .iter()
        .map(|collation| (*collation).to_string())
        .collect()
}

/// SQL Server 常用排序规则；服务器允许任意已安装排序规则名，下拉只是常用候选。
fn sqlserver_create_database_collation_options() -> Vec<String> {
    [
        "SQL_Latin1_General_CP1_CI_AS",
        "Latin1_General_CI_AS",
        "Latin1_General_100_CI_AS_SC",
        "Chinese_PRC_CI_AS",
        "Chinese_PRC_100_CI_AS_SC",
        "Japanese_CI_AS",
        "Korean_Wansung_CI_AS",
    ]
    .iter()
    .map(|value| (*value).to_string())
    .collect()
}

fn create_database_modal(
    form: CreateDatabaseForm,
    name_input: Entity<InputState>,
    charset_select: Entity<SelectState<SearchableVec<String>>>,
    collation_select: Entity<SelectState<SearchableVec<String>>>,
    owner_input: Entity<InputState>,
    template_input: Entity<InputState>,
    first_collection_input: Entity<InputState>,
    running: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let needs_charset = matches!(
        form.database_kind,
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb | DatabaseKind::Postgres
    );
    let is_postgres = form.database_kind == DatabaseKind::Postgres;
    let is_sqlserver = form.database_kind == DatabaseKind::SqlServer;
    // MongoDB 的库由首个集合隐式创建：数据库名与集合名都必须填。
    let is_mongo = form.database_kind == DatabaseKind::MongoDb;
    let can_submit = !running
        && !form.database_name.trim().is_empty()
        && (!is_mongo || !form.first_collection.trim().is_empty())
        && (!needs_charset
            || (!form.charset.trim().is_empty() && !form.collation.trim().is_empty()));

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_create_database_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(460.))
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
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("CreateDatabaseModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_create_database_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(create_database_header(colors, cx))
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(create_database_text_field(
                            tr("app.database.field.name"),
                            name_input,
                            running,
                            colors,
                        ))
                        .when(is_mongo, |this| {
                            this.child(create_database_text_field(
                                tr("app.mongo.create_database.first_collection"),
                                first_collection_input,
                                running,
                                colors,
                            ))
                        })
                        .when(needs_charset && !is_sqlserver, |this| {
                            this.child(create_database_select_field(
                                if is_postgres {
                                    tr("app.database.field.encoding")
                                } else {
                                    tr("app.database.field.charset")
                                },
                                charset_select,
                                if is_postgres { "UTF8" } else { "utf8mb4" },
                                running,
                                colors,
                            ))
                            .child(create_database_select_field(
                                if is_postgres {
                                    tr("app.database.field.locale")
                                } else {
                                    tr("app.database.field.collation")
                                },
                                collation_select.clone(),
                                if is_postgres { "C" } else { "utf8mb4_unicode_ci" },
                                running,
                                colors,
                            ))
                        })
                        .when(is_sqlserver && !needs_charset, |this| {
                            this.child(create_database_select_field(
                                tr("app.database.field.collation"),
                                collation_select,
                                "SQL_Latin1_General_CP1_CI_AS",
                                running,
                                colors,
                            ))
                        })
                        .when(is_postgres, |this| {
                            this.child(create_database_text_field(tr("app.database.field.owner"), owner_input, running, colors))
                                .child(create_database_text_field(tr("app.database.field.template"), template_input, running, colors))
                        }),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("create-database-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_create_database_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("create-database-confirm")
                                .label(if running {
                                    tr("app.common.running")
                                } else {
                                    tr("app.common.execute")
                                })
                                .primary()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_create_database(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn create_database_header(colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .px_5()
        .pt_4()
        .pb_4()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Database, 16., colors.text))
                .child(
                    div()
                        .text_size(ui_text_size(17.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.database.create.title")),
                ),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 15., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_create_database_modal(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

/// 新建 PostgreSQL schema 弹框：仅一个名称输入 + 执行/取消。
fn create_schema_modal(
    database_name: String,
    name_input: Entity<InputState>,
    running: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let can_submit = !running;
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_create_schema_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(420.))
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
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("CreateSchemaModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_create_schema_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(create_schema_header(database_name, colors, cx))
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(create_database_text_field(
                            tr("app.schema.field.name"),
                            name_input,
                            running,
                            colors,
                        )),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("create-schema-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_create_schema_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("create-schema-confirm")
                                .label(if running {
                                    tr("app.common.running")
                                } else {
                                    tr("app.common.execute")
                                })
                                .primary()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_create_schema(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn create_schema_header(database_name: String, colors: UiColors, cx: &mut Context<NavicatMain>) -> impl IntoElement {
    div()
        .px_5()
        .pt_4()
        .pb_4()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Database, 16., colors.text))
                .child(
                    div()
                        .text_size(ui_text_size(17.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.schema.create.title").replace("%{database}", &database_name)),
                ),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 15., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_create_schema_modal(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn create_database_text_field(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    running: bool,
    colors: UiColors,
) -> impl IntoElement {
    let label: SharedString = label.into();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(create_database_input_frame(input, running, colors))
}

fn create_database_select_field(
    label: impl Into<SharedString>,
    select: Entity<SelectState<SearchableVec<String>>>,
    placeholder: &'static str,
    running: bool,
    colors: UiColors,
) -> impl IntoElement {
    let label: SharedString = label.into();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .h(px(38.))
                .child(
                    Select::new(&select)
                        .placeholder(placeholder)
                        .disabled(running)
                        .w_full()
                        .h_full()
                        .menu_width(px(420.)),
                ),
        )
}

fn create_database_input_frame(
    input: Entity<InputState>,
    running: bool,
    colors: UiColors,
) -> impl IntoElement {
    div()
        .h(px(38.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .disabled(running)
                .w_full()
                .h_full()
                .text_size(ui_text_size(14.)),
        )
}
