// 终端画布渲染：把 fluxdb-core 的 TermGrid 画到 GPUI。
//
// 复用 sql_editor 的自定义 Element 模式（request_layout / prepaint / paint）：
//   - prepaint 阶段按实测 cell 尺寸计算 cols/rows，并据此 reflow grid + resize PTY。
//   - paint 阶段按逐列 fg/bg/bold 上色绘制每一行，并在底部绘制状态栏与危险命令确认条。
//   - 通过 `window.handle_input` 挂上输入 handler，使可打印字符/IME 进入 EntityInputHandler。

use fluxdb_core::terminal::{TermRowView, byte_cursor_to_cell};
use gpui::TextAlign;

/// 终端用等宽字体与字号（与需要宽字符显示的 grid 对齐）。
pub(crate) const TERMINAL_FONT: &str = crate::PLATFORM_MONOSPACE_FONT;
pub(crate) const TERMINAL_FONT_SIZE: f32 = 12.0;

/// 终端前景/背景色（随主题深浅取两套近似配色）。
#[derive(Clone, Copy)]
struct TermColors {
    bg: gpui::Rgba,
    default_fg: gpui::Rgba,
    completion_hint_fg: gpui::Rgba,
    status_bg: gpui::Rgba,
    status_fg: gpui::Rgba,
    danger_bg: gpui::Rgba,
    danger_fg: gpui::Rgba,
    /// 选区高亮：带 alpha、铺在文本**下面**。不透明色画在字形之上会把字盖掉，
    /// 选中区域就变成一条没有文字的实心色块。
    selection_bg: gpui::Rgba,
}

impl TermColors {
    fn for_theme(is_dark: bool) -> Self {
        // gpui 0.2.2 的 `rgb` 只接受单个打包的 0xRRGGBB 值。
        if is_dark {
            TermColors {
                bg: rgb(0x101216),
                default_fg: rgb(0xd8dce0),
                completion_hint_fg: rgb(0x73777d),
                status_bg: rgb(0x1f2329),
                status_fg: rgb(0x9ca3aa),
                danger_bg: rgb(0x4a1d1d),
                danger_fg: rgb(0xffb4b4),
                // 带 alpha 的挂蓝色：压住文字仍能读，和 Zed/Navop 的选区观感一致。
                selection_bg: rgba(0x2f5d9e66),
            }
        } else {
            TermColors {
                bg: rgb(0xffffff),
                default_fg: rgb(0x1f2428),
                completion_hint_fg: rgb(0x9aa0a6),
                status_bg: rgb(0xeff1f4),
                status_fg: rgb(0x5c6269),
                danger_bg: rgb(0xfbe4e4),
                danger_fg: rgb(0x9b1c1c),
                selection_bg: rgba(0x2f6fbf4d),
            }
        }
    }
}

