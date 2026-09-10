/// 顶部栏图标按钮：统一尺寸/圆角/手形光标/hover/tooltip，直接以 AppIcon 渲染。
/// 「仅图标」按钮的通用封装，用于顶部栏收起侧边栏 / 首页等无标签按钮。
fn topbar_icon_button(
    icon: AppIcon,
    tooltip: &'static str,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .size(px(30.))
        .ml(px(2.))
        .rounded(colors.radius_lg)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .id(tooltip)
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(app_icon_box(icon, 30., 17., colors.muted))
}

/// 顶部栏连接信息文本：超长省略 + 悬停 tooltip 展示完整连接名。
fn connection_info_label(name: String, colors: UiColors) -> Stateful<Div> {
    let tooltip_name = name.clone();
    div()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .text_color(colors.text)
        .id("topbar-connection-name")
        .tooltip(move |window, cx| Tooltip::new(tooltip_name.clone()).build(window, cx))
        .child(name)
}

/// 顶部栏：只保留原生窗口控制（红绿灯预留区）、侧边栏收起、首页与当前连接信息。
/// 展开/收起两态内容不同：
/// - 展开（show_connection_browser）：红绿灯 + 收起侧边栏 + 首页 + 连接信息。
/// - 收起：红绿灯 + 连接信息（收起/首页按钮移入收起侧边栏条）。
fn topbar(
    state: &AppState,
    show_connection_browser: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 当前活动标签所属连接名；无活动标签或 Settings 标签（无连接）时不展示。
    let connection_info = state.active_tab().and_then(|tab| {
        tab_workspace_scope(tab).map(|scope| connection_name(state, scope.connection_id))
    });

    div()
        .h(px(36.))
        .bg(colors.app_bg)
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_1()
        // 保留 macOS 原生红绿灯（关闭/最小化/全屏）预留区。
        .pl(px(84.))
        .pr_3()
        // 侧边栏展开/收起切换按钮（图标随状态变化）+ 首页按钮，两态都保留在顶部栏。
        .child(
            topbar_icon_button(
                if show_connection_browser {
                    AppIcon::PanelLeftClose
                } else {
                    AppIcon::PanelLeftOpen
                },
                if show_connection_browser {
                    "收起侧边栏"
                } else {
                    "展开侧边栏"
                },
                colors,
            )
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.show_connection_browser = !this.show_connection_browser;
                cx.stop_propagation();
                cx.notify();
            })),
        )
        .child(topbar_icon_button(AppIcon::Home, "首页", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.dispatch(AppCommand::DeactivateTab, cx);
                cx.stop_propagation();
            }),
        ))
        .when_some(connection_info, |this, name| {
            // 当前打开的数据库连接信息（文本），与活动表/库保持关联。
            this.child(
                div()
                    .flex_none()
                    .max_w(px(300.))
                    .h(px(28.))
                    .px_3()
                    .rounded(colors.radius_lg)
                    .bg(colors.panel_bg)
                    .border_1()
                    .border_color(colors.border)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(app_icon_box(AppIcon::Database, 26., 15., colors.muted))
                    .child(connection_info_label(name, colors)),
            )
        })
        .child(
            div()
                .flex_1()
                .h_full()
                .on_mouse_down(MouseButton::Left, |event, window, cx| {
                    if event.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                    cx.stop_propagation();
                }),
        )
        // 顶部栏最右侧：历史 + 设置（业务按钮移除后收纳在此，保持功能可达）。
        .child(topbar_icon_button(AppIcon::CalendarClock, "历史", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.toggle_history(window, cx);
                cx.stop_propagation();
            }),
        ))
        .child(topbar_icon_button(AppIcon::Settings, "设置", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.dispatch(AppCommand::OpenSettings, cx);
                cx.stop_propagation();
            }),
        ))
}

