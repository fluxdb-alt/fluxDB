// editor_component/input.rs —— 输入处理（键盘动作分发、IME、鼠标命中）。
//
// 依赖 mod.rs 定义的动作与 Editor 状态；此处只做输入转发与坐标换算，
// 不包含任何 SQL 业务逻辑。

/// Editor 的 IME/键鼠输入处理。
impl Editor {
    // ------------------------------------------------------------ 动作分发

    pub(crate) fn dispatch_action(&mut self, action: &dyn Action, cx: &mut Context<Self>) {
        if action.as_any().is::<Backspace>() {
            self.backspace(cx);
        } else if action.as_any().is::<Delete>() {
            self.delete(cx);
        } else if self.completion_visible && completion_accepts_action(action) {
            self.accept_selected(cx);
        } else if action.as_any().is::<Enter>() {
            let secondary = action
                .as_any()
                .downcast_ref::<Enter>()
                .map(|e| e.secondary)
                .unwrap_or(false);
            self.newline(secondary, cx);
        } else if action.as_any().is::<Escape>() {
            self.escape(cx);
        } else if self.completion_visible && action.as_any().is::<MoveUp>() {
            self._completion_up(cx);
        } else if self.completion_visible && action.as_any().is::<MoveDown>() {
            self._completion_down(cx);
        } else if action.as_any().is::<MoveUp>() {
            self.move_for_action(CursorMove::Up, false, cx);
        } else if action.as_any().is::<MoveDown>() {
            self.move_for_action(CursorMove::Down, false, cx);
        } else if action.as_any().is::<MoveLeft>() {
            self.move_for_action(CursorMove::Left, false, cx);
        } else if action.as_any().is::<MoveRight>() {
            self.move_for_action(CursorMove::Right, false, cx);
        } else if action.as_any().is::<MoveHome>() {
            self.move_for_action(CursorMove::Home, false, cx);
        } else if action.as_any().is::<MoveEnd>() {
            self.move_for_action(CursorMove::End, false, cx);
        } else if action.as_any().is::<MoveToStart>() {
            self.move_for_action(CursorMove::Start, false, cx);
        } else if action.as_any().is::<MoveToEnd>() {
            self.move_for_action(CursorMove::EndAll, false, cx);
        } else if action.as_any().is::<MoveToPreviousWord>() {
            self.move_for_action(CursorMove::PrevWord, false, cx);
        } else if action.as_any().is::<MoveToNextWord>() {
            self.move_for_action(CursorMove::NextWord, false, cx);
        } else if action.as_any().is::<SelectLeft>() {
            self.move_for_action(CursorMove::Left, true, cx);
        } else if action.as_any().is::<SelectRight>() {
            self.move_for_action(CursorMove::Right, true, cx);
        } else if action.as_any().is::<SelectUp>() {
            self.move_for_action(CursorMove::Up, true, cx);
        } else if action.as_any().is::<SelectDown>() {
            self.move_for_action(CursorMove::Down, true, cx);
        } else if action.as_any().is::<SelectHome>() {
            self.move_for_action(CursorMove::Home, true, cx);
        } else if action.as_any().is::<SelectEnd>() {
            self.move_for_action(CursorMove::End, true, cx);
        } else if action.as_any().is::<SelectToStart>() {
            self.move_for_action(CursorMove::Start, true, cx);
        } else if action.as_any().is::<SelectToEnd>() {
            self.move_for_action(CursorMove::EndAll, true, cx);
        } else if action.as_any().is::<SelectToPreviousWord>() {
            self.move_for_action(CursorMove::PrevWord, true, cx);
        } else if action.as_any().is::<SelectToNextWord>() {
            self.move_for_action(CursorMove::NextWord, true, cx);
        } else if action.as_any().is::<SelectAll>() {
            self.select_all(cx);
        } else if action.as_any().is::<SelectLine>() {
            self.select_line(cx);
        } else if action.as_any().is::<Undo>() {
            self.undo(cx);
        } else if action.as_any().is::<Redo>() {
            self.redo(cx);
        } else if action.as_any().is::<Copy>() {
            self.copy(cx);
        } else if action.as_any().is::<Cut>() {
            self.cut(cx);
        } else if action.as_any().is::<Paste>() {
            self.paste(cx);
        } else if action.as_any().is::<IndentInline>()
            && self.snippet_session.is_some()
        {
            // Snippet 会话活动时 Tab 切到下一个占位，不缩进。
            self.snippet_next(cx);
        } else if action.as_any().is::<IndentInline>() {
            self.handle_indent(false, cx);
        } else if action.as_any().is::<OutdentInline>()
            && self.snippet_session.is_some()
        {
            // Snippet 会话活动时 Shift-Tab 切到上一个占位，不反缩进。
            self.snippet_prev(cx);
        } else if action.as_any().is::<OutdentInline>() {
            self.handle_indent(true, cx);
        } else if action.as_any().is::<ToggleLineComment>() {
            self.toggle_line_comment(cx);
        } else if action.as_any().is::<ToggleFold>() {
            self.toggle_fold_at_cursor(cx);
        } else if action.as_any().is::<FoldAll>() {
            self.fold_all(cx);
        } else if action.as_any().is::<UnfoldAll>() {
            self.unfold_all(cx);
        } else if action.as_any().is::<TriggerCompletion>() {
            self.request_completion(true, cx);
        } else if action.as_any().is::<OpenFind>() {
            self.open_find(cx);
        } else if action.as_any().is::<CloseFind>() {
            self.close_find(cx);
        } else if action.as_any().is::<FindNext>() {
            self.find_next(cx);
        } else if action.as_any().is::<FindPrevious>() {
            self.find_previous(cx);
        } else {
            // 其它动作：忽略。
        }
    }