// 终端视图：外层 focusable div 承载 key context + 动作转发，内层 TerminalCanvas 负责绘制。
impl Render for TerminalComponent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self.focus_handle.clone();
        let component = cx.entity();
        div()
            .id(("terminal", component.entity_id()))
            // 仅调试构建记录：端到端用例靠它定位终端表面并点击取得焦点。
            .debug_selector(|| "terminal-surface".to_string())
            .flex_1()
            .size_full()
            .relative()
            .overflow_hidden()
            .key_context(TERMINAL_CONTEXT)
            .track_focus(&focus_handle)
            .tab_index(0)
            .on_mouse_down(MouseButton::Left, {
                let focus_handle = focus_handle.clone();
                move |_, window, _cx| {
                    focus_handle.focus(window, _cx);
                }
            })
            .on_action(window.listener_for(&component, TerminalComponent::enter))
            .on_action(window.listener_for(&component, TerminalComponent::escape))
            .on_action(window.listener_for(&component, TerminalComponent::ctrl_c))
            .on_action(window.listener_for(&component, TerminalComponent::ctrl_d))
            .on_action(window.listener_for(&component, TerminalComponent::backspace))
            .on_action(window.listener_for(&component, TerminalComponent::delete))
            .on_action(window.listener_for(&component, TerminalComponent::move_left))
            .on_action(window.listener_for(&component, TerminalComponent::move_right))
            .on_action(window.listener_for(&component, TerminalComponent::move_up))
            .on_action(window.listener_for(&component, TerminalComponent::move_down))
            .on_action(window.listener_for(&component, TerminalComponent::move_home))
            .on_action(window.listener_for(&component, TerminalComponent::move_end))
            .on_action(window.listener_for(&component, TerminalComponent::tab))
            .on_action(window.listener_for(&component, TerminalComponent::shift_tab))
            .on_action(window.listener_for(&component, TerminalComponent::copy))
            .on_action(window.listener_for(&component, TerminalComponent::paste))
            .on_action(window.listener_for(&component, TerminalComponent::clear))
            // 鼠标：左键按下开始选区（并聚焦），拖动更新，抬起/移出结束；滚轮滚动回看。
            // 连击次数决定选区类型：2 次选词、3 次选行（与 Zed / 主流终端一致）。
            .on_mouse_down(MouseButton::Left, {
                let component = component.clone();
                move |event, _window, cx| {
                    let click_count = event.click_count;
                    component.update(cx, |comp, cx| {
                        comp.begin_mouse_selection(event.position, click_count);
                        cx.notify();
                    });
                }
            })
            // 右键：先保证有选区可复制（无选区就选指针下的词），菜单由下方的
            // `.context_menu(...)` 打开——它自己处理窗口级右键与外部点击关闭。
            .on_mouse_down(MouseButton::Right, {
                let component = component.clone();
                move |event, _window, cx| {
                    component.update(cx, |comp, cx| {
                        comp.begin_context_menu_selection(event.position);
                        cx.notify();
                    });
                }
            })
            // 拖动/抬起不挂在这个 div 上：div 的 move 监听只在指针悬停时回调，
            // 一拖出终端边界选区就冻结。改在画布 paint 里挂窗口级监听（见 TerminalCanvas::paint）。
            .on_scroll_wheel({
                let component = component.clone();
                move |event, _window, cx| {
                    let delta = event.delta.pixel_delta(px(1.0));
                    component.update(cx, |comp, cx| {
                        comp.scroll_by_delta(f32::from(delta.y));
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            })
            // 画布同样绝对定位并铺满：它若留在流内，绝对定位的补全浮层就以它的底边为
            // 静态基准，`top(N)` 会落到容器底部之外（面板比窗口高时直接被裁掉）。
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(TerminalCanvas {
                        component: component.clone(),
                    }),
            )
            .when(self.completion_active(), |this| {
                // 补全浮层：贴着输入光标画，候选行用 gpui-component 的 `ListItem`
                // （悬停 / 选中 / 键盘高亮都走同一套主题色，与编辑器补全浮层一致）。
                let items = self.completion_items.clone();
                let selected = self.completion_index;
                let anchor = self.completion_popup_anchor();
                let scroll = self.completion_scroll.clone();
                let theme = ComponentTheme::global(cx);
                let hover_entity = component.clone();
                let click_entity = component.clone();
                // 走 deferred + anchored（与 filter_ui/popover.rs 同一套）：浮层按窗口坐标绘制，
                // 不受终端面板 `overflow_hidden` 裁切。写死成面板子元素的话，超出行高的部分
                // 会被切掉，用户看到的就是「只露出一条边」。
                this.child(
                    gpui::deferred(
                        gpui::anchored()
                            .anchor(gpui::Anchor::TopLeft)
                            .position(anchor)
                            // 光标在行尾时浮层会顶到右边缘，交给 anchored 夹回窗口内。
                            .snap_to_window_with_margin(px(8.))
                            .child(completion_popup(
                                &items,
                                selected,
                                &scroll,
                                theme.radius_lg,
                                &theme.colors,
                                move |index, cx| {
                                    hover_entity.update(cx, |comp, cx| {
                                        comp.select_completion(index, cx);
                                    });
                                },
                                move |index, cx| {
                                    click_entity.update(cx, |comp, cx| {
                                        comp.accept_completion(index, cx);
                                    });
                                },
                            )),
                    )
                    .with_priority(2),
                )
            })
            // 右键菜单：gpui-component 的 `context_menu` 自己挂窗口级右键监听、按窗口坐标
            // 定位、点外部/Esc 关闭、关闭后把焦点还回终端，不用自建弹层与遮罩。
            // 菜单内容按“当前有没有选区”变化：无选区时“复制”置灰。
            .context_menu({
                let component = component.clone();
                move |menu, _window, cx| {
                    terminal_context_menu(menu, &component, cx)
                }
            })
    }
}

