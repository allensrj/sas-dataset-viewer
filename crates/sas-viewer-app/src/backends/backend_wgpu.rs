use crate::backends::RenderBackend;
use egui::{ClippedPrimitive, Color32, Context, TexturesDelta, ViewportId};
use egui_wgpu::{wgpu, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub struct WgpuBackend {
    window: Arc<Window>,
    painter: egui_wgpu::winit::Painter,
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
                        "未找到可用的 wgpu 图形适配器（已枚举 {} 个适配器，均不兼容当前窗口）",
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
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: Some(2),
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

impl WgpuBackend {
    pub fn new(event_loop: &ActiveEventLoop, egui_ctx: &Context) -> Result<Self, String> {
        let attrs = Window::default_attributes()
            .with_title("SAS 数据集查看器 (XPT / SAS7BDAT)")
            .with_inner_size(LogicalSize::new(1440.0, 880.0))
            .with_min_inner_size(LogicalSize::new(920.0, 560.0))
            .with_visible(false);

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("创建 wgpu 窗口失败: {e}"))?,
        );

        let mut painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            build_wgpu_config(),
            1,
            None,
            false,
            true,
        ));

        pollster::block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))
            .map_err(|e| format!("初始化 wgpu 表面/设备失败: {e}"))?;

        Ok(Self { window, painter })
    }
}

impl RenderBackend for WgpuBackend {
    fn window(&self) -> &Arc<Window> {
        &self.window
    }

    fn max_texture_side(&self) -> Option<usize> {
        self.painter.max_texture_side()
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            self.painter.on_window_resized(ViewportId::ROOT, w, h);
        }
    }

    fn paint(
        &mut self,
        pixels_per_point: f32,
        clear_color: Color32,
        clipped_primitives: &[ClippedPrimitive],
        textures_delta: &TexturesDelta,
    ) {
        let rgba = clear_color.to_normalized_gamma_f32();
        self.painter.paint_and_update_textures(
            ViewportId::ROOT,
            pixels_per_point,
            rgba,
            clipped_primitives,
            textures_delta,
            Vec::new(),
        );
    }

    fn destroy(&mut self) {
        self.painter.destroy();
    }
}
