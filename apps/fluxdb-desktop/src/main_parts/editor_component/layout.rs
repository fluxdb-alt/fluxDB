// editor_component/layout.rs —— 布局快照：把 buffer/display map 切成可视行渲染数据结构。
//
// 该模块只做“纯布局计算”，不触碰数据库；把 buffer 行、折叠、软换行、选区映射为
// 像素级的可视行与命中区域，供 render.rs 消费。

/// 同一 buffer 行上的 CodeLens，渲染层把多个动作以 ` | ` 合并显示。
#[derive(Clone, Debug)]
pub(crate) struct CodeLensLine {
    pub row: usize,
    pub indent_column: usize,
    pub lenses: Vec<CodeLens>,
}

/// 一张待渲染的可视行。
#[derive(Clone, Debug)]
pub(crate) struct VisualLineRender {
    /// 该片段在整个文档中的 visual row，用于折叠/软换行后的 y 坐标。
    pub visual_row: usize,
    /// 该可视行对应的 buffer 行。
    pub buffer_row: usize,
    /// 是否 buffer 行首片段（用于折叠箭头 / 行号）。
    pub first_fragment: bool,
    /// 该可视行在 buffer 行内的字节区间。
    pub byte_range: CoreRange,
    /// 当前光标是否在这一行。
    pub current_line: bool,
}

/// 布局快照终点。
#[derive(Clone, Debug)]
pub(crate) struct LayoutSnapshot {
    pub line_height: Pixels,
    pub font_size: f32,
    pub gutter_width: Pixels,
    pub visible_lines: Vec<VisualLineRender>,
    pub cursor_local: Option<Bounds<Pixels>>, // viewport 内局部坐标
    #[allow(dead_code)] // 全局坐标仅滚动对齐时使用，暂未接入，保留。
    pub cursor_global: Option<Bounds<Pixels>>, // 全局坐标
    pub line_hit_regions: Vec<LineHitRegion>,
    pub selected_range: Option<CoreRange>,
    /// 行背景装饰：(buffer_row, 状态键)。由 DecorationProvider 产出，paint 按状态键着色。
    pub line_backgrounds: Vec<(usize, String)>,
    pub code_lenses: Vec<CodeLensLine>,
    /// 每个可视行的像素 y；背景装饰复用，避免再次实现 block 几何。
    pub line_y: Vec<Pixels>,
}

