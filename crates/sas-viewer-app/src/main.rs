#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backends;
mod config;
mod panels;
mod sample_gen;
mod state;
mod theme;
mod window_host;

use backends::BackendKind;
use std::path::PathBuf;

#[cfg(not(any(
    feature = "backend-wgpu",
    feature = "backend-glow",
    feature = "backend-cpu"
)))]
compile_error!("At least one backend feature (`backend-wgpu`, `backend-glow`, or `backend-cpu`) must be enabled.");

#[allow(unreachable_code)]
fn default_backend() -> BackendKind {
    // Dedicated AWS build (only backend-cpu enabled): launch directly in CPU GDI mode, zero GPU probing.
    #[cfg(all(
        feature = "backend-cpu",
        not(feature = "backend-wgpu"),
        not(feature = "backend-glow")
    ))]
    {
        return BackendKind::Cpu;
    }

    // Windows desktop build: prefer glow (OpenGL) first to avoid DX12 resize/acquire stalls (#1/#3),
    // with wgpu and CPU GDI fallbacks available.
    #[cfg(all(target_os = "windows", feature = "backend-glow"))]
    {
        return BackendKind::Glow;
    }

    // macOS / Linux / wgpu-only builds: prefer native Metal / Vulkan / DX12 via wgpu.
    #[cfg(feature = "backend-wgpu")]
    {
        return BackendKind::Wgpu;
    }

    #[cfg(all(not(feature = "backend-wgpu"), feature = "backend-glow"))]
    {
        return BackendKind::Glow;
    }
}

/// Ordered fallback chain for the current build's enabled features.
fn next_fallback(current: BackendKind) -> Option<BackendKind> {
    match current {
        #[cfg(feature = "backend-glow")]
        BackendKind::Glow => {
            #[cfg(feature = "backend-wgpu")]
            {
                return Some(BackendKind::Wgpu);
            }
            #[cfg(all(not(feature = "backend-wgpu"), feature = "backend-cpu"))]
            {
                return Some(BackendKind::Cpu);
            }
            #[cfg(all(not(feature = "backend-wgpu"), not(feature = "backend-cpu")))]
            {
                return None;
            }
        }
        #[cfg(feature = "backend-wgpu")]
        BackendKind::Wgpu => {
            #[cfg(feature = "backend-cpu")]
            {
                return Some(BackendKind::Cpu);
            }
            #[cfg(not(feature = "backend-cpu"))]
            {
                return None;
            }
        }
        #[cfg(feature = "backend-cpu")]
        BackendKind::Cpu => None,
    }
}

fn append_startup_log(line: &str) {
    use std::io::Write;
    let log_path = std::env::temp_dir().join("sas-viewer-error.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "{line}");
    }
}

fn show_fatal_error(msg: &str) {
    let log_path = std::env::temp_dir().join("sas-viewer-error.log");
    append_startup_log(msg);
    let full = format!(
        "SAS 数据集查看器启动失败：\n\n{}\n\n详细日志已保存至：\n{}",
        msg,
        log_path.display()
    );
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("SAS 数据集查看器 - 启动错误")
        .set_description(&full)
        .show();
}

fn spawn_with_backend(kind: BackendKind, no_fallback: bool, paths: &[PathBuf]) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(kind.cli_flag());
    if no_fallback {
        cmd.arg("--no-fallback");
    }
    for p in paths {
        cmd.arg(p);
    }
    cmd.spawn().is_ok()
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("程序发生未捕获异常：\n{info}");
        show_fatal_error(&msg);
    }));

    let mut initial_paths: Vec<PathBuf> = Vec::new();
    let mut mode = default_backend();
    let mut no_fallback = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            #[cfg(feature = "backend-wgpu")]
            "--renderer=wgpu" => mode = BackendKind::Wgpu,
            #[cfg(feature = "backend-glow")]
            "--renderer=glow" => mode = BackendKind::Glow,
            #[cfg(feature = "backend-cpu")]
            "--renderer=software" | "--renderer=cpu" => mode = BackendKind::Cpu,
            "--no-fallback" => no_fallback = true,
            _ if arg.starts_with("--renderer=") => {
                // Ignore unsupported renderer flags gracefully in single-backend builds
            }
            _ => initial_paths.push(PathBuf::from(arg)),
        }
    }

    if let Err(err) = window_host::run_app(mode, initial_paths.clone()) {
        append_startup_log(&format!("[fallback] {} failed: {err}", mode.display_name()));
        if !no_fallback {
            if let Some(fb) = next_fallback(mode) {
                let is_last = next_fallback(fb).is_none();
                if spawn_with_backend(fb, is_last, &initial_paths) {
                    return;
                }
            }
        }
        show_fatal_error(&format!("{} 渲染器启动失败：\n{err}", mode.display_name()));
    }
}
