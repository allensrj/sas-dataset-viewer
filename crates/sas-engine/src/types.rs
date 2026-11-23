use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SasFormatKind {
    Sas7bdat,
    Xport,
}

impl SasFormatKind {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "sas7bdat" => Some(Self::Sas7bdat),
            "xpt" => Some(Self::Xport),
            _ => None,
        }
    }

    pub fn display_badge(&self) -> &'static str {
        match self {
            Self::Sas7bdat => "SAS7BDAT",
            Self::Xport => "XPT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EncodingChoice {
    #[default]
    Auto,
    Utf8,
    Gb18030,
    Windows1252,
    Latin1,
}

impl EncodingChoice {
    pub const ALL: &'static [EncodingChoice] = &[
        EncodingChoice::Auto,
        EncodingChoice::Utf8,
        EncodingChoice::Gb18030,
        EncodingChoice::Windows1252,
        EncodingChoice::Latin1,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Auto => "自动检测",
            Self::Utf8 => "UTF-8",
            Self::Gb18030 => "GB18030 (中文)",
            Self::Windows1252 => "Windows-1252",
            Self::Latin1 => "ISO-8859-1",
        }
    }

    pub fn iconv_name(&self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Utf8 => Some("UTF-8"),
            Self::Gb18030 => Some("GB18030"),
            Self::Windows1252 => Some("WINDOWS-1252"),
            Self::Latin1 => Some("ISO-8859-1"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VarType {
    Numeric,
    String,
}

impl VarType {
    pub fn short_label(&self) -> &'static str {
        match self {
            Self::Numeric => "Num",
            Self::String => "Char",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SasDateKind {
    None,
    Date,
    Date9,
    DateMmDdYy,
    DateDdMmYy,
    DateTime,
    DateTimeSas,
    Time,
}

impl SasDateKind {
    pub fn is_temporal(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableMeta {
    pub index: usize,
    pub name: String,
    pub label: String,
    pub format: String,
    pub var_type: VarType,
    pub storage_width: usize,
    pub display_width: i32,
    pub date_kind: SasDateKind,
    pub decimal_places: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMetadata {
    pub path: PathBuf,
    pub format_kind: SasFormatKind,
    pub table_name: String,
    pub file_label: String,
    pub file_encoding: String,
    pub encoding_choice: EncodingChoice,
    pub declared_row_count: i64,
    pub is_64bit: bool,
    pub compression: String,
    pub file_size_bytes: u64,
}
