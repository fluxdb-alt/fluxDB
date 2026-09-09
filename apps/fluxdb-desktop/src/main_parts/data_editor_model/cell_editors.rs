fn data_table_cell_hover_action(cx: &mut Context<TableState<DataPageTableDelegate>>) -> Div {
    let container_bg = if cx.theme().is_dark() {
        rgb(0x252a31)
    } else {
        rgb(0xf3f5f8)
    };
    let border = if cx.theme().is_dark() {
        rgb(0x535b66)
    } else {
        rgb(0xc8ced8)
    };
    let text = if cx.theme().is_dark() {
        rgb(0xd1d6de)
    } else {
        rgb(0x5f6672)
    };

    div()
        .absolute()
        .right(px(-6.))
        .top_0()
        .bottom_0()
        .w(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .bg(container_bg)
        .child(
            div()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(border)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .line_height(px(11.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(text)
                .child("!"),
        )
}

fn data_cell_temporal_picker(
    kind: DataCellTemporalKind,
    edit_input: Entity<InputState>,
    part_input: Entity<InputState>,
    part_editing: Option<TemporalPartEditState>,
    cell: DataCellEditState,
    view: WeakEntity<NavicatMain>,
    nullable: bool,
    cx: &mut Context<TableState<DataPageTableDelegate>>,
) -> impl IntoElement {
    let is_dark = cx.theme().is_dark();
    let current = edit_input.read(cx).value().to_string();
    let target = TemporalEditTarget::DataCell(cell);
    let panel_width = match kind {
        DataCellTemporalKind::Date => 290.,
        DataCellTemporalKind::Time => 252.,
        DataCellTemporalKind::DateTime => 300.,
    };

    div()
        .absolute()
        .left(px(-7.))
        .bottom(px(-5.))
        .size(px(1.))
        .child(
            deferred(
                anchored().anchor(Anchor::TopLeft).child(
                    data_cell_temporal_editor_body(
                        kind,
                        edit_input,
                        part_input,
                        part_editing,
                        target,
                        view.clone(),
                        nullable,
                        is_dark,
                        current,
                    )
                    .occlude()
                    .w(px(panel_width))
                    .shadow_lg()
                    .on_mouse_down_out({
                        let view = view.clone();
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.commit_data_cell_edit(cx);
                            });
                        }
                    }),
                ),
            )
            .with_priority(3),
        )
}

fn data_cell_temporal_editor_body(
    kind: DataCellTemporalKind,
    edit_input: Entity<InputState>,
    part_input: Entity<InputState>,
    part_editing: Option<TemporalPartEditState>,
    target: TemporalEditTarget,
    view: WeakEntity<NavicatMain>,
    nullable: bool,
    is_dark: bool,
    current: String,
) -> Div {
    let bg = if is_dark {
        rgb(0x20242b)
    } else {
        rgb(0xffffff)
    };
    let border = if is_dark {
        rgb(0x3b4450)
    } else {
        rgb(0xd6dbe3)
    };
    let text = if is_dark {
        rgb(0xe6e9ef)
    } else {
        rgb(0x202124)
    };
    let muted = if is_dark {
        rgb(0x9aa4b2)
    } else {
        rgb(0x667085)
    };
    let input_bg = if is_dark {
        rgb(0x111214)
    } else {
        rgb(0xffffff)
    };

    div()
        .rounded(px(7.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(text)
        .p_1p5()
        .flex()
        .flex_col()
        .gap_1()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(28.))
                .border_1()
                .border_color(if is_dark {
                    rgb(0xc7cbd3)
                } else {
                    rgb(0x5f6877)
                })
                .bg(input_bg)
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .child(app_icon(AppIcon::CalendarClock, 15., muted))
                .child(
                    Input::new(&edit_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(11.))
                        .font_family("Menlo"),
                ),
        )
        .when(kind.has_date(), |this| {
            this.child(data_cell_date_spinner_row(
                kind,
                current.as_str(),
                part_input.clone(),
                part_editing,
                target,
                view.clone(),
                is_dark,
            ))
        })
        .when(kind.has_time(), |this| {
            this.child(data_cell_time_picker_row(
                kind,
                current.as_str(),
                part_input.clone(),
                part_editing,
                target,
                view.clone(),
                muted,
                is_dark,
            ))
        })
        .child(data_cell_temporal_actions(
            kind, edit_input, nullable, is_dark, muted,
        ))
}

