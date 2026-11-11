// Oracle 用户与权限面板共用的小构件。行/输入/下拉沿用 SQL Server 面板的版式，
// 保证同一套字号、间距与边框语义；只有 Oracle 特有的文案与语义在这里单独实现。

fn oracle_section_title(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(text.into())
}

/// 详情区容器：与其它面板一致的滚动与内边距。
fn oracle_panel_shell(body: Div, colors: UiColors) -> AnyElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .child(body)
        .into_any_element()
}

fn oracle_form_row(label: SharedString, colors: UiColors) -> Div {
    h_flex().h(px(38.)).gap_3().items_center().child(
        div()
            .w(px(120.))
            .flex_none()
            .text_size(ui_text_size(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(colors.text)
            .child(label),
    )
}

fn oracle_form_input(
    label: SharedString,
    input: Entity<InputState>,
    id: &'static str,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    oracle_form_row(label, colors)
        .debug_selector(move || format!("{id}-row"))
        .child(oracle_form_input_box(input, id, enabled, window, colors, cx))
}

/// 输入框本体（不含标签列），供需要追加后缀控件的行复用。
fn oracle_form_input_box(
    input: Entity<InputState>,
    id: &'static str,
    enabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w(px(360.))
        .h(px(34.))
        .rounded(colors.radius)
        .debug_selector(move || id.to_string())
        .border_1()
        .border_color(user_admin_input_border_color(focused, colors))
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| {
            style.border_color(user_admin_input_hover_border_color(focused, colors))
        })
        .child(
            Input::new(&input)
                .id(id)
                .small()
                .appearance(false)
                .focus_bordered(false)
                .disabled(!enabled)
                .w_full()
                .h_full()
                .px_2()
                .text_size(ui_text_size(13.))
                .text_color(colors.text),
        )
}

fn oracle_form_hint(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .ml(px(132.))
        .max_w(px(360.))
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(text.into())
}

/// 表单错误行：与 MySQL / PG 同一位置与红字语义。
fn oracle_form_error(text: impl Into<SharedString>) -> Div {
    div()
        .ml(px(132.))
        .max_w(px(360.))
        .text_size(ui_text_size(12.))
        .text_color(rgb(0xff3b45))
        .child(text.into())
}

/// 只读信息行：表空间、状态、锁定时间等目录事实。
fn oracle_readonly_row(label: SharedString, value: String, colors: UiColors) -> Div {
    oracle_form_row(label, colors).child(
        div()
            .w(px(360.))
            .text_color(colors.text)
            .text_size(ui_text_size(13.))
            .child(if value.is_empty() { "-".into() } else { value }),
    )
}

#[derive(Clone, Copy)]
enum TagTone {
    Neutral,
    Inherited,
    Option,
}

/// 授权行徽标：区分「直接授予」「继承」「WITH ADMIN/GRANT OPTION」。
fn oracle_tag(text: impl Into<SharedString>, tone: TagTone, colors: UiColors) -> Div {
    let fg = match tone {
        TagTone::Neutral => colors.muted,
        TagTone::Inherited => rgb(0xd97706),
        TagTone::Option => rgb(0x2563eb),
    };
    div()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(ui_text_size(11.))
        .text_color(fg)
        .child(text.into())
}

/// 一行授权：标题 + 说明 + 撤销入口（仅直接授予）。继承行不给撤销入口，
/// 这是 §8「继承来的权限不能显示成可直接撤销」在界面上的落点。
/// `revoke` 为撤销命令；`None` 表示这一行只读。
fn oracle_privilege_row(
    id: &'static str,
    index: usize,
    title: String,
    detail: SharedString,
    inherited: bool,
    with_option: bool,
    revoke: Option<AppCommand>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let row = h_flex()
        .id(SharedString::from(format!("{id}-{index}")))
        .min_h(px(44.))
        .px_3()
        .py_1()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .debug_selector(move || format!("{id}-{index}"))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_size(ui_text_size(13.)).text_color(colors.text).child(title))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(detail)
                        .when(inherited, |row| {
                            row.child(oracle_tag(
                                tr("app.oracle_security.inherited"),
                                TagTone::Inherited,
                                colors,
                            ))
                        })
                        .when(with_option, |row| {
                            row.child(oracle_tag(
                                tr("app.oracle_security.with_option"),
                                TagTone::Option,
                                colors,
                            ))
                        }),
                ),
        );
    match revoke {
        Some(command) => row
            .child(
                user_admin_button(
                    tr("app.oracle_security.revoke"),
                    AppIcon::Trash,
                    true,
                    false,
                    colors,
                )
                .debug_selector(move || format!("{id}-{index}-revoke"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(command.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
            )
            .into_any_element(),
        None => row
            .child(oracle_tag(
                if inherited {
                    tr("app.oracle_security.readonly_inherited")
                } else {
                    tr("app.oracle_security.readonly")
                },
                TagTone::Neutral,
                colors,
            ))
            .into_any_element(),
    }
}

/// 账号行副标题：自带账号 / 通用账号 / 普通账号。
fn oracle_user_subtitle(user: &fluxdb_core::OracleUser, current: &str) -> SharedString {
    if user.oracle_maintained {
        tr("app.oracle_security.oracle_maintained")
    } else if user.name.eq_ignore_ascii_case(current) {
        tr("app.oracle_security.current_user")
    } else if user.common {
        tr("app.oracle_security.common_user")
    } else {
        tr("app.oracle_security.local_user")
    }
}
