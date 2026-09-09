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
            *col > 0 && *row < self.rows.len() && *col <= self.rows.get(*row).map_or(0, Vec::len)
        });
        cells
    }

    fn effective_selected_rows(&self) -> BTreeSet<usize> {
        let mut rows = self.selected_rows.clone();
        if let Some(row) = self.selected_row {
            rows.insert(row);
        }
        rows.retain(|row| *row < self.rows.len());
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

    fn selection_copy_label(&self) -> Option<String> {
        let rows = self.selected_row_count();
        if rows > 1 {
            return Some(format!("复制选中 {rows} 行 (TSV)"));
        }

        let cells = self.selected_cell_count();
        (cells > 1).then(|| format!("复制选中 {cells} 个单元格 (TSV)"))
    }

    fn selection_export_label(&self) -> Option<String> {
        let rows = self.selected_row_count();
        if rows > 0 {
            return Some(if rows == 1 {
                "导出当前行 (TSV)".to_string()
            } else {
                format!("导出选中 {rows} 行 (TSV)")
            });
        }

        let cells = self.selected_cell_count();
        if cells > 0 {
            return Some(if cells == 1 {
                "导出当前单元格 (TSV)".to_string()
            } else {
                format!("导出选中 {cells} 个单元格 (TSV)")
            });
        }

        None
    }

    fn selection_export(&self) -> Option<DataTableSelectionExport> {
        let rows = self.effective_selected_rows();
        if !rows.is_empty() {
            return Some(DataTableSelectionExport {
                kind: DataTableSelectionKind::Rows,
                text: data_table_rows_tsv(self.rows.as_slice(), &rows),
                rows: rows.len(),
                cells: rows.len() * self.rows.first().map_or(0, Vec::len),
            });
        }

        let cells = self.effective_selected_cells();
        if cells.is_empty() {
            return None;
        }

        Some(DataTableSelectionExport {
            kind: DataTableSelectionKind::Cells,
            text: data_table_cells_tsv(self.rows.as_slice(), &cells),
            rows: cells
                .iter()
                .map(|(row, _)| *row)
                .collect::<BTreeSet<_>>()
                .len(),
            cells: cells.len(),
        })
    }
}

fn data_table_rows_tsv(rows: &[Vec<SharedString>], selected_rows: &BTreeSet<usize>) -> String {
    rows.iter()
        .enumerate()
        .filter(|(row_ix, _)| selected_rows.contains(row_ix))
        .map(|(_, row)| {
            row.iter()
                .map(|cell| data_table_tsv_cell(cell.as_ref()))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn data_table_cells_tsv(
    rows: &[Vec<SharedString>],
    selected_cells: &BTreeSet<(usize, usize)>,
) -> String {
    let selected_rows = rows
        .iter()
        .enumerate()
        .filter_map(|(row_ix, _)| {
            selected_cells
                .iter()
                .any(|(row, _)| *row == row_ix)
                .then_some(row_ix)
        })
        .collect::<Vec<_>>();
    let selected_cols = (1..=rows.first().map_or(0, Vec::len))
        .filter(|col_ix| selected_cells.iter().any(|(_, col)| col == col_ix))
        .collect::<Vec<_>>();

    selected_rows
        .iter()
        .map(|row_ix| {
            selected_cols
                .iter()
                .map(|col_ix| {
                    if selected_cells.contains(&(*row_ix, *col_ix)) {
                        rows.get(*row_ix)
                            .and_then(|row| row.get(col_ix.saturating_sub(1)))
                            .map(|cell| data_table_tsv_cell(cell.as_ref()))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
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
