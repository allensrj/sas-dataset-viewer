//! Unified `winit 0.30` + `egui-winit 0.31` window host shared across ALL render backends
//! (`wgpu`, `glow`, and `cpu`).
//!
//! Directly resolves Issue #3 (Windows modal move/size freeze):
//! - Bypasses `eframe`'s hardcoded synchronous Windows repaint inside `on_window_event`.
//! - Never triggers redundant full-frame repaints on pure window translation (`WindowEvent::Moved`).
//! - Coalesces `WindowEvent::Resized` and applies surface resize once at paint time.
//! - Creates the window invisible and shows it only after the first frame is presented (no startup flash).

use crate::app::SasViewerApp;
use crate::backends::{BackendKind, RenderBackend};
use egui::ViewportId;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

#[derive(Debug, Clone, Copy)]
enum HostEvent {
    RequestRepaint(Instant),
}

fn create_backend(
    kind: BackendKind,
    event_loop: &ActiveEventLoop,
    _egui_ctx: &egui::Context,
) -> Result<Box<dyn RenderBackend>, String> {
    match kind {
        #[cfg(feature = "backend-wgpu")]
        BackendKind::Wgpu => {
            let b = crate::backends::backend_wgpu::WgpuBackend::new(event_loop, _egui_ctx)?;
            Ok(Box::new(b))
        }
        #[cfg(feature = "backend-glow")]
        BackendKind::Glow => {
            let b = crate::backends::backend_glow::GlowBackend::new(event_loop)?;
            Ok(Box::new(b))
        }
        #[cfg(feature = "backend-cpu")]
        BackendKind::Cpu => {
            // In the dedicated AWS build (only backend-cpu compiled in), keep title clean;
            // when used as fallback inside win-x64, annotate the title so mode is obvious.
            let suffix = if cfg!(any(feature = "backend-wgpu", feature = "backend-glow")) {
                "CPU 兼容模式"
            } else {
                "AWS 兼容版"
            };
            let b = crate::backends::backend_cpu::CpuBackend::new(event_loop, suffix)?;
            Ok(Box::new(b))
        }
    }
}

/// egui 的文字光栅化没有 hinting：在 100% 缩放下，界面里的中文只有 11~13 物理像素高，
/// 笔画本身约 1 像素，而抗锯齿过渡几乎与笔画同宽，字形就会显得发糊、笔画粘连；
/// 同样的字体在 200% 缩放下有 2 倍像素，过渡只相当于笔画的一半，看起来就干净。
///
/// Windows 自带控件靠 ClearType/hinting 把竖笔画吸附到整像素，所以同缩放比例下更锐利；
/// egui 不提供这个能力，最实际的缓解办法是把低 DPI 屏上的 UI 默认放大一档，
/// 让同一字号落在更多物理像素上。
const LOW_DPI_THRESHOLD: f32 = 1.5;
const LOW_DPI_ZOOM: f32 = 1.2;

fn default_zoom_for(native_ppp: f32) -> f32 {
    if native_ppp < LOW_DPI_THRESHOLD {
        LOW_DPI_ZOOM
    } else {
        1.0
    }
}

struct WindowHost {
    kind: BackendKind,
    initial_paths: Vec<PathBuf>,
    proxy: EventLoopProxy<HostEvent>,
    egui_ctx: egui::Context,
    backend: Option<Box<dyn RenderBackend>>,
    egui_state: Option<egui_winit::State>,
    app: Option<SasViewerApp>,
    pending_resize: Option<PhysicalSize<u32>>,
    next_repaint: Option<Instant>,
    window_shown: bool,
    init_error: Option<String>,
    /// 已经为哪个「原生缩放因子」应用过默认缩放，避免每帧覆盖用户手动缩放。
    zoom_applied_for: Option<f32>,
}

impl WindowHost {
    fn new(
        kind: BackendKind,
        initial_paths: Vec<PathBuf>,
        proxy: EventLoopProxy<HostEvent>,
    ) -> Self {
        Self {
            kind,
            initial_paths,
            proxy,
            egui_ctx: egui::Context::default(),
            backend: None,
            egui_state: None,
            app: None,
            pending_resize: None,
            next_repaint: None,
            window_shown: false,
            init_error: None,
            zoom_applied_for: None,
        }
    }

    /// 按当前显示器的原生缩放设置默认 UI 缩放。
    ///
    /// 只在「原生缩放因子发生变化」时应用一次（启动、拖到另一块屏幕），
    /// 这样用户随后用 `Ctrl +` / `Ctrl -` / `Ctrl 0` 手动调整的缩放不会被每帧覆盖。
    fn apply_default_zoom(&mut self, window: &Window) {
        let native_ppp = window.scale_factor() as f32;
        if self.zoom_applied_for == Some(native_ppp) {
            return;
        }
        self.zoom_applied_for = Some(native_ppp);
        self.egui_ctx.set_zoom_factor(default_zoom_for(native_ppp));
    }