/// 根据编辑器当前状态构建布局快照（砌墙：request_layout 阶段计算成本高，放 prepaint）。
impl Editor {
    pub(crate) fn build_layout(&self, window: &Window) -> LayoutSnapshot {
        let started = Instant::now();
        let viewport = self.scroll_handle.bounds();
        let scroll = self.scroll_handle.offset();
        let line_height = self.line_height(window);
        let gutter_width = self.line_number_width(window);

        let code_lens_visual_rows = self.code_lens_visual_rows();
        let first_visual = self.first_visible_visual_row(&viewport, scroll, line_height);
        // 视口内可视行范围。
        let base_visible_count = (viewport.size.height / line_height).ceil() as usize + 1;
        // 只为视口附近的 CodeLens 预留额外行；不能把全文 CodeLens 数量加进来，
        // 否则 1.6 万行 SQL（数百条语句）会把每帧绘制范围扩大到数百行。
        let lens_start = code_lens_visual_rows.partition_point(|row| *row < first_visual);
        let lens_end = code_lens_visual_rows.partition_point(|row| {
            *row < first_visual.saturating_add(base_visible_count)
        });
        let visible_count = base_visible_count
            + lens_end.saturating_sub(lens_start)
            + 1;
        let start = first_visual;
        let end = (first_visual + visible_count).min(self.display.visual_row_count());

        let mut visible_lines = Vec::new();
        let mut line_hit_regions = Vec::new();
        let cursor_point = self.buffer.offset_to_point(self.cursor_offset());
        // 折叠候选（可折叠起始行）与已折叠起始行，均为升序，供每行二分命中。
        let (fold_candidates, folded_rows): (Vec<usize>, Vec<usize>) =
            if self.profile.show_folding {
                let candidates = self
                    .fold_candidates()
                    .into_iter()
                    .map(|(_, f)| f.start_row)
                    .collect();
                let folded = self
                    .active_folds()
                    .into_iter()
                    .map(|f| f.start_row)
                    .collect();
                (candidates, folded)
            } else {
                (Vec::new(), Vec::new())
            };
        let selected_range = if self.selection.range().is_empty() {
            None
        } else {
            Some(self.selection.range())
        };
        let _selected = self.selection.range();

        // 行背景装饰：向 DecorationProvider 索取可视区内的装饰（按 buffer 行）。
        // 仅单行无软换行时 buffer 行 == 可视行，故此处直接用 buffer_row 键控。
        let line_backgrounds = self.collect_line_backgrounds(start, end);
        let code_lenses = self.collect_code_lenses(first_visual, end);

        for visual_row in start..end {
            let Some(visual) = self.display.visual_line_at(visual_row) else {
                continue;
            };
            let buffer_row = visual.buffer_row;
            let line_start = self.buffer.line_start(buffer_row);
            let line_end = self.buffer.line_end_offset(buffer_row);
            // 按 VisualLine.column_start/column_end 只取本可视行（wrap 片段）的字节区间，
            // 整行单片段时即整个内容行。
            let frag_start = (line_start + visual.column_start).min(line_end);
            let frag_end = (line_start + visual.column_end).min(line_end);
            let byte_range = CoreRange::new(frag_start, frag_end);
            // 注意：本循环不取 `byte_range` 的文本。文本只在 paint 阶段绕过 shaped
            // 缓存（首次进视口 / 内容变更）时按需提取，滚动帧对已缓存行不做任何字符串
            // 分配。`byte_range` 已携带字节区间，供形状与命中换算使用。
            // 折叠箭头命中（仅首片段）。展开状态下只在当前行或悬停 gutter 时显示，
            // 已折叠行始终保留入口，行为与 Zed 的 gutter crease 一致。
            let is_foldable = fold_candidates.binary_search(&buffer_row).is_ok();
            // 已折叠状态源自稳定折叠集（DM-106），按当前快照解析的起始行判断。
            let is_folded = folded_rows.binary_search(&buffer_row).is_ok();
            let show_fold_toggle = is_folded
                || (is_foldable && (buffer_row == cursor_point.row || self.gutter_hovered));
            if visual.first_fragment && show_fold_toggle {
                let y = viewport.top()
                    + scroll.y
                    + self.y_for_visual_row(visual_row, line_height);
                // 折叠命中区与固定 gutter 对齐，不随横向滚动。
                let fold_bounds = Bounds::new(
                    GPoint::new(
                        viewport.left()
                            + px(EDITOR_PADDING_X)
                            + gutter_width
                            - px(EDITOR_FOLD_GUTTER),
                        y,
                    ),
                    Size::new(px(EDITOR_FOLD_GUTTER), line_height),
                );
                line_hit_regions.push(LineHitRegion {
                    row: buffer_row,
                    bounds: fold_bounds,
                    kind: LineHitKind::Fold,
                });
            }

            visible_lines.push(VisualLineRender {
                visual_row,
                buffer_row,
                first_fragment: visual.first_fragment,
                byte_range,
                current_line: buffer_row == cursor_point.row,
            });
        }
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "visible_rows_query",
            editor_id = self.perf_editor_id,
            edit_id = self.perf_edit_id,
            buffer_version = self.buffer.version(),
            request_id = 0u64,
            task_id = 0u64,
            layer = "display",
            elapsed_us = started.elapsed().as_micros() as u64,
            first_visual,
            requested_rows = end.saturating_sub(start),
            returned_rows = visible_lines.len(),
        );
        let line_y = visible_lines
            .iter()
            .map(|line| self.y_for_visual_row(line.visual_row, line_height))
            .collect();

        // 光标局部坐标。
        let cursor_local = self.cursor_local_bounds(window, &first_visible_visual_row_helper(first_visual, line_height));

