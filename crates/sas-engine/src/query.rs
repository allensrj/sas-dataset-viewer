use crate::column::{ColumnData, ColumnarTable, StringColumn, MISSING_SYSTEM, MISSING_VALID};
use crate::sas_format::{format_missing_tag, format_numeric_value};
use crate::types::VariableMeta;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FilterOp {
    // String operators
    StrEq(String),
    StrNeq(String),
    StrIn(Vec<String>),
    StrNotIn(Vec<String>),
    StrContains(String),
    StrNotContains(String),
    StrIsEmpty,
    StrIsNotEmpty,
    // Numeric operators (evaluated on underlying f64 numeric logic)
    NumEq(f64),
    NumNeq(f64),
    NumIn { values: Vec<f64>, missing_tags: Vec<u8> },
    NumNotIn { values: Vec<f64>, missing_tags: Vec<u8> },
    NumGt(f64),
    NumGte(f64),
    NumLt(f64),
    NumLte(f64),
    NumBetween(f64, f64),
    NumIsMissing,
    NumIsNotMissing,
    NumEqTaggedMissing(u8),
}

fn format_str_set_preview(vals: &[String]) -> String {
    let items: Vec<String> = vals
        .iter()
        .take(3)
        .map(|s| {
            if s.is_empty() {
                "(空)".to_string()
            } else {
                format!("\"{}\"", s)
            }
        })
        .collect();
    if vals.len() > 3 {
        format!("({}, +{}项)", items.join(", "), vals.len() - 3)
    } else {
        format!("({})", items.join(", "))
    }
}

fn format_num_set_preview(values: &[f64], missing_tags: &[u8], meta: &VariableMeta) -> String {
    let mut parts: Vec<String> = values
        .iter()
        .map(|v| format_numeric_value(*v, meta))
        .collect();
    for &t in missing_tags {
        parts.push(format_missing_tag(t));
    }
    let total = parts.len();
    let head: Vec<String> = parts.into_iter().take(3).collect();
    if total > 3 {
        format!("({}, +{}项)", head.join(", "), total - 3)
    } else {
        format!("({})", head.join(", "))
    }
}

