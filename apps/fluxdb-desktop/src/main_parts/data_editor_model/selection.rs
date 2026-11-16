impl DataPageTableDelegate {
    fn selected_cell_count(&self) -> usize {
        self.effective_selected_cells().len()
    }

    fn selected_row_count(&self) -> usize {
        self.effective_selected_rows().len()
    }

    fn effective_selected_cells(&self) -> BTreeSet<(usize, usize)> {
        let mut cells = self.selected_cells.clone();
        if let Some(cell) = self.selected_cell {
            cells.insert(cell);
        }
        cells.retain(|(row, col)| {
            *col > 0
                && *row < self.page.rows.len()
                && self.source_column_indexes.get(*col).is_some_and(Option::is_some)
        });
        cells
    }

    fn effective_selected_rows(&self) -> BTreeSet<usize> {
        let mut rows = self.selected_rows.clone();
        if let Some(row) = self.selected_row {
            rows.insert(row);
        }
        rows.retain(|row| *row < self.page.rows.len());
        rows
    }

    fn has_cell_selected(&self, row_ix: usize, col_ix: usize) -> bool {
        self.selected_cell == Some((row_ix, col_ix))
            || self.selected_cells.contains(&(row_ix, col_ix))
    }

    fn has_row_selected(&self, row_ix: usize) -> bool {
        self.selected_row == Some(row_ix) || self.selected_rows.contains(&row_ix)
    }

    fn row_has_selected_cell(&self, row_ix: usize) -> bool {
        self.selected_cell.is_some_and(|(row, _)| row == row_ix)
            || self.selected_cells.iter().any(|(row, _)| *row == row_ix)
    }

    fn select_cell_for_click(&mut self, row_ix: usize, col_ix: usize, additive: bool) {
        self.selected_row = None;
        self.selected_rows.clear();
        if additive {
            if !self.selected_cells.remove(&(row_ix, col_ix)) {
                self.selected_cells.insert((row_ix, col_ix));
                self.selected_cell = Some((row_ix, col_ix));
                self.selection_anchor = Some(DataTableSelectionAnchor::Cell {
                    row: row_ix,
                    col: col_ix,
                });
            } else if self.selected_cell == Some((row_ix, col_ix)) {
                self.selected_cell = self.selected_cells.iter().next_back().copied();
                self.selection_anchor =
                    self.selected_cell
                        .map(|(row, col)| DataTableSelectionAnchor::Cell { row, col });
            }
            if self.selected_cells.is_empty() {
                self.selected_cell = None;
                self.selection_anchor = None;
            }
            return;
        }

        self.selected_cells.clear();
        self.selected_cells.insert((row_ix, col_ix));
        self.selected_cell = Some((row_ix, col_ix));
        self.selection_anchor = Some(DataTableSelectionAnchor::Cell {
            row: row_ix,
            col: col_ix,
        });
    }

    fn select_row_for_click(&mut self, row_ix: usize, additive: bool) {
        self.selected_cell = None;
        self.selected_cells.clear();
        if additive {
            if !self.selected_rows.remove(&row_ix) {
                self.selected_rows.insert(row_ix);
                self.selected_row = Some(row_ix);
                self.selection_anchor = Some(DataTableSelectionAnchor::Row { row: row_ix });
            } else if self.selected_row == Some(row_ix) {
                self.selected_row = self.selected_rows.iter().next_back().copied();
                self.selection_anchor = self
                    .selected_row
                    .map(|row| DataTableSelectionAnchor::Row { row });
            }
            if self.selected_rows.is_empty() {
                self.selected_row = None;
                self.selection_anchor = None;
            }
            return;
        }

        self.selected_rows.clear();
        self.selected_rows.insert(row_ix);
        self.selected_row = Some(row_ix);
        self.selection_anchor = Some(DataTableSelectionAnchor::Row { row: row_ix });
    }

    fn select_cell_range_for_click(&mut self, row_ix: usize, col_ix: usize) {
        let (anchor_row, anchor_col) = match self.selection_anchor {
            Some(DataTableSelectionAnchor::Cell { row, col }) => (row, col),
            Some(DataTableSelectionAnchor::Row { row }) => (row, col_ix),
            None => self.selected_cell.unwrap_or((row_ix, col_ix)),
        };

        self.selected_row = None;
        self.selected_rows.clear();
        self.selected_cells = data_table_cell_range(anchor_row, anchor_col, row_ix, col_ix);
        self.selected_cell = Some((row_ix, col_ix));
        self.selection_anchor = Some(DataTableSelectionAnchor::Cell {
            row: anchor_row,
            col: anchor_col,
        });
    }

    fn select_row_range_for_click(&mut self, row_ix: usize) {
        let anchor_row = match self.selection_anchor {
            Some(DataTableSelectionAnchor::Row { row }) => row,
            Some(DataTableSelectionAnchor::Cell { row, .. }) => row,
            None => self
                .selected_row
                .or_else(|| self.selected_cell.map(|(row, _)| row))
                .unwrap_or(row_ix),
        };

        self.selected_cell = None;
        self.selected_cells.clear();
        self.selected_rows = data_table_index_range(anchor_row, row_ix);
        self.selected_row = Some(row_ix);
        self.selection_anchor = Some(DataTableSelectionAnchor::Row { row: anchor_row });
    }

    /// 选中当前页所有可见单元格（⌘A / Ctrl+A）。
    ///
    /// 只覆盖当前分页；跨页全量请用导出。空页或没有可见数据列时清空选区。
    fn select_all_cells(&mut self) {
        let last_row = self.page.rows.len().saturating_sub(1);
        let last_col = self
            .source_column_indexes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(col, index)| (col > 0 && index.is_some()).then_some(col));

        self.selected_row = None;
        self.selected_rows.clear();
        let (Some(last_col), false) = (last_col, self.page.rows.is_empty()) else {
            self.selected_cell = None;
            self.selected_cells.clear();
            self.selection_anchor = None;
            return;
        };

        self.selected_cells = data_table_cell_range(0, 1, last_row, last_col);
        self.selected_cell = Some((last_row, last_col));
        self.selection_anchor = Some(DataTableSelectionAnchor::Cell { row: 0, col: 1 });
    }

    fn selection_copy_label(&self) -> Option<String> {
        let rows = self.selected_row_count();
        if rows > 1 {
            return Some(tr("app.data.selection.copy_rows").replace("%{rows}", &rows.to_string()));
        }

        let cells = self.selected_cell_count();
        (cells > 1).then(|| tr("app.data.selection.copy_cells").replace("%{cells}", &cells.to_string()))
    }

    fn selection_export_label(&self) -> Option<String> {
        let rows = self.selected_row_count();
        if rows > 0 {
            return Some(if rows == 1 {
                tr("app.data.selection.export_current_row").to_string()
            } else {
                tr("app.data.selection.export_rows").replace("%{rows}", &rows.to_string())
            });
        }

        let cells = self.selected_cell_count();
        if cells > 0 {
            return Some(if cells == 1 {
                tr("app.data.selection.export_current_cell").to_string()
            } else {
                tr("app.data.selection.export_cells").replace("%{cells}", &cells.to_string())
            });
        }

        None
    }

    /// 选区里是否有只加载了预览的单元格。
    ///
    /// 复制/导出会把值当权威数据带出去，片段混进去就是静默丢数据，必须由调用方拒绝。
    fn selection_contains_truncated(&self) -> bool {
        let cells = self.effective_selected_cells();
        let rows = self.effective_selected_rows();
        // 整行选中检查该行全部可见列；单元格选中只检查被选中的格子。
        // 全选会产生 rows×cols 个单元格，这里按 O(选中数) 遍历，避免逐行扫描整个选区。
        let row_has_truncated = |row_ix: usize| {
            self.page.rows.get(row_ix).is_some_and(|row| {
                self.source_column_indexes
                    .iter()
                    .skip(1)
                    .filter_map(|index| index.and_then(|source| row.values.get(source)))
                    .any(fluxdb_core::CellValue::is_truncated)
            })
        };
        rows.iter().any(|row_ix| row_has_truncated(*row_ix))
            || cells.iter().any(|(row_ix, col_ix)| {
                self.page.rows.get(*row_ix).is_some_and(|row| {
                    self.source_column_indexes
                        .get(*col_ix)
                        .copied()
                        .flatten()
                        .and_then(|source| row.values.get(source))
                        .is_some_and(fluxdb_core::CellValue::is_truncated)
                })
            })
    }

    fn selection_export(&self) -> Option<DataTableSelectionExport> {
        let rows = self.effective_selected_rows();
        if !rows.is_empty() {
            return Some(DataTableSelectionExport {
                kind: DataTableSelectionKind::Rows,
                text: data_table_rows_tsv(&self.page, &self.source_column_indexes, &rows),
                rows: rows.len(),
                cells: rows.len()
                    * self
                        .source_column_indexes
                        .iter()
                        .filter(|index| index.is_some())
                        .count(),
            });
        }

        let cells = self.effective_selected_cells();
        if cells.is_empty() {
            return None;
        }

        Some(DataTableSelectionExport {
            kind: DataTableSelectionKind::Cells,
            text: data_table_cells_tsv(&self.page, &self.source_column_indexes, &cells),
            rows: cells
                .iter()
                .map(|(row, _)| *row)
                .collect::<BTreeSet<_>>()
                .len(),
            cells: cells.len(),
        })
    }
}

