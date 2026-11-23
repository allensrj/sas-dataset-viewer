use crate::sas_format::{format_missing_tag, format_numeric_value};
use crate::types::{DatasetMetadata, VariableMeta};
use ahash::AHashMap;
use std::cmp::Ordering;

/// Missing tag encoding for numeric cells:
/// 0 = valid f64 value
/// 1 = system missing (`.`)
/// b'A'..=b'Z' | b'_' = SAS tagged missing (`.A`..`.Z`, `._`)
pub const MISSING_VALID: u8 = 0;
pub const MISSING_SYSTEM: u8 = 1;

#[derive(Debug, Clone)]
pub struct NumericColumn {
    pub values: Vec<f64>,
    pub missing: Vec<u8>,
}

impl NumericColumn {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            values: Vec::with_capacity(cap),
            missing: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[inline]
    pub fn push_valid(&mut self, val: f64) {
        self.values.push(val);
        self.missing.push(MISSING_VALID);
    }

    #[inline]
    pub fn push_system_missing(&mut self) {
        self.values.push(f64::NAN);
        self.missing.push(MISSING_SYSTEM);
    }

    #[inline]
    pub fn push_tagged_missing(&mut self, tag: u8) {
        let normalized = if tag.is_ascii_alphabetic() {
            tag.to_ascii_uppercase()
        } else if tag == b'_' {
            b'_'
        } else {
            MISSING_SYSTEM
        };
        self.values.push(f64::NAN);
        self.missing.push(normalized);
    }

    #[inline]
    pub fn set_at(&mut self, row: usize, val: f64, miss_tag: u8) {
        if row >= self.values.len() {
            self.values.resize(row + 1, f64::NAN);
            self.missing.resize(row + 1, MISSING_SYSTEM);
        }
        self.values[row] = val;
        self.missing[row] = miss_tag;
    }

    #[inline]
    pub fn missing_tag(&self, row: usize) -> u8 {
        self.missing[row]
    }

    #[inline]
    pub fn is_missing(&self, row: usize) -> bool {
        self.missing[row] != MISSING_VALID
    }

    #[inline]
    pub fn get_valid(&self, row: usize) -> Option<f64> {
        if self.missing[row] == MISSING_VALID {
            Some(self.values[row])
        } else {
            None
        }
    }

    pub fn format_cell(&self, row: usize, meta: &VariableMeta) -> String {
        let tag = self.missing[row];
        if tag != MISSING_VALID {
            format_missing_tag(tag)
        } else {
            format_numeric_value(self.values[row], meta)
        }
    }

    /// SAS sort ordering: ._ < . < .A < ... < .Z < valid numbers (ascending)
    #[inline]
    pub fn compare_rows(&self, a: usize, b: usize) -> Ordering {
        let ma = self.missing[a];
        let mb = self.missing[b];
        match (ma == MISSING_VALID, mb == MISSING_VALID) {
            (true, true) => self.values[a].total_cmp(&self.values[b]),
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            (false, false) => missing_rank(ma).cmp(&missing_rank(mb)),
        }
    }

    pub fn approx_memory_bytes(&self) -> usize {
        self.values.capacity() * std::mem::size_of::<f64>()
            + self.missing.capacity() * std::mem::size_of::<u8>()
    }
}

#[inline]
fn missing_rank(tag: u8) -> u8 {
    match tag {
        b'_' => 0,
        MISSING_SYSTEM => 1,
        b'A'..=b'Z' => 2 + (tag - b'A'),
        _ => 1,
    }
}

/// Adaptive string column:
/// - Starts as Dictionary-encoded (huge RAM savings & O(1) filtering/stats for SAS categorical cols)
/// - Automatically converts to Arrow-style Flat byte buffer when cardinality exceeds 40%
#[derive(Debug, Clone)]
pub enum StringColumn {
    Dict {
        dict: Vec<String>,
        map: AHashMap<String, u32>,
        ids: Vec<u32>,
    },
    Flat {
        data: Vec<u8>,
        offsets: Vec<u32>, // len = rows + 1
    },
}

