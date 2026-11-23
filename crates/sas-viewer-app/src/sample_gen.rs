use sas_engine::{SasFormatKind, SasWriter, WriterCell, WriterColumnSpec};
use std::path::{Path, PathBuf};

/// Generate realistic clinical trial `.sas7bdat` and `.xpt` sample datasets into `dir`
/// (including a 200,000-row lab dataset for immediate large-file responsiveness testing).
pub fn generate_sample_datasets(dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建示例目录: {e}"))?;
    let mut created = Vec::new();

    // 1. dm.sas7bdat (Demographics - 1,200 rows)
    let dm_path = dir.join("dm.sas7bdat");
    write_dm(&dm_path, 1_200)?;
    created.push(dm_path);

    // 2. ae.xpt (Adverse Events - 5,000 rows, XPORT v5)
    let ae_path = dir.join("ae.xpt");
    write_ae_xpt(&ae_path, 5_000)?;
    created.push(ae_path);

    // 3. lb_200k.sas7bdat (Lab Results - 200,000 rows, RLE compressed)
    let lb_path = dir.join("lb_200k.sas7bdat");
    write_lb_large(&lb_path, 200_000)?;
    created.push(lb_path);

    Ok(created)
}

fn write_dm(path: &Path, rows: usize) -> Result<(), String> {
    let cols = vec![
        WriterColumnSpec {
            name: "STUDYID".into(),
            label: "研究编号".into(),
            format: "$16.".into(),
            is_string: true,
            storage_width: 16,
        },
        WriterColumnSpec {
            name: "USUBJID".into(),
            label: "受试者唯一编号".into(),
            format: "$20.".into(),
            is_string: true,
            storage_width: 24,
        },
        WriterColumnSpec {
            name: "SITEID".into(),
            label: "研究中心编号".into(),
            format: "$10.".into(),
            is_string: true,
            storage_width: 12,
        },
        WriterColumnSpec {
            name: "ARM".into(),
            label: "计划治疗分组".into(),
            format: "$32.".into(),
            is_string: true,
            storage_width: 36,
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
            label: "年龄 (岁)".into(),
            format: "8.".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "BMI".into(),
            label: "体质指数 (kg/m²)".into(),
            format: "8.1".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "RFSTDT".into(),
            label: "首次用药日期".into(),
            format: "YYMMDD10.".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "BRTHDT".into(),
            label: "出生日期".into(),
            format: "DATE9.".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "COUNTRY".into(),
            label: "国家/地区".into(),
            format: "$8.".into(),
            is_string: true,
            storage_width: 12,
        },
    ];

    let mut w = SasWriter::create(
        path,
        SasFormatKind::Sas7bdat,
        "DM",
        "受试者人口学基线数据集 (DM)",
        &cols,
        rows,
        true,
    )?;

    let arms = [
        "试验组 A (100mg QD)",
        "试验组 B (200mg QD)",
        "安慰剂对照组 (Placebo)",
    ];
    let sexes = ["M", "F"];
    let countries = ["CHN", "CHN", "CHN", "USA", "JPN", "DEU"];
    let base_rfst = 23742.0; // 2025-01-01

    for i in 0..rows {
        let site = format!("SITE-{:02}", (i % 20) + 1);
        let subj = format!("ONCO2025-{:02}-{:04}", (i % 20) + 1, i + 1);
        let arm = arms[i % arms.len()];
        let sex = sexes[(i / 3 + i) % 2];
        let age = if i % 67 == 0 {
            WriterCell::Missing
        } else if i % 67 == 1 {
            WriterCell::TaggedMissing('A')
        } else {
            WriterCell::Double((20 + (i * 7) % 58) as f64)
        };
        let bmi = if i % 43 == 0 {
            WriterCell::Missing
        } else {
            WriterCell::Double(18.5 + ((i * 13) % 140) as f64 * 0.1)
        };
        let rfst = WriterCell::Double(base_rfst + (i % 300) as f64);
        let brth = WriterCell::Double(base_rfst - (20.0 + ((i * 7) % 58) as f64) * 365.25);
        let country = countries[i % countries.len()];

        w.write_row(&[
            WriterCell::Str("ONCO-2025-01"),
            WriterCell::Str(&subj),
            WriterCell::Str(&site),
            WriterCell::Str(arm),
            WriterCell::Str(sex),
            age,
            bmi,
            rfst,
            brth,
            WriterCell::Str(country),
        ])?;
    }
    w.finish()
}