fn new_connection_modal(
    kind: DatabaseKind,
    active_tab: NewConnectionTab,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    editing: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let view = cx.entity();
    let title = if editing {
        "编辑连接"
    } else {
        "新建连接"
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.08, 0.62)
        } else {
            opaque_grey(0.6, 0.36)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .relative()
                .w(px(820.))
                .h(px(560.))
                .rounded(colors.radius_lg)
                .bg(colors.panel_bg)
                .border_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_color(colors.text)
                .key_context("NewConnectionModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_new_connection(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div().absolute().top(px(12.)).right(px(14.)).child(
                        Button::new("new-connection-close")
                            .label("×")
                            .ghost()
                            .w(px(34.))
                            .h(px(34.))
                            .text_size(px(24.))
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.cancel_new_connection(cx);
                                    });
                                    cx.stop_propagation();
                                }
                            }),
                    ),
                )
                .child(
                    div()
                        .h(px(62.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().w(px(34.))),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .gap_5()
                        .px_5()
                        .pb_2()
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(350.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    div()
                                        .h(px(34.))
                                        .rounded(colors.radius_lg)
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(colors.input_bg)
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_size(px(14.))
                                        .text_color(colors.muted)
                                        .child(app_icon(AppIcon::Search, 14., colors.muted))
                                        .child("搜索数据库类型"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "MySQL",
                                            DatabaseKind::MySql,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "TiDB",
                                            DatabaseKind::TiDb,
                                            kind,
                                            true,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "SQLite",
                                            DatabaseKind::Sqlite,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "Redis",
                                            DatabaseKind::Redis,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "MongoDB",
                                            DatabaseKind::MongoDb,
                                            kind,
                                            true,
                                            colors,
                                            cx,
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .overflow_hidden()
                                .child(connection_tab_bar(active_tab, colors, cx))
                                .child(div().h(px(1.)).bg(colors.border))
                                .child(
                                    div()
                                        .id("new-connection-form-scroll")
                                        .flex_1()
                                        .min_h(px(0.))
                                        .overflow_scroll()
                                        .scrollbar_width(px(8.))
                                        .child(new_connection_tab_content(
                                            active_tab, kind, form, inputs, colors, window, cx,
                                        )),
                                ),
                        ),
                )
                .child(
                    div()
                        .h(px(58.))
                        .flex_none()
                        .flex()
                        .items_end()
                        .gap_3()
                        .px_5()
                        .pb_5()
                        .when_some(form.test_status.as_ref(), |this, status| {
                            this.child(
                                connection_test_status(status)
                                    .flex_1()
                                    .min_w(px(0.))
                                    .justify_start(),
                            )
                        })
                        .when_none(&form.test_status, |this| {
                            this.child(div().flex_1().min_w(px(0.)))
                        })
                        .child(Button::new("new-connection-test").label("测试").on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.test_new_connection(cx);
                                });
                                cx.stop_propagation();
                            }
                        }))
                        .child(
                            Button::new("new-connection-save")
                                .label("保存并连接")
                                .primary()
                                .w(px(116.))
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.create_connection_from_form(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                }),
                        ),
                ),
        )
}

fn kind_tile(
    label: &'static str,
    kind: DatabaseKind,
    selected: DatabaseKind,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let selected = kind == selected;
    let disabled = locked && !selected;

    div()
        .w(px(168.))
        .h(px(112.))
        .rounded(colors.radius_lg)
        .bg(if disabled {
            if colors.is_dark {
                rgb(0x1b1e23)
            } else {
                rgb(0xf1f2f4)
            }
        } else if selected {
            if colors.is_dark {
                rgb(0x1a3157)
            } else {
                rgb(0xe9f1ff)
            }
        } else {
            colors.panel_alt
        })
        .border_1()
        .border_color(if disabled {
            if colors.is_dark {
                rgb(0x2b3038)
            } else {
                rgb(0xd6dae0)
            }
        } else if selected {
            rgb(0x5b8def)
        } else {
            colors.border
        })
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .when(!locked, |this| {
            this.hover(move |style| {
                style
                    .bg(if selected {
                        if colors.is_dark {
                            rgb(0x213c66)
                        } else {
                            rgb(0xe3edff)
                        }
                    } else {
                        colors.hover
                    })
                    .border_color(if selected {
                        rgb(0x3478f6)
                    } else {
                        rgb(0xb9c0ca)
                    })
            })
        })
        .child(
            div()
                .size(px(48.))
                .rounded(colors.radius_lg)
                .bg(if disabled {
                    if colors.is_dark {
                        rgb(0x24272c)
                    } else {
                        rgb(0xe3e5e8)
                    }
                } else if selected {
                    rgb(0x24272d)
                } else {
                    rgb(0x2a2d33)
                })
                .flex()
                .items_center()
                .justify_center()
                .child(database_kind_icon(kind)),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if disabled {
                    if colors.is_dark {
                        rgb(0x6f7783)
                    } else {
                        rgb(0x9aa1ac)
                    }
                } else if selected {
                    if colors.is_dark {
                        rgb(0x9cc2ff)
                    } else {
                        rgb(0x1d4f91)
                    }
                } else {
                    colors.text
                })
                .child(label),
        )
        .when(!locked, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.set_new_connection_kind(kind, window, cx);
                    cx.stop_propagation();
                }),
            )
        })
}