/// 终端右键菜单：复制 / 粘贴 / 清屏。
///
/// 点击处理器持有 Weak 引用：菜单是延迟构建的实体，强引用会让终端与菜单互相持有。
/// 无选区时“复制”置灰——空选区点了也没有内容可复制（与 Zed 的 `has_selection` 分支同效）。
fn terminal_context_menu(
    menu: gpui_component::menu::PopupMenu,
    component: &Entity<crate::TerminalComponent>,
    cx: &mut App,
) -> gpui_component::menu::PopupMenu {
    let has_selection = component.read(cx).grid.has_selection();
    let weak = component.downgrade();
    menu
        .min_w(px(180.))
        .item(
            PopupMenuItem::new(tr("app.context.copy"))
                .icon(IconName::Copy)
                .disabled(!has_selection)
                .on_click({
                    let weak = weak.clone();
                    move |_, _, cx| {
                        // 复制与 `cmd-c` 共用 `copy_selection`，不另写一套逻辑。
                        if let Some(component) = weak.upgrade() {
                            component.update(cx, |comp, cx| comp.copy_selection(cx));
                        }
                    }
                }),
        )
        .item(
            PopupMenuItem::new(tr("app.context.paste"))
                .icon(IconName::Replace)
                .on_click({
                    let weak = weak.clone();
                    move |_, _, cx| {
                        if let Some(component) = weak.upgrade() {
                            component.update(cx, |comp, cx| comp.paste_from_clipboard(cx));
                        }
                    }
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new(tr("app.terminal.clear_screen"))
                .icon(IconName::Delete)
                .on_click(move |_, _, cx| {
                    if let Some(component) = weak.upgrade() {
                        component.update(cx, |comp, cx| comp.clear_screen(cx));
                    }
                }),
        )
}

/// 渲染补全候选浮层的面板本体（位置由 `completion_popup_anchor` + `anchored` 决定）。
///
/// 候选行用 gpui-component 的 `ListItem`：悬停底色、选中底色、内边距、字号回落都由组件
/// 按主题给，和编辑器补全浮层是同一套观感；这里只锁行高与字号（终端是 12px 等宽）。
/// 底色不用 `popover`——浅色主题下它就是纯白，与终端面板同色，浮层只剩一条边框
/// （截图上就是「显示出了一点」），所以用 `muted`。
fn completion_popup(
    items: &[TerminalCompletionItem],
    selected: Option<usize>,
    scroll: &ScrollHandle,
    radius: gpui::Pixels,
    colors: &gpui_component::ThemeColor,
    on_hover: impl Fn(usize, &mut App) + 'static,
    on_click: impl Fn(usize, &mut App) + 'static,
) -> impl IntoElement {
    // 显式给高度：只写 `max_h` 时绝对定位元素仍会被拉伸到上限，一条候选也撑成一整块黑板。
    let height = crate::TerminalComponent::completion_popup_height(items.len());
    let row_h = crate::TerminalComponent::COMPLETION_ROW_HEIGHT;
    let row_hover = std::rc::Rc::new(on_hover);
    let row_click = std::rc::Rc::new(on_click);
    let rows: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let label = item.label.clone();
            let detail = item.detail.clone();
            let hover = row_hover.clone();
            let click = row_click.clone();
            gpui_component::list::ListItem::new(("terminal-completion", i))
                // 仅调试构建记录：端到端用例据此断言候选行没被外壳裁掉半截。
                .debug_selector(move || format!("terminal-completion-row:{i}"))
                // 鼠标划过就改高亮：键盘与鼠标共用 `completion_index`，不会出现两套选中态。
                .on_mouse_enter(move |_, _, cx| hover(i, cx))
                // 按下即应用（与编辑器补全一致：点一下就写回输入行，不用再按回车）。
                .on_mouse_down(MouseButton::Left, move |_, _, cx| click(i, cx))
                .h(px(row_h))
                .px_2()
                .rounded(px(3.0))
                .cursor_pointer()
                // `ListItem` 自己写死了 `text_base()`（16px），不覆盖的话行框只有 30px、
                // 字被上下各切一刀——截图上就是「use 只剩上半截」。
                .text_size(px(TERMINAL_FONT_SIZE))
                .selected(Some(i) == selected)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                // 仅调试构建记录：端到端用例据此断言文字没被行框裁掉。
                                .debug_selector(move || format!("terminal-completion-label:{i}"))
                                .text_color(colors.popover_foreground)
                                .child(label),
                        )
                        .when(detail.is_some(), |this| {
                            this.child(
                                div()
                                    .text_color(colors.muted_foreground)
                                    .text_sm()
                                    .child(detail.unwrap_or_default()),
                            )
                        }),
                )
        })
        .collect();

    div()
        .id("terminal-completion-popup")
        // 仅调试构建记录：端到端用例据此断言浮层真的画出来了。
        .debug_selector(|| "terminal-completion-popup".to_string())
        // 位置由外层 `anchored()` 给；这里不再自己定位，否则会与锚点叠加。
        // 高度按候选数固定；宽度按内容撑开，只给上下限防止缩成一条或铺满屏幕。
        .h(px(height))
        .min_w(px(180.0))
        .max_w(px(520.0))
        // 背景不能用 `popover`：浅色主题里它就是纯白，与终端面板同色，浮层只剩一条边框
        // （像素比对了卡片内 (255,255,255) 与面板完全一致）。
        // `muted` 浅色是 #f5f5f5、深色是 neutral-800，两个主题下都比面板深一档。
        // 字号也必须显式给：不写就继承根字号（16px 而非终端的 12px），字被行框裁掉半截。
        .text_size(px(TERMINAL_FONT_SIZE))
        .line_height(px(TERMINAL_FONT_SIZE + 4.0))
        .bg(colors.muted)
        .text_color(colors.popover_foreground)
        // 阴影与编辑器浮层同一个规格（`0 8 18 rgba(0,0,0,.22)`）：`shadow_md` 太淡，
        // 浮层压在纯白终端面上时几乎看不出边界。
        .shadow(vec![gpui_component::box_shadow(
            px(0.),
            px(8.),
            px(18.),
            px(0.),
            gpui::hsla(0., 0., 0., 0.22),
        )])
        .rounded(radius)
        .border_1()
        .border_color(colors.border)
        .p(px(4.))
        // 浮层自己吞掉鼠标与滚轮：否则点候选会同时开始终端选区、滚浮层会翻走终端历史。
        // 放在最深一层（List 项）之后仍会先跑，所以点击应用不会被打断。
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        // 候选行直接作为滚动区的子元素：`ScrollHandle::scroll_to_item` 按直接子元素的下标
        // 滚动，多包一层 flex 容器就只有 0 号子元素，键盘选到第 9 项以后浮层不会跟着滚。
        .child(
            div()
                .id("terminal-completion-scroll")
                .w_full()
                .max_h(px(height - 8.0))
                .flex()
                .flex_col()
                .track_scroll(scroll)
                .overflow_y_scrollbar()
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .children(rows),
        )
}