fn cell_detail_temporal_editor(
    kind: DataCellTemporalKind,
    edit_input: Entity<InputState>,
    part_input: Entity<InputState>,
    part_editing: Option<TemporalPartEditState>,
    tab_id: TabId,
    view: WeakEntity<NavicatMain>,
    nullable: bool,
    is_dark: bool,
    current: String,
) -> Div {
    data_cell_temporal_editor_body(
        kind,
        edit_input,
        part_input,
        part_editing.filter(|editing| {
            matches!(editing.target, TemporalEditTarget::CellDetail(active) if active == tab_id)
        }),
        TemporalEditTarget::CellDetail(tab_id),
        view,
        nullable,
        is_dark,
        current,
    )
}

fn data_cell_date_spinner_row(
    kind: DataCellTemporalKind,
    current: &str,
    part_input: Entity<InputState>,
    part_editing: Option<TemporalPartEditState>,
    target: TemporalEditTarget,
    view: WeakEntity<NavicatMain>,
    is_dark: bool,
) -> Div {
    let (year, month, day) = temporal_date_parts(current);
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(data_cell_temporal_spin_box(
            TemporalPart::Year,
            format!("{year:04}"),
            px(96.),
            kind,
            part_input.clone(),
            part_editing,
            target,
            view.clone(),
            is_dark,
        ))
        .child(data_cell_temporal_spin_box(
            TemporalPart::Month,
            format!("{month:02}"),
            px(78.),
            kind,
            part_input.clone(),
            part_editing,
            target,
            view.clone(),
            is_dark,
        ))
        .child(data_cell_temporal_spin_box(
            TemporalPart::Day,
            format!("{day:02}"),
            px(78.),
            kind,
            part_input,
            part_editing,
            target,
            view,
            is_dark,
        ))
}

fn data_cell_time_picker_row(
    kind: DataCellTemporalKind,
    current: &str,
    part_input: Entity<InputState>,
    part_editing: Option<TemporalPartEditState>,
    target: TemporalEditTarget,
    view: WeakEntity<NavicatMain>,
    muted: gpui::Rgba,
    is_dark: bool,
) -> Div {
    let (hour, minute, second) = temporal_time_parts(current);
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(data_cell_temporal_spin_box(
            TemporalPart::Hour,
            format!("{hour:02}"),
            px(70.),
            kind,
            part_input.clone(),
            part_editing,
            target,
            view.clone(),
            is_dark,
        ))
        .child(div().text_size(px(13.)).text_color(muted).child(":"))
        .child(data_cell_temporal_spin_box(
            TemporalPart::Minute,
            format!("{minute:02}"),
            px(70.),
            kind,
            part_input.clone(),
            part_editing,
            target,
            view.clone(),
            is_dark,
        ))
        .child(div().text_size(px(13.)).text_color(muted).child(":"))
        .child(data_cell_temporal_spin_box(
            TemporalPart::Second,
            format!("{second:02}"),
            px(70.),
            kind,
            part_input,
            part_editing,
            target,
            view,
            is_dark,
        ))
}