fn database_kind_icon(kind: DatabaseKind) -> impl IntoElement {
    img(database_kind_icon_path(kind)).size(px(32.))
}

fn database_kind_icon_path(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "db/mysql.svg",
        DatabaseKind::TiDb => "db/tidb.svg",
        DatabaseKind::Sqlite => "db/sqlite.svg",
        DatabaseKind::MongoDb => "db/mongodb.svg",
        DatabaseKind::Redis => "db/redis.svg",
    }
}

/// 新建连接表单顶部分页栏：用 gpui-component 分段 TabBar 渲染（与「连接信息」表单统一样式）。
fn connection_tab_bar(
    active_tab: NewConnectionTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    const TABS: [(NewConnectionTab, &str); 4] = [
        (NewConnectionTab::Connection, "连接信息"),
        (NewConnectionTab::Tls, "TLS/SSL"),
        (NewConnectionTab::Ssh, "SSH 隧道/代理"),
        (NewConnectionTab::Advanced, "高级"),
    ];
    let _ = colors;
    let selected_index = TABS
        .iter()
        .position(|(tab, _)| *tab == active_tab)
        .unwrap_or(0);
    let view = cx.entity();
    TABS.iter()
        .map(|(_, label)| Tab::from(*label))
        .fold(
            TabBar::new("new-connection-tabs")
                .segmented()
                .small()
                .selected_index(selected_index)
                .on_click(move |index, _, cx| {
                    if let Some((tab, _)) = TABS.get(*index) {
                        let _ = view.update(cx, |this, cx| {
                            this.set_new_connection_tab(*tab, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        )
        .into_element()
}

fn new_connection_tab_content(
    tab: NewConnectionTab,
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    match tab {
        NewConnectionTab::Connection => connection_form(kind, form, inputs, colors, window, cx),
        // TLS / SSH / Advanced：Redis 与 MySQL/TiDB 都有实际表单，其余类型显示提示占位。
        NewConnectionTab::Tls => match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::Redis => {
                tls_form(kind, form, inputs, colors, window, cx)
            }
            _ => redis_only_settings_hint(colors),
        },
        NewConnectionTab::Ssh => match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::Redis => {
                ssh_form(kind, form, inputs, colors, window, cx)
            }
            _ => redis_only_settings_hint(colors),
        },
        NewConnectionTab::Advanced => match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => mysql_advanced_form(form, inputs, colors, window, cx),
            DatabaseKind::Redis => advanced_form(form, inputs, colors, window, cx),
            _ => redis_only_settings_hint(colors),
        },
    }
}

/// 非 Redis 连接在 TLS/SSH/高级页签下的提示占位。
fn redis_only_settings_hint(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .w_full()
                .h(px(120.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_center()
                .px_4()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("该设置仅对 Redis 连接可用"),
        )
}

/// TLS 页签：启用开关 + 证书/私钥文件路径 + SNI + 校验证书。
fn tls_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let enabled = form.tls_enabled;
    let is_mysql = matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("传输层安全 (TLS)", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "启用 TLS",
                    ConnectionToggleField::TlsEnabled,
                    form.tls_enabled,
                    colors,
                    cx,
                )),
        )
        // TLS 参数明细：启用时正常显示，未启用时整体置灰。
        .when(enabled, |this| {
            this.child(tls_parameters_block(
                is_mysql, form, inputs, colors, window, cx,
            ))
        })
        .when(!enabled, |this| {
            this.child(tls_parameters_block(
                is_mysql, form, inputs, colors, window, cx,
            )
            .opacity(0.45))
        })
}

/// TLS 参数明细块：CA 证书 / 客户端证书 / 客户端密钥 / SNI / 校验证书。
/// MySQL/TiDB 额外渲染 ssl_mode 与字符集。
fn tls_parameters_block(
    is_mysql: bool,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .when(is_mysql, |this| {
            this.child(mysql_tls_mode_block(form, inputs, colors, window, cx))
        })
        .child(
            h_form()
                .label_width(px(112.))
                .child(file_field_row_light(
                    "CA 证书",
                    ConnectionField::TlsCa,
                    "选择 TLS CA 证书文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(file_field_row_light(
                    "客户端证书",
                    ConnectionField::TlsClientCert,
                    "选择 TLS 客户端证书文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(file_field_row_light(
                    "客户端密钥",
                    ConnectionField::TlsClientKey,
                    "选择 TLS 客户端私钥文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "SNI / 主机名",
                    ConnectionField::TlsSni,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(toggle_row_light(
                    "校验服务器证书",
                    ConnectionToggleField::TlsVerify,
                    form.tls_verify,
                    colors,
                    cx,
                )),
        )
}

/// MySQL/TiDB TLS 模式块：SSL 模式 + 连接字符集。
fn mysql_tls_mode_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "SSL 模式",
                    ConnectionField::MysqlTlsSslMode,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "连接字符集",
                    ConnectionField::MysqlCharset,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// SSH 页签：启用开关 + 跳板机参数 + 认证方式。
fn ssh_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 认证方式："password" 显示/可用密码，私钥方式显示/可用私钥与口令。
    let password_mode = form.ssh_auth != "private_key";
    let enabled = form.ssh_enabled;
    let is_mysql = matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("SSH 隧道", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "启用 SSH 隧道",
                    ConnectionToggleField::SshEnabled,
                    form.ssh_enabled,
                    colors,
                    cx,
                )),
        )
        // 隧道参数：启用时正常显示，未启用时整体置灰。
        .when(enabled, |this| {
            this.child(ssh_tunnel_block(form, inputs, colors, window, cx))
        })
        .when(!enabled, |this| {
            this.child(ssh_tunnel_block(form, inputs, colors, window, cx).opacity(0.45))
        })
        // MySQL/TiDB 额外：SSH 连接超时 + 心跳间隔。
        .when(is_mysql, |this| {
            this.child(mysql_ssh_tuning_block(inputs, colors, window, cx))
        })
        // 密码认证区块：私钥模式时置灰。
        .when(password_mode, |this| {
            this.child(ssh_password_block(inputs, colors, window, cx))
        })
        .when(!password_mode, |this| {
            this.child(ssh_password_block(inputs, colors, window, cx).opacity(0.45))
        })
        // 私钥认证区块：密码模式时置灰。
        .when(password_mode, |this| {
            this.child(ssh_private_key_block(inputs, colors, window, cx).opacity(0.45))
        })
        .when(!password_mode, |this| {
            this.child(ssh_private_key_block(inputs, colors, window, cx))
        })
}