    fn render_frame(&mut self) {
        let (Some(backend), Some(egui_state), Some(app)) = (
            self.backend.as_mut(),
            self.egui_state.as_mut(),
            self.app.as_mut(),
        ) else {
            return;
        };

        let window = backend.window().clone();
        if window.is_minimized() == Some(true) {
            return;
        }

        if let Some(sz) = self.pending_resize.take() {
            if sz.width > 0 && sz.height > 0 {
                backend.resize(sz);
            }
        }

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }

        let raw_input = egui_state.take_egui_input(&window);
        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            app.update_ui(ctx);
        });

        egui_state.handle_platform_output(&window, full_output.platform_output);

        let ppp = full_output.pixels_per_point;
        let clipped = self.egui_ctx.tessellate(full_output.shapes, ppp);
        let clear = self.egui_ctx.style().visuals.panel_fill;

        backend.paint(ppp, clear, &clipped, &full_output.textures_delta);

        // Reveal window only after the first frame is cleanly presented (eliminates startup flash)
        if !self.window_shown {
            window.set_visible(true);
            self.window_shown = true;
        }

        if let Some(vp) = full_output.viewport_output.get(&ViewportId::ROOT) {
            if vp.repaint_delay == Duration::ZERO {
                self.next_repaint = Some(Instant::now());
                window.request_redraw();
            } else if vp.repaint_delay < Duration::from_secs(60) {
                let target = Instant::now() + vp.repaint_delay;
                self.next_repaint = Some(match self.next_repaint {
                    Some(prev) => prev.min(target),
                    None => target,
                });
            }
        }
    }
}

impl ApplicationHandler<HostEvent> for WindowHost {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.backend.is_some() {
            return;
        }

        let backend = match create_backend(self.kind, event_loop, &self.egui_ctx) {
            Ok(b) => b,
            Err(err) => {
                self.init_error = Some(err);
                event_loop.exit();
                return;
            }
        };

        let window = backend.window().clone();
        let max_tex = backend.max_texture_side();

        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            event_loop.system_theme(),
            max_tex,
        );

        let proxy = self.proxy.clone();
        self.egui_ctx.set_request_repaint_callback(move |info| {
            let when = Instant::now() + info.delay;
            let _ = proxy.send_event(HostEvent::RequestRepaint(when));
        });

        let app = SasViewerApp::from_ctx(&self.egui_ctx, self.initial_paths.clone());

        self.backend = Some(backend);
        self.egui_state = Some(egui_state);
        self.app = Some(app);

        // 低 DPI 屏默认放大（见 LOW_DPI_THRESHOLD 的说明）
        self.apply_default_zoom(&window);

        // Paint first frame while window is invisible, then reveal smoothly
        self.render_frame();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostEvent) {
        match event {
            HostEvent::RequestRepaint(when) => {
                let now = Instant::now();
                if when <= now {
                    self.next_repaint = None;
                    if let Some(backend) = &self.backend {
                        backend.window().request_redraw();
                    }
                } else {
                    self.next_repaint = Some(match self.next_repaint {
                        Some(prev) => prev.min(when),
                        None => when,
                    });
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let mut should_redraw = false;
        let mut rescale_zoom = false;

        match &event {
            WindowEvent::CloseRequested => {
                if let Some(mut b) = self.backend.take() {
                    b.destroy();
                }
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                // Ignore 0x0 (Windows minimize event) and coalesce resize into RedrawRequested
                if size.width > 0 && size.height > 0 {
                    self.pending_resize = Some(*size);
                    should_redraw = true;
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                should_redraw = true;
                rescale_zoom = true;
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
                return;
            }
            _ => {}
        }

        if let (Some(backend), Some(egui_state)) = (self.backend.as_ref(), self.egui_state.as_mut())
        {
            let window = backend.window();
            let resp = egui_state.on_window_event(window, &event);
            // Key Issue #3 fix: pure window movement (Moved) does not invalidate the client buffer;
            // skipping redundant full-frame repaints lets Windows DWM move the window at native 120Hz.
            if resp.repaint && !matches!(event, WindowEvent::Moved(_)) {
                should_redraw = true;
            }
            if should_redraw {
                window.request_redraw();
            }
        }

        // 窗口被拖到另一块缩放比例不同的显示器上：重新计算默认缩放
        if rescale_zoom {
            let window = self.backend.as_ref().map(|b| b.window().clone());
            if let Some(window) = window {
                self.apply_default_zoom(&window);
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(target) = self.next_repaint {
            let now = Instant::now();
            if now >= target {
                self.next_repaint = None;
                if let Some(backend) = &self.backend {
                    backend.window().request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(target));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(mut b) = self.backend.take() {
            b.destroy();
        }
    }
}

pub fn run_app(kind: BackendKind, initial_paths: Vec<PathBuf>) -> Result<(), String> {
    let event_loop = EventLoop::<HostEvent>::with_user_event()
        .build()
        .map_err(|e| format!("无法创建窗口事件循环: {e}"))?;
    let proxy = event_loop.create_proxy();
    let mut host = WindowHost::new(kind, initial_paths, proxy);
    event_loop
        .run_app(&mut host)
        .map_err(|e| format!("事件循环运行异常: {e}"))?;
    if let Some(err) = host.init_error {
        return Err(err);
    }
    Ok(())
}
