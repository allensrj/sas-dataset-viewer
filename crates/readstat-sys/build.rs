use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();
    let rs_src = root_dir.join("third_party").join("readstat").join("src");
    let csrc = manifest_dir.join("csrc");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .include(&rs_src)
        .include(rs_src.join("sas"))
        .warnings(false)
        .opt_level(3);

    if target_os == "windows" {
        // Use our zero-dependency Rust encoding_rs-backed iconv shim on Windows
        build.include(csrc.join("win_iconv_shim"));
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    let core_files = [
        "CKHashTable.c",
        "readstat_bits.c",
        "readstat_convert.c",
        "readstat_error.c",
        "readstat_io_unistd.c",
        "readstat_malloc.c",
        "readstat_metadata.c",
        "readstat_parser.c",
        "readstat_value.c",
        "readstat_variable.c",
        "readstat_writer.c",
        "sas/ieee.c",
        "sas/readstat_sas.c",
        "sas/readstat_sas_rle.c",
        "sas/readstat_sas_rdc.c",
        "sas/readstat_sas7bdat_read.c",
        "sas/readstat_sas7bdat_write.c",
        "sas/readstat_xport.c",
        "sas/readstat_xport_parse_format.c",
        "sas/readstat_xport_read.c",
        "sas/readstat_xport_write.c",
    ];

    for f in &core_files {
        let p = rs_src.join(f);
        println!("cargo:rerun-if-changed={}", p.display());
        build.file(p);
    }

    let bridge = csrc.join("readstat_bridge.c");
    println!("cargo:rerun-if-changed={}", bridge.display());
    build.file(bridge);

    build.compile("readstat_sas");

    if target_os == "macos" {
        println!("cargo:rustc-link-lib=iconv");
    }
}