        let layout = LayoutSnapshot {
            line_height,
            font_size: self.font_size,
            gutter_width,
            visible_lines,
            cursor_local,
            cursor_global: None,
            line_hit_regions,
            selected_range,
            line_backgrounds,
            code_lenses,
            line_y,
        };
        let elapsed_us = started.elapsed().as_micros() as u64;
        tracing::debug!(
            target: "gdb_editor_perf",
            op = "visible_layout",
            editor_id = self.perf_editor_id,
            edit_id = self.perf_edit_id,
            elapsed_us,
            viewport_height_px = f32::from(viewport.size.height),
            line_height_px = f32::from(line_height),
            text_bytes = self.buffer.len(),
            line_count = self.buffer.line_count(),
            visible_rows = layout.visible_lines.len(),
            code_lens = layout.code_lenses.len(),
            decorations = layout.line_backgrounds.len(),
            visual_rows = self.display.visual_row_count(),
            buffer_version = self.buffer.version(),
        );
        if elapsed_us > fluxdb_editor_core::FRAME_BUDGET_US {
            tracing::warn!(
                target: "gdb_editor_perf",
                op = "visible_layout_slow",
                editor_id = self.perf_editor_id,
                edit_id = self.perf_edit_id,
                elapsed_us,
                text_bytes = self.buffer.len(),
                visible_rows = layout.visible_lines.len(),
                buffer_version = self.buffer.version(),
            );
        }
        layout
    }

    /// 返回 CodeLens 所在的 visual row，用于把 block 高度纳入滚动和命中坐标。
    pub(crate) fn code_lens_visual_rows(&self) -> Vec<usize> {
        let Some(provider) = self.providers.code_lens.clone() else {
            return Vec::new();
        };
        let version = self.buffer.version();
        let visual_rows = self.display.visual_row_count();
        if let Some((cached_version, cached_visual_rows, rows)) =
            self.code_lens_visual_rows_cache.borrow().as_ref()
            && *cached_version == version
            && *cached_visual_rows == visual_rows
        {
            return rows.clone();
        }
        let snapshot = self.buffer.snapshot();
        let mut rows = self
            .all_code_lenses(&provider, &snapshot)
            .iter()
            .map(|lens| {
                let point = snapshot.offset_to_point(lens.range.start.min(snapshot.len()));
                self.display.visual_row_for_column(point.row, point.column) as usize
            })
            .collect::<Vec<_>>();
        rows.sort_unstable();
        rows.dedup();
        *self.code_lens_visual_rows_cache.borrow_mut() =
            Some((version, visual_rows, rows.clone()));
        rows
    }

    fn all_code_lenses(
        &self,
        provider: &Arc<dyn CodeLensProvider>,
        snapshot: &BufferSnapshot,
    ) -> Arc<Vec<CodeLens>> {
        let key = (snapshot.version(), snapshot.len(), snapshot.line_count());
        if let Some((version, bytes, lines, lenses)) = self.code_lens_cache.borrow().as_ref()
            && (*version, *bytes, *lines) == key
        {
            return lenses.clone();
        }
        let lenses = Arc::new(provider.code_lenses(snapshot, CoreRange::new(0, snapshot.len())));
        *self.code_lens_cache.borrow_mut() = Some((key.0, key.1, key.2, lenses.clone()));
        lenses
    }

    /// wrap 行 `r` 的内容所在 Block 显示行 = r + Σ(位于 r 及其 before 前缀的块高)。
    /// 经 DisplayMap 的 Block 摘要（`block_content_row`）取前缀，作为「该行上方块高
    /// 行数」的唯一来源（DM-312 消除 4 处重复的 `partition_point(lens ≤ r)` 单调计数）。
    fn block_lens_prefix(&self, wrap_row: usize) -> usize {
        self.display.block_extra_rows_before(wrap_row)
    }

    pub(crate) fn y_for_visual_row(&self, wrap_row: usize, line_height: Pixels) -> Pixels {
        let stride = f32::from(line_height) + EDITOR_LINE_GAP;
        let extras = self.block_lens_prefix(wrap_row) as f32 * CODE_LENS_HEIGHT;
        px(EDITOR_PADDING_Y + extras)
            + px(wrap_row as f32 * stride)
    }

    fn collect_code_lenses(&self, first_visual: usize, end_visual: usize) -> Vec<CodeLensLine> {
        let Some(provider) = self.providers.code_lens.clone() else {
            return Vec::new();
        };
        let snapshot = self.buffer.snapshot();
        let first_row = self
            .display
            .visual_line_at(first_visual)
            .map(|line| line.buffer_row)
            .unwrap_or(0);
        let last_row = if end_visual > first_visual {
            self.display
                .visual_line_at(end_visual.saturating_sub(1))
                .map(|line| line.buffer_row)
                .unwrap_or(first_row)
        } else {
            first_row
        };
        let visible = CoreRange::new(
            self.buffer.line_start(first_row),
            self.buffer.line_end_offset(last_row),
        );
        let mut grouped: std::collections::BTreeMap<usize, Vec<CodeLens>> =
            std::collections::BTreeMap::new();
        // all_code_lenses 返回按 range.start 升序的全文 lens（缓存命中，滚动不重算）。
        // 按字节区间二分裁剪到可见窗口两侧：start < visible.start 或 > visible.end 的
        // 语句起始行必在首次可视行之前/末次可视行之后（原本就 continue），二分可安全
        // 跳过，把每帧 O(全文 lens) 的过滤收敛为 O(log n + 相交数)。
        let lenses = self.all_code_lenses(&provider, &snapshot);
        let bounds = lenses.partition_point(|l| l.range.start <= visible.end);
        let start_bounds = lenses.partition_point(|l| l.range.start < visible.start);
        for lens in lenses[start_bounds..bounds].iter()
        {
            // 覆盖进可见区之前的长语句（起始行在 first_row 之前）仍需显式跳过。
            if lens.range.end < visible.start {
                continue;
            }
            let row = snapshot.offset_to_point(lens.range.start.min(snapshot.len())).row;
            if row < first_row || row > last_row {
                continue;
            }
            grouped.entry(row).or_default().push(lens.clone());
        }
        grouped
            .into_iter()
            .map(|(row, lenses)| CodeLensLine {
                row,
                indent_column: snapshot
                    .line_text(row)
                    .chars()
                    .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                    .count(),
                lenses,
            })
            .collect()
    }

    /// 向 DecorationProvider 索取视口内的行背景装饰，返回 (buffer_row, 状态键) 列表。
    fn collect_line_backgrounds(&self, first_visual: usize, end_visual: usize) -> Vec<(usize, String)> {
        let Some(provider) = self.providers.decorations.clone() else {
            return Vec::new();
        };
        let snapshot = self.buffer.snapshot();
        // 视口的字节区间：以首/尾可视行的 buffer 行为界（单行无 wrap 时 = 可视行）。
        let first_row = self
            .display
            .visual_line_at(first_visual)
            .map(|v| v.buffer_row)
            .unwrap_or(0);
        let last_row = if end_visual > first_visual {
            self.display
                .visual_line_at(end_visual - 1)
                .map(|v| v.buffer_row)
                .unwrap_or(first_row)
        } else {
            first_row
        };
        let visible_range = CoreRange::new(
            self.buffer.line_start(first_row),
            self.buffer.line_end_offset(last_row),
        );
        let set = provider.decorations(&snapshot, visible_range);
        set.decorations
            .into_iter()
            .filter_map(|deco| match deco {
                fluxdb_editor_core::Decoration::LineBackground(row, key) => Some((row, key)),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn first_visible_visual_row(
        &self,
        _viewport: &Bounds<Pixels>,
        scroll: gpui::Point<Pixels>,
        line_height: Pixels,
    ) -> usize {
        if f32::from(line_height) <= 0.0 {
            return 0;
        }
        // ScrollHandle 的 offset 向下滚动时为负值；按内容坐标反推首个可见 visual row。
        let content_top = (-f32::from(scroll.y) - EDITOR_PADDING_Y).max(0.0);
        let stride = f32::from(line_height) + EDITOR_LINE_GAP;
        let row_count = self.display.visual_row_count();
        if row_count == 0 {
            return 0;
        }

        // 块前缀（该行上方块高行数）取自 DisplayMap 的 Block 摘要（DM-312），
        // 与 hit/滚动共享同一数据源。
        first_visible_visual_row_for_top(row_count, content_top, stride, |r| {
            self.block_lens_prefix(r)
        })
    }
}

/// 首个可见 wrap 行。行顶部坐标单调（文本行按 stride、块行按 CODE_LENS_HEIGHT），
/// 二分反推，O(log n)；块前缀由 `lens_prefix`（来自 Block 摘要）提供。
fn first_visible_visual_row_for_top(
    row_count: usize,
    content_top: f32,
    stride: f32,
    lens_prefix: impl Fn(usize) -> usize,
) -> usize {
    if row_count == 0 || stride <= 0.0 {
        return 0;
    }
    let mut low = 0usize;
    let mut high = row_count;
    while low < high {
        let row = low + (high - low) / 2;
        let top = row as f32 * stride + lens_prefix(row) as f32 * CODE_LENS_HEIGHT;
        if top <= content_top {
            low = row + 1;
        } else {
            high = row;
        }
    }
    low.saturating_sub(1).min(row_count.saturating_sub(1))
}

/// 根据内容坐标定位可视行。行顶部单调累计，二分反推（O(log n)），块前缀取自
/// Block 摘要（DM-312）。
fn visual_row_for_content_y(
    row_count: usize,
    content_y: f32,
    stride: f32,
    lens_prefix: impl Fn(usize) -> usize,
) -> usize {
    if row_count == 0 || stride <= 0.0 {
        return 0;
    }
    let mut low = 0usize;
    let mut high = row_count;
    while low < high {
        let row = low + (high - low) / 2;
        let bottom = (row + 1) as f32 * stride + lens_prefix(row) as f32 * CODE_LENS_HEIGHT;
        if bottom <= content_y {
            low = row + 1;
        } else {
            high = row;
        }
    }
    low.min(row_count.saturating_sub(1))
}

impl Editor {
    fn cursor_local_bounds(
        &self,
        window: &Window,
        _first_visual: &usize,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.cursor_offset();
        let point = self.buffer.offset_to_point(cursor);
        let visual_row = self
            .display
            .visual_row_for_column(point.row, point.column) as usize;
        let line_height = self.line_height(window);
        let visual = self.display.visual_line_at(visual_row)?;
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
        let local_offset =
            cursor.saturating_sub(fragment_start).min(fragment_end - fragment_start);
        let x_offset = shaped.x_for_index(local_offset);
        let gutter = self.line_number_width(window);
        let viewport = self.scroll_handle.bounds();
        let scroll = self.scroll_handle.offset();
        let x = viewport.left()
            + scroll.x
            + px(EDITOR_PADDING_X)
            + gutter
            + px(EDITOR_CONTENT_GAP)
            + x_offset;
        let y = viewport.top()
            + scroll.y
            + self.y_for_visual_row(visual_row, line_height);
        Some(Bounds::from_corners(
            GPoint::new(x, y),
            GPoint::new(x + px(1.), y + line_height),
        ))
    }
}

/// 辅助：转为具体值。
fn first_visible_visual_row_helper(first_visual: usize, _line_height: Pixels) -> usize {
    first_visual
}

#[cfg(test)]
mod tests {
    use super::{first_visible_visual_row_for_top, visual_row_for_content_y};

    /// 由 lens 所在 wrap 行构造「该行上方块高行数」闭包（模拟 Block 摘要前缀）。
    fn lens_prefix(rows: &[usize]) -> impl Fn(usize) -> usize + '_ {
        move |r| rows.partition_point(|lens_row| *lens_row <= r)
    }

    #[test]
    fn first_visible_row_uses_binary_search_with_code_lens_height() {
        let none: &[usize] = &[];
        assert_eq!(first_visible_visual_row_for_top(100, 0.0, 10.0, lens_prefix(none)), 0);
        assert_eq!(first_visible_visual_row_for_top(100, 19.9, 10.0, lens_prefix(none)), 1);
        assert_eq!(first_visible_visual_row_for_top(100, 20.0, 10.0, lens_prefix(&[2])), 1);
        assert_eq!(first_visible_visual_row_for_top(100, 34.0, 10.0, lens_prefix(&[2])), 2);
    }

    #[test]
    fn lens_rows_are_counted_by_sorted_prefix() {
        let rows = [2, 5, 9, 20];
        let p = lens_prefix(&rows);
        assert_eq!(p(1), 0);
        assert_eq!(p(9), 3);
        assert_eq!(p(100), 4);
    }

    #[test]
    fn content_y_uses_binary_search_with_lens_rows() {
        let none: &[usize] = &[];
        assert_eq!(visual_row_for_content_y(100, 0.0, 10.0, lens_prefix(none)), 0);
        assert_eq!(visual_row_for_content_y(100, 9.9, 10.0, lens_prefix(none)), 0);
        assert_eq!(visual_row_for_content_y(100, 10.0, 10.0, lens_prefix(none)), 1);
        assert_eq!(visual_row_for_content_y(100, 20.0, 10.0, lens_prefix(&[1])), 1);
        assert_eq!(visual_row_for_content_y(100, 30.0, 10.0, lens_prefix(&[1])), 1);
        assert_eq!(visual_row_for_content_y(100, 40.0, 10.0, lens_prefix(&[1])), 2);
    }
}