/// 终端画布 Element。
pub(crate) struct TerminalCanvas {
    pub component: Entity<TerminalComponent>,
}

/// prepaint 阶段缓存：cell 尺寸与布局参数。
struct TerminalLayout {
    cell_w: Pixels,
    line_h: Pixels,
    status_h: Pixels,
    /// 全角字符的绘图字号缩放比（使 CJK 字形前进恰为 2 倍 cell_w，光标按格对齐不发生漂移）。
    cjk_scale: f32,
    /// 画布的命中区：窗口级鼠标监听据此判断指针是否还在终端上（拖出后靠它区分
    /// “拖选继续”与“点到别处”）。
    hitbox: gpui::Hitbox,
}

impl IntoElement for TerminalCanvas {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalCanvas {
    type RequestLayoutState = ();
    type PrepaintState = TerminalLayout;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        // 终端元素填满整个可用区域（宽高均取 100%）。
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let cell_w = measure_terminal_cell(window, cx);
        let line_h = px(TERMINAL_FONT_SIZE + 3.0);
        let status_h = px(24.0);
        // 全角缩放到 2*cell_w，避免 CJK 字形窄于其占位格导致光标相对字形逐渐漂移。
        let cjk_scale = {
            let cjk_w = f32::from(measure_cjk_cell(window, cx)).max(1.0);
            (2.0 * f32::from(cell_w)) / cjk_w
        };

        let (grid_cols, grid_rows) = {
            let w = bounds.size.width;
            let h = bounds.size.height;
            let cols = (f32::from(w) / f32::from(cell_w)).floor().max(2.0) as u16;
            let rows = ((f32::from(h) - f32::from(status_h)) / f32::from(line_h))
                .floor()
                .max(1.0) as u16;
            (cols, rows)
        };

