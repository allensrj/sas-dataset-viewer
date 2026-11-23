use crate::types::SasFormatKind;
use readstat_sys::*;
use std::ffi::{c_char, CString};
use std::path::Path;

pub struct WriterColumnSpec {
    pub name: String,
    pub label: String,
    pub format: String,
    pub is_string: bool,
    pub storage_width: usize,
}

pub enum WriterCell<'a> {
    Double(f64),
    Str(&'a str),
    Missing,
    TaggedMissing(char),
}

pub struct SasWriter {
    handle: *mut RsWriterHandle,
    col_count: usize,
}

impl SasWriter {
    pub fn create(
        path: &Path,
        format_kind: SasFormatKind,
        table_name: &str,
        file_label: &str,
        cols: &[WriterColumnSpec],
        row_count: usize,
        compress_rle: bool,
    ) -> Result<Self, String> {
        let c_path = CString::new(path.to_str().ok_or("Invalid UTF-8 path")?)
            .map_err(|_| "Null byte in path")?;
        let c_table = CString::new(table_name).map_err(|_| "Invalid table_name")?;
        let c_label = CString::new(file_label).map_err(|_| "Invalid file_label")?;

        let mut owned_cstrings = Vec::with_capacity(cols.len() * 3);
        let mut c_cols = Vec::with_capacity(cols.len());
        for c in cols {
            let cn = CString::new(c.name.as_str()).map_err(|_| "Invalid col name")?;
            let cl = CString::new(c.label.as_str()).map_err(|_| "Invalid col label")?;
            let cf = CString::new(c.format.as_str()).map_err(|_| "Invalid col format")?;
            c_cols.push(RsWriterCol {
                name: cn.as_ptr(),
                label: cl.as_ptr(),
                format: cf.as_ptr(),
                is_string: if c.is_string { 1 } else { 0 },
                storage_width: c.storage_width,
            });
            owned_cstrings.push(cn);
            owned_cstrings.push(cl);
            owned_cstrings.push(cf);
        }

        let fmt_code = match format_kind {
            SasFormatKind::Sas7bdat => FORMAT_SAS7BDAT,
            SasFormatKind::Xport => FORMAT_XPORT,
        };

        let handle = unsafe {
            rs_bridge_writer_open(
                c_path.as_ptr(),
                fmt_code,
                c_table.as_ptr(),
                c_label.as_ptr(),
                c_cols.as_ptr(),
                c_cols.len() as i32,
                row_count as std::ffi::c_long,
                if compress_rle { 1 } else { 0 },
            )
        };
        drop(owned_cstrings);

        if handle.is_null() {
            return Err(format!("Failed to open SAS writer at {}", path.display()));
        }
        Ok(Self {
            handle,
            col_count: cols.len(),
        })
    }

    pub fn write_row(&mut self, cells: &[WriterCell<'_>]) -> Result<(), String> {
        if cells.len() != self.col_count {
            return Err("Cell count mismatch".to_string());
        }
        unsafe {
            let rc = rs_bridge_writer_begin_row(self.handle);
            if rc != READSTAT_OK {
                return Err(format!("begin_row failed: {rc}"));
            }
            for (idx, cell) in cells.iter().enumerate() {
                let rc = match cell {
                    WriterCell::Double(v) => {
                        rs_bridge_writer_put_double(self.handle, idx as i32, *v)
                    }
                    WriterCell::Str(s) => {
                        let cs = CString::new(*s).unwrap_or_else(|_| CString::new("").unwrap());
                        rs_bridge_writer_put_string(self.handle, idx as i32, cs.as_ptr())
                    }
                    WriterCell::Missing => rs_bridge_writer_put_missing(self.handle, idx as i32),
                    WriterCell::TaggedMissing(tag) => rs_bridge_writer_put_tagged_missing(
                        self.handle,
                        idx as i32,
                        *tag as c_char,
                    ),
                };
                if rc != READSTAT_OK {
                    return Err(format!("put col {idx} failed: {rc}"));
                }
            }
            let rc = rs_bridge_writer_end_row(self.handle);
            if rc != READSTAT_OK {
                return Err(format!("end_row failed: {rc}"));
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), String> {
        let rc = unsafe { rs_bridge_writer_close(self.handle) };
        self.handle = std::ptr::null_mut();
        if rc != READSTAT_OK {
            return Err(format!("writer close failed: {rc}"));
        }
        Ok(())
    }
}

impl Drop for SasWriter {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                rs_bridge_writer_close(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}
