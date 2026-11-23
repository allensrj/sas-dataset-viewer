pub mod win_iconv;

use std::ffi::{c_char, c_double, c_int, c_long, c_void};

pub const READSTAT_OK: c_int = 0;
pub const READSTAT_ERROR_USER_ABORT: c_int = 4;
pub const READSTAT_HANDLER_OK: c_int = 0;
pub const READSTAT_HANDLER_ABORT: c_int = 1;

pub const FORMAT_SAS7BDAT: c_int = 0;
pub const FORMAT_XPORT: c_int = 1;

pub type MetadataCb = Option<
    unsafe extern "C" fn(
        row_count: i64,
        var_count: i64,
        table_name: *const c_char,
        file_label: *const c_char,
        file_encoding: *const c_char,
        is64bit: c_int,
        compression: c_int,
        ctx: *mut c_void,
    ) -> c_int,
>;

pub type VariableCb = Option<
    unsafe extern "C" fn(
        index: c_int,
        name: *const c_char,
        label: *const c_char,
        format: *const c_char,
        is_string: c_int,
        storage_width: usize,
        display_width: c_int,
        ctx: *mut c_void,
    ) -> c_int,
>;

pub type ValueCb = Option<
    unsafe extern "C" fn(
        obs_index: c_int,
        var_index: c_int,
        is_string: c_int,
        missing_kind: c_int,
        tag: c_char,
        num_val: c_double,
        str_val: *const c_char,
        ctx: *mut c_void,
    ) -> c_int,
>;

pub type ProgressCb = Option<unsafe extern "C" fn(progress: c_double, ctx: *mut c_void) -> c_int>;

#[repr(C)]
pub struct RsWriterCol {
    pub name: *const c_char,
    pub label: *const c_char,
    pub format: *const c_char,
    pub is_string: c_int,
    pub storage_width: usize,
}

#[repr(C)]
pub struct RsWriterHandle {
    _private: [u8; 0],
}

extern "C" {
    pub fn rs_bridge_parse_file(
        path: *const c_char,
        format_kind: c_int,
        input_encoding: *const c_char,
        row_offset: c_long,
        row_limit: c_long,
        on_metadata: MetadataCb,
        on_variable: VariableCb,
        on_value: ValueCb,
        on_progress: ProgressCb,
        user_ctx: *mut c_void,
        err_buf: *mut c_char,
        err_buf_len: usize,
    ) -> c_int;

    pub fn rs_bridge_writer_open(
        path: *const c_char,
        format_kind: c_int,
        table_name: *const c_char,
        file_label: *const c_char,
        cols: *const RsWriterCol,
        col_count: c_int,
        row_count: c_long,
        compress_rle: c_int,
    ) -> *mut RsWriterHandle;

    pub fn rs_bridge_writer_begin_row(h: *mut RsWriterHandle) -> c_int;
    pub fn rs_bridge_writer_put_double(
        h: *mut RsWriterHandle,
        col_idx: c_int,
        val: c_double,
    ) -> c_int;
    pub fn rs_bridge_writer_put_string(
        h: *mut RsWriterHandle,
        col_idx: c_int,
        val: *const c_char,
    ) -> c_int;
    pub fn rs_bridge_writer_put_missing(h: *mut RsWriterHandle, col_idx: c_int) -> c_int;
    pub fn rs_bridge_writer_put_tagged_missing(
        h: *mut RsWriterHandle,
        col_idx: c_int,
        tag: c_char,
    ) -> c_int;
    pub fn rs_bridge_writer_end_row(h: *mut RsWriterHandle) -> c_int;
    pub fn rs_bridge_writer_close(h: *mut RsWriterHandle) -> c_int;
}