        // 记录布局参数供鼠标事件做像素→cell 换算；尺寸变化时 reflow grid 并 resize PTY。
        let content_h = f32::from(bounds.size.height) - f32::from(status_h);
        self.component.update(cx, |comp, _cx| {
            comp.set_layout(
                f32::from(cell_w),
                f32::from(line_h),
                f32::from(status_h),
                content_h,
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
            );
            if comp.grid.cols() != grid_cols || comp.grid.rows() != grid_rows {
                comp.grid.resize(grid_cols, grid_rows);
                if let Some(pty) = comp.transport.as_mut() {
                    pty.resize(grid_cols, grid_rows);
                }
            }
        });

        // 命中区在 prepaint 插入（paint 阶段不允许 insert_hitbox），
        // 供 paint 里注册的窗口级拖动/抬起监听判断指针是否还在终端上。
        let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);

        TerminalLayout {
            cell_w,
            line_h,
            status_h,
            cjk_scale,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let colors = TermColors::for_theme(cx.theme().is_dark());
        // 正文绘制统一相对终端画布的左上角：加上 bounds.origin 偏移，
        // 否则终端画布不在窗口 (0,0) 时(左侧有树/抽屉等)正文会被画到画布左侧之外而被裁剪，导致“有字看不见”。
        let left = f32::from(bounds.origin.x);

        // 背景。
        window.paint_quad(fill(bounds, colors.bg));

        // 挂输入 handler：可打印字符 / IME 进入 TerminalComponent（EntityInputHandler）。
        let focus_handle = self.component.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.component.clone()),
            cx,
        );

        // 挂窗口级拖动/抬起监听：元素自身的 `on_mouse_move` 只在指针悬停时回调，
        // 拖出终端边界后选区就冻结。这里按 Zed/Navop 的做法监听整个窗口——
        // 按下状态且（正在拖选或其下就是终端）时更新端点，超出上下边界则启动自动滚动。
        // 监听每帧重新注册，闭包拿的是本帧测出的 hitbox / bounds。
        let hitbox = prepaint.hitbox.clone();
        {
            let component = self.component.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Bubble {
                    return;
                }
                // 在窗口外松手收不到 MouseUp：靠下一次“无按键移动”收尾，避免拖选状态卡住。
                if event.pressed_button.is_none() {
                    component.update(cx, |comp, cx| {
                        if comp.selection_origin.is_some() {
                            comp.end_mouse_selection();
                            cx.notify();
                        }
                    });
                    return;
                }
                let hovered = hitbox.is_hovered(window);
                component.update(cx, |comp, cx| {
                    if !comp.dragging_selection && !hovered {
                        return;
                    }
                    // 只有真的按下过（有起点）才拖选，避免无锚点的移动改选区。
                    if comp.selection_origin.is_none() {
                        return;
                    }
                    if comp.update_mouse_selection(event.position) {
                        cx.notify();
                    }
                    comp.update_selection_autoscroll(event.position);
                    comp.ensure_selection_autoscroll(cx);
                });
            });
        }
        {
            let component = self.component.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
                if phase != gpui::DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                {
                    return;
                }
                component.update(cx, |comp, cx| {
                    if comp.selection_origin.is_some() {
                        comp.end_mouse_selection();
                        cx.notify();
                    }
                });
            });
        }

        // 读 grid 快照。
        let (visible_lines, cursor, following) = {
            let comp = self.component.read(cx);
            (
                comp.grid.visible_lines(),
                comp.grid.cursor(),
                comp.grid.following_bottom(),
            )
        };

        // 展示提示符沿用 redis-cli 原生的 `host:port[db]>`，历史命令行与当前行保持一致。
        let (prompt_state, native_prompt, display_prompt) = {
            let comp = self.component.read(cx);
            (
                comp.state,
                comp.adapter.prompt(),
                comp.adapter.display_prompt(),
            )
        };
        let show_display_prompt = following
            && matches!(
                prompt_state,
                fluxdb_core::terminal::TerminalSessionState::PtyRunning
                    | fluxdb_core::terminal::TerminalSessionState::Ready
                    | fluxdb_core::terminal::TerminalSessionState::AwaitingConfirmation
            );
        let current_line = visible_lines.get(cursor.y as usize).map(String::as_str);
        let current_prompt_line = show_display_prompt
            .then(|| {
                current_line
                    .and_then(|line| normalize_prompt_line(line, &native_prompt, &display_prompt))
            })
            .flatten();
        let display_cursor_x =
            if let Some((_, native_prefix_len, input_len)) = current_prompt_line.as_ref() {
                // 光标 x 是 redis 回显的**字节**位置；先吸附到最近字符边界的**单元列**，
                // 再减去提示符列得到输入内单元列光标。不然含宽字符时光标会落进字内/偏位。
                let row_cells: Vec<char> = current_line
                    .map(|s| s.chars().collect())
                    .unwrap_or_default();
                let input_cursor = byte_cursor_to_cell(&row_cells, cursor.x as usize)
                    .saturating_sub(*native_prefix_len);
                display_prompt.chars().count() + input_cursor.min(*input_len)
            } else {
                cursor.x as usize
            };

        // 跟随底部时按 screen 行取逐列样式对齐像素；否则退化纯文本。
        let following =
            following && visible_lines.len() as u16 == self.component.read(cx).grid.rows();

        // 选区高亮先铺：底色必须在字形之下，否则选中的文字会被整块盖掉。
        if self.component.read(cx).grid.has_selection() {
            paint_selection(&self.component, bounds, prepaint, &colors, window, cx);
        }

        let mut i = 0usize;
        for line_text in &visible_lines {
            let top = bounds.origin.y + px(i as f32 * f32::from(prepaint.line_h));
            if display_prompt != native_prompt {
                if let Some((line, _, _)) =
                    normalize_prompt_line(line_text, &native_prompt, &display_prompt)
                {
                    paint_prompt_line(&line, bounds, left, top, prepaint, &colors, window, cx);
                    i += 1;
                    continue;
                }
            }
            if following {
                let view = self.component.read(cx).grid.row_view(i);
                paint_term_row(&view, line_text, left, top, prepaint, &colors, window, cx);
            } else {
                paint_plain_line(line_text, left, top, prepaint, &colors, window, cx);
            }
            i += 1;
        }

        // 参数只作为淡色幽灵文本绘制，不写入 grid，因此不会改变实际光标位置。
        let inline_hint = if following {
            self.component.read(cx).inline_completion_hint()
        } else {
            None
        };
        if let Some(hint) = inline_hint.as_deref() {
            let row_top = bounds.origin.y + px(cursor.y as f32 * f32::from(prepaint.line_h));
            paint_completion_hint(
                hint,
                display_cursor_x,
                left,
                row_top,
                prepaint,
                &colors,
                window,
                cx,
            );
        }

        // 光标块（仅跟随底部时绘制）。
        if following {
            let row_top = bounds.origin.y + px(cursor.y as f32 * f32::from(prepaint.line_h));
            let x_px = bounds.origin.x + px(display_cursor_x as f32 * f32::from(prepaint.cell_w));
            // 细竖线光标在空格、中文和幽灵提示前都能明确显示位置。
            let caret = Bounds::new(point(x_px, row_top), size(px(1.5), prepaint.line_h));
            // 记录光标矩形（window 坐标），供模型层 `bounds_for_range` 返回，让输入法候选栏
            // 跟随光标显示（否则 macOS 候选栏落到屏幕左下角）。
            self.component
                .update(cx, |comp, _| comp.ime_caret_bounds = Some(caret));
            window.paint_quad(fill(caret, colors.default_fg));
        }

        // 状态栏：展示 adapter 提供的会话标题 + 状态文案（真正的 adapter 信息，而非占位）。
        let (session_title, session_status) = {
            let comp = self.component.read(cx);
            let status = comp.adapter.status_text(comp.state);
            let status = comp
                .failure_reason
                .as_deref()
                .map(|reason| format!("{status}: {reason}"))
                .unwrap_or(status);
            (comp.adapter.title(), status)
        };
        paint_status_bar(
            bounds,
            prepaint,
            &session_title,
            &session_status,
            &colors,
            window,
            cx,
        );

        // 危险命令确认条。
        let pending = self.component.read(cx).pending_dangerous.clone();
        if let Some(dangerous) = pending {
            paint_danger_bar(bounds, prepaint, &dangerous, &colors, window, cx);
        }
    }
}

