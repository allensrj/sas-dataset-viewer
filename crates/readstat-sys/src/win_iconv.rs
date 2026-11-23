//! Zero-dependency iconv implementation backed by `encoding_rs` for Windows builds.
//! Supports UTF-8, GB18030, GBK, Windows-1252, ISO-8859-1, Shift-JIS, Big5, EUC-KR, etc.

use encoding_rs::{Encoding, UTF_8};
#[cfg(target_os = "windows")]
use std::ffi::{c_char, c_int, c_void, CStr};

#[allow(dead_code)]
pub struct IconvHandle {
    from: &'static Encoding,
    to: &'static Encoding,
}

pub fn resolve_encoding(name: &str) -> Option<&'static Encoding> {
    let cleaned = name
        .trim()
        .split("//")
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();

    match cleaned.as_str() {
        "" | "UTF-8" | "UTF8" => Some(UTF_8),
        "US-ASCII" | "ASCII" => Some(encoding_rs::WINDOWS_1252),
        "CP1252" | "WINDOWS-1252" | "WIN1252" | "LATIN1" | "ISO-8859-1" | "ISO8859-1" => {
            Some(encoding_rs::WINDOWS_1252)
        }
        "GBK" | "GB2312" | "GB18030" | "CP936" | "WINDOWS-936" | "EUC-CN" => {
            Some(encoding_rs::GB18030)
        }
        "BIG5" | "CP950" | "WINDOWS-950" => Some(encoding_rs::BIG5),
        "SHIFT_JIS" | "SHIFT-JIS" | "SJIS" | "CP932" | "WINDOWS-932" => {
            Some(encoding_rs::SHIFT_JIS)
        }
        "EUC-KR" | "CP949" | "WINDOWS-949" => Some(encoding_rs::EUC_KR),
        "CP1250" | "WINDOWS-1250" | "LATIN2" | "ISO-8859-2" => Some(encoding_rs::WINDOWS_1250),
        "CP1251" | "WINDOWS-1251" => Some(encoding_rs::WINDOWS_1251),
        "UTF-16LE" | "UTF16LE" => Some(encoding_rs::UTF_16LE),
        "UTF-16BE" | "UTF16BE" => Some(encoding_rs::UTF_16BE),
        other => Encoding::for_label(other.as_bytes()),
    }
}

pub fn convert_bytes(from: &'static Encoding, to: &'static Encoding, input: &[u8]) -> Vec<u8> {
    if from == to {
        return input.to_vec();
    }
    let (decoded, _, _) = from.decode(input);
    if to == UTF_8 {
        decoded.into_owned().into_bytes()
    } else {
        let (encoded, _, _) = to.encode(&decoded);
        encoded.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gb18030_and_cp1252_conversion() {
        let gb = resolve_encoding("GB18030").unwrap();
        let utf8 = resolve_encoding("UTF-8").unwrap();
        // "中文" in GB18030 is [0xD6, 0xD0, 0xCE, 0xC4]
        let out = convert_bytes(gb, utf8, &[0xD6, 0xD0, 0xCE, 0xC4]);
        assert_eq!(String::from_utf8(out).unwrap(), "中文");

        let cp = resolve_encoding("WINDOWS-1252").unwrap();
        let out2 = convert_bytes(cp, utf8, b"caf\xe9");
        assert_eq!(String::from_utf8(out2).unwrap(), "café");
    }
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub unsafe extern "C" fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void {
    if tocode.is_null() || fromcode.is_null() {
        return (-1isize) as *mut c_void;
    }
    let to_str = CStr::from_ptr(tocode).to_string_lossy();
    let from_str = CStr::from_ptr(fromcode).to_string_lossy();

    let Some(to) = resolve_encoding(&to_str) else {
        return (-1isize) as *mut c_void;
    };
    let Some(from) = resolve_encoding(&from_str) else {
        return (-1isize) as *mut c_void;
    };

    Box::into_raw(Box::new(IconvHandle { from, to })) as *mut c_void
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub unsafe extern "C" fn iconv(
    cd: *mut c_void,
    inbuf: *mut *mut c_char,
    inbytesleft: *mut usize,
    outbuf: *mut *mut c_char,
    outbytesleft: *mut usize,
) -> usize {
    if cd.is_null() || cd == (-1isize) as *mut c_void {
        return usize::MAX;
    }
    if inbuf.is_null() || (*inbuf).is_null() || inbytesleft.is_null() {
        return 0;
    }
    if outbuf.is_null() || (*outbuf).is_null() || outbytesleft.is_null() {
        return usize::MAX;
    }

    let handle = &*(cd as *const IconvHandle);
    let in_len = *inbytesleft;
    let out_cap = *outbytesleft;
    let input = std::slice::from_raw_parts((*inbuf) as *const u8, in_len);

    let converted = convert_bytes(handle.from, handle.to, input);
    let copy_len = converted.len().min(out_cap);

    std::ptr::copy_nonoverlapping(converted.as_ptr(), (*outbuf) as *mut u8, copy_len);
    *inbuf = (*inbuf).add(in_len);
    *inbytesleft = 0;
    *outbuf = (*outbuf).add(copy_len);
    *outbytesleft = out_cap - copy_len;

    0
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub unsafe extern "C" fn iconv_close(cd: *mut c_void) -> c_int {
    if !cd.is_null() && cd != (-1isize) as *mut c_void {
        drop(Box::from_raw(cd as *mut IconvHandle));
    }
    0
}
