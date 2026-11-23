use crate::types::{SasDateKind, VariableMeta};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

/// SAS epoch is 1960-01-01 (3653 days before Unix epoch 1970-01-01).
const SAS_EPOCH_UNIX_DAYS: i64 = -3653;
const SAS_EPOCH_UNIX_SECS: i64 = SAS_EPOCH_UNIX_DAYS * 86_400;

const MONTHS_UPPER: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Detect SAS date/time/datetime format kind and optional decimal places from SAS format string.
pub fn parse_sas_format(format: &str) -> (SasDateKind, Option<u8>) {
    let trimmed = format.trim().to_ascii_uppercase();
    if trimmed.is_empty() || trimmed.starts_with('$') {
        return (SasDateKind::None, None);
    }

    // Extract alphabetic prefix (e.g. "YYMMDD10." -> "YYMMDD", "DATETIME20." -> "DATETIME")
    let prefix: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
        .collect();

    // Check explicit ISO/E8601/B8601 formats which have digits inside the name
    let date_kind = if trimmed.starts_with("IS8601DT")
        || trimmed.starts_with("E8601DT")
        || trimmed.starts_with("B8601DT")
    {
        SasDateKind::DateTime
    } else if trimmed.starts_with("IS8601DA")
        || trimmed.starts_with("E8601DA")
        || trimmed.starts_with("B8601DA")
    {
        SasDateKind::Date
    } else if trimmed.starts_with("IS8601TM")
        || trimmed.starts_with("E8601TM")
        || trimmed.starts_with("B8601TM")
    {
        SasDateKind::Time
    } else {
        match prefix.as_str() {
            "DATETIME" | "MDYAMPM" => SasDateKind::DateTimeSas,
            "DTDATE" | "DATE" => SasDateKind::Date9,
            "YYMMDD" | "YYMMDDN" | "YYMMDDP" | "YYMMDDD" | "YYMMDDS" | "YYMM" | "MONYY"
            | "WEEKDATE" | "WORDDATE" | "JULIAN" => SasDateKind::Date,
            "MMDDYY" | "MMDDYYN" | "MMDDYYS" | "MMDDYYP" => SasDateKind::DateMmDdYy,
            "DDMMYY" | "DDMMYYN" | "DDMMYYS" | "DDMMYYP" => SasDateKind::DateDdMmYy,
            "TIME" | "TOD" | "HHMM" | "MMSS" | "TIMEAMPM" => SasDateKind::Time,
            _ => SasDateKind::None,
        }
    };

    let decimals = if !date_kind.is_temporal() {
        // Parse optional decimal part in e.g. "8.2", "F10.3", "COMMA12.2"
        trimmed.split('.').nth(1).and_then(|d| {
            if d.is_empty() {
                None
            } else {
                d.parse::<u8>().ok().filter(|&v| v <= 12)
            }
        })
    } else {
        None
    };

    (date_kind, decimals)
}

/// Format a SAS missing tag byte into SAS convention string (`.`, `.A`-`.Z`, `._`).
pub fn format_missing_tag(tag: u8) -> String {
    match tag {
        1 => ".".to_string(),
        b'A'..=b'Z' | b'a'..=b'z' => format!(".{}", (tag as char).to_ascii_uppercase()),
        b'_' => "._".to_string(),
        _ => ".".to_string(),
    }
}

/// Format a valid numeric SAS value according to variable metadata (dates formatted per user spec).
pub fn format_numeric_value(val: f64, meta: &VariableMeta) -> String {
    if !val.is_finite() {
        return ".".to_string();
    }
    match meta.date_kind {
        SasDateKind::None => format_plain_number(val, meta.decimal_places),
        SasDateKind::Date => sas_days_to_date(val)
            .map(|d| format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
            .unwrap_or_else(|| format_plain_number(val, None)),
        SasDateKind::Date9 => sas_days_to_date(val)
            .map(|d| {
                let m = MONTHS_UPPER[(d.month() - 1) as usize];
                format!("{:02}{}{:04}", d.day(), m, d.year())
            })
            .unwrap_or_else(|| format_plain_number(val, None)),
        SasDateKind::DateMmDdYy => sas_days_to_date(val)
            .map(|d| format!("{:02}/{:02}/{:04}", d.month(), d.day(), d.year()))
            .unwrap_or_else(|| format_plain_number(val, None)),
        SasDateKind::DateDdMmYy => sas_days_to_date(val)
            .map(|d| format!("{:02}/{:02}/{:04}", d.day(), d.month(), d.year()))
            .unwrap_or_else(|| format_plain_number(val, None)),
        SasDateKind::DateTime => sas_secs_to_datetime(val)
            .map(|dt| {
                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    dt.year(),
                    dt.month(),
                    dt.day(),
                    dt.hour(),
                    dt.minute(),
                    dt.second()
                )
            })
            .unwrap_or_else(|| format_plain_number(val, None)),
        SasDateKind::DateTimeSas => sas_secs_to_datetime(val)
            .map(|dt| {
                let m = MONTHS_UPPER[(dt.month() - 1) as usize];
                format!(
                    "{:02}{}{:04}:{:02}:{:02}:{:02}",
                    dt.day(),
                    m,
                    dt.year(),
                    dt.hour(),
                    dt.minute(),
                    dt.second()
                )
            })
            .unwrap_or_else(|| format_plain_number(val, None)),
        SasDateKind::Time => format_sas_time(val),
    }
}