impl StringColumn {
    pub fn with_capacity(cap: usize) -> Self {
        let mut dict = Vec::with_capacity(64);
        let mut map = AHashMap::with_capacity(64);
        dict.push(String::new());
        map.insert(String::new(), 0);
        Self::Dict {
            dict,
            map,
            ids: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Dict { ids, .. } => ids.len(),
            Self::Flat { offsets, .. } => offsets.len().saturating_sub(1),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push_str(&mut self, s: &str) {
        // Trim SAS trailing ASCII spaces
        let s = s.trim_end_matches(' ');
        match self {
            Self::Dict { dict, map, ids } => {
                let id = if s.is_empty() {
                    0
                } else if let Some(&existing) = map.get(s) {
                    existing
                } else {
                    let next_id = dict.len() as u32;
                    let owned = s.to_owned();
                    dict.push(owned.clone());
                    map.insert(owned, next_id);
                    next_id
                };
                ids.push(id);

                // Check whether to convert to Flat buffer for high-cardinality columns
                if ids.len() >= 8192 && (ids.len() & 4095) == 0 {
                    if dict.len() > 4096 && dict.len() * 10 > ids.len() * 4 {
                        self.convert_to_flat();
                    }
                }
            }
            Self::Flat { data, offsets } => {
                data.extend_from_slice(s.as_bytes());
                offsets.push(data.len() as u32);
            }
        }
    }

    pub fn ensure_len(&mut self, target_len: usize) {
        while self.len() < target_len {
            self.push_str("");
        }
    }

    fn convert_to_flat(&mut self) {
        let Self::Dict { dict, ids, .. } = self else {
            return;
        };
        let mut total_bytes = 0usize;
        for &id in ids.iter() {
            total_bytes += dict[id as usize].len();
        }
        let mut data = Vec::with_capacity(total_bytes);
        let mut offsets = Vec::with_capacity(ids.len() + 1);
        offsets.push(0);
        for &id in ids.iter() {
            data.extend_from_slice(dict[id as usize].as_bytes());
            offsets.push(data.len() as u32);
        }
        *self = Self::Flat { data, offsets };
    }

    /// Release the lookup HashMap once loading finishes to save memory, keeping fast Vec access.
    pub fn shrink_after_load(&mut self) {
        if let Self::Dict { map, .. } = self {
            map.clear();
            map.shrink_to_fit();
        }
    }

    #[inline]
    pub fn get_str(&self, row: usize) -> &str {
        match self {
            Self::Dict { dict, ids, .. } => &dict[ids[row] as usize],
            Self::Flat { data, offsets } => {
                let start = offsets[row] as usize;
                let end = offsets[row + 1] as usize;
                // SAFETY: only valid UTF-8 &str slices are pushed into `data`.
                unsafe { std::str::from_utf8_unchecked(&data[start..end]) }
            }
        }
    }

    #[inline]
    pub fn compare_rows(&self, a: usize, b: usize) -> Ordering {
        self.get_str(a).cmp(self.get_str(b))
    }

    /// Estimate max character display width (counting CJK wide chars as 2) very quickly.
    pub fn estimate_max_display_chars(&self, sample_limit: usize) -> usize {
        match self {
            Self::Dict { dict, .. } => dict
                .iter()
                .map(|s| display_char_units(s))
                .max()
                .unwrap_or(0),
            Self::Flat { .. } => {
                let n = self.len();
                if n == 0 {
                    return 0;
                }
                let mut max_w = 0usize;
                if n <= sample_limit {
                    for i in 0..n {
                        max_w = max_w.max(display_char_units(self.get_str(i)));
                    }
                } else {
                    let step = (n / sample_limit).max(1);
                    let mut i = 0;
                    while i < n {
                        max_w = max_w.max(display_char_units(self.get_str(i)));
                        i += step;
                    }
                    max_w = max_w.max(display_char_units(self.get_str(n - 1)));
                }
                max_w
            }
        }
    }

    pub fn approx_memory_bytes(&self) -> usize {
        match self {
            Self::Dict { dict, map, ids } => {
                let dict_bytes: usize = dict
                    .iter()
                    .map(|s| s.capacity() + std::mem::size_of::<String>())
                    .sum();
                let map_bytes = map.capacity() * (std::mem::size_of::<String>() + 8);
                dict_bytes + map_bytes + ids.capacity() * std::mem::size_of::<u32>()
            }
            Self::Flat { data, offsets } => {
                data.capacity() + offsets.capacity() * std::mem::size_of::<u32>()
            }
        }
    }
}

/// Approximate terminal/UI character width (CJK chars count as 2, ASCII as 1).
#[inline]
pub fn display_char_units(s: &str) -> usize {
    s.chars()
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum()
}

#[derive(Debug, Clone)]
pub enum ColumnData {
    Numeric(NumericColumn),
    String(StringColumn),
}

impl ColumnData {
    pub fn len(&self) -> usize {
        match self {
            Self::Numeric(c) => c.len(),
            Self::String(c) => c.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn format_cell(&self, row: usize, meta: &VariableMeta) -> String {
        match self {
            Self::Numeric(c) => c.format_cell(row, meta),
            Self::String(c) => c.get_str(row).to_string(),
        }
    }

    pub fn compare_rows(&self, a: usize, b: usize) -> Ordering {
        match self {
            Self::Numeric(c) => c.compare_rows(a, b),
            Self::String(c) => c.compare_rows(a, b),
        }
    }

    pub fn approx_memory_bytes(&self) -> usize {
        match self {
            Self::Numeric(c) => c.approx_memory_bytes(),
            Self::String(c) => c.approx_memory_bytes(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColumnarTable {
    pub metadata: DatasetMetadata,
    pub variables: Vec<VariableMeta>,
    pub columns: Vec<ColumnData>,
    pub row_count: usize,
    pub load_elapsed_ms: u64,
    pub is_complete: bool,
}

impl ColumnarTable {
    pub fn format_cell(&self, row: usize, col: usize) -> String {
        self.columns[col].format_cell(row, &self.variables[col])
    }

    pub fn approx_memory_bytes(&self) -> usize {
        self.columns.iter().map(|c| c.approx_memory_bytes()).sum()
    }

    /// Estimate optimal column pixel width for a given column.
    /// `full_scan`: if true, inspects dictionary or large sample for batch auto-fit.
    pub fn estimate_column_width_px(
        &self,
        col: usize,
        min_px: f32,
        max_px: f32,
        full_scan: bool,
    ) -> f32 {
        let meta = &self.variables[col];
        let header_units = display_char_units(&meta.name).max(
            display_char_units(&meta.label).min(if full_scan { 28 } else { 18 }),
        );
        let sample_limit = if full_scan { 4096 } else { 256 };

        let data_units = match &self.columns[col] {
            ColumnData::String(sc) => sc.estimate_max_display_chars(sample_limit),
            ColumnData::Numeric(nc) => {
                let n = nc.len();
                if n == 0 {
                    4
                } else {
                    let step = (n / sample_limit).max(1);
                    let mut max_u = 0usize;
                    let mut i = 0;
                    while i < n {
                        let s = nc.format_cell(i, meta);
                        max_u = max_u.max(display_char_units(&s));
                        i += step;
                    }
                    max_u
                }
            }
        };

        let units = header_units.max(data_units);
        // ~7.6 px per ASCII unit + padding for sort badge & cell margins
        let px = (units as f32) * 7.8 + 30.0;
        px.clamp(min_px, max_px)
    }
}
