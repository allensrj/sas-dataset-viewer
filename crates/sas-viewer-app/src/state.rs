use sas_engine::{
    build_multi_cell_filter, compute_column_stats, compute_visible_rows, load_dataset_streaming,
    parse_numeric_or_date_input, search_table_cells, CellSearchMatcher, CellSearchOptions,
    ColumnStats, ColumnarTable, EncodingChoice, FilterOp, LoadEvent, VarType, ViewQuery,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

const MAX_SEARCH_MATCHES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Variables,
    Statistics,
    Shortcuts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusRegion {
    DataGrid,
    StatsList,
    VarList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsFreqSortField {
    Value,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsFreqSort {
    pub field: StatsFreqSortField,
    pub ascending: bool,
}

impl Default for StatsFreqSort {
    fn default() -> Self {
        Self {
            field: StatsFreqSortField::Count,
            ascending: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDialogOpKind {
    StrEq,
    StrNeq,
    StrIn,
    StrNotIn,
    StrContains,
    StrNotContains,
    NumEq,
    NumNeq,
    NumIn,
    NumNotIn,
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
            Self::StrIn | Self::NumIn => "属于多个值 (In, 逗号分隔)",
            Self::StrNotIn | Self::NumNotIn => "不属于多个值 (Not In)",
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
    pub focus_region: FocusRegion,
    pub selected_cell: Option<(usize, usize)>,       // primary / anchor (visible_row_idx, orig_col_idx)
    pub selected_cells: BTreeSet<(usize, usize)>,    // multi-cell selection set
    pub selected_var: Option<usize>,                 // orig_col_idx
    pub scroll_to_col: Option<usize>,
    pub scroll_to_row: Option<usize>,

    // Statistics panel state (always computed on current filtered visible_rows)
    pub stats_col: Option<usize>,
    pub stats_cache_key: Option<(u64, usize)>,
    pub cached_stats: Option<ColumnStats>,
    pub stats_freq_sort: StatsFreqSort,
    pub stats_selected_indices: BTreeSet<usize>,
    pub stats_anchor_idx: Option<usize>,

    // In-dataset cell value search (VS Code-style Search bar + Aa / Whole Word / Regex + Prev/Next)
    pub search_query: String,
    pub search_options: CellSearchOptions,
    pub search_executed_query: String,
    pub search_executed_options: CellSearchOptions,
    pub search_matcher: Option<CellSearchMatcher>,
    pub search_error: Option<String>,
    pub search_filter_version: u64,
    pub search_vis_cols: Vec<usize>,
    pub search_matches: Vec<(usize, usize)>, // (visible_row_idx, orig_col_idx)
    pub search_current_idx: Option<usize>,
    pub search_truncated: bool,

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
            focus_region: FocusRegion::DataGrid,
            selected_cell: None,
            selected_cells: BTreeSet::new(),
            selected_var: None,
            scroll_to_col: None,
            scroll_to_row: None,
            stats_col: None,
            stats_cache_key: None,
            cached_stats: None,
            stats_freq_sort: StatsFreqSort::default(),
            stats_selected_indices: BTreeSet::new(),
            stats_anchor_idx: None,
            search_query: String::new(),
            search_options: CellSearchOptions::default(),
            search_executed_query: String::new(),
            search_executed_options: CellSearchOptions::default(),
            search_matcher: None,
            search_error: None,
            search_filter_version: u64::MAX,
            search_vis_cols: Vec::new(),
            search_matches: Vec::new(),
            search_current_idx: None,
            search_truncated: false,
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
        self.selected_cells.clear();
        self.cached_stats = None;
        self.stats_cache_key = None;
        self.stats_selected_indices.clear();
        self.clear_cell_search();
        self.start_loading(Some(ctx));
    }

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
                    self.install_table(snap);
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
                    self.install_table(full);
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

    fn install_table(&mut self, table: Arc<ColumnarTable>) {
        let ncols = table.variables.len();
        let need_init_cols = self.column_order.len() != ncols;
        if need_init_cols {
            self.column_order = (0..ncols).collect();
            self.column_visible = vec![true; ncols];
            // Balanced, uniform default widths (matching v0.2.0) on initial open
            self.column_widths = (0..ncols)
                .map(|c| table.estimate_default_column_width_px(c))
                .collect();
            if ncols > 0 && self.selected_var.is_none() {
                self.selected_var = Some(0);
            }
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

        // Clamp / clear stale row selections
        let nrows = self.visible_rows.len();
        self.selected_cells.retain(|&(r, _)| r < nrows);
        if let Some((row, col)) = self.selected_cell {
            if nrows == 0 {
                self.selected_cell = None;
            } else if row >= nrows {
                let clamped = (nrows - 1, col);
                self.selected_cell = Some(clamped);
                self.selected_cells.insert(clamped);
            }
        }

        // Refresh stats if viewing a variable (and clear stale stats row selection)
        self.stats_selected_indices.clear();
        self.stats_anchor_idx = None;
        if let Some(scol) = self.stats_col {
            self.refresh_stats_for(scol);
        }

        // Re-run active value search if user had an active search query
        if !self.search_query.is_empty() && !self.search_executed_query.is_empty() {
            self.run_cell_search(false);
        } else {
            self.search_matches.clear();
            self.search_current_idx = None;
        }
    }

    pub fn visible_columns(&self) -> Vec<usize> {
        self.column_order
            .iter()
            .copied()
            .filter(|&c| self.column_visible.get(c).copied().unwrap_or(false))
            .collect()
    }

    pub fn active_target_col(&self) -> Option<usize> {
        if let Some((_, col)) = self.selected_cell {
            Some(col)
        } else {
            self.selected_var
        }
    }

    // ===== Multi-Cell Selection Helpers =====

    pub fn select_single_cell(&mut self, vis_r: usize, orig_col: usize) {
        self.selected_cells.clear();
        self.selected_cells.insert((vis_r, orig_col));
        self.selected_cell = Some((vis_r, orig_col));
        self.selected_var = Some(orig_col);
        self.focus_region = FocusRegion::DataGrid;
    }

    pub fn toggle_cell_selection(&mut self, vis_r: usize, orig_col: usize) {
        let cell = (vis_r, orig_col);
        if self.selected_cells.contains(&cell) && self.selected_cells.len() > 1 {
            self.selected_cells.remove(&cell);
        } else {
            self.selected_cells.insert(cell);
        }
        self.selected_cell = Some(cell);
        self.selected_var = Some(orig_col);
        self.focus_region = FocusRegion::DataGrid;
    }

    pub fn select_cell_range_to(&mut self, vis_r: usize, orig_col: usize) {
        let Some((anchor_r, anchor_col)) = self.selected_cell else {
            self.select_single_cell(vis_r, orig_col);
            return;
        };
        let r_lo = anchor_r.min(vis_r);
        let r_hi = anchor_r.max(vis_r);
        self.selected_cells.clear();

        if anchor_col == orig_col {
            for r in r_lo..=r_hi {
                self.selected_cells.insert((r, orig_col));
            }
        } else {
            let vis_cols = self.visible_columns();
            let c1 = vis_cols.iter().position(|&c| c == anchor_col);
            let c2 = vis_cols.iter().position(|&c| c == orig_col);
            if let (Some(i1), Some(i2)) = (c1, c2) {
                let c_lo = i1.min(i2);
                let c_hi = i1.max(i2);
                for r in r_lo..=r_hi {
                    for &c in &vis_cols[c_lo..=c_hi] {
                        self.selected_cells.insert((r, c));
                    }
                }
            } else {
                self.selected_cells.insert((vis_r, orig_col));
            }
        }
        self.selected_var = Some(orig_col);
        self.focus_region = FocusRegion::DataGrid;
    }

    /// Collect all selected original dataset row indices belonging to `col_idx`.
    pub fn selected_orig_rows_for_col(&self, col_idx: usize) -> Vec<usize> {
        let mut rows = Vec::new();
        for &(vis_r, c) in &self.selected_cells {
            if c == col_idx {
                if let Some(&orig) = self.visible_rows.get(vis_r) {
                    rows.push(orig as usize);
                }
            }
        }
        if rows.is_empty() {
            if let Some((vis_r, c)) = self.selected_cell {
                if c == col_idx {
                    if let Some(&orig) = self.visible_rows.get(vis_r) {
                        rows.push(orig as usize);
                    }
                }
            }
        }
        rows
    }

    // ===== Statistics & Multi-Value Stats Selection =====

    pub fn refresh_stats_for(&mut self, col_idx: usize) {
        let Some(table) = &self.table else {
            return;
        };
        if col_idx >= table.variables.len() {
            return;
        }
        let col_changed = self.stats_col != Some(col_idx);
        self.stats_col = Some(col_idx);
        let key = (self.filter_version, col_idx);
        if self.stats_cache_key == Some(key) && self.cached_stats.is_some() {
            return;
        }
        if col_changed {
            self.stats_selected_indices.clear();
            self.stats_anchor_idx = None;
            self.stats_freq_sort = StatsFreqSort::default();
        }
        self.cached_stats = compute_column_stats(table, &self.visible_rows, col_idx);
        self.stats_cache_key = Some(key);
        self.apply_stats_freq_sort();
    }

    /// Cycle sort mode on the right Statistics frequency table (Value: asc -> desc -> default Count desc;
    /// Count/Percent: desc <-> asc). Preserves any currently selected values!
    pub fn toggle_stats_freq_sort(&mut self, field: StatsFreqSortField) {
        self.stats_freq_sort = match (field, self.stats_freq_sort.field, self.stats_freq_sort.ascending) {
            (StatsFreqSortField::Value, StatsFreqSortField::Count, _) => StatsFreqSort {
                field: StatsFreqSortField::Value,
                ascending: true,
            },
            (StatsFreqSortField::Value, StatsFreqSortField::Value, true) => StatsFreqSort {
                field: StatsFreqSortField::Value,
                ascending: false,
            },
            (StatsFreqSortField::Value, StatsFreqSortField::Value, false) => StatsFreqSort {
                field: StatsFreqSortField::Count,
                ascending: false,
            },
            (StatsFreqSortField::Count, StatsFreqSortField::Value, _) => StatsFreqSort {
                field: StatsFreqSortField::Count,
                ascending: false,
            },
            (StatsFreqSortField::Count, StatsFreqSortField::Count, asc) => StatsFreqSort {
                field: StatsFreqSortField::Count,
                ascending: !asc,
            },
        };
        self.apply_stats_freq_sort();
    }

    /// Sort `cached_stats`'s frequency entries in-place according to `stats_freq_sort`,
    /// preserving selected rows by value identity.
    pub fn apply_stats_freq_sort(&mut self) {
        use std::cmp::Ordering;

        let Some(stats) = &mut self.cached_stats else {
            return;
        };
        let sort = self.stats_freq_sort;
        let is_numeric = matches!(stats, ColumnStats::Numeric(_));
        let entries = match stats {
            ColumnStats::String(ss) => &mut ss.frequencies,
            ColumnStats::Numeric(ns) => &mut ns.frequencies,
        };

        let entry_key = |e: &sas_engine::FreqEntry| -> (bool, u8, Option<u64>, String) {
            (
                e.is_missing,
                e.missing_tag,
                e.raw_numeric.map(|v| v.to_bits()),
                e.value.clone(),
            )
        };
        let selected_keys: Vec<(bool, u8, Option<u64>, String)> = self
            .stats_selected_indices
            .iter()
            .filter_map(|&idx| entries.get(idx).map(entry_key))
            .collect();
        let anchor_key = self
            .stats_anchor_idx
            .and_then(|idx| entries.get(idx).map(entry_key));

        let missing_rank = |tag: u8| -> u8 {
            match tag {
                b'_' => 0,
                sas_engine::MISSING_SYSTEM => 1,
                b'A'..=b'Z' => 2 + (tag - b'A'),
                _ => 1,
            }
        };

        let cmp_value_asc = |a: &sas_engine::FreqEntry, b: &sas_engine::FreqEntry| -> Ordering {
            if is_numeric {
                match (a.is_missing, b.is_missing) {
                    (false, false) => match (a.raw_numeric, b.raw_numeric) {
                        (Some(va), Some(vb)) => va.total_cmp(&vb),
                        _ => a.value.cmp(&b.value),
                    },
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (true, true) => missing_rank(a.missing_tag).cmp(&missing_rank(b.missing_tag)),
                }
            } else {
                match (a.is_missing, b.is_missing) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => a.value.cmp(&b.value),
                }
            }
        };

        entries.sort_by(|a, b| match sort.field {
            StatsFreqSortField::Value => {
                let ord = cmp_value_asc(a, b);
                if sort.ascending {
                    ord
                } else {
                    ord.reverse()
                }
            }
            StatsFreqSortField::Count => {
                let ord = if sort.ascending {
                    a.count.cmp(&b.count)
                } else {
                    b.count.cmp(&a.count)
                };
                ord.then_with(|| cmp_value_asc(a, b))
            }
        });

        // Re-map selection indices to their new sorted positions
        if !selected_keys.is_empty() {
            self.stats_selected_indices.clear();
            for (new_idx, e) in entries.iter().enumerate() {
                let k = entry_key(e);
                if selected_keys.contains(&k) {
                    self.stats_selected_indices.insert(new_idx);
                }
                if anchor_key.as_ref() == Some(&k) {
                    self.stats_anchor_idx = Some(new_idx);
                }
            }
        }
    }

    /// Build inclusive (`=` / `in`) or exclusive (`!=` / `not in`) FilterOp from selected rows in the Statistics list.
    pub fn build_stats_selection_filter(&self, negate: bool) -> Option<(usize, FilterOp)> {
        let col_idx = self.stats_col?;
        let stats = self.cached_stats.as_ref()?;
        if self.stats_selected_indices.is_empty() {
            return None;
        }

        match stats {
            ColumnStats::String(ss) => {
                let mut vals: Vec<String> = Vec::new();
                for &idx in &self.stats_selected_indices {
                    if let Some(entry) = ss.frequencies.get(idx) {
                        let raw = if entry.is_missing {
                            String::new()
                        } else {
                            entry.value.clone()
                        };
                        if !vals.contains(&raw) {
                            vals.push(raw);
                        }
                    }
                }
                if vals.is_empty() {
                    return None;
                }
                let op = if vals.len() == 1 {
                    let s = vals.remove(0);
                    if s.is_empty() {
                        if negate {
                            FilterOp::StrIsNotEmpty
                        } else {
                            FilterOp::StrIsEmpty
                        }
                    } else if negate {
                        FilterOp::StrNeq(s)
                    } else {
                        FilterOp::StrEq(s)
                    }
                } else if negate {
                    FilterOp::StrNotIn(vals)
                } else {
                    FilterOp::StrIn(vals)
                };
                Some((col_idx, op))
            }
            ColumnStats::Numeric(ns) => {
                let mut values: Vec<f64> = Vec::new();
                let mut missing_tags: Vec<u8> = Vec::new();
                for &idx in &self.stats_selected_indices {
                    if let Some(entry) = ns.frequencies.get(idx) {
                        if entry.is_missing {
                            if !missing_tags.contains(&entry.missing_tag) {
                                missing_tags.push(entry.missing_tag);
                            }
                        } else if let Some(v) = entry.raw_numeric {
                            if !values.iter().any(|x| (x - v).abs() <= 1e-11) {
                                values.push(v);
                            }
                        }
                    }
                }
                let total = values.len() + missing_tags.len();
                if total == 0 {
                    return None;
                }
                let op = if total == 1 {
                    if let Some(&v) = values.first() {
                        if negate {
                            FilterOp::NumNeq(v)
                        } else {
                            FilterOp::NumEq(v)
                        }
                    } else {
                        let tag = missing_tags[0];
                        if negate {
                            FilterOp::NumNotIn {
                                values: Vec::new(),
                                missing_tags: vec![tag],
                            }
                        } else {
                            FilterOp::NumEqTaggedMissing(tag)
                        }
                    }
                } else if negate {
                    FilterOp::NumNotIn {
                        values,
                        missing_tags,
                    }
                } else {
                    FilterOp::NumIn {
                        values,
                        missing_tags,
                    }
                };
                Some((col_idx, op))
            }
        }
    }

    /// Shortcut `D`: unified quick `=` / `IN` filter across DataGrid, StatsList, and VarList!
    pub fn trigger_quick_d_filter(&mut self) -> Option<String> {
        let table = self.table.clone()?;

        // 1. If focused in Statistics frequency list & items are selected -> filter by selected value(s)!
        if self.focus_region == FocusRegion::StatsList && !self.stats_selected_indices.is_empty() {
            if let Some((col_idx, op)) = self.build_stats_selection_filter(false) {
                let n_sel = self.stats_selected_indices.len();
                self.query.set_quick_filter_for_column(col_idx, op);
                self.recompute_view();
                return Some(if n_sel > 1 {
                    format!("已按统计栏选中的 {} 个值执行筛选", n_sel)
                } else {
                    "已按统计栏选中值执行等值 (=) 筛选".to_string()
                });
            }
        }

        // 2. If focused in DataGrid -> filter by single or multiple selected cells in active column!
        if self.focus_region == FocusRegion::DataGrid {
            if let Some((_, col_idx)) = self.selected_cell {
                let orig_rows = self.selected_orig_rows_for_col(col_idx);
                if let Some(op) = build_multi_cell_filter(&table, &orig_rows, col_idx, false) {
                    let multi = matches!(op, FilterOp::StrIn(_) | FilterOp::NumIn { .. });
                    self.query.set_quick_filter_for_column(col_idx, op);
                    self.recompute_view();
                    return Some(if multi {
                        "已按多选单元格值执行多值筛选".to_string()
                    } else {
                        "已应用快速等值 (=) 筛选".to_string()
                    });
                }
            }
        }

        // 3. Fallback (VarList): open focused quick filter dialog for selected variable
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
                return Some(format!("请输入 {} 的筛选值", meta.name));
            }
        }
        None
    }

    pub fn clear_all_filters_and_sorts(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.recompute_view();
        }
    }

    /// Clear sorting only (restoring initial dataset row order while preserving active filters).
    pub fn clear_sorts_only(&mut self) {
        if !self.query.sorts.is_empty() {
            self.query.sorts.clear();
            self.recompute_view();
        }
    }

    pub fn hide_all_variables(&mut self) {
        for v in &mut self.column_visible {
            *v = false;
        }
    }

    pub fn show_all_variables(&mut self) {
        for v in &mut self.column_visible {
            *v = true;
        }
    }

    /// Collect names of all currently checked variables in display order.
    pub fn checked_variable_names(&self) -> Vec<String> {
        let Some(table) = &self.table else {
            return Vec::new();
        };
        self.column_order
            .iter()
            .copied()
            .filter(|&c| self.column_visible.get(c).copied().unwrap_or(false))
            .filter_map(|c| table.variables.get(c).map(|m| m.name.clone()))
            .collect()
    }

    /// Inspect how a preset variable list matches the current dataset's variables (case-insensitive).
    /// Returns `(matched_dataset_var_names, unmatched_preset_names)`.
    pub fn preview_var_preset(&self, preset_vars: &[String]) -> (Vec<String>, Vec<String>) {
        let Some(table) = &self.table else {
            return (Vec::new(), preset_vars.to_vec());
        };
        let mut matched = Vec::new();
        let mut unmatched = Vec::new();
        for name in preset_vars {
            if let Some(meta) = table
                .variables
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(name))
            {
                matched.push(meta.name.clone());
            } else {
                unmatched.push(name.clone());
            }
        }
        (matched, unmatched)
    }

    /// Apply a preset list of variable names to `column_visible` (case-insensitive).
    /// Returns `(matched_count, unmatched_names)`. If zero variables match, leaves checkboxes unchanged.
    pub fn apply_var_preset(&mut self, preset_vars: &[String]) -> (usize, Vec<String>) {
        let Some(table) = &self.table else {
            return (0, preset_vars.to_vec());
        };
        let mut matched_flags = vec![false; table.variables.len()];
        let mut matched_count = 0usize;
        let mut unmatched = Vec::new();

        for name in preset_vars {
            let mut found = false;
            for (idx, meta) in table.variables.iter().enumerate() {
                if meta.name.eq_ignore_ascii_case(name) {
                    if !matched_flags[idx] {
                        matched_flags[idx] = true;
                        matched_count += 1;
                    }
                    found = true;
                }
            }
            if !found {
                unmatched.push(name.clone());
            }
        }

        if matched_count > 0 {
            if self.column_visible.len() != matched_flags.len() {
                self.column_visible = matched_flags;
            } else {
                self.column_visible.copy_from_slice(&matched_flags);
            }
        }

        (matched_count, unmatched)
    }

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

    /// Auto-fit single column strictly to data max width, clamped to [6, 30] ASCII chars.
    pub fn auto_fit_column(&mut self, col_idx: usize) {
        if let Some(table) = &self.table {
            if col_idx < self.column_widths.len() {
                self.column_widths[col_idx] = table.estimate_column_width_px(col_idx, 6, 30, true);
            }
        }
    }

    /// Batch auto-fit all visible columns strictly to data max width, clamped to [6, 30] ASCII chars.
    pub fn auto_fit_all_visible_columns(&mut self) {
        if let Some(table) = &self.table {
            for col_idx in self.visible_columns() {
                if col_idx < self.column_widths.len() {
                    self.column_widths[col_idx] =
                        table.estimate_column_width_px(col_idx, 6, 30, true);
                }
            }
        }
    }

    /// Reset all columns back to balanced, uniform default widths (v0.2.0 style).
    pub fn reset_all_column_widths(&mut self) {
        if let Some(table) = &self.table {
            for col_idx in 0..self.column_widths.len() {
                self.column_widths[col_idx] = table.estimate_default_column_width_px(col_idx);
            }
        }
    }

    // ===== VS Code-Style In-Dataset Cell Value Search =====

    pub fn clear_cell_search(&mut self) {
        self.search_query.clear();
        self.search_executed_query.clear();
        self.search_matcher = None;
        self.search_error = None;
        self.search_vis_cols.clear();
        self.search_matches.clear();
        self.search_current_idx = None;
        self.search_truncated = false;
    }

    fn search_needs_refresh(&self) -> bool {
        self.search_executed_query != self.search_query
            || self.search_executed_options != self.search_options
            || self.search_filter_version != self.filter_version
            || self.search_vis_cols != self.visible_columns()
    }

    /// Search across visible rows × visible columns (strings, numbers, formatted dates, SAS missing)
    /// honoring VS Code options: Match Case (`Aa`), Whole Word (`全字`), and Regex (`.*`).
    pub fn run_cell_search(&mut self, jump_to_first: bool) {
        if self.search_query.is_empty() {
            self.search_executed_query.clear();
            self.search_matcher = None;
            self.search_error = None;
            self.search_vis_cols.clear();
            self.search_matches.clear();
            self.search_current_idx = None;
            self.search_truncated = false;
            return;
        }
        let Some(table) = self.table.clone() else {
            return;
        };

        let vis_cols = self.visible_columns();
        match CellSearchMatcher::compile(&self.search_query, self.search_options) {
            Ok(matcher) => {
                let (matches, truncated) = search_table_cells(
                    &table,
                    &self.visible_rows,
                    &vis_cols,
                    &matcher,
                    MAX_SEARCH_MATCHES,
                );
                self.search_executed_query = self.search_query.clone();
                self.search_executed_options = self.search_options;
                self.search_filter_version = self.filter_version;
                self.search_vis_cols = vis_cols;
                self.search_matcher = Some(matcher);
                self.search_error = None;
                self.search_matches = matches;
                self.search_truncated = truncated;

                if self.search_matches.is_empty() {
                    self.search_current_idx = None;
                } else if jump_to_first {
                    self.jump_to_search_match(0);
                } else {
                    self.search_current_idx = Some(0);
                }
            }
            Err(err) => {
                self.search_executed_query = self.search_query.clone();
                self.search_executed_options = self.search_options;
                self.search_filter_version = self.filter_version;
                self.search_vis_cols = vis_cols;
                self.search_matcher = None;
                self.search_error = Some(err);
                self.search_matches.clear();
                self.search_current_idx = None;
                self.search_truncated = false;
            }
        }
    }

    pub fn search_next(&mut self) {
        if self.search_query.is_empty() {
            return;
        }
        if self.search_needs_refresh() {
            self.run_cell_search(true);
            return;
        }
        let n = self.search_matches.len();
        if n == 0 {
            return;
        }
        let next = match self.search_current_idx {
            Some(cur) => (cur + 1) % n,
            None => 0,
        };
        self.jump_to_search_match(next);
    }

    pub fn search_prev(&mut self) {
        if self.search_query.is_empty() {
            return;
        }
        if self.search_needs_refresh() {
            self.run_cell_search(true);
            return;
        }
        let n = self.search_matches.len();
        if n == 0 {
            return;
        }
        let prev = match self.search_current_idx {
            Some(0) | None => n - 1,
            Some(cur) => cur - 1,
        };
        self.jump_to_search_match(prev);
    }

    fn jump_to_search_match(&mut self, match_idx: usize) {
        if let Some(&(vis_r, col_idx)) = self.search_matches.get(match_idx) {
            self.search_current_idx = Some(match_idx);
            self.select_single_cell(vis_r, col_idx);
            self.scroll_to_row = Some(vis_r);
            self.scroll_to_col = Some(col_idx);
        }
    }

    /// Apply filter from custom dialog (supports single & comma/newline-separated multi-value In/NotIn!).
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

        let parse_str_list = |raw: &str| -> Vec<String> {
            raw.split(|c| c == ',' || c == '，' || c == '\n' || c == ';')
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        let op_res: Result<FilterOp, String> = match dlg.op_kind {
            FilterDialogOpKind::StrEq => Ok(FilterOp::StrEq(dlg.input_primary.clone())),
            FilterDialogOpKind::StrNeq => Ok(FilterOp::StrNeq(dlg.input_primary.clone())),
            FilterDialogOpKind::StrIn => {
                let vals = parse_str_list(&dlg.input_primary);
                if vals.is_empty() {
                    Err("请输入至少一个值（多个值可用逗号分隔）".to_string())
                } else {
                    Ok(FilterOp::StrIn(vals))
                }
            }
            FilterDialogOpKind::StrNotIn => {
                let vals = parse_str_list(&dlg.input_primary);
                if vals.is_empty() {
                    Err("请输入至少一个值（多个值可用逗号分隔）".to_string())
                } else {
                    Ok(FilterOp::StrNotIn(vals))
                }
            }
            FilterDialogOpKind::StrContains => Ok(FilterOp::StrContains(dlg.input_primary.clone())),
            FilterDialogOpKind::StrNotContains => {
                Ok(FilterOp::StrNotContains(dlg.input_primary.clone()))
            }
            FilterDialogOpKind::NumIn | FilterDialogOpKind::NumNotIn => {
                let tokens = parse_str_list(&dlg.input_primary);
                let mut values = Vec::new();
                let mut missing_tags = Vec::new();
                let mut parse_err: Option<String> = None;
                for tok in &tokens {
                    if tok == "." {
                        missing_tags.push(sas_engine::MISSING_SYSTEM);
                    } else if tok.len() == 2 && tok.starts_with('.') {
                        let ch = tok.as_bytes()[1].to_ascii_uppercase();
                        if ch.is_ascii_uppercase() || ch == b'_' {
                            missing_tags.push(ch);
                        } else {
                            parse_err = Some(format!("无法识别的数值/缺失值: {tok}"));
                            break;
                        }
                    } else if let Some(v) = parse_numeric_or_date_input(tok, meta.date_kind) {
                        values.push(v);
                    } else {
                        parse_err = Some(format!("无法识别的数值或日期: {tok}"));
                        break;
                    }
                }
                if let Some(err) = parse_err {
                    Err(err)
                } else if values.is_empty() && missing_tags.is_empty() {
                    Err("请输入至少一个数值（多个值可用逗号分隔）".to_string())
                } else if dlg.op_kind == FilterDialogOpKind::NumIn {
                    Ok(FilterOp::NumIn {
                        values,
                        missing_tags,
                    })
                } else {
                    Ok(FilterOp::NumNotIn {
                        values,
                        missing_tags,
                    })
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sas_engine::{FreqEntry, NumericStats, MISSING_SYSTEM, MISSING_VALID};

    #[test]
    fn test_stats_freq_sort_numeric_value_and_selection_preservation() {
        let mut tab = DatasetTabState {
            id: 1,
            path: PathBuf::from("test.xpt"),
            display_name: "test.xpt".into(),
            encoding: EncodingChoice::Auto,
            table: None,
            loading: false,
            load_progress: 1.0,
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
            focus_region: FocusRegion::StatsList,
            selected_cell: None,
            selected_cells: BTreeSet::new(),
            selected_var: None,
            scroll_to_col: None,
            scroll_to_row: None,
            stats_col: Some(0),
            stats_cache_key: Some((0, 0)),
            cached_stats: Some(ColumnStats::Numeric(NumericStats {
                total_rows: 10,
                valid_count: 8,
                missing_count: 2,
                tagged_missing_counts: Vec::new(),
                distinct_count: 2,
                mean: None,
                std_dev: None,
                min: Some(2.0),
                q1: None,
                median: None,
                q3: None,
                max: Some(10.0),
                sum: None,
                frequencies: vec![
                    FreqEntry {
                        value: "10".into(),
                        is_missing: false,
                        raw_numeric: Some(10.0),
                        missing_tag: MISSING_VALID,
                        count: 5,
                        percent: 50.0,
                    },
                    FreqEntry {
                        value: "2".into(),
                        is_missing: false,
                        raw_numeric: Some(2.0),
                        missing_tag: MISSING_VALID,
                        count: 3,
                        percent: 30.0,
                    },
                    FreqEntry {
                        value: ".".into(),
                        is_missing: true,
                        raw_numeric: None,
                        missing_tag: MISSING_SYSTEM,
                        count: 2,
                        percent: 20.0,
                    },
                ],
                truncated: false,
            })),
            stats_freq_sort: StatsFreqSort::default(),
            stats_selected_indices: BTreeSet::from([1]), // selected "2"
            stats_anchor_idx: Some(1),
            search_query: String::new(),
            search_options: CellSearchOptions::default(),
            search_executed_query: String::new(),
            search_executed_options: CellSearchOptions::default(),
            search_matcher: None,
            search_error: None,
            search_filter_version: 0,
            search_vis_cols: Vec::new(),
            search_matches: Vec::new(),
            search_current_idx: None,
            search_truncated: false,
            var_search: String::new(),
            filter_dialog: None,
            dragging_col: None,
            grid_scroll_offset: egui::Vec2::ZERO,
            last_query_ms: 0,
        };

        // 1st click on Value -> ascending numeric order: ".", "2", "10" (NOT lexicographical "10" < "2"!)
        tab.toggle_stats_freq_sort(StatsFreqSortField::Value);
        let vals = |t: &DatasetTabState| -> Vec<String> {
            match t.cached_stats.as_ref().unwrap() {
                ColumnStats::Numeric(ns) => ns.frequencies.iter().map(|e| e.value.clone()).collect(),
                _ => unreachable!(),
            }
        };
        assert_eq!(vals(&tab), vec![".", "2", "10"]);
        // Selection "2" stayed selected at index 1!
        assert!(tab.stats_selected_indices.contains(&1));

        // 2nd click on Value -> descending numeric order: "10", "2", "."
        tab.toggle_stats_freq_sort(StatsFreqSortField::Value);
        assert_eq!(vals(&tab), vec!["10", "2", "."]);
        assert!(tab.stats_selected_indices.contains(&1));

        // 3rd click on Value -> back to default Count desc: "10"(5), "2"(3), "."(2)
        tab.toggle_stats_freq_sort(StatsFreqSortField::Value);
        assert_eq!(vals(&tab), vec!["10", "2", "."]);

        // Click on Count -> Count asc: "."(2), "2"(3), "10"(5)
        tab.toggle_stats_freq_sort(StatsFreqSortField::Count);
        assert_eq!(vals(&tab), vec![".", "2", "10"]);
    }

    #[test]
    fn test_apply_var_preset_case_insensitive_and_safe_on_zero_match() {
        use sas_engine::{
            ColumnData, DatasetMetadata, SasDateKind, SasFormatKind, StringColumn, VariableMeta,
        };

        let make_var = |index: usize, name: &str| VariableMeta {
            index,
            name: name.to_string(),
            label: String::new(),
            format: String::new(),
            var_type: VarType::String,
            storage_width: 8,
            display_width: 8,
            date_kind: SasDateKind::None,
            decimal_places: None,
        };
        let empty_str_col = || {
            ColumnData::String(StringColumn::Flat {
                data: Vec::new(),
                offsets: vec![0],
            })
        };
        let table = Arc::new(ColumnarTable {
            metadata: DatasetMetadata {
                path: PathBuf::from("dm.sas7bdat"),
                format_kind: SasFormatKind::Sas7bdat,
                table_name: "DM".into(),
                file_label: String::new(),
                file_encoding: "UTF-8".into(),
                encoding_choice: EncodingChoice::Auto,
                declared_row_count: 0,
                is_64bit: true,
                compression: String::new(),
                file_size_bytes: 0,
            },
            variables: vec![
                make_var(0, "STUDYID"),
                make_var(1, "USUBJID"),
                make_var(2, "SUBJID"),
                make_var(3, "SITEID"),
                make_var(4, "AGE"),
            ],
            columns: vec![
                empty_str_col(),
                empty_str_col(),
                empty_str_col(),
                empty_str_col(),
                empty_str_col(),
            ],
            row_count: 0,
            load_elapsed_ms: 0,
            is_complete: true,
        });

        let mut tab = DatasetTabState {
            id: 1,
            path: PathBuf::from("dm.sas7bdat"),
            display_name: "dm.sas7bdat".into(),
            encoding: EncodingChoice::Auto,
            table: None,
            loading: false,
            load_progress: 1.0,
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
            focus_region: FocusRegion::DataGrid,
            selected_cell: None,
            selected_cells: BTreeSet::new(),
            selected_var: None,
            scroll_to_col: None,
            scroll_to_row: None,
            stats_col: None,
            stats_cache_key: None,
            cached_stats: None,
            stats_freq_sort: StatsFreqSort::default(),
            stats_selected_indices: BTreeSet::new(),
            stats_anchor_idx: None,
            search_query: String::new(),
            search_options: CellSearchOptions::default(),
            search_executed_query: String::new(),
            search_executed_options: CellSearchOptions::default(),
            search_matcher: None,
            search_error: None,
            search_filter_version: 0,
            search_vis_cols: Vec::new(),
            search_matches: Vec::new(),
            search_current_idx: None,
            search_truncated: false,
            var_search: String::new(),
            filter_dialog: None,
            dragging_col: None,
            grid_scroll_offset: egui::Vec2::ZERO,
            last_query_ms: 0,
        };
        tab.install_table(table);
        assert_eq!(tab.column_visible, vec![true, true, true, true, true]);

        // Case-insensitive match + unknown variable reported as unmatched
        let (matched, unmatched) = tab.apply_var_preset(&[
            "studyid".into(),
            "USUBJID".into(),
            "SubjId".into(),
            "UNKNOWN_VAR".into(),
        ]);
        assert_eq!(matched, 3);
        assert_eq!(unmatched, vec!["UNKNOWN_VAR"]);
        assert_eq!(tab.column_visible, vec![true, true, true, false, false]);

        // Zero-match preset does NOT blank out existing checkboxes
        let (matched0, unmatched0) = tab.apply_var_preset(&["NOPE".into()]);
        assert_eq!(matched0, 0);
        assert_eq!(unmatched0, vec!["NOPE"]);
        assert_eq!(tab.column_visible, vec![true, true, true, false, false]);
    }
}