fn data_table_rows_tsv(
    page: &DataPage,
    source_column_indexes: &[Option<usize>],
    selected_rows: &BTreeSet<usize>,
) -> String {
    selected_rows
        .iter()
        .filter_map(|row_ix| page.rows.get(*row_ix))
        .map(|row| {
            source_column_indexes
                .iter()
                .filter_map(|index| index.and_then(|source| row.values.get(source)))
                .map(|value| data_table_tsv_cell(data_cell_display_text(value).as_str()))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn data_table_cells_tsv(
    page: &DataPage,
    source_column_indexes: &[Option<usize>],
    selected_cells: &BTreeSet<(usize, usize)>,
) -> String {
    let selected_rows = selected_cells
        .iter()
        .map(|(row, _)| *row)
        .collect::<BTreeSet<_>>();
    let selected_cols = selected_cells
        .iter()
        .map(|(_, col)| *col)
        .collect::<BTreeSet<_>>();

    selected_rows
        .iter()
        .map(|row_ix| {
            selected_cols
                .iter()
                .map(|col_ix| {
                    if !selected_cells.contains(&(*row_ix, *col_ix)) {
                        return String::new();
                    }
                    source_column_indexes
                        .get(*col_ix)
                        .copied()
                        .flatten()
                        .and_then(|source| page.rows.get(*row_ix)?.values.get(source))
                        .map(|value| data_table_tsv_cell(data_cell_display_text(value).as_str()))
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn data_table_tsv_cell(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\t' | '\n' | '\r' => ' ',
            _ => ch,
        })
        .collect()
}

fn data_table_index_range(start: usize, end: usize) -> BTreeSet<usize> {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    (start..=end).collect()
}

fn data_table_cell_range(
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
) -> BTreeSet<(usize, usize)> {
    let rows = data_table_index_range(start_row, end_row);
    let cols = data_table_index_range(start_col, end_col);
    rows.iter()
        .flat_map(|row| cols.iter().map(move |col| (*row, *col)))
        .collect()
}
