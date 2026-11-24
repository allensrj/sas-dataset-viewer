#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod panels;
mod sample_gen;
mod software_renderer;
mod state;
mod theme;

use app::SasViewerApp;
use eframe::egui_wgpu::{wgpu, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererMode {
    Wgpu,
    Glow,
    Software,
}

fn build_wgpu_config() -> WgpuConfiguration {
    let setup = WgpuSetupCreateNew {
        instance_descriptor: wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(|| {
                if cfg!(target_os = "windows") {
                    wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::GL
                } else {
                    wgpu::Backends::PRIMARY | wgpu::Backends::GL
                }
            }),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
        },
        power_preference: wgpu::PowerPreference::HighPerformance,
        native_adapter_selector: Some(Arc::new(|adapters, surface| {
            let compatible: Vec<&wgpu::Adapter> = adapters
                .iter()
                .filter(|a| surface.map_or(true, |s| a.is_surface_supported(s)))
                .collect();

            let rank = |a: &wgpu::Adapter| -> (u8, u8) {
                let info = a.get_info();
                let dev_rank = match info.device_type {
                    wgpu::DeviceType::DiscreteGpu => 0,
                    wgpu::DeviceType::IntegratedGpu => 1,
                    wgpu::DeviceType::VirtualGpu => 2,
                    wgpu::DeviceType::Cpu => 3,
                    wgpu::DeviceType::Other => 4,
                };
                let backend_rank = match info.backend {
                    wgpu::Backend::Metal => 0,
                    wgpu::Backend::Dx12 => 0,
                    wgpu::Backend::Vulkan => 1,
                    wgpu::Backend::Gl => 2,
                    _ => 3,
                };
                (dev_rank, backend_rank)
            };

            compatible
                .into_iter()
                .min_by_key(|a| rank(a))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "未找到可用的图形适配器（已枚举 {} 个适配器，均不兼容当前窗口）",
                        adapters.len()
                    )
                })
        })),
        device_descriptor: Arc::new(|adapter| {
            let limits =
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
            wgpu::DeviceDescriptor {
                label: Some("sas-viewer-wgpu-device"),
                required_features: wgpu::Features::default(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::default(),
            }
        }),
        trace_path: None,
    };

    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

fn make_native_options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SAS 数据集查看器 (XPT / SAS7BDAT)")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([920.0, 560.0])
            .with_drag_and_drop(true),
        renderer,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        wgpu_options: build_wgpu_config(),
        ..Default::default()
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

fn spawn_with_renderer(flag: &str, no_fallback: bool, paths: &[PathBuf]) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(flag);
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
    let mut mode = RendererMode::Wgpu;
    let mut no_fallback = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--renderer=wgpu" => mode = RendererMode::Wgpu,
            "--renderer=glow" => mode = RendererMode::Glow,
            "--renderer=software" => mode = RendererMode::Software,
            "--no-fallback" => no_fallback = true,
            _ => initial_paths.push(PathBuf::from(arg)),
        }
    }

    match mode {
        RendererMode::Wgpu => {
            let paths = initial_paths.clone();
            if let Err(err) = eframe::run_native(
                "SAS Dataset Viewer",
                make_native_options(eframe::Renderer::Wgpu),
                Box::new(move |cc| Ok(Box::new(SasViewerApp::new(cc, paths)))),
            ) {
                append_startup_log(&format!("[fallback] wgpu failed: {err}"));
                if !no_fallback && spawn_with_renderer("--renderer=glow", false, &initial_paths) {
                    return;
                }
                show_fatal_error(&format!("Wgpu 渲染器启动失败：\n{err}"));
            }
        }
        RendererMode::Glow => {
            let paths = initial_paths.clone();
            if let Err(err) = eframe::run_native(
                "SAS Dataset Viewer",
                make_native_options(eframe::Renderer::Glow),
                Box::new(move |cc| Ok(Box::new(SasViewerApp::new(cc, paths)))),
            ) {
                append_startup_log(&format!("[fallback] glow failed: {err}"));
                // Tier 3 fallback: pure-CPU Win32 GDI software renderer (zero GPU/OpenGL/DX12 required!)
                if !no_fallback
                    && spawn_with_renderer("--renderer=software", true, &initial_paths)
                {
                    return;
                }
                show_fatal_error(&format!("Glow 渲染器启动失败：\n{err}"));
            }
        }
        RendererMode::Software => {
            if let Err(err) = software_renderer::run_software_app(initial_paths) {
                show_fatal_error(&format!("CPU 软件渲染模式启动失败：\n{err}"));
            }
        }
    }
}