fn data_cell_temporal_spin_box(
    part: TemporalPart,
    value: String,
    width: Pixels,
    kind: DataCellTemporalKind,
    part_input: Entity<InputState>,
    part_editing: Option<TemporalPartEditState>,
    target: TemporalEditTarget,
    view: WeakEntity<NavicatMain>,
    is_dark: bool,
) -> Div {
    let border = if is_dark {
        rgb(0x343c48)
    } else {
        rgb(0xd6dbe3)
    };
    let text = if is_dark {
        rgb(0xe6e9ef)
    } else {
        rgb(0x202124)
    };
    let button_bg = if is_dark {
        rgb(0x1b1f25)
    } else {
        rgb(0xf3f5f8)
    };
    let active = part_editing.is_some_and(|editing| {
        editing.kind == kind && editing.part == part && editing.target == target
    });
    let view_for_value = view.clone();
    div()
        .w(width)
        .h(px(32.))
        .rounded(px(7.))
        .border_1()
        .border_color(border)
        .overflow_hidden()
        .flex()
        .items_center()
        .bg(if is_dark {
            rgb(0x181c22)
        } else {
            rgb(0xffffff)
        })
        .child(
            div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .font_family("Menlo")
                .text_color(text)
                .cursor_text()
                .when(active, |this| {
                    this.child(
                        Input::new(&part_input)
                            .appearance(false)
                            .focus_bordered(false)
                            .w_full()
                            .h_full()
                            .text_size(px(14.))
                            .font_family("Menlo"),
                    )
                })
                .when(!active, |this| {
                    this.child(value.clone()).on_mouse_down(
                        MouseButton::Left,
                        move |_, window, cx| {
                            let _ = view_for_value.update(cx, |this, cx| {
                                this.begin_temporal_part_edit(
                                    TemporalPartEditState { target, part, kind },
                                    value.clone(),
                                    window,
                                    cx,
                                );
                            });
                            cx.stop_propagation();
                        },
                    )
                }),
        )
        .child(
            div()
                .w(px(24.))
                .h_full()
                .border_l_1()
                .border_color(border)
                .flex()
                .flex_col()
                .child(data_cell_temporal_step_button(
                    AppIcon::ChevronUp,
                    part,
                    1,
                    kind,
                    target,
                    view.clone(),
                    button_bg,
                    text,
                ))
                .child(data_cell_temporal_step_button(
                    AppIcon::ChevronDown,
                    part,
                    -1,
                    kind,
                    target,
                    view,
                    button_bg,
                    text,
                )),
        )
}

fn data_cell_temporal_step_button(
    icon: AppIcon,
    part: TemporalPart,
    delta: i32,
    kind: DataCellTemporalKind,
    target: TemporalEditTarget,
    view: WeakEntity<NavicatMain>,
    bg: gpui::Rgba,
    color: gpui::Rgba,
) -> Div {
    div()
        .flex_1()
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .bg(bg)
        .hover(|style| style.bg(rgb(0x2f66d0)).text_color(rgb(0xffffff)))
        .child(app_icon(icon, 12., color))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            let _ = view.update(cx, |this, cx| {
                this.temporal_part_editing = None;
                let edit_input = match target {
                    TemporalEditTarget::DataCell(_) => this.data_cell_edit_input.clone(),
                    TemporalEditTarget::CellDetail(_) => this.cell_detail_input.clone(),
                };
                let current = edit_input.read(cx).value().to_string();
                let next = temporal_value_after_step(current.as_str(), kind, part, delta);
                edit_input.update(cx, |input, cx| {
                    input.set_value(next, window, cx);
                    input.focus(window, cx);
                });
            });
            cx.stop_propagation();
        })
}

fn data_cell_temporal_actions(
    kind: DataCellTemporalKind,
    edit_input: Entity<InputState>,
    nullable: bool,
    is_dark: bool,
    muted: gpui::Rgba,
) -> Div {
    let text = if is_dark {
        rgb(0xe6e9ef)
    } else {
        rgb(0x202124)
    };
    div()
        .h(px(24.))
        .flex()
        .items_center()
        .justify_between()
        .child(data_cell_temporal_action_button(
            AppIcon::CircleSlash,
            "NULL",
            nullable,
            edit_input.clone(),
            Some("NULL".to_string()),
            text,
            muted,
        ))
        .child(data_cell_temporal_action_button(
            AppIcon::CalendarClock,
            "Now",
            true,
            edit_input,
            Some(temporal_now_value(kind)),
            text,
            muted,
        ))
}

fn data_cell_temporal_action_button(
    icon: AppIcon,
    label: &'static str,
    enabled: bool,
    edit_input: Entity<InputState>,
    value: Option<String>,
    text: gpui::Rgba,
    muted: gpui::Rgba,
) -> Div {
    div()
        .h(px(22.))
        .px_1()
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap_1()
        .opacity(if enabled { 1.0 } else { 0.42 })
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if enabled { text } else { muted })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(rgb(0x2f66d0)).text_color(rgb(0xffffff)))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if let Some(value) = value.clone() {
                        edit_input.update(cx, |input, cx| {
                            input.set_value(value, window, cx);
                            input.focus(window, cx);
                        });
                    }
                    cx.stop_propagation();
                })
        })
        .child(app_icon(icon, 14., if enabled { text } else { muted }))
        .child(label)
}