/// SSH 隧道参数块：主机 / 端口 / 用户名 / 认证方式。
fn ssh_tunnel_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "主机",
                    ConnectionField::SshHost,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "端口",
                    ConnectionField::SshPort,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "用户名",
                    ConnectionField::SshUsername,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(connection_select_field(
                    "认证方式",
                    &inputs.ssh_auth_select,
                    "",
                )),
        )
}

/// SSH 密码认证块。
fn ssh_password_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "密码",
                    ConnectionField::SshPassword,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// SSH 私钥认证块：私钥文件 + 口令。
fn ssh_private_key_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(file_field_row_light(
                    "私钥文件",
                    ConnectionField::SshPrivateKey,
                    "选择 SSH 私钥文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "口令 (passphrase)",
                    ConnectionField::SshPassphrase,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// MySQL/TiDB SSH 调优块：连接超时 + 心跳间隔。
fn mysql_ssh_tuning_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("SSH 调优", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "连接超时 (秒, 0=继承)",
                    ConnectionField::MysqlSshConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "心跳间隔 (秒, 0=不发送)",
                    ConnectionField::MysqlSshKeepalive,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// MySQL/TiDB 高级页签：代理 + 连接/查询/空闲 TTL 超时 + TCP 保活。
fn mysql_advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let proxy_enabled = form.mysql_proxy_enabled;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("代理", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "启用代理",
                    ConnectionToggleField::MysqlProxyEnabled,
                    form.mysql_proxy_enabled,
                    colors,
                    cx,
                )),
        )
        // 代理参数：启用时正常显示，未启用时整体置灰。
        .when(proxy_enabled, |this| {
            this.child(mysql_proxy_block(form, inputs, colors, window, cx))
        })
        .when(!proxy_enabled, |this| {
            this.child(mysql_proxy_block(form, inputs, colors, window, cx).opacity(0.45))
        })
        .child(redis_section_label("高级连接选项", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "建连超时 (秒)",
                    ConnectionField::MysqlConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "查询超时 (秒, 0=不设限)",
                    ConnectionField::MysqlQueryTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "空闲 TTL (秒, 0=不回收)",
                    ConnectionField::MysqlIdleTtl,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(
            div()
                .pl_1()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("待连接复用启用后生效"),
        )
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "TCP 长连接保活",
                    ConnectionToggleField::MysqlTcpKeepalive,
                    form.mysql_tcp_keepalive,
                    colors,
                    cx,
                )),
        )
}

