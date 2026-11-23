use crate::column::{ColumnData, ColumnarTable, StringColumn, MISSING_SYSTEM, MISSING_VALID};
use crate::sas_format::{format_missing_tag, format_numeric_value};
use ahash::AHashMap;
use rayon::prelude::*;

const MAX_FREQ_ROWS_DISPLAY: usize = 500;

#[derive(Debug, Clone)]
pub struct FreqEntry {
    pub value: String,
    pub is_missing: bool,
    pub raw_numeric: Option<f64>,
    pub missing_tag: u8,
    pub count: usize,
    pub percent: f64,
}

#[derive(Debug, Clone)]
pub struct StringStats {
    pub total_rows: usize,
    pub valid_count: usize,
    pub missing_count: usize,
    pub distinct_count: usize,
    pub frequencies: Vec<FreqEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct NumericStats {
    pub total_rows: usize,
    pub valid_count: usize,
    pub missing_count: usize,
    pub tagged_missing_counts: Vec<(String, usize)>,
    pub distinct_count: usize,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    pub min: Option<f64>,
    pub q1: Option<f64>,
    pub median: Option<f64>,
    pub q3: Option<f64>,
    pub max: Option<f64>,
    pub sum: Option<f64>,
    pub low_card_freqs: Option<Vec<FreqEntry>>,
}

#[derive(Debug, Clone)]
pub enum ColumnStats {
    String(StringStats),
    Numeric(NumericStats),
}

/// Compute comprehensive statistics for `col_idx` over the given `visible_rows` (currently filtered data).
pub fn compute_column_stats(
    table: &ColumnarTable,
    visible_rows: &[u32],
    col_idx: usize,
) -> Option<ColumnStats> {
    let col = table.columns.get(col_idx)?;
    let meta = table.variables.get(col_idx)?;
    let total = visible_rows.len();

    match col {
        ColumnData::String(sc) => {
            let mut entries: Vec<FreqEntry> = Vec::new();
            let mut missing_count = 0usize;
            let mut distinct_non_empty = 0usize;

            match sc {
                StringColumn::Dict { dict, ids, .. } => {
                    let dict_len = dict.len();
                    let counts: Vec<usize> = if total >= 32_768 && dict_len <= 65_536 {
                        visible_rows
                            .par_chunks(16_384)
                            .fold(
                                || vec![0usize; dict_len],
                                |mut local, chunk| {
                                    for &r in chunk {
                                        local[ids[r as usize] as usize] += 1;
                                    }
                                    local
                                },
                            )
                            .reduce(
                                || vec![0usize; dict_len],
                                |mut a, b| {
                                    for (x, y) in a.iter_mut().zip(b.iter()) {
                                        *x += *y;
                                    }
                                    a
                                },
                            )
                    } else {
                        let mut c = vec![0usize; dict_len];
                        for &r in visible_rows {
                            c[ids[r as usize] as usize] += 1;
                        }
                        c
                    };

                    for (id, &cnt) in counts.iter().enumerate() {
                        if cnt == 0 {
                            continue;
                        }
                        let val = &dict[id];
                        let is_miss = val.is_empty();
                        if is_miss {
                            missing_count = cnt;
                        } else {
                            distinct_non_empty += 1;
                        }
                        entries.push(FreqEntry {
                            value: if is_miss {
                                "(空)".to_string()
                            } else {
                                val.clone()
                            },
                            is_missing: is_miss,
                            raw_numeric: None,
                            missing_tag: 0,
                            count: cnt,
                            percent: if total > 0 {
                                (cnt as f64) * 100.0 / (total as f64)
                            } else {
                                0.0
                            },
                        });
                    }
                }
                StringColumn::Flat { .. } => {
                    let mut freq_map: AHashMap<&str, usize> = AHashMap::new();
                    for &r in visible_rows {
                        let s = sc.get_str(r as usize);
                        *freq_map.entry(s).or_insert(0) += 1;
                    }
                    for (val, cnt) in freq_map {
                        let is_miss = val.is_empty();
                        if is_miss {
                            missing_count = cnt;
                        } else {
                            distinct_non_empty += 1;
                        }
                        entries.push(FreqEntry {
                            value: if is_miss {
                                "(空)".to_string()
                            } else {
                                val.to_string()
                            },
                            is_missing: is_miss,
                            raw_numeric: None,
                            missing_tag: 0,
                            count: cnt,
                            percent: if total > 0 {
                                (cnt as f64) * 100.0 / (total as f64)
                            } else {
                                0.0
                            },
                        });
                    }
                }
            }

            entries.sort_unstable_by(|a, b| {
                b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value))
            });
            let truncated = entries.len() > MAX_FREQ_ROWS_DISPLAY;
            if truncated {
                entries.truncate(MAX_FREQ_ROWS_DISPLAY);
            }

            Some(ColumnStats::String(StringStats {
                total_rows: total,
                valid_count: total.saturating_sub(missing_count),
                missing_count,
                distinct_count: distinct_non_empty,
                frequencies: entries,
                truncated,
            }))
        }

