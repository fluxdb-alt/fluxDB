// editor_component/completion_ui.rs —— 补全 / hover 交互与浮层几何。
//
// 浮层本体已 Element 化（见 completion_popup.rs）：本文件仅负责选中项移动、
// 放置计算与宽度定型；滚动交给原生 ScrollHandle，不再手绘切片。

impl Editor {
    fn move_completion_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.completion_items.is_empty() {
            return;
        }
        self.completion_selected =
            completion_index_after_step(self.completion_selected, self.completion_items.len(), delta);
        // 保持选中可见交给原生 ScrollHandle（最小滚动滚入视口）。
        self.completion_scroll_handle.scroll_to_item(self.completion_selected);
        self.request_completion_documentation(cx);
        cx.notify();
    }

    /// F005：异步加载选中项右侧 metadata 详情。
    ///
    /// 每次切换先 bump 取消令牌，令上一 in-flight 详情判废（latest-wins），再在后台
    /// 线程调用 provider 的 `documentation`（SQL provider 走内存 CompletionIndex，
    /// 无数据库访问）。提交时用令牌 + 仍在选中同一项 双守卫，旧结果不覆盖新选择。
    pub(crate) fn request_completion_documentation(&mut self, cx: &mut Context<Self>) {
        self.completion_doc_state = None;
        let Some(item) = self.completion_items.get(self.completion_selected).cloned() else {
            self._completion_doc_task = None;
            return;
        };
        let Some(provider) = self.providers.completion.clone() else {
            self.completion_doc_state = Some(DocumentationState::Error("无详情 provider".into()));
            cx.notify();
            return;
        };
        // DM 同款：新请求先 bump，旧任务提交时判废。
        let doc_id = self._completion_doc_token.request_id();
        let doc_token = self._completion_doc_token.clone();
        let selected = self.completion_selected;
        let kind = item.kind;
        let label = item.label.clone();
        let comment = (!item.documentation.is_empty()).then(|| item.documentation.clone());
        let request = DocumentationRequest {
            kind,
            label,
            comment,
            latest_request: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(doc_id)),
            request_id: doc_id,
        };
        let task = cx.background_spawn(async move {
            // 同步解析（SQL provider 走内存索引）；在后台线程执行，不阻塞主线程。
            provider.documentation(request)
        });
        let task = cx.spawn(async move |this, cx| {
            let state = task.await;
            let _ = this.update(cx, |this, cx| {
                // 令牌判废（已有新选择/浮层关闭）或选中项已切换 → 丢弃旧结果。
                if !doc_token.check(doc_id) || this.completion_selected != selected {
                    return;
                }
                // provider 未返回详情（None，如 Redis 命令无文档）→ 不显示右侧面板，
                // doc_state 保持 None，浮层仅候选列表、宽度不加宽。
                let Some(state) = state else {
                    return;
                };
                this.completion_doc_state = Some(state);
                cx.notify();
            });
        });
        self._completion_doc_task = Some(task);
        cx.notify();
    }

    /// 计算补全浮层放置，返回「编辑器根本地」左上角与尺寸。
    ///
    /// 原 paint 版的 completion_geometry 不再做滚动切片（行已 Element 化），只负责
    /// 定位 + 定宽；宽度由 render() 在打开时对全量候选定型缓存于 `completion_width`，
    /// 滚动到不同标签不会抖动。
    pub(crate) fn completion_placement(&self, window: &Window) -> Option<(GPoint, Size<Pixels>)> {
        if !self.completion_visible {
            return None;
        }
        // 加载态占位：已触发补全但候选尚未就绪（空列表 + loading）时，仍放置一个单行浮层。
        let loading_placeholder = self.completion_loading && self.completion_items.is_empty();
        let total = self.completion_items.len();
        let rows = if total == 0 {
            if loading_placeholder { 1 } else { return None; }
        } else {
            completion_visible_row_count(total)
        };
        // 锚定几何：浮层打开时记录一次锚点，后续位置不随光标 notify 重算，避免
        // “补全框跟着光标走”（对齐 dbeaver/JFace）。anchor 未设置时回退到当前光标。
        let anchor = self
            .completion_anchor_offset
            .unwrap_or_else(|| self.cursor_offset());
        let cursor_bounds = self.cursor_rect_for_offset(anchor, window);
        let viewport = self.scroll_handle.bounds();
        // 宽度：打开时定型一次。占位（无候选）宽度走默认钳制。
        let mut width = if total == 0 {
            px(completion_popup_width(0.0))
        } else {
            self.completion_width.unwrap_or_else(|| px(completion_popup_width(0.0)))
        };
        // F005：有 metadata 详情时右侧固定加宽（详情面板 + 分隔条）。
        if self.completion_doc_state.is_some() {
            width += px(COMPLETION_DOC_WIDTH + COMPLETION_DOC_DIVIDER);
        }
        let row_height = px(COMPLETION_ROW_HEIGHT);
        // 浮层内边距 4*2 + 边框 1*2 = 10px 纵向开销。
        // 高度 = 行高×可见行数 + 10，自适应候选数；候选过少时不低于最小高度。
        let height =
            (row_height * rows as f32 + px(10.)).max(px(COMPLETION_POPUP_MIN_HEIGHT));

        // 放在光标下方；若越界则上移。
        let mut top = cursor_bounds.bottom() + px(4.);
        if top + height > viewport.bottom() {
            top = (cursor_bounds.top() - height - px(4.)).max(viewport.top());
        }
        let left = cursor_bounds.left().min(viewport.right() - width - px(8.));
        // 视口本地坐标：作为编辑器根 `.absolute()` 子节点偏移。
        let origin = GPoint::new(left - viewport.left(), top - viewport.top());
        Some((origin, Size::new(width, height)))
    }

    /// 收集浮层 Element 所需数据快照（render() 调用；不借 render 的 cx 读 Editor）。
    pub(crate) fn completion_popup_data(&self, window: &Window) -> Option<CompletionPopupData> {
        let (origin, size) = self.completion_placement(window)?;
        // 同名候选判定歧义，与旧 paint 版一致：仅歧义候选显示 detail。
        let mut seen_labels = BTreeSet::new();
        let mut ambiguous = BTreeSet::new();
        for item in &self.completion_items {
            let key = item.label.to_ascii_lowercase();
            if !seen_labels.insert(key.clone()) {
                ambiguous.insert(key);
            }
        }
        Some(CompletionPopupData {
            origin,
            size,
            theme: self.theme,
            font: self.editor_font(),
            font_size: px(self.font_size),
            query: self.completion_query.clone(),
            selected: self.completion_selected,
            scroll_handle: self.completion_scroll_handle.clone(),
            items: self.completion_items.clone(),
            loading: self.completion_loading && self.completion_items.is_empty(),
            ambiguous,
            doc_state: self.completion_doc_state.clone(),
        })
    }

    /// 补全选中项下移。
    pub(crate) fn _completion_down(&mut self, cx: &mut Context<Self>) {
        self.move_completion_selection(1, cx);
    }

    /// 补全选中项上移。
    pub(crate) fn _completion_up(&mut self, cx: &mut Context<Self>) {
        self.move_completion_selection(-1, cx);
    }

    /// 接受当前选中项。
    #[allow(dead_code)] // 鼠标点击/键盘确认补全入口，暂由键盘 Enter 直接接受，保留备用。
    pub(crate) fn accept_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = self.completion_items.get(self.completion_selected).cloned() {
            self.accept_completion(item, cx);
        }
    }

}