/// MySQL/TiDB 代理参数块：类型 / 主机 / 端口 / 用户名 / 密码。
fn mysql_proxy_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "代理类型",
                    ConnectionField::MysqlProxyType,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "主机",
                    ConnectionField::MysqlProxyHost,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "端口",
                    ConnectionField::MysqlProxyPort,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "用户名 (可选)",
                    ConnectionField::MysqlProxyUsername,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "密码 (可选)",
                    ConnectionField::MysqlProxyPassword,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// 高级页签：Sentinel / Cluster / 云自动发现 / 连接串导入。
fn advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("Sentinel 模式", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "主库名",
                    ConnectionField::SentinelMasterName,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "节点列表 (host:port, 逗号或换行分隔)",
                    ConnectionField::SentinelEndpoints,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(redis_section_label("Cluster 模式", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "起始节点 (host:port, 逗号或换行分隔)",
                    ConnectionField::ClusterStartNodes,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(toggle_row_light(
                    "允许重定向到从节点",
                    ConnectionToggleField::ClusterAllowReadonly,
                    form.cluster_allow_readonly,
                    colors,
                    cx,
                )),
        )
        .child(redis_section_label("云自动发现", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(connection_select_field(
                    "云提供方",
                    &inputs.cloud_provider_select,
                    "",
                ))
                .child(field_row_light(
                    "订阅 / 账号",
                    ConnectionField::CloudSubscription,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "资源 / 数据库",
                    ConnectionField::CloudResource,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(redis_section_label("连接串导入", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(discovery_uri_row(inputs, colors, window, cx)),
        )
}

/// 连接串导入行：URI 输入框 + 「导入/发现」按钮。
fn discovery_uri_row(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label("连接串 / URI").items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(ConnectionField::DiscoveryUri, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(
                div()
                    .h(px(34.))
                    .flex_none()
                    .px_3()
                    .rounded(colors.radius_lg)
                    .border_1()
                    .border_color(rgb(0x1687ff))
                    .bg(if colors.is_dark { rgb(0x16324f) } else { rgb(0xe6f2ff) })
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(0x1687ff))
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .child("导入 / 发现")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.import_redis_connection_string(cx);
                            cx.stop_propagation();
                        }),
                    ),
            ),
    )
}

/// 小节标题。
fn redis_section_label(text: &str, colors: UiColors) -> Div {
    div()
        .pt_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(text.to_string())
}

/// 新建连接弹框里的下拉行：用 gpui-component Select 渲染，绑定根实体上的 SelectState 光标。
fn connection_select_field(
    label: &'static str,
    select: &Entity<SelectState<SearchableVec<String>>>,
    search_placeholder: &'static str,
) -> Field {
    field()
        .label(label)
        .items_center()
        .child(Select::new(select).small().search_placeholder(search_placeholder))
}

/// 布尔开关行：把 Checkbox 的点击写回表单对应开关字段。
fn toggle_row_light(
    label: &'static str,
    field_id: ConnectionToggleField,
    checked: bool,
    _colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Field {
    let view = cx.entity();
    let id = connection_toggle_id(field_id);
    field().label(label).items_center().child(
        Checkbox::new(id)
            .checked(checked)
            .on_click(move |new_checked, _, cx| {
                let field_id = field_id;
                let new_checked = *new_checked;
                let _ = view.update(cx, |this, cx| {
                    this.set_connection_toggle_field(field_id, new_checked, cx);
                });
            }),
    )
}

/// 文件选择行：文件路径输入 + 「选择文件」按钮。
fn file_field_row_light(
    label: &'static str,
    field_id: ConnectionField,
    prompt: &'static str,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label(label).items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(field_id, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(file_picker_button_light(field_id, prompt, colors, cx).flex_none()),
    )
}

/// 「选择文件」按钮：调用对应字段的通用文件选择器。
fn file_picker_button_light(
    field_id: ConnectionField,
    prompt: &'static str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .size(px(38.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
        .child(
            div()
                .relative()
                .w(px(18.))
                .h(px(14.))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(2.))
                        .w(px(8.))
                        .h(px(4.))
                        .rounded(colors.radius * 0.5)
                        .bg(colors.muted),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w(px(18.))
                        .h(px(12.))
                        .rounded(colors.radius * 0.5)
                        .border_2()
                        .border_color(colors.muted)
                        .bg(colors.input_bg),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                let prompt = prompt;
                this.choose_connection_file(field_id, prompt, window, cx);
                cx.stop_propagation();
            }),
        )
}

