use sas_engine::{
    build_cell_equals_filter, compute_column_stats, compute_visible_rows, load_dataset_streaming,
    parse_numeric_or_date_input, ColumnStats, ColumnarTable, EncodingChoice, FilterOp, LoadEvent,
    VarType, ViewQuery,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Variables,
    Statistics,
    Shortcuts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDialogOpKind {
    StrEq,
    StrNeq,
    StrContains,
    StrNotContains,
    NumEq,
    NumNeq,
    NumGt,
    NumGte,
    NumLt,
    NumLte,
    NumBetween,
}

impl FilterDialogOpKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::StrEq | Self::NumEq => "等于 (=)",
            Self::StrNeq | Self::NumNeq => "不等于 (!=)",
            Self::StrContains => "包含 (Contains)",
            Self::StrNotContains => "不包含 (Not Contains)",
            Self::NumGt => "大于 (>)",
            Self::NumGte => "大于等于 (>=)",
            Self::NumLt => "小于 (<)",
            Self::NumLte => "小于等于 (<=)",
            Self::NumBetween => "区间 (Between)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterDialogState {
    pub col_idx: usize,
    pub op_kind: FilterDialogOpKind,
    pub input_primary: String,
    pub input_secondary: String,
    pub replace_same_col: bool,
    pub error_msg: Option<String>,
    pub just_opened: bool,
}

pub struct DatasetTabState {
    pub id: u64,
    pub path: PathBuf,
    pub display_name: String,
    pub encoding: EncodingChoice,
    pub table: Option<Arc<ColumnarTable>>,
    pub loading: bool,
    pub load_progress: f32,
    pub rows_loaded_so_far: usize,
    pub load_error: Option<String>,
    pub cancel_flag: Arc<AtomicBool>,
    pub loader_rx: Option<Receiver<LoadEvent>>,

    // View & Query state
    pub query: ViewQuery,
    pub visible_rows: Vec<u32>,
    pub filter_version: u64,
    pub column_order: Vec<usize>,
    pub column_visible: Vec<bool>,
    pub column_widths: Vec<f32>,

    // Selection & cross-panel focus
    pub selected_cell: Option<(usize, usize)>, // (visible_row_idx, orig_col_idx)
    pub selected_var: Option<usize>,           // orig_col_idx
    pub scroll_to_col: Option<usize>,
    pub scroll_to_row: Option<usize>,

    // Statistics panel state (always computed on current filtered visible_rows)
    pub stats_col: Option<usize>,
    pub stats_cache_key: Option<(u64, usize)>,
    pub cached_stats: Option<ColumnStats>,

    // UI transient state
    pub var_search: String,
    pub filter_dialog: Option<FilterDialogState>,
    pub dragging_col: Option<usize>,
    pub grid_scroll_offset: egui::Vec2,
    pub last_query_ms: u64,
}

impl DatasetTabState {
    pub fn new(
        id: u64,
        path: PathBuf,
        display_name: String,
        encoding: EncodingChoice,
        repaint_ctx: Option<egui::Context>,
    ) -> Self {
        let mut tab = Self {
            id,
            path,
            display_name,
            encoding,
            table: None,
            loading: false,
            load_progress: 0.0,
            rows_loaded_so_far: 0,
            load_error: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            loader_rx: None,
            query: ViewQuery::default(),
            visible_rows: Vec::new(),
            filter_version: 0,
            column_order: Vec::new(),
            column_visible: Vec::new(),
            column_widths: Vec::new(),
            selected_cell: None,
            selected_var: None,
            scroll_to_col: None,
            scroll_to_row: None,
            stats_col: None,
            stats_cache_key: None,
            cached_stats: None,
            var_search: String::new(),
            filter_dialog: None,
            dragging_col: None,
            grid_scroll_offset: egui::Vec2::ZERO,
            last_query_ms: 0,
        };
        tab.start_loading(repaint_ctx);
        tab
    }