impl FilterOp {
    pub fn display_chip(&self, meta: &VariableMeta) -> String {
        match self {
            FilterOp::StrEq(v) => format!("{} = \"{}\"", meta.name, v),
            FilterOp::StrNeq(v) => format!("{} != \"{}\"", meta.name, v),
            FilterOp::StrIn(vals) => format!("{} ∈ {}", meta.name, format_str_set_preview(vals)),
            FilterOp::StrNotIn(vals) => {
                format!("{} ∉ {}", meta.name, format_str_set_preview(vals))
            }
            FilterOp::StrContains(v) => format!("{} 包含 \"{}\"", meta.name, v),
            FilterOp::StrNotContains(v) => format!("{} 不包含 \"{}\"", meta.name, v),
            FilterOp::StrIsEmpty => format!("{} 为空", meta.name),
            FilterOp::StrIsNotEmpty => format!("{} 非空", meta.name),
            FilterOp::NumEq(v) => format!("{} = {}", meta.name, format_numeric_value(*v, meta)),
            FilterOp::NumNeq(v) => format!("{} != {}", meta.name, format_numeric_value(*v, meta)),
            FilterOp::NumIn {
                values,
                missing_tags,
            } => format!(
                "{} ∈ {}",
                meta.name,
                format_num_set_preview(values, missing_tags, meta)
            ),
            FilterOp::NumNotIn {
                values,
                missing_tags,
            } => format!(
                "{} ∉ {}",
                meta.name,
                format_num_set_preview(values, missing_tags, meta)
            ),
            FilterOp::NumGt(v) => format!("{} > {}", meta.name, format_numeric_value(*v, meta)),
            FilterOp::NumGte(v) => format!("{} >= {}", meta.name, format_numeric_value(*v, meta)),
            FilterOp::NumLt(v) => format!("{} < {}", meta.name, format_numeric_value(*v, meta)),
            FilterOp::NumLte(v) => format!("{} <= {}", meta.name, format_numeric_value(*v, meta)),
            FilterOp::NumBetween(lo, hi) => format!(
                "{} ∈ [{}, {}]",
                meta.name,
                format_numeric_value(*lo, meta),
                format_numeric_value(*hi, meta)
            ),
            FilterOp::NumIsMissing => format!("{} 缺失", meta.name),
            FilterOp::NumIsNotMissing => format!("{} 非缺失", meta.name),
            FilterOp::NumEqTaggedMissing(tag) => {
                format!("{} = {}", meta.name, format_missing_tag(*tag))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterCondition {
    pub col_idx: usize,
    pub op: FilterOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    pub col_idx: usize,
    pub ascending: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewQuery {
    pub filters: Vec<FilterCondition>,
    pub sorts: Vec<SortKey>,
}

impl ViewQuery {
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty() && self.sorts.is_empty()
    }

    pub fn clear(&mut self) {
        self.filters.clear();
        self.sorts.clear();
    }

    /// Quick `=` filter (`D` shortcut): AND-stacks across different columns, replaces existing filter on the same column.
    pub fn set_quick_filter_for_column(&mut self, col_idx: usize, op: FilterOp) {
        self.filters.retain(|f| f.col_idx != col_idx);
        self.filters.push(FilterCondition { col_idx, op });
    }

    /// Add filter condition (allows stacking multiple conditions even on same column via right-click menu).
    pub fn add_filter(&mut self, col_idx: usize, op: FilterOp) {
        self.filters.push(FilterCondition { col_idx, op });
    }

    /// Toggle or stack multi-column sort.
    /// If `append` is false, replaces existing sort keys (or cycles asc -> desc -> none if sole key).
    /// If `append` is true, adds/toggles this column inside the stacked sort list.
    pub fn toggle_sort(&mut self, col_idx: usize, append: bool) {
        if !append {
            if self.sorts.len() == 1 && self.sorts[0].col_idx == col_idx {
                if self.sorts[0].ascending {
                    self.sorts[0].ascending = false;
                } else {
                    self.sorts.clear();
                }
            } else {
                self.sorts.clear();
                self.sorts.push(SortKey {
                    col_idx,
                    ascending: true,
                });
            }
        } else if let Some(pos) = self.sorts.iter().position(|s| s.col_idx == col_idx) {
            if self.sorts[pos].ascending {
                self.sorts[pos].ascending = false;
            } else {
                self.sorts.remove(pos);
            }
        } else {
            self.sorts.push(SortKey {
                col_idx,
                ascending: true,
            });
        }
    }

    pub fn set_sort_direction(&mut self, col_idx: usize, ascending: bool, append: bool) {
        if !append {
            self.sorts.clear();
            self.sorts.push(SortKey { col_idx, ascending });
        } else if let Some(existing) = self.sorts.iter_mut().find(|s| s.col_idx == col_idx) {
            existing.ascending = ascending;
        } else {
            self.sorts.push(SortKey { col_idx, ascending });
        }
    }

    pub fn remove_sort(&mut self, col_idx: usize) {
        self.sorts.retain(|s| s.col_idx != col_idx);
    }

    pub fn sort_rank(&self, col_idx: usize) -> Option<(usize, bool)> {
        self.sorts
            .iter()
            .position(|s| s.col_idx == col_idx)
            .map(|idx| (idx + 1, self.sorts[idx].ascending))
    }

    pub fn has_filter_on(&self, col_idx: usize) -> bool {
        self.filters.iter().any(|f| f.col_idx == col_idx)
    }
}

/// Precompiled predicate for blazing-fast parallel row evaluation.
enum CompiledFilter<'a> {
    DictMask {
        ids: &'a [u32],
        mask: Vec<bool>,
    },
    FlatString {
        col: &'a StringColumn,
        op: &'a FilterOp,
        needle_lower: Option<String>,
    },
    Numeric {
        values: &'a [f64],
        missing: &'a [u8],
        op: &'a FilterOp,
    },
}

impl<'a> CompiledFilter<'a> {
    fn compile(table: &'a ColumnarTable, cond: &'a FilterCondition) -> Option<Self> {
        let col = table.columns.get(cond.col_idx)?;
        match col {
            ColumnData::String(sc) => match sc {
                StringColumn::Dict { dict, ids, .. } => {
                    let needle_lower = match &cond.op {
                        FilterOp::StrContains(s) | FilterOp::StrNotContains(s) => {
                            Some(s.to_lowercase())
                        }
                        _ => None,
                    };
                    let mask: Vec<bool> = dict
                        .iter()
                        .map(|val| eval_str_op(val, &cond.op, needle_lower.as_deref()))
                        .collect();
                    Some(Self::DictMask { ids, mask })
                }
                StringColumn::Flat { .. } => {
                    let needle_lower = match &cond.op {
                        FilterOp::StrContains(s) | FilterOp::StrNotContains(s) => {
                            Some(s.to_lowercase())
                        }
                        _ => None,
                    };
                    Some(Self::FlatString {
                        col: sc,
                        op: &cond.op,
                        needle_lower,
                    })
                }
            },
            ColumnData::Numeric(nc) => Some(Self::Numeric {
                values: &nc.values,
                missing: &nc.missing,
                op: &cond.op,
            }),
        }
    }

    #[inline]
    fn matches(&self, row: usize) -> bool {
        match self {
            Self::DictMask { ids, mask } => mask[ids[row] as usize],
            Self::FlatString {
                col,
                op,
                needle_lower,
            } => eval_str_op(col.get_str(row), op, needle_lower.as_deref()),
            Self::Numeric {
                values,
                missing,
                op,
            } => eval_num_op(values[row], missing[row], op),
        }
    }
}

#[inline]
fn eval_str_op(val: &str, op: &FilterOp, needle_lower: Option<&str>) -> bool {
    match op {
        FilterOp::StrEq(target) => val == target,
        FilterOp::StrNeq(target) => val != target,
        FilterOp::StrIn(targets) => targets.iter().any(|t| t == val),
        FilterOp::StrNotIn(targets) => !targets.iter().any(|t| t == val),
        FilterOp::StrContains(_) => {
            let needle = needle_lower.unwrap_or("");
            if needle.is_empty() {
                true
            } else {
                val.to_lowercase().contains(needle)
            }
        }
        FilterOp::StrNotContains(_) => {
            let needle = needle_lower.unwrap_or("");
            if needle.is_empty() {
                false
            } else {
                !val.to_lowercase().contains(needle)
            }
        }
        FilterOp::StrIsEmpty => val.is_empty(),
        FilterOp::StrIsNotEmpty => !val.is_empty(),
        _ => true,
    }
}

#[inline]
fn eval_num_op(val: f64, miss: u8, op: &FilterOp) -> bool {
    const EPS: f64 = 1e-11;
    match op {
        FilterOp::NumIsMissing => miss != MISSING_VALID,
        FilterOp::NumIsNotMissing => miss == MISSING_VALID,
        FilterOp::NumEqTaggedMissing(tag) => {
            if *tag == MISSING_SYSTEM {
                miss == MISSING_SYSTEM
            } else {
                miss == *tag
            }
        }
        FilterOp::NumEq(target) => miss == MISSING_VALID && (val - *target).abs() <= EPS,
        FilterOp::NumNeq(target) => miss != MISSING_VALID || (val - *target).abs() > EPS,
        FilterOp::NumIn {
            values,
            missing_tags,
        } => {
            if miss == MISSING_VALID {
                values.iter().any(|t| (val - *t).abs() <= EPS)
            } else {
                missing_tags.contains(&miss)
            }
        }
        FilterOp::NumNotIn {
            values,
            missing_tags,
        } => {
            if miss == MISSING_VALID {
                !values.iter().any(|t| (val - *t).abs() <= EPS)
            } else {
                !missing_tags.contains(&miss)
            }
        }
        FilterOp::NumGt(target) => miss == MISSING_VALID && val > *target,
        FilterOp::NumGte(target) => miss == MISSING_VALID && val >= *target - EPS,
        FilterOp::NumLt(target) => miss == MISSING_VALID && val < *target,
        FilterOp::NumLte(target) => miss == MISSING_VALID && val <= *target + EPS,
        FilterOp::NumBetween(lo, hi) => {
            miss == MISSING_VALID && val >= *lo - EPS && val <= *hi + EPS
        }
        _ => true,
    }
}

/// Build the exact visible row index vector by running parallel filter + parallel multi-column sort.
pub fn compute_visible_rows(table: &ColumnarTable, query: &ViewQuery) -> Vec<u32> {
    let n = table.row_count;
    if n == 0 {
        return Vec::new();
    }

    // Fast path: no filter and no sort -> identity range
    if query.filters.is_empty() && query.sorts.is_empty() {
        return (0..n as u32).collect();
    }

    // Step 1: Parallel filtering
    let mut rows: Vec<u32> = if query.filters.is_empty() {
        (0..n as u32).collect()
    } else {
        let compiled: Vec<CompiledFilter<'_>> = query
            .filters
            .iter()
            .filter_map(|f| CompiledFilter::compile(table, f))
            .collect();

        if n >= 16_384 {
            const CHUNK: usize = 8_192;
            let num_chunks = (n + CHUNK - 1) / CHUNK;
            let chunks: Vec<Vec<u32>> = (0..num_chunks)
                .into_par_iter()
                .map(|chunk_idx| {
                    let start = chunk_idx * CHUNK;
                    let end = (start + CHUNK).min(n);
                    let mut local = Vec::new();
                    for r in start..end {
                        if compiled.iter().all(|pred| pred.matches(r)) {
                            local.push(r as u32);
                        }
                    }
                    local
                })
                .collect();
            let total: usize = chunks.iter().map(|c| c.len()).sum();
            let mut merged = Vec::with_capacity(total);
            for c in chunks {
                merged.extend_from_slice(&c);
            }
            merged
        } else {
            let mut out = Vec::with_capacity(n / 4);
            for r in 0..n {
                if compiled.iter().all(|pred| pred.matches(r)) {
                    out.push(r as u32);
                }
            }
            out
        }
    };

    // Step 2: Parallel multi-column sorting
    if !query.sorts.is_empty() && rows.len() > 1 {
        let sorts = &query.sorts;
        let cmp = |&a: &u32, &b: &u32| -> Ordering {
            let ra = a as usize;
            let rb = b as usize;
            for sk in sorts {
                if let Some(col) = table.columns.get(sk.col_idx) {
                    let ord = col.compare_rows(ra, rb);
                    if ord != Ordering::Equal {
                        return if sk.ascending { ord } else { ord.reverse() };
                    }
                }
            }
            // Stable tie-breaker by original row order
            a.cmp(&b)
        };

        if rows.len() >= 8_192 {
            rows.par_sort_unstable_by(cmp);
        } else {
            rows.sort_unstable_by(cmp);
        }
    }

    rows
}

/// Build a quick `=` FilterOp from a specific cell (used by `D` shortcut).
pub fn build_cell_equals_filter(table: &ColumnarTable, row: usize, col_idx: usize) -> Option<FilterOp> {
    let col = table.columns.get(col_idx)?;
    match col {
        ColumnData::String(sc) => {
            let s = sc.get_str(row);
            if s.is_empty() {
                Some(FilterOp::StrIsEmpty)
            } else {
                Some(FilterOp::StrEq(s.to_string()))
            }
        }
        ColumnData::Numeric(nc) => {
            let tag = nc.missing_tag(row);
            if tag == MISSING_VALID {
                Some(FilterOp::NumEq(nc.values[row]))
            } else {
                Some(FilterOp::NumEqTaggedMissing(tag))
            }
        }
    }
}

/// Build an inclusive (`in` / `=`) or exclusive (`not in` / `!=`) FilterOp from multiple rows in `col_idx`.
pub fn build_multi_cell_filter(
    table: &ColumnarTable,
    rows: &[usize],
    col_idx: usize,
    negate: bool,
) -> Option<FilterOp> {
    if rows.is_empty() {
        return None;
    }
    let col = table.columns.get(col_idx)?;
    match col {
        ColumnData::String(sc) => {
            let mut uniq: Vec<String> = Vec::new();
            for &r in rows {
                if r < sc.len() {
                    let s = sc.get_str(r).to_string();
                    if !uniq.contains(&s) {
                        uniq.push(s);
                    }
                }
            }
            if uniq.is_empty() {
                return None;
            }
            if uniq.len() == 1 {
                let s = uniq.remove(0);
                if s.is_empty() {
                    Some(if negate {
                        FilterOp::StrIsNotEmpty
                    } else {
                        FilterOp::StrIsEmpty
                    })
                } else {
                    Some(if negate {
                        FilterOp::StrNeq(s)
                    } else {
                        FilterOp::StrEq(s)
                    })
                }
            } else {
                Some(if negate {
                    FilterOp::StrNotIn(uniq)
                } else {
                    FilterOp::StrIn(uniq)
                })
            }
        }
        ColumnData::Numeric(nc) => {
            let mut values: Vec<f64> = Vec::new();
            let mut missing_tags: Vec<u8> = Vec::new();
            for &r in rows {
                if r < nc.len() {
                    let tag = nc.missing_tag(r);
                    if tag == MISSING_VALID {
                        let v = nc.values[r];
                        if !values.iter().any(|x| (x - v).abs() <= 1e-11) {
                            values.push(v);
                        }
                    } else if !missing_tags.contains(&tag) {
                        missing_tags.push(tag);
                    }
                }
            }
            let total = values.len() + missing_tags.len();
            if total == 0 {
                return None;
            }
            if total == 1 {
                if let Some(&v) = values.first() {
                    Some(if negate {
                        FilterOp::NumNeq(v)
                    } else {
                        FilterOp::NumEq(v)
                    })
                } else {
                    let tag = missing_tags[0];
                    Some(if negate {
                        FilterOp::NumNotIn {
                            values: Vec::new(),
                            missing_tags: vec![tag],
                        }
                    } else {
                        FilterOp::NumEqTaggedMissing(tag)
                    })
                }
            } else {
                Some(if negate {
                    FilterOp::NumNotIn {
                        values,
                        missing_tags,
                    }
                } else {
                    FilterOp::NumIn {
                        values,
                        missing_tags,
                    }
                })
            }
        }
    }
}