/// 布尔开关的稳定元素 ID。
fn connection_toggle_id(field: ConnectionToggleField) -> &'static str {
    match field {
        ConnectionToggleField::TlsEnabled => "new-connection-toggle-tls",
        ConnectionToggleField::TlsVerify => "new-connection-toggle-tls-verify",
        ConnectionToggleField::SshEnabled => "new-connection-toggle-ssh",
        ConnectionToggleField::ClusterAllowReadonly => "new-connection-toggle-cluster-readonly",
        ConnectionToggleField::MysqlProxyEnabled => "new-connection-toggle-mysql-proxy",
        ConnectionToggleField::MysqlTcpKeepalive => "new-connection-toggle-mysql-tcp-keepalive",
    }
}

fn connection_form(
    kind: DatabaseKind,
    form_state: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let base = h_form()
        .label_width(px(112.))
        .child(field_row_light(
            "名称",
            ConnectionField::Name,
            inputs,
            false,
            colors,
            window,
            cx,
        ))
        .child(color_row_light(&form_state.color, colors, cx));

    let form = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => base
            .child(host_port_row_light("主机", inputs, colors, window, cx))
            .child(field_row_light(
                "用户名",
                ConnectionField::Username,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "密码",
                ConnectionField::Password,
                inputs,
                true,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "数据库",
                ConnectionField::Database,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "URL 参数",
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::MongoDb => base
            .child(connection_method_row_light(
                "连接方式",
                "表单",
                "URL",
                colors,
            ))
            .child(host_port_row_light("主机", inputs, colors, window, cx))
            .child(checkbox_row_light("SRV (MongoDB Atlas)", colors))
            .child(field_row_light(
                "用户名",
                ConnectionField::Username,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "密码",
                ConnectionField::Password,
                inputs,
                true,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "默认库",
                ConnectionField::MongoDefaultDb,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "认证库",
                ConnectionField::MongoAuthDb,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(dropdown_row_light("认证机制", "默认", colors)),
        DatabaseKind::Redis => base
            .child(host_port_row_light("主机", inputs, colors, window, cx))
            .child(field_row_light(
                "用户名",
                ConnectionField::Username,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "密码",
                ConnectionField::Password,
                inputs,
                true,
                colors,
                window,
                cx,
            ))
            // Redis 没有连接串，TLS / Sentinel 这些开关走同一个参数输入框
            .child(field_row_light(
                "参数",
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::Sqlite => base.child(sqlite_file_row_light(inputs, colors, window, cx)),
    };

    div().w_full().child(form)
}

