/// 内部工具默认策略选择器：只读 / 写审批 / 帮我审批 / 自动执行。
///
/// 这是内部 Agent 工具的审批方式，不作用于对外 MCP 数据源（数据源用写入策略）。
fn execution_policy_label(policy: fluxdb_core::ExecutionPolicy) -> SharedString {
    match policy {
        fluxdb_core::ExecutionPolicy::ReadOnly => tr("app.ai.policy.mode.read_only"),
        fluxdb_core::ExecutionPolicy::WriteApproval => tr("app.ai.policy.mode.write_approval"),
        fluxdb_core::ExecutionPolicy::AiApproval => tr("app.ai.policy.mode.ai_approval"),
        fluxdb_core::ExecutionPolicy::Automatic => tr("app.ai.policy.mode.automatic"),
    }
}

fn ai_tools_policy_button(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let selected = state.ai_execution_policy.tools_default;
    let owner = cx.entity().downgrade();
    Button::new("execution-policy-tools")
        .debug_selector(|| "execution-policy-tools".into())
        .small()
        .cursor_pointer()
        .min_w(px(150.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(execution_policy_label(selected))
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            // check_side 必须显式设为右侧：保持默认左侧时，组件库会为每个未选中项渲染一个
            // 空图标占位（svg 空路径），每帧都用空路径去查资源目录并报读取失败；勾选标记
            // 改由组件画在行尾。
            let mut menu = menu
                .check_side(gpui_component::Side::Right)
                .min_w(px(190.));
            for mode in fluxdb_core::ExecutionPolicy::ALL {
                let owner = owner.clone();
                let option_id = format!("execution-policy-option:tools:{mode:?}");
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(SharedString::from(option_id.clone()))
                            .debug_selector({
                                let id = option_id.clone();
                                move || id.clone()
                            })
                            .child(execution_policy_label(mode))
                    })
                    .checked(selected == mode)
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.change_tools_policy(mode, window, cx)
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

fn ai_policy_row(title: &str, note: &str, control: gpui::AnyElement, colors: UiColors) -> Div {
    div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .py_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.text)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(note.to_string()),
                ),
        )
        .child(control)
}

fn ai_execution_policy_panel(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    let mut panel = settings_panel_group(tr("app.ai.policy.title"), colors).child(ai_policy_row(
        &tr("app.ai.policy.tools_default"),
        &tr("app.ai.policy.tools_default_detail"),
        ai_tools_policy_button(state, colors, cx),
        colors,
    ));
    // 阈值与权重只参与“帮我审批”的写入风险评分；其他策略下不参与判定，不展示以免误导。
    if state.ai_execution_policy.tools_default == fluxdb_core::ExecutionPolicy::AiApproval {
        panel = panel
            .child(ai_policy_row(
                &tr("app.ai.policy.risk_threshold"),
                &tr("app.ai.policy.risk_threshold_detail"),
                ai_write_risk_threshold_control(state, colors, cx),
                colors,
            ))
            .child(ai_policy_row(
                &tr("app.ai.policy.review_weight"),
                &tr("app.ai.policy.review_weight_detail"),
                ai_write_review_weight_control(state, colors, cx),
                colors,
            ));
    }
    panel
}

/// 阈值档位：按钮 + 下拉菜单，与页面其它策略控件保持同一种交互，避免引入新的数字控件。
///
/// 档位是命名过的取值而不是自由输入：阈值决定放行范围，改动必须让人看清跨度。
const WRITE_RISK_THRESHOLD_STEPS: [f64; 5] = [0.20, 0.35, 0.50, 0.65, 0.80];

/// 评审权重档位；上限固定为 [`fluxdb_core::MAX_WRITE_REVIEW_WEIGHT`]。
const WRITE_REVIEW_WEIGHT_STEPS: [f64; 3] = [0.0, 0.25, 0.5];

