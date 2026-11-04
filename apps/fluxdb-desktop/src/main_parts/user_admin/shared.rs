// 用户与权限公共壳：页签条、确认框、通用按钮与表单控件。
// 四个库的专属面板都复用这里的实现，避免同一交互四种外观。

/// 一个库的「用户与权限」面板。
///
/// 每个库的列表、页签集合、面板内容、保存路径都不同，但呈现方式一致（同一工具栏、
/// 同一页签条度量、同一 SQL 确认框）。公共壳只经这个 trait 调用具体库，
/// 不再在壳里堆 `if is_postgres / if is_sqlserver / ...` 的分支。
///
/// 只做渲染与命令分发：不拼 SQL、不直接访问数据库，所有写入都经 `AppCommand`。
trait UserAdminPanel {
    /// 渲染整个面板（含工具栏、左侧列表、右侧页签）。
    ///
    /// `state` 用于查连接配置；`this` 持有输入框实体，渲染前需把草稿同步进输入框。
    fn render(
        &self,
        state: &AppState,
        tab_id: TabId,
        admin: &UserAdminState,
        connection_name: String,
        this: &mut NavicatMain,
        window: &mut Window,
        colors: UiColors,
        cx: &mut Context<NavicatMain>,
    ) -> Div;
}

/// 按连接类型取对应面板实现；不支持用户管理的库返回 `None`。
///
/// 这是四个库唯一的汇合点：新增库只在这里加一条，不动其它库的代码。
fn user_admin_panel_for(kind: DatabaseKind) -> Option<&'static dyn UserAdminPanel> {
    match kind {
        DatabaseKind::Postgres => Some(&PostgresUserAdminPanel),
        DatabaseKind::SqlServer => Some(&SqlServerUserAdminPanel),
        // 达梦复用 Oracle 账号面板：账号 = schema，授权模型同源。
        DatabaseKind::Oracle | DatabaseKind::Dameng => Some(&OracleUserAdminPanel),
        // MySQL / TiDB 走 provider 驱动的默认面板。
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => Some(&MysqlUserAdminPanel),
        _ => None,
    }
}

/// 「用户与权限」页入口：按连接类型分派到对应库的面板。
fn user_admin_content(
    state: &AppState,
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == admin.connection_id);
    let connection_name = connection
        .map(|connection| connection.config.name.clone())
        .unwrap_or_else(|| tr("app.user_admin.connection").to_string());
    let panel = connection
        .and_then(|connection| user_admin_panel_for(connection.config.kind))
        // 未识别的连接类型：按 MySQL 面板兜底，面板内部会给出「不支持」提示。
        .unwrap_or(&MysqlUserAdminPanel);
    panel.render(
        state,
        tab_id,
        admin,
        connection_name,
        this,
        window,
        colors,
        cx,
    )
}

/// 该页签是否需要读取授权明细（MySQL：所属角色 / 权限页）。
///
/// 抽成函数是因为 `let` 链里的条件不能带 `||`，直接内联会编译失败。
fn user_admin_tab_needs_grants(tab: UserAdminDetailTab) -> bool {
    tab.is_mysql(UserAdminMySqlTab::MemberOf) || tab.is_mysql(UserAdminMySqlTab::Privileges)
}

/// 该页签是否需要读取角色成员（MySQL：所属角色 / 角色成员页）。
fn user_admin_tab_needs_members(tab: UserAdminDetailTab) -> bool {
    tab.is_mysql(UserAdminMySqlTab::MemberOf) || tab.is_mysql(UserAdminMySqlTab::Members)
}

/// 左侧账号/角色搜索框的占位文案：各库措辞不同，但都在渲染面板前统一刷新。
fn sys_user_admin_search_placeholder(
    this: &mut NavicatMain,
    placeholder: SharedString,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    this.user_admin_search_input
        .update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
}

fn sync_input_value(
    input: &Entity<InputState>,
    expected: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    if input.read(cx).value().as_ref() != expected {
        input.update(cx, |input, cx| input.set_value(expected.to_string(), window, cx));
    }
}

fn sync_select_value(
    select: &Entity<SelectState<SearchableVec<String>>>,
    expected: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    let selected = select.read(cx).selected_value().map(String::as_str);
    if selected == Some(expected) || (expected.is_empty() && selected.is_none()) {
        return;
    }
    select.update(cx, |select, cx| {
        select.set_selected_value(&expected.to_string(), window, cx);
    });
}

