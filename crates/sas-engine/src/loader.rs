use crate::column::{ColumnData, ColumnarTable, NumericColumn, StringColumn, MISSING_SYSTEM};
use crate::sas_format::parse_sas_format;
use crate::types::{DatasetMetadata, EncodingChoice, SasFormatKind, VarType, VariableMeta};
use readstat_sys::*;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;

const FIRST_BATCH_ROWS: usize = 5_000;
const PROGRESS_STEP_ROWS: usize = 50_000;

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("不支持的文件格式（仅支持 .sas7bdat 和 .xpt）: {0}")]
    UnsupportedFormat(String),
    #[error("文件路径包含非法字符: {0}")]
    InvalidPath(String),
    #[error("无法读取文件元信息: {0}")]
    Io(#[from] std::io::Error),
    #[error("ReadStat 解析错误 (code {code}): {message}")]
    ReadStat { code: i32, message: String },
    #[error("加载已取消")]
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum LoadEvent {
    /// Emitted rapidly after header + first 5,000 rows so the UI displays data almost immediately.
    FirstBatch(Arc<ColumnarTable>),
    /// Emitted periodically while streaming remaining rows of a large file.
    Progress { rows_loaded: usize, fraction: f32 },
    /// Emitted when all rows are fully loaded into columnar memory.
    Complete(Arc<ColumnarTable>),
    /// Emitted on error.
    Error(String),
}

struct ParseState<F>
where
    F: FnMut(LoadEvent),
{
    path: PathBuf,
    format_kind: SasFormatKind,
    encoding_choice: EncodingChoice,
    file_size_bytes: u64,
    start_time: Instant,
    metadata: Option<DatasetMetadata>,
    variables: Vec<VariableMeta>,
    columns: Vec<ColumnData>,
    rows_seen: usize,
    last_obs_index: Option<usize>,
    first_batch_emitted: bool,
    last_progress_rows: usize,
    cancel: Arc<AtomicBool>,
    emit: F,
}

impl<F: FnMut(LoadEvent)> ParseState<F> {
    fn build_snapshot(&self, is_complete: bool) -> ColumnarTable {
        let mut metadata = self.metadata.clone().unwrap_or_else(|| DatasetMetadata {
            path: self.path.clone(),
            format_kind: self.format_kind,
            table_name: self
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("DATASET")
                .to_string(),
            file_label: String::new(),
            file_encoding: self
                .encoding_choice
                .iconv_name()
                .unwrap_or("UTF-8")
                .to_string(),
            encoding_choice: self.encoding_choice,
            declared_row_count: self.rows_seen as i64,
            is_64bit: true,
            compression: "None".to_string(),
            file_size_bytes: self.file_size_bytes,
        });
        if metadata.table_name.is_empty() {
            metadata.table_name = self
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("DATASET")
                .to_string();
        }
        ColumnarTable {
            metadata,
            variables: self.variables.clone(),
            columns: self.columns.clone(),
            row_count: self.rows_seen,
            load_elapsed_ms: self.start_time.elapsed().as_millis() as u64,
            is_complete,
        }
    }

    fn finalize_into_table(mut self) -> ColumnarTable {
        for col in &mut self.columns {
            match col {
                ColumnData::Numeric(nc) => {
                    while nc.len() < self.rows_seen {
                        nc.push_system_missing();
                    }
                }
                ColumnData::String(sc) => {
                    sc.ensure_len(self.rows_seen);
                    sc.shrink_after_load();
                }
            }
        }
        let mut metadata = self.metadata.unwrap_or_else(|| DatasetMetadata {
            path: self.path.clone(),
            format_kind: self.format_kind,
            table_name: String::new(),
            file_label: String::new(),
            file_encoding: self
                .encoding_choice
                .iconv_name()
                .unwrap_or("UTF-8")
                .to_string(),
            encoding_choice: self.encoding_choice,
            declared_row_count: self.rows_seen as i64,
            is_64bit: true,
            compression: "None".to_string(),
            file_size_bytes: self.file_size_bytes,
        });
        if metadata.table_name.is_empty() {
            metadata.table_name = self
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("DATASET")
                .to_string();
        }
        ColumnarTable {
            metadata,
            variables: self.variables,
            columns: self.columns,
            row_count: self.rows_seen,
            load_elapsed_ms: self.start_time.elapsed().as_millis() as u64,
            is_complete: true,
        }
    }
}

unsafe fn cstr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

unsafe extern "C" fn on_metadata_trampoline<F: FnMut(LoadEvent)>(
    row_count: i64,
    var_count: i64,
    table_name: *const c_char,
    file_label: *const c_char,
    file_encoding: *const c_char,
    is64bit: c_int,
    compression: c_int,
    ctx: *mut c_void,
) -> c_int {
    let state = &mut *(ctx as *mut ParseState<F>);
    if state.cancel.load(Ordering::Relaxed) {
        return READSTAT_HANDLER_ABORT;
    }
    let comp_name = match compression {
        1 => "RLE (Rows)",
        2 => "RDC (Binary)",
        _ => "None",
    };
    let enc = cstr_to_string(file_encoding);
    let effective_enc = if !enc.is_empty() {
        enc
    } else {
        state
            .encoding_choice
            .iconv_name()
            .unwrap_or("UTF-8")
            .to_string()
    };
    state.metadata = Some(DatasetMetadata {
        path: state.path.clone(),
        format_kind: state.format_kind,
        table_name: cstr_to_string(table_name),
        file_label: cstr_to_string(file_label),
        file_encoding: effective_enc,
        encoding_choice: state.encoding_choice,
        declared_row_count: row_count,
        is_64bit: is64bit != 0,
        compression: comp_name.to_string(),
        file_size_bytes: state.file_size_bytes,
    });
    if var_count > 0 {
        state.variables.reserve(var_count as usize);
        state.columns.reserve(var_count as usize);
    }
    READSTAT_HANDLER_OK
}

unsafe extern "C" fn on_variable_trampoline<F: FnMut(LoadEvent)>(
    index: c_int,
    name: *const c_char,
    label: *const c_char,
    format: *const c_char,
    is_string: c_int,
    storage_width: usize,
    display_width: c_int,
    ctx: *mut c_void,
) -> c_int {
    let state = &mut *(ctx as *mut ParseState<F>);
    if state.cancel.load(Ordering::Relaxed) {
        return READSTAT_HANDLER_ABORT;
    }

    let name_str = cstr_to_string(name);
    let label_str = cstr_to_string(label);
    let fmt_str = cstr_to_string(format);
    let var_type = if is_string != 0 {
        VarType::String
    } else {
        VarType::Numeric
    };
    let (date_kind, decimal_places) = if var_type == VarType::Numeric {
        parse_sas_format(&fmt_str)
    } else {
        (crate::types::SasDateKind::None, None)
    };

    let meta = VariableMeta {
        index: index as usize,
        name: name_str,
        label: label_str,
        format: fmt_str,
        var_type,
        storage_width,
        display_width,
        date_kind,
        decimal_places,
    };

    let init_cap = state
        .metadata
        .as_ref()
        .map(|m| {
            if m.declared_row_count > 0 {
                (m.declared_row_count as usize).min(2_000_000)
            } else {
                1024
            }
        })
        .unwrap_or(1024);

    let col = match var_type {
        VarType::Numeric => ColumnData::Numeric(NumericColumn::with_capacity(init_cap)),
        VarType::String => ColumnData::String(StringColumn::with_capacity(init_cap)),
    };

    state.variables.push(meta);
    state.columns.push(col);
    READSTAT_HANDLER_OK
}

unsafe extern "C" fn on_value_trampoline<F: FnMut(LoadEvent)>(
    obs_index: c_int,
    var_index: c_int,
    is_string: c_int,
    missing_kind: c_int,
    tag: c_char,
    num_val: c_double,
    str_val: *const c_char,
    ctx: *mut c_void,
) -> c_int {
    let state = &mut *(ctx as *mut ParseState<F>);
    let obs = obs_index as usize;
    let col_idx = var_index as usize;

    // Check cancellation & batch emission at row boundaries
    if state.last_obs_index != Some(obs) {
        if let Some(prev_obs) = state.last_obs_index {
            let completed_rows = prev_obs + 1;
            if completed_rows > state.rows_seen {
                state.rows_seen = completed_rows;
            }
            if !state.first_batch_emitted && state.rows_seen >= FIRST_BATCH_ROWS {
                // Ensure all columns are aligned before snapshotting
                for col in &mut state.columns {
                    match col {
                        ColumnData::Numeric(nc) => {
                            while nc.len() < state.rows_seen {
                                nc.push_system_missing();
                            }
                        }
                        ColumnData::String(sc) => sc.ensure_len(state.rows_seen),
                    }
                }
                let snap = Arc::new(state.build_snapshot(false));
                (state.emit)(LoadEvent::FirstBatch(snap));
                state.first_batch_emitted = true;
            }
            if state.rows_seen - state.last_progress_rows >= PROGRESS_STEP_ROWS {
                if state.cancel.load(Ordering::Relaxed) {
                    return READSTAT_HANDLER_ABORT;
                }
                state.last_progress_rows = state.rows_seen;
                let total = state
                    .metadata
                    .as_ref()
                    .map(|m| m.declared_row_count)
                    .unwrap_or(-1);
                let frac = if total > 0 {
                    (state.rows_seen as f32 / total as f32).clamp(0.0, 0.99)
                } else {
                    -1.0
                };
                (state.emit)(LoadEvent::Progress {
                    rows_loaded: state.rows_seen,
                    fraction: frac,
                });
            }
        }
        state.last_obs_index = Some(obs);
    }

    if col_idx >= state.columns.len() {
        return READSTAT_HANDLER_OK;
    }

    match &mut state.columns[col_idx] {
        ColumnData::Numeric(nc) => {
            if obs == nc.len() {
                match missing_kind {
                    1 => nc.push_system_missing(),
                    2 => nc.push_tagged_missing(tag as u8),
                    _ => {
                        if num_val.is_nan() {
                            nc.push_system_missing();
                        } else {
                            nc.push_valid(num_val);
                        }
                    }
                }
            } else {
                let (val, mtag) = match missing_kind {
                    1 => (f64::NAN, MISSING_SYSTEM),
                    2 => (f64::NAN, (tag as u8).to_ascii_uppercase()),
                    _ if num_val.is_nan() => (f64::NAN, MISSING_SYSTEM),
                    _ => (num_val, 0),
                };
                nc.set_at(obs, val, mtag);
            }
        }
        ColumnData::String(sc) => {
            if sc.len() < obs {
                sc.ensure_len(obs);
            }
            if missing_kind != 0 || is_string == 0 || str_val.is_null() {
                sc.push_str("");
            } else {
                let bytes = CStr::from_ptr(str_val).to_bytes();
                match std::str::from_utf8(bytes) {
                    Ok(s) => sc.push_str(s),
                    Err(_) => {
                        let lossy = String::from_utf8_lossy(bytes);
                        sc.push_str(&lossy);
                    }
                }
            }
        }
    }

    READSTAT_HANDLER_OK
}

/// Stream-load a `.sas7bdat` or `.xpt` dataset, emitting `FirstBatch` quickly and `Complete` when finished.
pub fn load_dataset_streaming<F>(
    path: &Path,
    encoding: EncodingChoice,
    cancel: Arc<AtomicBool>,
    emit: F,
) -> Result<Arc<ColumnarTable>, LoadError>
where
    F: FnMut(LoadEvent),
{
    let format_kind = SasFormatKind::from_path(path)
        .ok_or_else(|| LoadError::UnsupportedFormat(path.display().to_string()))?;
    let file_size_bytes = std::fs::metadata(path)?.len();
    let path_str = path
        .to_str()
        .ok_or_else(|| LoadError::InvalidPath(path.display().to_string()))?;
    let c_path =
        CString::new(path_str).map_err(|_| LoadError::InvalidPath(path_str.to_string()))?;

    let c_enc = encoding.iconv_name().and_then(|e| CString::new(e).ok());
    let enc_ptr = c_enc.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());

    let fmt_code = match format_kind {
        SasFormatKind::Sas7bdat => FORMAT_SAS7BDAT,
        SasFormatKind::Xport => FORMAT_XPORT,
    };

    let mut state = ParseState {
        path: path.to_path_buf(),
        format_kind,
        encoding_choice: encoding,
        file_size_bytes,
        start_time: Instant::now(),
        metadata: None,
        variables: Vec::new(),
        columns: Vec::new(),
        rows_seen: 0,
        last_obs_index: None,
        first_batch_emitted: false,
        last_progress_rows: 0,
        cancel: cancel.clone(),
        emit,
    };

    let mut err_buf = [0 as c_char; 512];
    let rc = unsafe {
        rs_bridge_parse_file(
            c_path.as_ptr(),
            fmt_code,
            enc_ptr,
            0,
            0,
            Some(on_metadata_trampoline::<F>),
            Some(on_variable_trampoline::<F>),
            Some(on_value_trampoline::<F>),
            None,
            (&mut state as *mut ParseState<F>) as *mut c_void,
            err_buf.as_mut_ptr(),
            err_buf.len(),
        )
    };

    if rc == READSTAT_ERROR_USER_ABORT || cancel.load(Ordering::Relaxed) {
        return Err(LoadError::Cancelled);
    }

    if rc != READSTAT_OK {
        let msg = unsafe { cstr_to_string(err_buf.as_ptr()) };
        return Err(LoadError::ReadStat {
            code: rc,
            message: msg,
        });
    }

    if let Some(last) = state.last_obs_index {
        state.rows_seen = state.rows_seen.max(last + 1);
    }

    // Grab emit before finalizing
    let mut emit_fn = state.emit;
    // Reconstruct state with dummy closure for finalize
    let finalize_state = ParseState {
        path: state.path,
        format_kind: state.format_kind,
        encoding_choice: state.encoding_choice,
        file_size_bytes: state.file_size_bytes,
        start_time: state.start_time,
        metadata: state.metadata,
        variables: state.variables,
        columns: state.columns,
        rows_seen: state.rows_seen,
        last_obs_index: state.last_obs_index,
        first_batch_emitted: state.first_batch_emitted,
        last_progress_rows: state.last_progress_rows,
        cancel: state.cancel,
        emit: |_| {},
    };
    let table = Arc::new(finalize_state.finalize_into_table());
    emit_fn(LoadEvent::Complete(table.clone()));
    Ok(table)
}

/// Synchronous load convenience wrapper.
pub fn load_dataset_sync(
    path: &Path,
    encoding: EncodingChoice,
) -> Result<Arc<ColumnarTable>, LoadError> {
    let cancel = Arc::new(AtomicBool::new(false));
    load_dataset_streaming(path, encoding, cancel, |_| {})
}
