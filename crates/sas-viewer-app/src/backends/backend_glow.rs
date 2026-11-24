use crate::backends::RenderBackend;
use egui::{ClippedPrimitive, Color32, TexturesDelta};
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{ApiPreference, DisplayBuilder};
use raw_window_handle::HasWindowHandle;
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub struct GlowBackend {
    window: Arc<Window>,
    gl_context: PossiblyCurrentContext,
    gl_surface: Surface<WindowSurface>,
    painter: egui_glow::Painter,
}

impl GlowBackend {
    pub fn new(event_loop: &ActiveEventLoop) -> Result<Self, String> {
        let attrs = Window::default_attributes()
            .with_title("SAS 数据集查看器 (XPT / SAS7BDAT)")
            .with_inner_size(LogicalSize::new(1440.0, 880.0))
            .with_min_inner_size(LogicalSize::new(920.0, 560.0))
            .with_visible(false);

        let template = ConfigTemplateBuilder::new()
            .prefer_hardware_accelerated(None)
            .with_depth_size(0)
            .with_stencil_size(0)
            .with_transparency(false);

        let display_builder = DisplayBuilder::new()
            .with_preference(ApiPreference::FallbackEgl)
            .with_window_attributes(Some(attrs));

        let (window_opt, gl_config) = display_builder
            .build(event_loop, template, |mut configs| {
                configs.next().expect("no matching glutin GL config")
            })
            .map_err(|e| format!("创建 OpenGL 显示配置失败: {e}"))?;

        let window = match window_opt {
            Some(w) => Arc::new(w),
            None => {
                let fallback_attrs = Window::default_attributes()
                    .with_title("SAS 数据集查看器 (XPT / SAS7BDAT)")
                    .with_inner_size(LogicalSize::new(1440.0, 880.0))
                    .with_min_inner_size(LogicalSize::new(920.0, 560.0))
                    .with_visible(false);
                Arc::new(
                    event_loop
                        .create_window(fallback_attrs)
                        .map_err(|e| format!("创建 OpenGL 窗口失败: {e}"))?,
                )
            }
        };

        let raw_window_handle = window
            .window_handle()
            .map_err(|e| format!("获取原生窗口句柄失败: {e}"))?
            .as_raw();

        let gl_display = gl_config.display();
        let ctx_attrs = ContextAttributesBuilder::new().build(Some(raw_window_handle));
        let fallback_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(None))
            .build(Some(raw_window_handle));

        let not_current_ctx = unsafe {
            gl_display
                .create_context(&gl_config, &ctx_attrs)
                .or_else(|_| gl_display.create_context(&gl_config, &fallback_attrs))
                .map_err(|e| format!("创建 OpenGL 上下文失败: {e}"))?
        };

        let (w_px, h_px): (u32, u32) = window.inner_size().into();
        let w_nz = NonZeroU32::new(w_px).unwrap_or(NonZeroU32::MIN);
        let h_nz = NonZeroU32::new(h_px).unwrap_or(NonZeroU32::MIN);
        let surf_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_window_handle,
            w_nz,
            h_nz,
        );

        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surf_attrs)
                .map_err(|e| format!("创建 OpenGL 窗口表面失败: {e}"))?
        };

        let gl_context = not_current_ctx
            .make_current(&gl_surface)
            .map_err(|e| format!("激活 OpenGL 上下文失败: {e}"))?;

        let _ = gl_surface.set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::MIN));

        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s).cast())
        });

        let painter = egui_glow::Painter::new(gl, "", None, true)
            .map_err(|e| format!("初始化 egui_glow 渲染器失败: {e}"))?;

        Ok(Self {
            window,
            gl_context,
            gl_surface,
            painter,
        })
    }
}

impl RenderBackend for GlowBackend {
    fn window(&self) -> &Arc<Window> {
        &self.window
    }

    fn max_texture_side(&self) -> Option<usize> {
        Some(self.painter.max_texture_side())
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            self.gl_surface.resize(&self.gl_context, w, h);
        }
    }

    fn paint(
        &mut self,
        pixels_per_point: f32,
        clear_color: Color32,
        clipped_primitives: &[ClippedPrimitive],
        textures_delta: &TexturesDelta,
    ) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let dims = [size.width, size.height];
        let rgba = clear_color.to_normalized_gamma_f32();
        self.painter.clear(dims, rgba);
        self.painter
            .paint_and_update_textures(dims, pixels_per_point, clipped_primitives, textures_delta);
        let _ = self.gl_surface.swap_buffers(&self.gl_context);
    }

    fn destroy(&mut self) {
        self.painter.destroy();
    }
}