    pub fn start_loading(&mut self, repaint_ctx: Option<egui::Context>) {
        // Cancel any previous loader
        self.cancel_flag.store(true, Ordering::Relaxed);
        self.cancel_flag = Arc::new(AtomicBool::new(false));
        self.loading = true;
        self.load_progress = 0.0;
        self.rows_loaded_so_far = 0;
        self.load_error = None;

        let (tx, rx) = mpsc::channel();
        self.loader_rx = Some(rx);

        let path = self.path.clone();
        let encoding = self.encoding;
        let cancel = self.cancel_flag.clone();

        std::thread::spawn(move || {
            let res = load_dataset_streaming(&path, encoding, cancel, |ev| {
                let _ = tx.send(ev);
                if let Some(ctx) = &repaint_ctx {
                    ctx.request_repaint();
                }
            });
            if let Err(err) = res {
                if !matches!(err, sas_engine::LoadError::Cancelled) {
                    let _ = tx.send(LoadEvent::Error(err.to_string()));
                    if let Some(ctx) = &repaint_ctx {
                        ctx.request_repaint();
                    }
                }
            }
        });
    }

    pub fn reload_with_encoding(&mut self, encoding: EncodingChoice, ctx: egui::Context) {
        self.encoding = encoding;
        self.table = None;
        self.visible_rows.clear();
        self.query.clear();
        self.selected_cell = None;
        self.cached_stats = None;
        self.stats_cache_key = None;
        self.start_loading(Some(ctx));
    }

    /// Poll background loader channel; returns true if state changed.
    pub fn poll_loader(&mut self) -> bool {
        let events: Vec<LoadEvent> = {
            let Some(rx) = &self.loader_rx else {
                return false;
            };
            let mut buf = Vec::new();
            while let Ok(ev) = rx.try_recv() {
                buf.push(ev);
            }
            buf
        };
        if events.is_empty() {
            return false;
        }
        let mut finished = false;
        for ev in events {
            match ev {
                LoadEvent::FirstBatch(snap) => {
                    self.rows_loaded_so_far = snap.row_count;
                    self.install_table(snap, false);
                }
                LoadEvent::Progress {
                    rows_loaded,
                    fraction,
                } => {
                    self.rows_loaded_so_far = rows_loaded;
                    self.load_progress = fraction;
                }
                LoadEvent::Complete(full) => {
                    self.rows_loaded_so_far = full.row_count;
                    self.load_progress = 1.0;
                    self.loading = false;
                    self.install_table(full, true);
                    finished = true;
                }
                LoadEvent::Error(msg) => {
                    self.loading = false;
                    self.load_error = Some(msg);
                    finished = true;
                }
            }
        }
        if finished {
            self.loader_rx = None;
        }
        true
    }

    fn install_table(&mut self, table: Arc<ColumnarTable>, is_final: bool) {
        let ncols = table.variables.len();
        let need_init_cols = self.column_order.len() != ncols;
        if need_init_cols {
            self.column_order = (0..ncols).collect();
            self.column_visible = vec![true; ncols];
            self.column_widths = (0..ncols)
                .map(|c| table.estimate_column_width_px(c, 74.0, 230.0, false))
                .collect();
            if ncols > 0 && self.selected_var.is_none() {
                self.selected_var = Some(0);
            }
        } else if is_final {
            // Re-estimate widths smoothly only if user hasn't customized? Keep existing widths to avoid jump!
        }

        self.table = Some(table);
        self.recompute_view();
    }

    pub fn recompute_view(&mut self) {
        let Some(table) = &self.table else {
            self.visible_rows.clear();
            return;
        };
        let t0 = std::time::Instant::now();
        self.visible_rows = compute_visible_rows(table, &self.query);
        self.last_query_ms = t0.elapsed().as_millis() as u64;
        self.filter_version = self.filter_version.wrapping_add(1);

        // Clamp selected cell row into new visible_rows range
        if let Some((row, col)) = self.selected_cell {
            if self.visible_rows.is_empty() {
                self.selected_cell = None;
            } else if row >= self.visible_rows.len() {
                self.selected_cell = Some((self.visible_rows.len() - 1, col));
            }
        }

        // Automatically refresh stats if stats panel is viewing a variable
        if let Some(scol) = self.stats_col {
            self.refresh_stats_for(scol);
        }
    }

    /// Currently visible column indices in display order.
    pub fn visible_columns(&self) -> Vec<usize> {
        self.column_order
            .iter()
            .copied()
            .filter(|&c| self.column_visible.get(c).copied().unwrap_or(false))
            .collect()
    }

