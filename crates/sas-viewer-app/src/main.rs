#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod panels;
mod sample_gen;
mod state;
mod theme;

use app::SasViewerApp;
use eframe::egui_wgpu::{wgpu, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use std::path::PathBuf;
use std::sync::Arc;

fn build_wgpu_config() -> WgpuConfiguration {
    let setup = WgpuSetupCreateNew {
        instance_descriptor: wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(|| {
                if cfg!(target_os = "windows") {
                    // Prefer DX12 (which includes built-in Windows WARP software rasterizer on AWS/RDP VMs without GPU)
                    wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::GL
                } else {
                    wgpu::Backends::PRIMARY | wgpu::Backends::GL
                }
            }),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
        },
        power_preference: wgpu::PowerPreference::HighPerformance,
        // Smart adapter selector:
        // 1. Hardware Discrete/Integrated GPU (laptops & desktops)
        // 2. Virtual GPU (hypervisors)
        // 3. CPU / Software rasterizer (Windows DX12 WARP "Microsoft Basic Render Driver" on AWS EC2 / headless servers!)
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
                    wgpu::DeviceType::Cpu => 3, // DX12 WARP / llvmpipe software fallback
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

fn write_error_log(msg: &str) -> PathBuf {
    let log_path = std::env::temp_dir().join("sas-viewer-error.log");
    let _ = std::fs::write(&log_path, msg);
    log_path
}

fn show_fatal_error(msg: &str) {
    let log_path = write_error_log(msg);
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

fn main() {
    // Capture panics and display a clear error dialog instead of vanishing silently on Windows
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("程序发生未捕获异常：\n{info}");
        show_fatal_error(&msg);
    }));

    let mut initial_paths: Vec<PathBuf> = Vec::new();
    let mut prefer_glow = false;
    let mut no_fallback = false;

    for arg in std::env::args().skip(1) {
        if arg == "--renderer=glow" {
            prefer_glow = true;
        } else if arg == "--renderer=wgpu" {
            prefer_glow = false;
        } else if arg == "--no-fallback" {
            no_fallback = true;
        } else {
            initial_paths.push(PathBuf::from(arg));
        }
    }

    let primary = if prefer_glow {
        eframe::Renderer::Glow
    } else {
        eframe::Renderer::Wgpu
    };

    let paths_primary = initial_paths.clone();
    let res = eframe::run_native(
        "SAS Dataset Viewer",
        make_native_options(primary),
        Box::new(move |cc| Ok(Box::new(SasViewerApp::new(cc, paths_primary)))),
    );

    if let Err(primary_err) = res {
        // Re-launch cleanly in a fresh process with Glow fallback (winit forbids creating two EventLoops in one process)
        if !prefer_glow && !no_fallback {
            if let Ok(exe) = std::env::current_exe() {
                let mut cmd = std::process::Command::new(exe);
                cmd.arg("--renderer=glow").arg("--no-fallback");
                for p in &initial_paths {
                    cmd.arg(p);
                }
                if cmd.spawn().is_ok() {
                    return;
                }
            }
        }
        let msg = format!("渲染器 ({primary:?}) 启动失败：\n{primary_err}");
        show_fatal_error(&msg);
    }
}