fn temporal_time_parts(value: &str) -> (u32, u32, u32) {
    let time = temporal_time_part(value).unwrap_or_else(|| "00:00:00".to_string());
    let mut parts = time.split(':').filter_map(|part| {
        part.chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .ok()
    });
    (
        parts.next().unwrap_or(0).min(23),
        parts.next().unwrap_or(0).min(59),
        parts.next().unwrap_or(0).min(59),
    )
}

fn temporal_date_parts(value: &str) -> (i32, u32, u32) {
    let date = parse_temporal_date_part(value).unwrap_or_else(|| Local::now().date_naive());
    (date.year(), date.month(), date.day())
}

fn temporal_now_value(kind: DataCellTemporalKind) -> String {
    let now = Local::now().naive_local();
    match kind {
        DataCellTemporalKind::Date => now.date().format("%Y-%m-%d").to_string(),
        DataCellTemporalKind::Time => now.time().format("%H:%M:%S").to_string(),
        DataCellTemporalKind::DateTime => now.format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

fn temporal_value_after_step(
    current: &str,
    kind: DataCellTemporalKind,
    part: TemporalPart,
    delta: i32,
) -> String {
    match part {
        TemporalPart::Year | TemporalPart::Month | TemporalPart::Day => {
            let base =
                parse_temporal_date_part(current).unwrap_or_else(|| Local::now().date_naive());
            let next = match part {
                TemporalPart::Year => {
                    let year = base.year() + delta;
                    let day = base.day().min(temporal_days_in_month(year, base.month()));
                    NaiveDate::from_ymd_opt(year, base.month(), day).unwrap_or(base)
                }
                TemporalPart::Month => temporal_shift_month(base, delta),
                TemporalPart::Day => base + ChronoDuration::days(delta as i64),
                _ => base,
            };
            replace_temporal_date_part(current, Some(kind), next)
        }
        TemporalPart::Hour | TemporalPart::Minute | TemporalPart::Second => {
            let (hour, minute, second) = temporal_time_parts(current);
            let stepped = |value: u32, max: u32| -> u32 {
                (value as i32 + delta).rem_euclid(max as i32 + 1) as u32
            };
            let (hour, minute, second) = match part {
                TemporalPart::Hour => (stepped(hour, 23), minute, second),
                TemporalPart::Minute => (hour, stepped(minute, 59), second),
                TemporalPart::Second => (hour, minute, stepped(second, 59)),
                _ => (hour, minute, second),
            };
            let time = format!("{hour:02}:{minute:02}:{second:02}");
            replace_temporal_time_part(current, Some(kind), time.as_str())
        }
    }
}

fn temporal_value_after_part_input(
    current: &str,
    editing: TemporalPartEditState,
    input: &str,
) -> Option<String> {
    let value = input.trim().parse::<i32>().ok()?;
    match editing.part {
        TemporalPart::Year | TemporalPart::Month | TemporalPart::Day => {
            let base =
                parse_temporal_date_part(current).unwrap_or_else(|| Local::now().date_naive());
            let year = if editing.part == TemporalPart::Year {
                value
            } else {
                base.year()
            };
            let month = if editing.part == TemporalPart::Month {
                value.clamp(1, 12) as u32
            } else {
                base.month()
            };
            let day = if editing.part == TemporalPart::Day {
                value.max(1) as u32
            } else {
                base.day()
            }
            .min(temporal_days_in_month(year, month));
            NaiveDate::from_ymd_opt(year, month, day)
                .map(|date| replace_temporal_date_part(current, Some(editing.kind), date))
        }
        TemporalPart::Hour | TemporalPart::Minute | TemporalPart::Second => {
            let (hour, minute, second) = temporal_time_parts(current);
            let (hour, minute, second) = match editing.part {
                TemporalPart::Hour => (value.clamp(0, 23) as u32, minute, second),
                TemporalPart::Minute => (hour, value.clamp(0, 59) as u32, second),
                TemporalPart::Second => (hour, minute, value.clamp(0, 59) as u32),
                _ => (hour, minute, second),
            };
            Some(replace_temporal_time_part(
                current,
                Some(editing.kind),
                format!("{hour:02}:{minute:02}:{second:02}").as_str(),
            ))
        }
    }
}

fn data_cell_choice_picker(
    kind: DataCellEditorKind,
    meta: DataTableColumnMeta,
    edit_input: Entity<InputState>,
    view: WeakEntity<NavicatMain>,
    cx: &mut Context<TableState<DataPageTableDelegate>>,
) -> impl IntoElement {
    let colors = ui_colors_from_theme(
        if cx.theme().is_dark() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        cx,
    );
    let mut options = data_cell_choice_options(kind, &meta);
    if meta.nullable {
        options.insert(
            0,
            ColumnChoice {
                value: "NULL".to_string(),
                label: String::new(),
            },
        );
    }
    let current = edit_input.read(cx).value().to_string();
    let is_set = kind == DataCellEditorKind::Set;
    let menu_width = data_cell_choice_menu_width(&options);
    let selected = current
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

    div()
        .absolute()
        .left(px(-7.))
        .bottom(px(-5.))
        .size(px(1.))
        .child(
            deferred(
                anchored().anchor(Anchor::TopLeft).child(
                    div()
                        .occlude()
                        .w(px(menu_width))
                        .max_h(px(260.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .shadow_lg()
                        .bg(menu_surface_bg(colors))
                        .text_color(colors.text)
                        .p_1()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .on_mouse_down_out({
                            let view = view.clone();
                            move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.commit_data_cell_edit(cx);
                                });
                            }
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                        .children(options.into_iter().map(|option| {
                            let checked = if is_set && option.value != "NULL" {
                                selected.contains(option.value.as_str())
                            } else {
                                current.trim() == option.value
                            };
                            data_cell_choice_item(
                                option,
                                checked,
                                is_set,
                                edit_input.clone(),
                                colors,
                            )
                        })),
                ),
            )
            .with_priority(4),
        )
}

fn data_cell_choice_item(
    option: ColumnChoice,
    checked: bool,
    is_set: bool,
    edit_input: Entity<InputState>,
    colors: UiColors,
) -> Div {
    let option_for_click = option.value.clone();
    let is_null = option.value == "NULL";
    let check_accent = if colors.is_dark {
        rgb(0x4f8cff)
    } else {
        rgb(0x2f6fed)
    };
    div()
        .h(px(24.))
        .rounded(colors.radius * 0.5)
        .px_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .size(px(14.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(is_set && !is_null, |this| {
                    this.rounded(colors.radius * 0.5)
                        .border_1()
                        .border_color(if checked { check_accent } else { colors.border })
                        .bg(if checked {
                            check_accent
                        } else {
                            colors.input_bg
                        })
                        .when(checked, |this| {
                            this.child(app_icon(AppIcon::Check, 11., rgb(0xffffff)))
                        })
                })
                .when(!is_set || is_null, |this| {
                    this.when(checked, |this| {
                        this.child(app_icon(AppIcon::Check, 13., colors.muted))
                    })
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .child(option.value),
        )
        .when(!option.label.trim().is_empty(), |this| {
            this.child(
                div()
                    .flex_none()
                    .max_w(px(96.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(colors.muted)
                    .child(option.label),
            )
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            let next = if is_set && option_for_click != "NULL" {
                let mut values = edit_input
                    .read(cx)
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && *value != "NULL")
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>();
                if !values.remove(option_for_click.as_str()) {
                    values.insert(option_for_click.clone());
                }
                values.into_iter().collect::<Vec<_>>().join(",")
            } else {
                option_for_click.clone()
            };
            edit_input.update(cx, |input, cx| {
                input.set_value(next, window, cx);
                input.focus(window, cx);
            });
            cx.stop_propagation();
        })
}

fn data_cell_choice_menu_width(options: &[ColumnChoice]) -> f32 {
    let longest = options
        .iter()
        .map(|option| option.value.chars().count() + option.label.chars().count())
        .max()
        .unwrap_or(4) as f32;
    (longest * 8. + 76.).clamp(128., 260.)
}