    /// Resolve which variable is currently targeted by keyboard shortcuts (`F`, `D`, `Alt+←/→`).
    pub fn active_target_col(&self) -> Option<usize> {
        if let Some((_, col)) = self.selected_cell {
            Some(col)
        } else {
            self.selected_var
        }
    }

    /// Compute/show stats for `col_idx` on currently filtered `visible_rows`.
    pub fn refresh_stats_for(&mut self, col_idx: usize) {
        let Some(table) = &self.table else {
            return;
        };
        if col_idx >= table.variables.len() {
            return;
        }
        self.stats_col = Some(col_idx);
        let key = (self.filter_version, col_idx);
        if self.stats_cache_key == Some(key) && self.cached_stats.is_some() {
            return;
        }
        self.cached_stats = compute_column_stats(table, &self.visible_rows, col_idx);
        self.stats_cache_key = Some(key);
    }

    /// Shortcut `D`: quick `=` filter.
    /// - If a cell is selected: immediately filter `col = cell_value` (AND across different cols, replaces same col).
    /// - If only a variable is selected (from right variable panel): open focused quick `=` dialog.
    pub fn trigger_quick_d_filter(&mut self) -> bool {
        let Some(table) = &self.table else {
            return false;
        };
        if let Some((vis_row, col_idx)) = self.selected_cell {
            if let Some(&orig_row) = self.visible_rows.get(vis_row) {
                if let Some(op) = build_cell_equals_filter(table, orig_row as usize, col_idx) {
                    self.query.set_quick_filter_for_column(col_idx, op);
                    self.recompute_view();
                    return true;
                }
            }
        }
        if let Some(col_idx) = self.selected_var {
            if let Some(meta) = table.variables.get(col_idx) {
                let op_kind = match meta.var_type {
                    VarType::String => FilterDialogOpKind::StrEq,
                    VarType::Numeric => FilterDialogOpKind::NumEq,
                };
                self.filter_dialog = Some(FilterDialogState {
                    col_idx,
                    op_kind,
                    input_primary: String::new(),
                    input_secondary: String::new(),
                    replace_same_col: true,
                    error_msg: None,
                    just_opened: true,
                });
                return true;
            }
        }
        false
    }