fn paint_term_row(
    view: &TermRowView,
    line: &str,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    // 先画默认前景整行，再对逐列彩色段覆盖。
    paint_plain_line(line, left, top, layout, colors, window, cx);
    let mut i = 0usize;
    while i < view.chars.len() {
        // 找到一个连续同前景色区间。
        let fg = view.fg[i];
        let mut j = i + 1;
        while j < view.chars.len() && view.fg[j] == fg {
            j += 1;
        }
        if let Some((r, g, b)) = fg {
            let seg: String = view.chars[i..j].iter().collect();
            // 逐列 (u8,u8,u8) → 打包 0xRRGGBB，供 gpui `rgb(hex: u32)` 使用。
            let packed = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            paint_color_run(&seg, i, rgb(packed), left, top, layout, window, cx);
        }
        i = j;
    }
}

/// 绘制产品统一的 Redis CLI 当前输入行，遮掉后端原生的 host:port 提示符。
///
/// 不再给整行铺背景：画布底色已经铺过，这里再铺一层同色矩形会把这一行的选区高亮盖掉。
fn paint_prompt_line(
    line: &str,
    _bounds: Bounds<Pixels>,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    paint_plain_line(line, left, top, layout, colors, window, cx);
}

/// 把 PTY 中的后端原生提示符转换为产品统一提示符。
/// 返回值同时保留原生前缀长度和输入长度，供光标位置换算。
fn normalize_prompt_line(
    line: &str,
    native_prompt: &str,
    display_prompt: &str,
) -> Option<(String, usize, usize)> {
    let native_prefix = native_prompt.trim_end();
    // 光标位置换算用的"原生前缀单元数"必须含提示符尾部空格：
    // `suffix` 剥掉的是 trim_end 后的前缀 + 1 个原生空格，而 `cursor.x` 从含尾部空格的提示符算起，
    // 若这里取 trim_end 长度，空输入时光标会被多推 1 格，提示符与光标之间出现多余边距。
    let native_prefix_len = native_prompt.chars().count();
    let suffix = line.strip_prefix(native_prefix)?;
    let input = suffix.strip_prefix(' ').unwrap_or(suffix);
    Some((
        format!("{display_prompt}{input}"),
        native_prefix_len,
        // 输入长度按"单元宽"累计（宽字符=2），否则光标 x 撞上宽字符时会被错误夹短。
        input.chars().map(|c| char_cell_width(c) as usize).sum(),
    ))
}