fn write_ae_xpt(path: &Path, rows: usize) -> Result<(), String> {
    let cols = vec![
        WriterColumnSpec {
            name: "STUDYID".into(),
            label: "Study Identifier".into(),
            format: "$16.".into(),
            is_string: true,
            storage_width: 16,
        },
        WriterColumnSpec {
            name: "USUBJID".into(),
            label: "Unique Subject Identifier".into(),
            format: "$20.".into(),
            is_string: true,
            storage_width: 24,
        },
        WriterColumnSpec {
            name: "AESEQ".into(),
            label: "Sequence Number".into(),
            format: "8.".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "AEDECOD".into(),
            label: "Dictionary-Derived Term".into(),
            format: "$32.".into(),
            is_string: true,
            storage_width: 32,
        },
        WriterColumnSpec {
            name: "AEBODSYS".into(),
            label: "Body System or Organ Class".into(),
            format: "$36.".into(),
            is_string: true,
            storage_width: 36,
        },
        WriterColumnSpec {
            name: "AESEV".into(),
            label: "Severity/Intensity".into(),
            format: "$12.".into(),
            is_string: true,
            storage_width: 12,
        },
        WriterColumnSpec {
            name: "AESER".into(),
            label: "Serious Event".into(),
            format: "$2.".into(),
            is_string: true,
            storage_width: 4,
        },
        WriterColumnSpec {
            name: "AESTDT".into(),
            label: "Start Date of Adverse Event".into(),
            format: "YYMMDD10.".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "AEDUR".into(),
            label: "Duration (Days)".into(),
            format: "8.".into(),
            is_string: false,
            storage_width: 8,
        },
    ];

    let mut w = SasWriter::create(
        path,
        SasFormatKind::Xport,
        "AE",
        "Adverse Events (XPT v5)",
        &cols,
        rows,
        false,
    )?;

    let terms = [
        ("HEADACHE", "NERVOUS SYSTEM DISORDERS"),
        ("NAUSEA", "GASTROINTESTINAL DISORDERS"),
        ("FATIGUE", "GENERAL DISORDERS"),
        ("RASH", "SKIN AND SUBCUTANEOUS TISSUE"),
        ("DIZZINESS", "NERVOUS SYSTEM DISORDERS"),
        ("PYREXIA", "GENERAL DISORDERS"),
        ("COUGH", "RESPIRATORY DISORDERS"),
        ("ALT INCREASED", "INVESTIGATIONS"),
    ];
    let sevs = ["MILD", "MILD", "MODERATE", "SEVERE"];
    let base_dt = 23750.0;

    for i in 0..rows {
        let subj_id = (i % 1200) + 1;
        let subj = format!("ONCO2025-{:02}-{:04}", (subj_id % 20) + 1, subj_id);
        let (pt, soc) = terms[(i * 3) % terms.len()];
        let sev = sevs[(i * 5) % sevs.len()];
        let ser = if sev == "SEVERE" && i % 3 == 0 { "Y" } else { "N" };
        let dur = if i % 40 == 0 {
            WriterCell::Missing
        } else {
            WriterCell::Double(((i % 28) + 1) as f64)
        };

        w.write_row(&[
            WriterCell::Str("ONCO-2025-01"),
            WriterCell::Str(&subj),
            WriterCell::Double(((i / 1200) + 1) as f64),
            WriterCell::Str(pt),
            WriterCell::Str(soc),
            WriterCell::Str(sev),
            WriterCell::Str(ser),
            WriterCell::Double(base_dt + (i % 240) as f64),
            dur,
        ])?;
    }
    w.finish()
}