    /// Shortcut `C`: clear all filters and sorts.
    pub fn clear_all_filters_and_sorts(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.recompute_view();
        }
    }

    /// Shortcut `H`: uncheck all variables (hide all columns).
    pub fn hide_all_variables(&mut self) {
        for v in &mut self.column_visible {
            *v = false;
        }
    }

    /// Shortcut `U`: check all variables (show all columns).
    pub fn show_all_variables(&mut self) {
        for v in &mut self.column_visible {
            *v = true;
        }
    }

    /// Move target column left (-1) or right (+1) among visible columns (and sync `column_order`).
    pub fn move_column_by(&mut self, col_idx: usize, delta: isize) -> bool {
        let vis = self.visible_columns();
        let Some(vis_pos) = vis.iter().position(|&c| c == col_idx) else {
            return false;
        };
        let target_vis_pos = vis_pos as isize + delta;
        if target_vis_pos < 0 || target_vis_pos >= vis.len() as isize {
            return false;
        }
        let neighbor_col = vis[target_vis_pos as usize];
        let Some(pos_a) = self.column_order.iter().position(|&c| c == col_idx) else {
            return false;
        };
        let Some(pos_b) = self.column_order.iter().position(|&c| c == neighbor_col) else {
            return false;
        };
        self.column_order.swap(pos_a, pos_b);
        self.scroll_to_col = Some(col_idx);
        true
    }

    /// Reorder column `src_col` to the position of `dst_col` in `column_order`.
    pub fn reorder_column(&mut self, src_col: usize, dst_col: usize) {
        if src_col == dst_col {
            return;
        }
        let Some(from_idx) = self.column_order.iter().position(|&c| c == src_col) else {
            return;
        };
        let Some(to_idx) = self.column_order.iter().position(|&c| c == dst_col) else {
            return;
        };
        let item = self.column_order.remove(from_idx);
        let insert_idx = to_idx.min(self.column_order.len());
        self.column_order.insert(insert_idx, item);
    }

    pub fn auto_fit_column(&mut self, col_idx: usize) {
        if let Some(table) = &self.table {
            if col_idx < self.column_widths.len() {
                self.column_widths[col_idx] =
                    table.estimate_column_width_px(col_idx, 60.0, 520.0, true);
            }
        }
    }

    pub fn auto_fit_all_visible_columns(&mut self) {
        if let Some(table) = &self.table {
            for col_idx in self.visible_columns() {
                if col_idx < self.column_widths.len() {
                    self.column_widths[col_idx] =
                        table.estimate_column_width_px(col_idx, 60.0, 480.0, true);
                }
            }
        }
    }

    pub fn reset_all_column_widths(&mut self) {
        if let Some(table) = &self.table {
            for col_idx in 0..self.column_widths.len() {
                self.column_widths[col_idx] =
                    table.estimate_column_width_px(col_idx, 74.0, 230.0, false);
            }
        }
    }

    /// Apply filter from custom dialog.
    pub fn apply_filter_dialog(&mut self) -> bool {
        let Some(dlg) = &self.filter_dialog else {
            return false;
        };
        let Some(table) = &self.table else {
            return false;
        };
        let Some(meta) = table.variables.get(dlg.col_idx) else {
            return false;
        };

        let op_res: Result<FilterOp, String> = match dlg.op_kind {
            FilterDialogOpKind::StrEq => Ok(FilterOp::StrEq(dlg.input_primary.clone())),
            FilterDialogOpKind::StrNeq => Ok(FilterOp::StrNeq(dlg.input_primary.clone())),
            FilterDialogOpKind::StrContains => Ok(FilterOp::StrContains(dlg.input_primary.clone())),
            FilterDialogOpKind::StrNotContains => {
                Ok(FilterOp::StrNotContains(dlg.input_primary.clone()))
            }
            FilterDialogOpKind::NumEq
            | FilterDialogOpKind::NumNeq
            | FilterDialogOpKind::NumGt
            | FilterDialogOpKind::NumGte
            | FilterDialogOpKind::NumLt
            | FilterDialogOpKind::NumLte => {
                match parse_numeric_or_date_input(&dlg.input_primary, meta.date_kind) {
                    Some(v) => Ok(match dlg.op_kind {
                        FilterDialogOpKind::NumEq => FilterOp::NumEq(v),
                        FilterDialogOpKind::NumNeq => FilterOp::NumNeq(v),
                        FilterDialogOpKind::NumGt => FilterOp::NumGt(v),
                        FilterDialogOpKind::NumGte => FilterOp::NumGte(v),
                        FilterDialogOpKind::NumLt => FilterOp::NumLt(v),
                        FilterDialogOpKind::NumLte => FilterOp::NumLte(v),
                        _ => unreachable!(),
                    }),
                    None => Err(if meta.date_kind.is_temporal() {
                        "请输入有效数值或日期（如 2025-01-01 或 01JAN2025）".to_string()
                    } else {
                        "请输入有效的数值".to_string()
                    }),
                }
            }
            FilterDialogOpKind::NumBetween => {
                let a = parse_numeric_or_date_input(&dlg.input_primary, meta.date_kind);
                let b = parse_numeric_or_date_input(&dlg.input_secondary, meta.date_kind);
                match (a, b) {
                    (Some(lo), Some(hi)) => Ok(FilterOp::NumBetween(lo.min(hi), lo.max(hi))),
                    _ => Err("请输入有效的起止数值/日期".to_string()),
                }
            }
        };

        match op_res {
            Ok(op) => {
                let col_idx = dlg.col_idx;
                let replace = dlg.replace_same_col;
                if replace {
                    self.query.set_quick_filter_for_column(col_idx, op);
                } else {
                    self.query.add_filter(col_idx, op);
                }
                self.filter_dialog = None;
                self.recompute_view();
                true
            }
            Err(msg) => {
                if let Some(d) = &mut self.filter_dialog {
                    d.error_msg = Some(msg);
                }
                false
            }
        }
    }

    pub fn selected_cell_text(&self) -> Option<String> {
        let table = self.table.as_ref()?;
        let (vis_row, col_idx) = self.selected_cell?;
        let &orig_row = self.visible_rows.get(vis_row)?;
        Some(table.format_cell(orig_row as usize, col_idx))
    }
}

impl Drop for DatasetTabState {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}