fn completion_index_after_step(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    ((current as isize + delta).rem_euclid(len as isize)) as usize
}

const COMPLETION_POPUP_VISIBLE_ROWS: usize = 10;
const COMPLETION_POPUP_CONTENT_PADDING: f32 = 118.;
/// F005：右侧 metadata 详情面板固定宽度（像素）。
const COMPLETION_DOC_WIDTH: f32 = 300.;
/// F005：候选列表与右侧详情面板之间的竖线分隔条宽度。
const COMPLETION_DOC_DIVIDER: f32 = 1.;

fn completion_visible_row_count(len: usize) -> usize {
    len.min(COMPLETION_POPUP_VISIBLE_ROWS)
}

fn completion_popup_width(longest_label_width: f32) -> f32 {
    (longest_label_width + COMPLETION_POPUP_CONTENT_PADDING)
        .clamp(COMPLETION_POPUP_MIN_WIDTH, COMPLETION_POPUP_MAX_WIDTH)
}

/// 按全量候选的最长标签定型补全浮层宽度（像素，已钳制到 [MIN, MAX]）。
///
/// 打开时调用一次并缓存于 `completion_width`；避免滚动到不同标签时逐帧重算宽度
/// 导致抖动，也避免 render 每帧对所有 label 做字形整形。
pub(crate) fn completion_popup_width_for_items(
    items: &[CompletionItem],
    font: &gpui::Font,
    font_size: gpui::Pixels,
    window: &Window,
) -> f32 {
    let longest = items
        .iter()
        .map(|item| {
            let run = TextRun {
                len: item.label.len(),
                font: font.clone(),
                color: gpui::Hsla::default(), // 仅用于定宽，形状结果与颜色无关。
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                window
                    .text_system()
                    .shape_line(SharedString::from(item.label.clone()), font_size, &[run], None)
                    .width,
            )
        })
        .fold(0., f32::max);
    completion_popup_width(longest)
}

#[cfg(test)]
mod completion_tests {
    use super::{completion_index_after_step, completion_popup_width};

    #[test]
    fn completion_index_after_step_wraps_both_directions() {
        assert_eq!(completion_index_after_step(0, 3, -1), 2);
        assert_eq!(completion_index_after_step(2, 3, 1), 0);
        assert_eq!(completion_index_after_step(1, 3, 1), 2);
    }

    #[test]
    fn completion_popup_width_clamps_like_zed() {
        assert_eq!(completion_popup_width(0.), 280.);
        assert_eq!(completion_popup_width(100.), 280.);
        assert_eq!(completion_popup_width(1000.), 540.);
    }
}