    fn escape(&mut self, cx: &mut Context<Self>) {
        // Esc：优先关闭查找面板，其次关闭补全 / hover 浮层，最后退出 snippet 会话。
        if self.find_state.open {
            self.close_find(cx);
            return;
        }
        if self.completion_visible {
            self.completion_visible = false;
            self.completion_scroll_handle.set_offset(gpui::Point::default());
            self.completion_width = None;
            self.hover_content = None;
            cx.notify();
        }
        if self.signature.is_some() {
            self.signature = None;
            cx.notify();
        }
        // 退出 snippet 会话：清除占位选中，恢复普通编辑。
        if self.snippet_session.is_some() {
            self.snippet_session = None;
            cx.notify();
        }
    }

    fn select_line(&mut self, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let point = self.buffer.offset_to_point(cursor);
        let start = self.buffer.line_start(point.row);
        let end = self.buffer.line_end_offset(point.row);
        self.selection = Selection::new(start, end);
        cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
        cx.notify()
    }

    // ------------------------------------------------------------ Snippet 会话

    /// 校验 snippet 会话是否仍有效；无效时自动清理。
    fn validate_snippet_session(&mut self) {
        let len = self.buffer.len();
        if let Some(session) = &self.snippet_session {
            if !session.is_valid(len) {
                self.snippet_session = None;
            }
        }
    }

    /// Tab：切到下一个占位。
    fn snippet_next(&mut self, cx: &mut Context<Self>) {
        self.validate_snippet_session();
        if let Some(session) = &mut self.snippet_session {
            session.next();
            if let Some(range) = session.current_range() {
                self.selection = Selection::new(range.start, range.end);
            }
            cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
            cx.notify();
        }
    }

    /// Shift-Tab：切到上一个占位。
    fn snippet_prev(&mut self, cx: &mut Context<Self>) {
        self.validate_snippet_session();
        if let Some(session) = &mut self.snippet_session {
            session.prev();
            if let Some(range) = session.current_range() {
                self.selection = Selection::new(range.start, range.end);
            }
            cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
            cx.notify();
        }
    }

    // ------------------------------------------------------------ 鼠标
    // 鼠标交互（点击选中 / 框选 / 滚动 / hover）由宿主编辑器面板转发到这里。