fn paint_color_run(
    text: &str,
    col: usize,
    color: gpui::Rgba,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    // 按单元宽度分段绘制，覆盖默认前景色段的着色；宽字符第 2 格(空白)不绘制。
    let chars: Vec<char> = text.chars().collect();
    paint_cell_runs(&chars, col, color, left, top, layout, window, cx);
}

fn paint_plain_line(
    line: &str,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let chars: Vec<char> = line.chars().collect();
    paint_cell_runs(
        &chars,
        0,
        colors.default_fg.into(),
        left,
        top,
        layout,
        window,
        cx,
    );
}

/// 按终端单元宽度把字符序列切成"半角 / 全角"连续段，逐段成形并画在对应列。
/// - 半角段用基准字号，前进 1 格/字符；全角段把字形缩放为 2 格宽并定位在起始列，
///   这样字形恰好占 2 格，光标(按格计数)相对字形不会漂移，且字间无空格空隙。
fn paint_cell_runs(
    chars: &[char],
    start_col: usize,
    color: gpui::Rgba,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    window: &mut Window,
    cx: &mut App,
) {
    let mut col = start_col;
    let mut i = 0usize;
    while i < chars.len() {
        let w = char_cell_width(chars[i]) as usize;
        if w > 1 {
            // 全角段：连续收集全角字符；每个占 2 格，步进跳过其第 2 个空白格。
            let mut seg = String::new();
            let mut cells = 0usize;
            while i < chars.len() {
                let cw = char_cell_width(chars[i]) as usize;
                if cw <= 1 {
                    break;
                }
                seg.push(chars[i]);
                cells += cw;
                i += cw;
            }
            paint_run(
                &seg,
                col,
                TERMINAL_FONT_SIZE * layout.cjk_scale,
                color,
                left,
                top,
                layout,
                window,
                cx,
            );
            col += cells;
        } else {
            // 半角段（含真实空格，如输入的空格与行尾填充格）。
            let mut j = i;
            let mut cells = 0usize;
            while j < chars.len() && char_cell_width(chars[j]) as usize == 1 {
                j += 1;
                cells += 1;
            }
            paint_run(
                &chars[i..j].iter().collect::<String>(),
                col,
                TERMINAL_FONT_SIZE,
                color,
                left,
                top,
                layout,
                window,
                cx,
            );
            col += cells;
            i = j;
        }
    }
}