fn write_lb_large(path: &Path, rows: usize) -> Result<(), String> {
    let cols = vec![
        WriterColumnSpec {
            name: "USUBJID".into(),
            label: "受试者编号".into(),
            format: "$20.".into(),
            is_string: true,
            storage_width: 24,
        },
        WriterColumnSpec {
            name: "VISIT".into(),
            label: "访视周期".into(),
            format: "$16.".into(),
            is_string: true,
            storage_width: 16,
        },
        WriterColumnSpec {
            name: "PARAMCD".into(),
            label: "检验项代码".into(),
            format: "$8.".into(),
            is_string: true,
            storage_width: 10,
        },
        WriterColumnSpec {
            name: "PARAM".into(),
            label: "实验室检查项目名称".into(),
            format: "$32.".into(),
            is_string: true,
            storage_width: 36,
        },
        WriterColumnSpec {
            name: "AVAL".into(),
            label: "分析测定值".into(),
            format: "10.2".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "BASE".into(),
            label: "基线测定值".into(),
            format: "10.2".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "CHG".into(),
            label: "较基线变化值".into(),
            format: "10.2".into(),
            is_string: false,
            storage_width: 8,
        },
        WriterColumnSpec {
            name: "ANRIND".into(),
            label: "参考范围标记".into(),
            format: "$8.".into(),
            is_string: true,
            storage_width: 10,
        },
        WriterColumnSpec {
            name: "ADT".into(),
            label: "采样日期".into(),
            format: "YYMMDD10.".into(),
            is_string: false,
            storage_width: 8,
        },
    ];

    let mut w = SasWriter::create(
        path,
        SasFormatKind::Sas7bdat,
        "ADLB",
        "20万行实验室检查性能测试集 (ADLB)",
        &cols,
        rows,
        true,
    )?;

    let visits = [
        "SCREENING",
        "BASELINE",
        "WEEK 4",
        "WEEK 8",
        "WEEK 12",
        "WEEK 24",
        "END OF STUDY",
    ];
    let params = [
        ("ALT", "丙氨酸氨基转移酶 (U/L)", 28.0),
        ("AST", "天门冬氨酸氨基转移酶 (U/L)", 24.0),
        ("WBC", "白细胞计数 (10^9/L)", 6.5),
        ("HGB", "血红蛋白 (g/L)", 135.0),
        ("PLT", "血小板计数 (10^9/L)", 220.0),
        ("CREAT", "血清肌酐 (umol/L)", 72.0),
    ];
    let inds = ["NORMAL", "NORMAL", "NORMAL", "NORMAL", "HIGH", "LOW"];
    let base_dt = 23742.0;

    for i in 0..rows {
        let subj_idx = (i / 42) + 1;
        let subj = format!("ONCO2025-{:02}-{:04}", (subj_idx % 20) + 1, subj_idx);
        let visit = visits[(i / 6) % visits.len()];
        let (pcd, pname, center) = params[i % params.len()];
        let base_v = center + (((i * 17) % 100) as f64 - 50.0) * 0.12;
        let chg_v = (((i * 31) % 120) as f64 - 60.0) * 0.08;
        let aval_v = (base_v + chg_v).max(0.1);
        let ind = inds[(i * 7) % inds.len()];

        let aval_cell = if i % 101 == 0 {
            WriterCell::Missing
        } else if i % 251 == 0 {
            WriterCell::TaggedMissing('B')
        } else {
            WriterCell::Double(aval_v)
        };

        w.write_row(&[
            WriterCell::Str(&subj),
            WriterCell::Str(visit),
            WriterCell::Str(pcd),
            WriterCell::Str(pname),
            aval_cell,
            WriterCell::Double(base_v),
            WriterCell::Double(chg_v),
            WriterCell::Str(ind),
            WriterCell::Double(base_dt + ((i / 6) % 210) as f64),
        ])?;
    }
    w.finish()
}