fn user_admin_badge(text: String, colors: UiColors) -> Div {
    div()
        .max_w(px(180.))
        .h(px(22.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .overflow_hidden()
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(div().truncate().child(text))
}

/// 用户与权限页的页签条（MySQL / PG / SQL Server / Oracle 共用一套外观）。
///
/// `tabs` 里的 key 是稳定英文标识，不进语言包；它与 `selector_prefix` 组成
/// `debug_selector`，让 E2E 能按名字点页签。切换动作由调用方给，各库按需附带加载。
fn user_admin_detail_tab_strip<F>(
    tabs: Vec<(UserAdminDetailTab, &'static str, SharedString)>,
    active_tab: UserAdminDetailTab,
    selector_prefix: &'static str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
    on_select: F,
) -> Div
where
    F: Fn(&mut NavicatMain, UserAdminDetailTab, &mut Context<NavicatMain>) + Clone + 'static,
{
    let mut strip = div()
        .h(px(36.))
        .min_w(px(0.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_end();
    for (detail_tab, key, label) in tabs {
        let active = active_tab == detail_tab;
        let selector = format!("{selector_prefix}-{key}");
        let on_select = on_select.clone();
        strip = strip.child(
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .h(px(36.))
                // 等分条宽：按文字撑开时标签长度不同、宽度参差，窄窗口下最后一个还会被裁掉。
                .flex_1()
                .min_w(px(0.))
                .justify_center()
                .px_3()
                .border_r_1()
                .border_color(colors.border)
                .bg(if active { colors.panel_bg } else { colors.panel_alt })
                .cursor_pointer()
                .hover(move |style| style.bg(if active { colors.panel_bg } else { colors.hover }))
                .flex()
                .items_center()
                .text_size(px(12.))
                .font_weight(if active {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if active { colors.text } else { colors.muted })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        on_select(this, detail_tab, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(label),
        );
    }
    strip
}

fn user_admin_check_box_visual(checked: bool, colors: UiColors) -> Div {
    div()
        .size(px(16.))
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(if checked { rgb(0x2563eb) } else { colors.border })
        .bg(if checked { rgb(0x2563eb) } else { colors.input_bg })
        .shadow(vec![box_shadow(
            px(0.),
            px(1.),
            px(3.),
            px(0.),
            hsla(0., 0., 0., 0.14),
        )])
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |this| this.child(app_icon(AppIcon::Check, 12., rgb(0xffffff))))
}

fn user_admin_sql_preview_code_view(
    editor_key: SharedString,
    sql: &str,
    colors: UiColors,
) -> Div {
    div()
        .size_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .child(
            TextView::markdown(editor_key, user_admin_sql_preview_markdown(sql))
                .selectable(true)
                .scrollable(true)
                .text_size(px(12.))
                .font_family(EDITOR_FONT)
                .size_full(),
        )
}

fn user_admin_input_border_color(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        if colors.is_dark { rgb(0x8ab4ff) } else { rgb(0x111111) }
    } else {
        colors.border
    }
}

fn user_admin_input_hover_border_color(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        user_admin_input_border_color(true, colors)
    } else if colors.is_dark {
        rgb(0x5b6675)
    } else {
        rgb(0xb8c0cc)
    }
}

fn user_admin_input_shadow(focused: bool) -> Vec<gpui::BoxShadow> {
    vec![box_shadow(
        px(0.),
        px(1.),
        px(4.),
        px(0.),
        hsla(0., 0., 0., if focused { 0.08 } else { 0.11 }),
    )]
}

/// 「保存」确认框：展示本次要执行的 SQL，确认后才真正提交。
///
/// 五种库（MySQL / PostgreSQL / SQL Server / Oracle / 达梦）共用这一个弹框，
/// 内容就是各库「SQL 预览」页里同一份渲染结果，保证「看到的就是跑的」。
/// 点遮罩、`Esc` 或「取消」只关弹框不执行；`applying` 期间按钮禁用防重复提交。
fn user_admin_sql_preview_modal(
    tab_id: TabId,
    pending: &fluxdb_app::UserAdminPendingSql,
    applying: bool,
    error: Option<&fluxdb_core::UserFacingError>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql = pending.sql.clone();
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .debug_selector(|| "user-admin-sql-preview-modal".into())
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
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(680.))
                .max_w(px(680.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., 0.22),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .px_4()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon_box(
                            if pending.danger {
                                AppIcon::Trash
                            } else {
                                AppIcon::FileSql
                            },
                            24.,
                            16.,
                            if pending.danger {
                                rgb(0xff3b45)
                            } else {
                                rgb(0x2563eb)
                            },
                        ))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_size(px(15.))
                                .child(tr("app.user_admin.confirm_sql_title")),
                        )
                        .child(div().flex_1())
                        .child(
                            user_admin_icon_button(AppIcon::Close, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .px_4()
                        .pt_3()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(tr("app.user_admin.confirm_sql_hint")),
                )
                .child(
                    // 固定高度：`user_admin_sql_preview_code_view` 是 `size_full`，
                    // 放进由内容撑开的容器会坍缩成 0 高（用户只看到一条细线）。
                    div()
                        .m_4()
                        .h(px(280.))
                        .child(user_admin_sql_preview_code_view(
                            SharedString::from(format!("user-admin-sql-preview-modal-{}", tab_id.0)),
                            &sql,
                            colors,
                        )),
                )
                .when_some(error, |this, error| {
                    this.child(
                        div()
                            .px_4()
                            .pb_2()
                            .text_size(px(12.))
                            .text_color(rgb(0xff3b45))
                            .child(error.message.clone()),
                    )
                })
                .child(
                    div()
                        .h(px(58.))
                        .px_4()
                        .border_t_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            user_admin_button(
                                tr("app.common.cancel"),
                                AppIcon::Close,
                                false,
                                applying,
                                colors,
                            )
                            .debug_selector(|| "user-admin-sql-preview-cancel".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::ClearUserAdminPendingSql(tab_id),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            user_admin_button(
                                if applying {
                                    tr("app.user_admin.executing")
                                } else {
                                    tr("app.user_admin.execute_sql")
                                },
                                AppIcon::Play,
                                pending.danger,
                                applying,
                                colors,
                            )
                            .debug_selector(|| "user-admin-sql-preview-execute".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if !applying {
                                        this.execute_user_admin_pending_sql(tab_id, cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn user_admin_icon_button(icon: AppIcon, colors: UiColors) -> Div {
    div()
        .size(px(28.))
        .rounded(colors.radius)
        .cursor_pointer()
        .hover(|style| style.bg(colors.hover))
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(icon, 16., colors.muted))
}

fn user_admin_add_user_button(disabled: bool, colors: UiColors) -> Stateful<Div> {
    let color = if disabled { colors.muted } else { rgb(0x2563eb) };
    div()
        .size(px(32.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(if disabled { colors.panel_alt } else { colors.input_bg })
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.55))
        .hover(move |style| if disabled { style } else { style.bg(colors.hover) })
        .flex()
        .items_center()
        .justify_center()
        .id("user-admin-add-user")
        .debug_selector(|| "user-admin-add-user".into())
        .tooltip(|window, cx| {
            Tooltip::new(tr("app.user_admin.add_user")).build(window, cx)
        })
        .child(app_icon(AppIcon::Plus, 16., color))
}

fn user_admin_button(
    label: impl Into<SharedString>,
    icon: AppIcon,
    danger: bool,
    disabled: bool,
    colors: UiColors,
) -> Div {
    let label = label.into();
    let fg = if danger {
        rgb(0xff3b45)
    } else if disabled {
        colors.muted
    } else {
        colors.text
    };
    div()
        .h(px(30.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(if disabled {
            colors.panel_alt
        } else {
            colors.input_bg
        })
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.55))
        .hover(move |style| if disabled { style } else { style.bg(colors.hover) })
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(fg)
        .child(app_icon_box(icon, 18., 14., fg))
        .child(label)
}

fn user_admin_input_box(input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .w_full()
        .h(px(32.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h(px(28.))
                .px_2()
                .text_size(px(13.)),
        )
}

fn user_admin_empty_row(text: impl Into<SharedString>, colors: UiColors) -> Div {
    let text = text.into();
    div()
        .p_4()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(text)
}

fn user_admin_error_row(text: &str, _colors: UiColors) -> Div {
    div()
        .p_4()
        .text_size(px(12.))
        .text_color(rgb(0xff3b45))
        .child(localize_backend_error_text(text))
}