pub fn format_plain_number(val: f64, decimals: Option<u8>) -> String {
    if !val.is_finite() {
        return ".".to_string();
    }
    let val = if val == 0.0 { 0.0 } else { val }; // avoid "-0"
    if let Some(d) = decimals {
        return format!("{:.prec$}", val, prec = d as usize);
    }
    // Integer fast path
    if val.fract() == 0.0 && val.abs() < 1e15 {
        return format!("{:.0}", val);
    }
    if val.abs() >= 1e12 || (val != 0.0 && val.abs() < 1e-5) {
        return format!("{:.6e}", val);
    }
    // Clean up floating-point noise (e.g. 0.30000000000000004 -> 0.3)
    let s = format!("{:.10}", val);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

pub fn sas_days_to_date(days: f64) -> Option<NaiveDate> {
    if !days.is_finite() || days.abs() > 365_000.0 {
        return None;
    }
    let unix_days = SAS_EPOCH_UNIX_DAYS + days.round() as i64;
    let unix_secs = unix_days.checked_mul(86_400)?;
    chrono::DateTime::from_timestamp(unix_secs, 0).map(|dt| dt.date_naive())
}

pub fn sas_secs_to_datetime(secs: f64) -> Option<NaiveDateTime> {
    if !secs.is_finite() || secs.abs() > 3.2e10 {
        return None;
    }
    let unix_secs = SAS_EPOCH_UNIX_SECS + secs.round() as i64;
    chrono::DateTime::from_timestamp(unix_secs, 0).map(|dt| dt.naive_utc())
}

pub fn format_sas_time(secs: f64) -> String {
    let neg = secs < 0.0;
    let total = secs.abs().round() as i64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if neg {
        format!("-{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}:{:02}", h, m, s)
    }
}

/// Parse user-entered numeric or formatted SAS date/time string into underlying SAS f64 value.
pub fn parse_numeric_or_date_input(input: &str, date_kind: SasDateKind) -> Option<f64> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    // Direct numeric always works
    if let Ok(v) = s.parse::<f64>() {
        if v.is_finite() {
            return Some(v);
        }
    }
    match date_kind {
        SasDateKind::Date
        | SasDateKind::Date9
        | SasDateKind::DateMmDdYy
        | SasDateKind::DateDdMmYy => parse_date_to_sas_days(s),
        SasDateKind::DateTime | SasDateKind::DateTimeSas => parse_datetime_to_sas_secs(s),
        SasDateKind::Time => parse_time_to_sas_secs(s),
        SasDateKind::None => None,
    }
}

fn parse_date_to_sas_days(s: &str) -> Option<f64> {
    let formats = ["%Y-%m-%d", "%d%b%Y", "%m/%d/%Y", "%d/%m/%Y", "%Y/%m/%d"];
    let upper = s.to_ascii_uppercase();
    for fmt in formats {
        if let Ok(d) = NaiveDate::parse_from_str(&upper, fmt) {
            let epoch = NaiveDate::from_ymd_opt(1960, 1, 1)?;
            return Some((d - epoch).num_days() as f64);
        }
    }
    None
}

fn parse_datetime_to_sas_secs(s: &str) -> Option<f64> {
    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%d%b%Y:%H:%M:%S",
        "%d%b%Y %H:%M:%S",
    ];
    let upper = s.to_ascii_uppercase();
    for fmt in formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&upper, fmt) {
            let epoch = NaiveDate::from_ymd_opt(1960, 1, 1)?.and_hms_opt(0, 0, 0)?;
            return Some((dt - epoch).num_seconds() as f64);
        }
    }
    // Also allow date-only input for datetime column (midnight)
    parse_date_to_sas_days(s).map(|days| days * 86_400.0)
}

fn parse_time_to_sas_secs(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 2 || parts.len() == 3 {
        let h: f64 = parts[0].parse().ok()?;
        let m: f64 = parts[1].parse().ok()?;
        let sec: f64 = if parts.len() == 3 {
            parts[2].parse().ok()?
        } else {
            0.0
        };
        return Some(h * 3600.0 + m * 60.0 + sec);
    }
    None
}