    #[allow(dead_code)]
    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event.button, MouseButton::Left | MouseButton::Right) {
            return;
        }
        self.focus_handle.focus(window, cx);
        if event.button != MouseButton::Left {
            return;
        }
        // 补全浮层外点关闭：点击落在浮层外 → 关闭浮层后继续正常文本光标处理。
        // （浮层内点击已由行 Element 自身 stop_propagation 消费，不会走到这里。）
        if self.completion_visible {
            let inside = self
                .completion_placement(window)
                .map(|(origin, size)| {
                    // completion_placement 返回视口本地原点，加上视口原点得全局命中区。
                    let viewport = self.scroll_handle.bounds().origin;
                    let global_origin = gpui::point(origin.x + viewport.x, origin.y + viewport.y);
                    Bounds::new(global_origin, size).contains(&event.position)
                })
                .unwrap_or(false);
            if !inside {
                self.completion_visible = false;
                self.completion_scroll_handle.set_offset(gpui::Point::default());
                self.completion_width = None;
                cx.notify();
            }
        }
        self.selecting_with_mouse = true;
        // 命中行区域（折叠箭头 / 运行按钮）。
        for region in &self.line_hit_regions {
            if region.bounds.contains(&event.position) {
                match region.kind {
                    LineHitKind::Fold => {
                        self.toggle_fold_row(region.row, cx);
                        return;
                    }
                    LineHitKind::Run => {
                        self.request_execution_selection(cx);
                        return;
                    }
                    LineHitKind::CodeLens(index) => {
                        if let Some(hit) = self.code_lens_hits.get(index).cloned() {
                            cx.emit(EditorEvent::CodeLensActivated {
                                range: hit.range,
                                action: hit.action,
                            });
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
        if event.click_count >= 2 && self.select_word_from_mouse(event.position, window, cx) {
            self.selecting_with_mouse = false;
            return;
        }
        // 点击行内部：把坐标换算为 buffer offset。
        self.set_cursor_from_mouse(event.position, window, cx);
    }

    #[allow(dead_code)]
    pub(crate) fn mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting_with_mouse {
            self.selecting_with_mouse = false;
            cx.notify();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let viewport = self.scroll_handle.bounds();
        let scroll = self.scroll_handle.offset();
        let gutter_left = viewport.left() + scroll.x + px(EDITOR_PADDING_X);
        let gutter_right = gutter_left + self.line_number_width(window);
        self.gutter_hovered = event.position.x >= gutter_left
            && event.position.x <= gutter_right
            && event.position.y >= viewport.top()
            && event.position.y <= viewport.bottom();
        // 更新 hover 命中。
        self.hovered_line_region = None;
        for (i, region) in self.line_hit_regions.iter().enumerate() {
            if region.bounds.contains(&event.position) {
                self.hovered_line_region = Some(i);
            }
        }
        if self.selecting_with_mouse {
            self.set_cursor_from_mouse_continue(event.position, &*window, cx);
        } else {
            // hover 请求（阈值由输入层控制）。
            if let Some(point) = self.point_from_mouse(event.position, &*window) {
                self.request_hover(point, cx);
            }
        }
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn scroll(
        &mut self,
        _event: &ScrollWheelEvent,
        delta: gpui::ScrollDelta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 补全浮层滚轮由浮层自身（completion_popup.rs 的 on_scroll_wheel stop_propagation）
        // 消费；此处的滚轮只驱动编辑器正文滚动。
        // 与 Zed 一致：离散滚轮的 X/Y 使用不同的单位，不能直接用同一个 pixel_delta
        // （GPUI 会把两轴都按行高换算，导致横向速度错误）。
        let line_height = f32::from(self.line_height(window));
        let glyph_width = self.measure_character_width(window);
        let (mut delta_x, mut delta_y) = match delta {
            gpui::ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            gpui::ScrollDelta::Lines(delta) => (delta.x * glyph_width, delta.y * line_height),
        };

        // 保持一次滚轮手势的主轴，避免斜向滚动同时改变两个方向造成跳动。
        if delta_x != 0.0 && delta_y != 0.0 {
            if delta_x.abs() > delta_y.abs() {
                delta_y = 0.0;
            } else {
                delta_x = 0.0;
            }
        }

        // 滚轮只更新「目标 offset」；实际位移由动画时钟逐帧指数趋近，保证平滑
        // 子像素滚动。越界在目标侧 clamp。
        let max = self.scroll_handle.max_offset();
        let now = Instant::now();
        // 新手势（动画已静默，或距上次滚轮 >150ms）：错误地以「当前 offset」为基准
        // 重新累计，避免把鼠标/滚动条/光标自动滚动等外部 set_offset 造成的位移，
        // 误当作目标被动画拉回。连续手势则在上次目标上累加，保留完整滚轮位移。
        let fresh_gesture = !self.scroll_animating
            || self
                .scroll_last_wheel_at
                .is_none_or(|t| now.duration_since(t) > Duration::from_millis(150));
        self.scroll_last_wheel_at = Some(now);
        if delta_x != 0.0 || delta_y != 0.0 {
            let base = if fresh_gesture {
                let cur = self.scroll_handle.offset();
                gpui::point(f32::from(cur.x), f32::from(cur.y))
            } else {
                self.scroll_target
            };
            self.scroll_target = gpui::point(
                (base.x + delta_x).clamp(-f32::from(max.x), 0.0),
                (base.y + delta_y).clamp(-f32::from(max.y), 0.0),
            );
        }
        self.begin_scroll_animation(window, cx);
        cx.stop_propagation();
    }

    /// 启动滚动动画时钟（幂等）：已在运行时直接返回，否则先跑一帧再自接力。
    fn begin_scroll_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_animating {
            return;
        }
        self.scroll_animating = true;
        self.scroll_last_tick = Some(Instant::now());
        self.scroll_tick(window, cx);
    }

    /// 单帧推进：把当前 offset 按 `1 - exp(-K*dt)` 指数趋近目标（无过冲），越界
    /// clamp；距目标足够近即停止。用 `cx.on_next_frame` 自接力到下一显示帧
    /// （对齐 vsync 的 ~60Hz），滚动帧不再受滚轮事件节拍（~18Hz）限制。
    fn scroll_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = self
            .scroll_last_tick
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32())
            // 窗口隐藏/卡顿时 dt 会很大，钳制避免一次跳一大段。
            .min(1.0 / 30.0);
        self.scroll_last_tick = Some(now);

        let ease = 1.0 - (-SCROLL_EASE_K * dt).exp();
        let target = self.scroll_target;
        let mut off = self.scroll_handle.offset();
        off.x = px(f32::from(off.x) + (target.x - f32::from(off.x)) * ease);
        off.y = px(f32::from(off.y) + (target.y - f32::from(off.y)) * ease);

        let settled = (target.x - f32::from(off.x)).abs() < SCROLL_STOP_DISTANCE
            && (target.y - f32::from(off.y)).abs() < SCROLL_STOP_DISTANCE;
        if settled {
            off = gpui::point(px(target.x), px(target.y));
        }
        self.scroll_handle.set_offset(off);
        // 注意：这里不再 `cx.notify()` —— notify Editor 子视图会走延迟 Effect::Notify
        // 队列，处理时撞 draw_phase gate 被丢弃（inv_skip 高、draw 掉到 ~13Hz）。重绘
        // 由 render.rs paint 里的 `request_animation_frame`（notify 顶层 current_view，
        // 实时 invalidate_view）驱动。scroll_tick 仅负责推进 offset 与排下一帧。

        if settled {
            self.scroll_animating = false;
            return;
        }
        // 接力下一帧：靠 paint 里 `request_animation_frame` 每帧 notify 顶层视图驱动
        // 重绘，本回调推进到下一显示帧后，`on_next_frame` 再排下一帧推进，形成持续
        // ~60Hz 循环。不能用 `window.request_animation_frame()`（this）：它在非
        // paint/prepaint 阶段调用 `current_view()` 会对空栈 unwrap 而 panic。
        cx.on_next_frame(window, |this, window, cx| this.scroll_tick(window, cx));
    }

    #[allow(dead_code)]
    fn set_cursor_from_mouse(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(point) = self.point_from_mouse(position, window) {
            let offset = self.buffer.point_to_offset(point).min(self.buffer.len());
            self.selection = Selection::point(offset);
            cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
            cx.notify();
        }
    }

    #[allow(dead_code)]
    fn set_cursor_from_mouse_continue(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(point) = self.point_from_mouse(position, window) {
            let offset = self.buffer.point_to_offset(point).min(self.buffer.len());
            self.selection.cursor = offset;
            cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
            cx.notify();
        }
    }

    fn select_word_from_mouse(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(point) = self.point_from_mouse(position, window) else {
            return false;
        };
        let offset = self.buffer.point_to_offset(point).min(self.buffer.len());
        let snapshot = self.buffer.snapshot();
        let row = snapshot.offset_to_point(offset).row;
        let line_start = snapshot.line_start(row);
        let line_end = snapshot.line_end_offset(row);
        let raw_line = snapshot.text_in_range(CoreRange::new(line_start, line_end));
        let line = raw_line.strip_suffix('\n').unwrap_or(&raw_line);
        let local_offset = offset.saturating_sub(line_start).min(line.len());
        let Some((start, end)) = word_range_in_text(line, local_offset, |ch| self.word_char(ch))
        else {
            return false;
        };
        self.selection = Selection::new(line_start + start, line_start + end);
        cx.emit(EditorEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
        true
    }
}

/// 返回双击位置对应的词（字节范围）；非词字符单独选中，空白不产生选区。
fn word_range_in_text(
    text: &str,
    offset: usize,
    is_word_char: impl Fn(char) -> bool,
) -> Option<(usize, usize)> {
    let mut cursor = offset.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    if cursor == text.len() {
        cursor = text[..cursor].char_indices().next_back().map(|(i, _)| i)?;
    }
    let ch = text[cursor..].chars().next()?;
    if ch.is_whitespace() {
        return None;
    }
    let char_end = cursor + ch.len_utf8();
    if !is_word_char(ch) {
        return Some((cursor, char_end));
    }

    let mut start = cursor;
    while start > 0 {
        let previous = text[..start].char_indices().next_back().map(|(i, _)| i)?;
        let previous_char = text[previous..start].chars().next()?;
        if !is_word_char(previous_char) {
            break;
        }
        start = previous;
    }
    let mut end = char_end;
    while end < text.len() {
        let next_char = text[end..].chars().next()?;
        if !is_word_char(next_char) {
            break;
        }
        end += next_char.len_utf8();
    }
    Some((start, end))
}

#[cfg(test)]
mod word_selection_tests {
    use super::word_range_in_text;

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    #[test]
    fn selects_word_at_start_middle_and_end() {
        let text = "SELECT user_name FROM users";
        assert_eq!(word_range_in_text(text, 0, is_word_char), Some((0, 6)));
        assert_eq!(word_range_in_text(text, 9, is_word_char), Some((7, 16)));
        assert_eq!(word_range_in_text(text, text.len(), is_word_char), Some((22, 27)));
    }

    #[test]
    fn handles_unicode_punctuation_and_whitespace() {
        let text = "你好，world";
        assert_eq!(word_range_in_text(text, 3, is_word_char), Some((0, 6)));
        assert_eq!(word_range_in_text(text, 6, is_word_char), Some((6, 9)));
        assert_eq!(word_range_in_text(text, 10, is_word_char), Some((9, 14)));
        assert_eq!(word_range_in_text("a b", 1, is_word_char), None);
    }
}

/// 由屏幕坐标换算 buffer 的 Point（行/字节列）。
impl Editor {
    pub(crate) fn point_from_mouse(
        &self,
        position: gpui::Point<Pixels>,
        window: &Window,
    ) -> Option<Point> {
        let started = Instant::now();
        let viewport = self.scroll_handle.bounds();
        let view_height = f32::from(viewport.size.height);
        if view_height <= 0.0 {
            return None;
        }
        let scroll = self.scroll_handle.offset();
        let line_height = f32::from(self.line_height(window));
        // 行号：视口内 y 相对偏移 ÷ 行高，再减去纵向内边距。
        let content_y = (f32::from(position.y)
            - f32::from(viewport.top())
            - f32::from(scroll.y)
            - EDITOR_PADDING_Y)
            .max(0.0);
        let stride = line_height + EDITOR_LINE_GAP;
        let line_count = self.display.visual_row_count();
        let lens_prefix = |r: usize| self.block_lens_prefix(r);
        let row = visual_row_for_content_y(line_count, content_y, stride, lens_prefix);
        if row >= line_count {
            return None;
        }
        let visual = self.display.visual_line_at(row)?;
        let buffer_row = visual.buffer_row;
        // 使用 shaped glyph 的实际位置命中，避免把 UTF-16/字节列当作等宽字符列。
        let gutter = f32::from(self.line_number_width(window));
        let text_x = f32::from(position.x)
            - f32::from(viewport.left())
            - f32::from(scroll.x)
            - EDITOR_PADDING_X
            - gutter
            - EDITOR_CONTENT_GAP;
        let line_start = self.buffer.line_start(buffer_row);
        let line_end = self.buffer.line_end_offset(buffer_row);
        let fragment_start = line_start + visual.column_start.min(line_end.saturating_sub(line_start));
        let fragment_end = line_start + visual.column_end.min(line_end.saturating_sub(line_start));
        let line = VisualLineRender {
            visual_row: row,
            buffer_row,
            first_fragment: visual.first_fragment,
            byte_range: CoreRange::new(fragment_start, fragment_end),
            current_line: true,
        };
        let shaped = self.shape_visual_line(&line, window);
        let byte = shaped.closest_index_for_x(px(text_x.max(0.0)));
        let point = Point::new(buffer_row, visual.column_start + byte);
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "hit_test",
            editor_id = self.perf_editor_id,
            edit_id = self.perf_edit_id,
            buffer_version = self.buffer.version(),
            request_id = 0u64,
            task_id = 0u64,
            layer = "display",
            elapsed_us = started.elapsed().as_micros() as u64,
            visual_row = row,
            buffer_row,
        );
        Some(point)
    }

    /// 计算某一字节偏移处的光标矩形（viewport 局部坐标），供 IME 的 bounds_for_range 使用。
    pub(crate) fn cursor_rect_for_offset(&self, offset: usize, window: &Window) -> Bounds<Pixels> {
        let point = self.buffer.offset_to_point(offset);
        let visual_row = self.display.visual_row_for_column(point.row, point.column) as usize;
        let line_height = f32::from(self.line_height(window));
        let visual = self.display.visual_line_at(visual_row);
        let x_offset = if let Some(visual) = visual {
            let line_start = self.buffer.line_start(point.row);
            let line_end = self.buffer.line_end_offset(point.row);
            let fragment_start = (line_start + visual.column_start).min(line_end);
            let fragment_end = (line_start + visual.column_end).min(line_end);
            let line = VisualLineRender {
                visual_row,
                buffer_row: point.row,
                first_fragment: visual.first_fragment,
                byte_range: CoreRange::new(fragment_start, fragment_end),
                current_line: true,
            };
            let shaped = self.shape_visual_line(&line, window);
            shaped.x_for_index(
                offset
                    .saturating_sub(fragment_start)
                    .min(fragment_end - fragment_start),
            )
        } else {
            px(0.)
        };
        let gutter = f32::from(self.line_number_width(window));
        let viewport = self.scroll_handle.bounds();
        let scroll = self.scroll_handle.offset();
        let x = f32::from(viewport.left())
            + f32::from(scroll.x)
            + EDITOR_PADDING_X
            + gutter
            + EDITOR_CONTENT_GAP
            + f32::from(x_offset);
        let y = f32::from(viewport.top())
            + f32::from(scroll.y)
            + f32::from(
                self.y_for_visual_row(visual_row, self.line_height(window)),
            );
        Bounds::from_corners(
            GPoint::new(px(x), px(y)),
            GPoint::new(px(x + 1.), px(y + line_height)),
        )
    }
}

/// UTF-16 offset → 字节 offset；若越界则返回 None。
///
/// 仅用于 IME 组合子文本（`marked_text`，局部短字符串）的相对换算；
/// 文档级 UTF-16 ↔ 字节换算一律走 `BufferSnapshot::utf16_to_byte` / `byte_to_utf16`，
/// 避免为换算构造全文字符串（见整改设计 4.1）。
fn utf16_to_byte_offset(text: &str, utf16: usize) -> Option<usize> {
    let mut byte = 0usize;
    let mut count = 0usize;
    for c in text.chars() {
        if count >= utf16 {
            break;
        }
        count += c.len_utf16();
        byte += c.len_utf8();
    }
    if count + 1 >= utf16 && byte <= text.len() {
        Some(byte)
    } else if utf16 == 0 {
        Some(0)
    } else {
        None
    }
}

/// 文档级 UTF-16 区间 → 字节区间；任一端点越界则返回 None。
fn snapshot_utf16_to_byte_range(snap: &BufferSnapshot, range_utf16: Range<usize>) -> Option<CoreRange> {
    let start = snap.utf16_to_byte(range_utf16.start)?;
    let end = snap.utf16_to_byte(range_utf16.end)?;
    Some(CoreRange::new(start, end))
}

/// 文档级字节区间 → UTF-16 区间（供 IME / 输入法使用）。
fn snapshot_byte_to_utf16_range(snap: &BufferSnapshot, range: &CoreRange) -> Range<usize> {
    snap.byte_to_utf16(range.start)..snap.byte_to_utf16(range.end)
}

fn ime_replacement_range(
    snap: &BufferSnapshot,
    range_utf16: Option<Range<usize>>,
    marked_range: Option<CoreRange>,
    selection_range: CoreRange,
) -> CoreRange {
    let chosen = range_utf16
        .and_then(|range| snapshot_utf16_to_byte_range(snap, range))
        .or(marked_range)
        .unwrap_or(selection_range);
    sanitize_ime_range(snap, chosen, selection_range)
}

/// 把 IME 替换区间吸附到当前快照的合法字符边界；退化/越界时回退到选区。
///
/// 关键在 `marked_range`：组合期间文档一旦被其它编辑改动，之前记录的字节区间可能相对当前
/// 文档「过期」——落在多字节字符中间甚至越界。若不加处理直接交给 `edit`，Rope 切片会因
/// 端点非字符边界而 panic（见 buffer.rs `append_range_to_string` / `replace`）。在此统一吸附：
/// 端点收敛到字符边界并保证 `start < end`，使 `apply_edit` 的旧区间总能安全切片。
fn sanitize_ime_range(snap: &BufferSnapshot, chosen: CoreRange, fallback: CoreRange) -> CoreRange {
    let start = snap.clamp_to_char_boundary(chosen.start);
    let end = snap.clamp_to_char_boundary(chosen.end);
    // 越界或吸付后退化（空区间）视为无效，回退到当前选区。
    if start >= end {
        return fallback;
    }
    CoreRange::new(start, end)
}

// ------------------------------------------------------------ IME / 输入法

impl gpui::EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let snap = self.buffer.snapshot();
        let range = snapshot_utf16_to_byte_range(&snap, range_utf16)?;
        actual_range.replace(snapshot_byte_to_utf16_range(&snap, &range));
        Some(snap.text_in_range(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let snap = self.buffer.snapshot();
        let range = self.selection_range();
        Some(UTF16Selection {
            range: snapshot_byte_to_utf16_range(&snap, &range),
            reversed: self.selection.cursor < self.selection.anchor,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        let snap = self.buffer.snapshot();
        self.ime_marked_range
            .as_ref()
            .map(|range| snapshot_byte_to_utf16_range(&snap, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 输入法未给显式区间时必须优先替换组合文本，否则每次更新都会追加一份拼音。
        let range = ime_replacement_range(
            &self.buffer.snapshot(),
            range_utf16,
            self.ime_marked_range,
            self.selection_range(),
        );
        self.ime_marked_range = None;
        self.selection = Selection::new(range.start, range.end);
        let cursor = range.start + text.len();
        self.apply_edit(text, cursor, cursor, false, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = ime_replacement_range(
            &self.buffer.snapshot(),
            range_utf16,
            self.ime_marked_range,
            self.selection_range(),
        );
        self.selection = Selection::new(range.start, range.end);
        let cursor = range.start + new_text.len();
        self.apply_edit(new_text, cursor, cursor, false, cx);

        if new_text.is_empty() {
            self.ime_marked_range = None;
        } else {
            // IME 组合区间标记为已替换的文本。
            let marked_range = range.start..range.start + new_text.len();
            if let Some(sel) = new_selected_range_utf16 {
                // 组合文本是局部短字符串，仅对其进行相对换算，不读取全文。
                let marked_text = self.buffer.text_in_range(CoreRange::new(marked_range.start, marked_range.end));
                let rel = utf16_to_byte_offset(&marked_text, sel.start)
                    .and_then(|s| utf16_to_byte_offset(&marked_text, sel.end).map(|e| (s, e)));
                if let Some((rel_start, rel_end)) = rel {
                    self.selection =
                        Selection::new(marked_range.start + rel_start, marked_range.start + rel_end);
                }
            } else {
                self.selection = Selection::point(marked_range.end);
            }
            self.ime_marked_range = Some(CoreRange::new(marked_range.start, marked_range.end));
        }
        self.completion_visible = false;
        self.completion_loading = false;
        // Changed(TextChange) 已由 apply_edit → after_edit 发出，此处不再重复广播。
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let snap = self.buffer.snapshot();
        let range = snapshot_utf16_to_byte_range(&snap, range_utf16)?;
        // 以区间起点作为边界矩形。
        Some(self.cursor_rect_for_offset(range.start, window))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let pt = self.point_from_mouse(point, window)?;
        Some(self.buffer.point_to_offset(pt).min(self.buffer.len()))
    }
}

fn completion_accepts_action(action: &dyn Action) -> bool {
    action.as_any().is::<Enter>() || action.as_any().is::<IndentInline>()
}

#[cfg(test)]
mod ime_tests {
    use super::*;

    #[test]
    fn marked_text_takes_precedence_when_ime_omits_replacement_range() {
        let snap = EditorBuffer::new_from("dasd").snapshot();
        assert_eq!(
            ime_replacement_range(
                &snap,
                None,
                Some(CoreRange::new(0, 4)),
                CoreRange::empty(4),
            ),
            CoreRange::new(0, 4),
        );
    }

    #[test]
    fn stale_marked_range_is_snapped_to_char_boundary() {
        // 文档 "你你颜"：你 0..3 / 你 3..6 / 颜 6..9。
        // 过期的 marked 区间起点落到 '颜' 中间（字节 8），必须吸附回字符边界 6。
        let snap = EditorBuffer::new_from("你你颜").snapshot();
        let range = ime_replacement_range(
            &snap,
            None,
            Some(CoreRange::new(8, 9)),
            CoreRange::empty(9),
        );
        assert_eq!(range, CoreRange::new(6, 9));
    }

    #[test]
    fn out_of_bounds_marked_range_falls_back_to_selection() {
        let snap = EditorBuffer::new_from("abc").snapshot();
        // 越界（起点吸附后=末尾，start==end）→ 回退到当前选区。
        let range = ime_replacement_range(
            &snap,
            None,
            Some(CoreRange::new(50, 60)),
            CoreRange::empty(3),
        );
        assert_eq!(range, CoreRange::empty(3));
    }

    #[test]
    fn completion_accepts_tab_and_enter_only() {
        assert!(completion_accepts_action(&Enter { secondary: false }));
        assert!(completion_accepts_action(&IndentInline));
        assert!(!completion_accepts_action(&OutdentInline));
        assert!(!completion_accepts_action(&Escape));
    }
}
