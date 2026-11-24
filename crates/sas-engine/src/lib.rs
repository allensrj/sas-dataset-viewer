pub mod column;
pub mod loader;
pub mod query;
pub mod sas_format;
pub mod stats;
pub mod types;
pub mod writer;

pub use column::{ColumnData, ColumnarTable, NumericColumn, StringColumn, MISSING_SYSTEM, MISSING_VALID};
pub use loader::{load_dataset_streaming, load_dataset_sync, LoadError, LoadEvent};
pub use query::{
    build_cell_equals_filter, build_multi_cell_filter, compute_visible_rows, FilterCondition,
    FilterOp, SortKey, ViewQuery,
};
pub use sas_format::{format_missing_tag, format_numeric_value, format_plain_number, parse_numeric_or_date_input, parse_sas_format};
pub use stats::{compute_column_stats, ColumnStats, FreqEntry, NumericStats, StringStats};
pub use types::{DatasetMetadata, EncodingChoice, SasDateKind, SasFormatKind, VarType, VariableMeta};
pub use writer::{SasWriter, WriterCell, WriterColumnSpec};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Instant;

    fn write_sample_file(path: &std::path::Path, kind: SasFormatKind, rows: usize) {
        let cols = vec![
            WriterColumnSpec {
                name: "USUBJID".into(),
                label: "受试者唯一编号".into(),
                format: "$20.".into(),
                is_string: true,
                storage_width: 24,
            },
            WriterColumnSpec {
                name: "ARM".into(),
                label: "治疗分组".into(),
                format: "$32.".into(),
                is_string: true,
                storage_width: 32,
            },
            WriterColumnSpec {
                name: "SEX".into(),
                label: "性别".into(),
                format: "$2.".into(),
                is_string: true,
                storage_width: 4,
            },
            WriterColumnSpec {
                name: "AGE".into(),
                label: "年龄(岁)".into(),
                format: "8.".into(),
                is_string: false,
                storage_width: 8,
            },
            WriterColumnSpec {
                name: "RFSTDTC".into(),
                label: "首次用药日期".into(),
                format: "YYMMDD10.".into(),
                is_string: false,
                storage_width: 8,
            },
            WriterColumnSpec {
                name: "AVAL".into(),
                label: "实验室检测值".into(),
                format: "10.2".into(),
                is_string: false,
                storage_width: 8,
            },
        ];

        let mut w = SasWriter::create(path, kind, "ADSL", "受试者人口学与基线数据", &cols, rows, true)
            .expect("create writer");
        let arms = ["试验组 A (100mg)", "试验组 B (200mg)", "安慰剂组 Placebo"];
        let sexes = ["M", "F"];
        // SAS days for 2025-01-01 = (2025-01-01 - 1960-01-01) = 23742
        let base_day = 23742.0;

        for i in 0..rows {
            let subj = format!("STUDY01-{:06}", i + 1);
            let arm = arms[i % arms.len()];
            let sex = sexes[i % sexes.len()];
            let age_cell = if i % 50 == 0 {
                WriterCell::Missing
            } else if i % 50 == 1 {
                WriterCell::TaggedMissing('A')
            } else {
                WriterCell::Double((18 + (i % 65)) as f64)
            };
            let dt_cell = WriterCell::Double(base_day + (i % 365) as f64);
            let aval_cell = WriterCell::Double(((i % 1000) as f64) * 0.15 + 10.0);

            w.write_row(&[
                WriterCell::Str(&subj),
                WriterCell::Str(arm),
                WriterCell::Str(sex),
                age_cell,
                dt_cell,
                aval_cell,
            ])
            .expect("write row");
        }
        w.finish().expect("finish writer");
    }

    #[test]
    fn test_roundtrip_sas7bdat_and_xpt_formats_missing_dates() {
        let tmp = std::env::temp_dir();
        let sas_path = tmp.join("test_sas_viewer_rt.sas7bdat");
        let xpt_path = tmp.join("test_sas_viewer_rt.xpt");

        write_sample_file(&sas_path, SasFormatKind::Sas7bdat, 200);
        write_sample_file(&xpt_path, SasFormatKind::Xport, 200);

        for path in [&sas_path, &xpt_path] {
            let table = load_dataset_sync(path, EncodingChoice::Utf8).expect("load dataset");
            assert_eq!(table.row_count, 200);
            assert_eq!(table.variables.len(), 6);
            assert_eq!(table.variables[0].name, "USUBJID");
            assert_eq!(table.variables[3].name, "AGE");
            assert_eq!(table.variables[4].date_kind, SasDateKind::Date);

            // Check SAS missing conventions
            assert_eq!(table.format_cell(0, 3), ".");
            assert_eq!(table.format_cell(1, 3), ".A");
            assert_eq!(table.format_cell(2, 3), "20");

            // Check SAS formatted date display (23742 = 2025-01-01)
            assert_eq!(table.format_cell(0, 4), "2025-01-01");

            // Check filtering & multi-column sorting
            let mut q = ViewQuery::default();
            q.add_filter(2, FilterOp::StrEq("F".into()));
            q.add_filter(3, FilterOp::NumGt(40.0));
            q.set_sort_direction(1, true, false);
            q.set_sort_direction(3, false, true);

            let rows = compute_visible_rows(&table, &q);
            assert!(!rows.is_empty());
            for &r in &rows {
                assert_eq!(table.format_cell(r as usize, 2), "F");
            }

            // Check stats on filtered rows
            let st_str = compute_column_stats(&table, &rows, 1).unwrap();
            match st_str {
                ColumnStats::String(ss) => {
                    assert_eq!(ss.total_rows, rows.len());
                    assert!(!ss.frequencies.is_empty());
                }
                _ => panic!("expected string stats"),
            }
            let st_num = compute_column_stats(&table, &rows, 3).unwrap();
            match st_num {
                ColumnStats::Numeric(ns) => {
                    assert_eq!(ns.valid_count, rows.len());
                    assert!(ns.min.unwrap() > 40.0);
                }
                _ => panic!("expected numeric stats"),
            }
        }

        let _ = std::fs::remove_file(&sas_path);
        let _ = std::fs::remove_file(&xpt_path);
    }

    #[test]
    fn test_200k_rows_performance_and_streaming() {
        let tmp = std::env::temp_dir();
        let big_path = tmp.join("test_sas_viewer_200k.sas7bdat");
        write_sample_file(&big_path, SasFormatKind::Sas7bdat, 200_000);

        let mut saw_first_batch = false;
        let t0 = Instant::now();
        let table = load_dataset_streaming(
            &big_path,
            EncodingChoice::Utf8,
            Arc::new(AtomicBool::new(false)),
            |ev| {
                if let LoadEvent::FirstBatch(snap) = ev {
                    saw_first_batch = true;
                    assert_eq!(snap.row_count, 5_000);
                }
            },
        )
        .expect("stream load 200k");
        let load_ms = t0.elapsed().as_millis();
        assert!(saw_first_batch);
        assert_eq!(table.row_count, 200_000);

        // Benchmark filter + 2-column sort
        let mut q = ViewQuery::default();
        q.add_filter(2, FilterOp::StrEq("M".into()));
        q.set_sort_direction(1, true, false);
        q.set_sort_direction(5, false, true);
        let t1 = Instant::now();
        let rows = compute_visible_rows(&table, &q);
        let query_ms = t1.elapsed().as_millis();
        assert_eq!(rows.len(), 100_000);

        // Benchmark stats on 100k filtered rows
        let t2 = Instant::now();
        let _s1 = compute_column_stats(&table, &rows, 1).unwrap();
        let _s2 = compute_column_stats(&table, &rows, 5).unwrap();
        let stats_ms = t2.elapsed().as_millis();

        eprintln!(
            "[200k benchmark] load={}ms, filter+2col_sort={}ms, stats(str+num)={}ms, mem={:.1}MB",
            load_ms,
            query_ms,
            stats_ms,
            table.approx_memory_bytes() as f64 / (1024.0 * 1024.0)
        );

        let _ = std::fs::remove_file(&big_path);
    }

    #[test]
    fn test_filters_dates_and_d_shortcut_semantics() {
        let tmp = std::env::temp_dir();
        let path = tmp.join("test_sas_viewer_filters.sas7bdat");
        write_sample_file(&path, SasFormatKind::Sas7bdat, 150);
        let table = load_dataset_sync(&path, EncodingChoice::Utf8).expect("load");

        // 1. String Contains (Chinese substring)
        let mut q = ViewQuery::default();
        q.add_filter(1, FilterOp::StrContains("安慰剂".into()));
        let rows = compute_visible_rows(&table, &q);
        assert_eq!(rows.len(), 50);

        // 2. Quick D filter: AND across different columns, replaces on same column
        q.set_quick_filter_for_column(2, FilterOp::StrEq("M".into()));
        let rows_m = compute_visible_rows(&table, &q);
        assert_eq!(rows_m.len(), 25);
        // Replace same column (col 2 -> "F") while keeping col 1 ("安慰剂")
        q.set_quick_filter_for_column(2, FilterOp::StrEq("F".into()));
        let rows_f = compute_visible_rows(&table, &q);
        assert_eq!(rows_f.len(), 25);
        assert_eq!(q.filters.len(), 2);

        // 3. Date string input parsing for numeric date column (2025-01-01 = 23742)
        let d_val = parse_numeric_or_date_input("2025-01-11", SasDateKind::Date).unwrap();
        assert_eq!(d_val, 23752.0);
        let d_val2 = parse_numeric_or_date_input("11JAN2025", SasDateKind::Date9).unwrap();
        assert_eq!(d_val2, 23752.0);

        // 4. Numeric Between & Tagged Missing
        let mut q2 = ViewQuery::default();
        q2.add_filter(3, FilterOp::NumEqTaggedMissing(b'A'));
        let rows_a = compute_visible_rows(&table, &q2);
        assert_eq!(rows_a.len(), 3); // i = 1, 51, 101

        // 5. Clear all (C) restores all 150 rows
        q2.clear();
        assert_eq!(compute_visible_rows(&table, &q2).len(), 150);

        // 6. Multi-value StrIn & NumIn filters
        let mut q3 = ViewQuery::default();
        q3.add_filter(
            1,
            FilterOp::StrIn(vec![
                "试验组 A (100mg)".into(),
                "安慰剂组 Placebo".into(),
            ]),
        );
        let rows_in = compute_visible_rows(&table, &q3);
        assert_eq!(rows_in.len(), 100);

        let mut q4 = ViewQuery::default();
        q4.add_filter(
            3,
            FilterOp::NumIn {
                values: vec![20.0, 21.0],
                missing_tags: vec![b'A'],
            },
        );
        let rows_num_in = compute_visible_rows(&table, &q4);
        assert!(rows_num_in.len() >= 5);

        // 7. Numeric stats always populate value frequencies (even when distinct > 25)
        let all_rows: Vec<u32> = (0..150).collect();
        let st_num = compute_column_stats(&table, &all_rows, 3).unwrap();
        match st_num {
            ColumnStats::Numeric(ns) => {
                assert!(ns.distinct_count > 25);
                assert!(!ns.frequencies.is_empty());
            }
            _ => panic!("expected numeric stats"),
        }

        // 8. Auto-fit width respects [6, 30] ASCII char bounds strictly from data
        let w_sex = table.estimate_column_width_px(2, 6, 30, true); // "M"/"F" -> floor 6 chars (~63.6px)
        assert!((w_sex - (6.0 * 7.6 + 18.0)).abs() < 0.1);
        let w_arm = table.estimate_column_width_px(1, 6, 30, true); // <= 30 chars (~246px)
        assert!(w_arm <= 30.0 * 7.6 + 18.1);

        let _ = std::fs::remove_file(&path);
    }
}