/// 用指定字号把一段文本成形并画在 (left + col*cell_w, top)。
fn paint_run(
    text: &str,
    col: usize,
    font_size: f32,
    color: gpui::Rgba,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let run = TextRun {
        len: text.len(), // TextRun.len 为字节数（gpui 用其做 utf8 切片），不能按 chars 数。
        font: terminal_font(),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text.to_string()),
        font_size.into(),
        &[run],
        None,
    );
    let x = px(left + col as f32 * f32::from(layout.cell_w));
    let _ = shaped.paint(point(x, top), layout.line_h, TextAlign::Left, None, window, cx);
}

fn paint_completion_hint(
    hint: &str,
    col: usize,
    left: f32,
    top: Pixels,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    if hint.is_empty() {
        return;
    }
    let run = TextRun {
        len: hint.len(),
        font: terminal_font(),
        color: colors.completion_hint_fg.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(hint.to_string()),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    let x = px(left + col as f32 * f32::from(layout.cell_w));
    let _ = shaped.paint(
        point(x, top),
        layout.line_h,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_selection(
    component: &Entity<TerminalComponent>,
    bounds: Bounds<Pixels>,
    layout: &TerminalLayout,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    // 逐行找出连续选中的列区间，绘一块半透明 bg。调用点在字形之前：高亮在文字下面。
    let comp = component.read(cx);
    for y in 0..comp.grid.rows() as usize {
        let mut x = 0usize;
        while x < comp.grid.cols() as usize {
            if !comp.grid.is_cell_selected(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < comp.grid.cols() as usize && comp.grid.is_cell_selected(x, y) {
                x += 1;
            }
            let cell = Bounds::new(
                point(
                    bounds.origin.x + px(start as f32 * f32::from(layout.cell_w)),
                    bounds.origin.y + px(y as f32 * f32::from(layout.line_h)),
                ),
                size(
                    px((x - start) as f32 * f32::from(layout.cell_w)),
                    layout.line_h,
                ),
            );
            window.paint_quad(fill(cell, colors.selection_bg));
        }
    }
}

fn paint_status_bar(
    bounds: Bounds<Pixels>,
    layout: &TerminalLayout,
    title: &str,
    status: &str,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let status_top = bounds.origin.y + bounds.size.height - layout.status_h;
    let bar = Bounds::new(
        point(bounds.origin.x, status_top),
        size(bounds.size.width, layout.status_h),
    );
    window.paint_quad(fill(bar, colors.status_bg));

    // 标题（adapter.title）加粗靠左，状态文案（adapter.status_text）跟随其后。
    // 用整段单 run 绘制，宽度受限于状态栏，超长由上层 clip。
    let text = format!("  {title}   ·   {status}");
    let run = TextRun {
        len: text.len(), // TextRun.len 为字节数（gpui 用其做 utf8 切片），不能按 chars 数。
        font: terminal_font(),
        color: colors.status_fg.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    let _ = shaped.paint(
        point(bounds.origin.x + px(8.0), status_top + px(4.0)),
        layout.line_h,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_danger_bar(
    bounds: Bounds<Pixels>,
    layout: &TerminalLayout,
    dangerous: &str,
    colors: &TermColors,
    window: &mut Window,
    cx: &mut App,
) {
    let h = px(28.0);
    let top = bounds.origin.y + bounds.size.height - layout.status_h - h;
    let bar = Bounds::new(point(bounds.origin.x, top), size(bounds.size.width, h));
    window.paint_quad(fill(bar, colors.danger_bg));

    let text = tr("app.terminal.danger_banner").replace("%{command}", &dangerous);
    let run = TextRun {
        len: text.len(), // TextRun.len 为字节数（gpui 用其做 utf8 切片），不能按 chars 数。
        font: terminal_font(),
        color: colors.danger_fg.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(text),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    let _ = shaped.paint(
        point(bounds.origin.x + px(8.0), top + px(5.0)),
        layout.line_h,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// 测一个全角字符（用中文字形 `的`）在基准字号下的前进宽度，用于计算全角缩放比。
fn measure_cjk_cell(window: &Window, _cx: &App) -> Pixels {
    let run = TextRun {
        len: 3, // `的` 为 3 字节 utf8
        font: terminal_font(),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from("的"),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    line.width
}

fn measure_terminal_cell(window: &Window, _cx: &App) -> Pixels {
    let run = TextRun {
        len: 1,
        font: terminal_font(),
        color: Hsla::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from("0"),
        (TERMINAL_FONT_SIZE as f32).into(),
        &[run],
        None,
    );
    px(f32::from(line.width).max(7.0))
}

fn terminal_font() -> gpui::Font {
    gpui::font(TERMINAL_FONT)
}