fn ai_write_risk_threshold_control(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let selected = state.ai_execution_policy.write_risk_threshold;
    let owner = cx.entity().downgrade();
    Button::new("execution-policy-threshold")
        .debug_selector(|| "execution-policy-threshold".into())
        .small()
        .cursor_pointer()
        .min_w(px(150.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(format!("{selected:.2}"))
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .check_side(gpui_component::Side::Right)
                .min_w(px(190.));
            for step in WRITE_RISK_THRESHOLD_STEPS {
                let owner = owner.clone();
                let option_id = format!("execution-policy-threshold-option:{step}");
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(SharedString::from(option_id.clone()))
                            .debug_selector({
                                let id = option_id.clone();
                                move || id.clone()
                            })
                            .child(format!("{step:.2}"))
                    })
                    .checked((selected - step).abs() < 1e-9)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            let mut settings =
                                this.controller.state().ai_execution_policy.clone();
                            settings.write_risk_threshold = step;
                            this.save_execution_policy(settings, cx);
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

fn ai_write_review_weight_control(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let selected = state.ai_execution_policy.write_review_weight;
    let owner = cx.entity().downgrade();
    Button::new("execution-policy-weight")
        .debug_selector(|| "execution-policy-weight".into())
        .small()
        .cursor_pointer()
        .min_w(px(150.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(format!("{selected:.2}"))
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu
                .check_side(gpui_component::Side::Right)
                .min_w(px(190.));
            for step in WRITE_REVIEW_WEIGHT_STEPS {
                let owner = owner.clone();
                let option_id = format!("execution-policy-weight-option:{step}");
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(SharedString::from(option_id.clone()))
                            .debug_selector({
                                let id = option_id.clone();
                                move || id.clone()
                            })
                            .child(format!("{step:.2}"))
                    })
                    .checked((selected - step).abs() < 1e-9)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            let mut settings =
                                this.controller.state().ai_execution_policy.clone();
                            settings.write_review_weight = step;
                            this.save_execution_policy(settings, cx);
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

impl NavicatMain {
    fn save_execution_policy(
        &mut self,
        settings: fluxdb_core::ExecutionPolicySettings,
        cx: &mut Context<Self>,
    ) {
        let expected_revision = self.controller.state().ai_runtime_revision;
        let event = self.controller.dispatch(AppCommand::SaveExecutionPolicy {
            settings,
            expected_revision,
        });
        match event {
            AppEvent::AiSettingsError(message) => {
                self.show_message(&message, AppMessageKind::Error, cx)
            }
            _ => self.show_message(
                tr("app.ai.policy.saved"),
                AppMessageKind::Success,
                cx,
            ),
        }
        cx.notify();
    }

    fn change_tools_policy(
        &mut self,
        value: fluxdb_core::ExecutionPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut settings = self.controller.state().ai_execution_policy.clone();
        settings.tools_default = value;
        if value == fluxdb_core::ExecutionPolicy::Automatic {
            let owner = cx.entity().downgrade();
            window.open_dialog(cx, move |dialog, _, _| {
                let owner = owner.clone();
                let settings = settings.clone();
                dialog
                    .title(tr("app.ai.policy.auto_title"))
                    .close_button(true)
                    .overlay_closable(true)
                    .keyboard(true)
                    .child(tr("app.ai.policy.auto_detail"))
                    .footer(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("execution-policy-cancel")
                                    .cursor_pointer()
                                    .child(tr("app.common.cancel"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("execution-policy-confirm")
                                    .danger()
                                    .cursor_pointer()
                                    .child(tr("app.ai.policy.auto_confirm"))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(gpui_component::dialog::Confirm {
                                                secondary: false,
                                            }),
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .on_ok(move |_, _, cx| {
                        let _ =
                            owner.update(cx, |this, cx| this.save_execution_policy(settings.clone(), cx));
                        true
                    })
            });
        } else {
            self.save_execution_policy(settings, cx);
        }
    }
}