        ColumnData::Numeric(nc) => {
            let mut valid_vals = Vec::with_capacity(total);
            let mut missing_counts = [0usize; 256];
            let mut missing_total = 0usize;
            let mut sum = 0.0f64;

            for &r in visible_rows {
                let idx = r as usize;
                let tag = nc.missing[idx];
                if tag == MISSING_VALID {
                    let v = nc.values[idx];
                    if v.is_finite() {
                        valid_vals.push(v);
                        sum += v;
                    } else {
                        missing_total += 1;
                        missing_counts[MISSING_SYSTEM as usize] += 1;
                    }
                } else {
                    missing_total += 1;
                    missing_counts[tag as usize] += 1;
                }
            }

            let mut tagged_missing_counts = Vec::new();
            for (byte_val, &cnt) in missing_counts.iter().enumerate() {
                if cnt > 0 && byte_val != MISSING_VALID as usize {
                    tagged_missing_counts.push((format_missing_tag(byte_val as u8), cnt));
                }
            }

            let valid_count = valid_vals.len();
            if valid_count == 0 {
                return Some(ColumnStats::Numeric(NumericStats {
                    total_rows: total,
                    valid_count: 0,
                    missing_count: missing_total,
                    tagged_missing_counts,
                    distinct_count: 0,
                    mean: None,
                    std_dev: None,
                    min: None,
                    q1: None,
                    median: None,
                    q3: None,
                    max: None,
                    sum: None,
                    low_card_freqs: None,
                }));
            }

            let mean = sum / (valid_count as f64);
            let std_dev = if valid_count >= 2 {
                let sum_sq: f64 = if valid_count >= 32_768 {
                    valid_vals
                        .par_iter()
                        .map(|&v| {
                            let d = v - mean;
                            d * d
                        })
                        .sum()
                } else {
                    valid_vals
                        .iter()
                        .map(|&v| {
                            let d = v - mean;
                            d * d
                        })
                        .sum()
                };
                Some((sum_sq / ((valid_count - 1) as f64)).sqrt())
            } else {
                None
            };

            if valid_count >= 16_384 {
                valid_vals.par_sort_unstable_by(|a, b| a.total_cmp(b));
            } else {
                valid_vals.sort_unstable_by(|a, b| a.total_cmp(b));
            }

            let min = valid_vals[0];
            let max = valid_vals[valid_count - 1];
            let q1 = quantile_sorted(&valid_vals, 0.25);
            let median = quantile_sorted(&valid_vals, 0.50);
            let q3 = quantile_sorted(&valid_vals, 0.75);

            // Count distinct values in O(n) single pass over sorted array
            let mut distinct_count = 1usize;
            let mut prev = valid_vals[0];
            for &v in &valid_vals[1..] {
                if v != prev {
                    distinct_count += 1;
                    prev = v;
                }
            }

            // If low-cardinality numeric (e.g. flag 0/1, visits, grades), build helpful frequency breakdown
            let low_card_freqs = if distinct_count <= 25 {
                let mut freqs = Vec::with_capacity(distinct_count + tagged_missing_counts.len());
                let mut cur_val = valid_vals[0];
                let mut cur_cnt = 1usize;
                for &v in &valid_vals[1..] {
                    if v == cur_val {
                        cur_cnt += 1;
                    } else {
                        freqs.push(FreqEntry {
                            value: format_numeric_value(cur_val, meta),
                            is_missing: false,
                            raw_numeric: Some(cur_val),
                            missing_tag: 0,
                            count: cur_cnt,
                            percent: (cur_cnt as f64) * 100.0 / (total as f64),
                        });
                        cur_val = v;
                        cur_cnt = 1;
                    }
                }
                freqs.push(FreqEntry {
                    value: format_numeric_value(cur_val, meta),
                    is_missing: false,
                    raw_numeric: Some(cur_val),
                    missing_tag: 0,
                    count: cur_cnt,
                    percent: (cur_cnt as f64) * 100.0 / (total as f64),
                });
                for (byte_val, &cnt) in missing_counts.iter().enumerate() {
                    if cnt > 0 && byte_val != MISSING_VALID as usize {
                        freqs.push(FreqEntry {
                            value: format_missing_tag(byte_val as u8),
                            is_missing: true,
                            raw_numeric: None,
                            missing_tag: byte_val as u8,
                            count: cnt,
                            percent: (cnt as f64) * 100.0 / (total as f64),
                        });
                    }
                }
                freqs.sort_unstable_by(|a, b| b.count.cmp(&a.count));
                Some(freqs)
            } else {
                None
            };

            Some(ColumnStats::Numeric(NumericStats {
                total_rows: total,
                valid_count,
                missing_count: missing_total,
                tagged_missing_counts,
                distinct_count,
                mean: Some(mean),
                std_dev,
                min: Some(min),
                q1: Some(q1),
                median: Some(median),
                q3: Some(q3),
                max: Some(max),
                sum: Some(sum),
                low_card_freqs,
            }))
        }
    }
}

/// Linear-interpolated quantile on a non-empty sorted slice.
fn quantile_sorted(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let pos = p * ((n - 1) as f64);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - (lo as f64);
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}
