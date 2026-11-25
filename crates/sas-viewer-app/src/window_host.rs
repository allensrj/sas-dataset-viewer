//! Unified `winit 0.30` + `egui-winit 0.31` window host shared across ALL render backends
//! (`wgpu`, `glow`, and `cpu`).
//!
//! Directly resolves Issue #3 (Windows modal move/size freeze):
//! - Bypasses `eframe`'s hardcoded synchronous Windows repaint inside `on_window_event`.
//! - Never triggers redundant full-frame repaints on pure window translation (`WindowEvent::Moved`).
//! - Coalesces `WindowEvent::Resized` and applies surface resize once at paint time.
//! - Creates the window invisible and shows it only after the first frame is presented (no startup flash).

use crate::app::{DisplayScaleContext, SasViewerApp};
use crate::backends::{BackendKind, RenderBackend};
use egui::ViewportId;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

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
/// egui 不提供这个能力，最实际的缓解办法是把低 DPI 屏（原生缩放 < 1.2，即 100% 屏）
/// 上的 UI 默认放大到 1.2x；而 125% 及以上屏幕本身有效像素密度已 >= 1.25x，保持 1.0x
/// 以避免在 1080p 125% 笔记本上被叠加放大到 150%。
const LOW_DPI_THRESHOLD: f32 = 1.2;
const LOW_DPI_ZOOM: f32 = 1.2;

fn default_zoom_for(native_ppp: f32) -> f32 {
    if native_ppp < LOW_DPI_THRESHOLD {
        LOW_DPI_ZOOM
    } else {
        1.0
    }
}

fn native_key(native_ppp: f32) -> i32 {
    (native_ppp * 1000.0).round() as i32
}

/// 按显示器（以原生缩放因子区分）记住用户手动设置的缩放倍率。
///
/// 没有这层记忆时，把窗口从主屏拖到副屏再拖回来，`apply_default_zoom` 会重新
/// 套用默认值，把用户按 `Ctrl -` 调好的倍率冲掉。未记录过的显示器才回落到
/// `default_zoom_for`。
#[derive(Debug)]
struct ZoomPrefs {
    /// 最近一次由本程序写入（或刚从用户改动同步过来）的缩放倍率。
    last_applied: f32,
    /// (原生缩放 × 1000, 缩放倍率)
    per_native: Vec<(i32, f32)>,
}

impl ZoomPrefs {
    fn new() -> Self {
        Self {
            last_applied: 1.0,
            per_native: Vec::new(),
        }
    }

    /// 若用户在本帧改了缩放（与上次应用的倍率不同），记录到当前显示器。
    fn sync_from_ui(&mut self, native_ppp: f32, current: f32) {
        if (current - self.last_applied).abs() < 1e-4 {
            return;
        }
        self.last_applied = current;
        self.remember(native_ppp, current);
    }

    /// 进入某个显示器时应当使用的倍率：优先恢复用户在这块屏上的选择。
    fn desired_for(&mut self, native_ppp: f32) -> f32 {
        let key = native_key(native_ppp);
        let zoom = self
            .per_native
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, z)| *z)
            .unwrap_or_else(|| default_zoom_for(native_ppp));
        self.last_applied = zoom;
        zoom
    }

    fn remember(&mut self, native_ppp: f32, zoom: f32) {
        self.remember_key(native_key(native_ppp), zoom);
    }

    fn remember_key(&mut self, key: i32, zoom: f32) {
        if let Some(slot) = self.per_native.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = zoom;
        } else {
            self.per_native.push((key, zoom));
        }
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
    zoom_prefs: ZoomPrefs,
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
            zoom_prefs: ZoomPrefs::new(),
        }
    }

    /// 按当前显示器的原生缩放设置默认 UI 缩放。
    ///
    /// 只在「原生缩放因子发生变化」时应用一次（启动、拖到另一块屏幕），
    /// 这样用户随后用 `Ctrl +` / `Ctrl -` / `Ctrl 0` 手动调整的缩放不会被每帧覆盖。
    /// 切换显示器前会先把上一块屏幕可能刚发生的手动缩放同步进 [`ZoomPrefs`]，
    /// 避免“按完 Ctrl- 立刻拖到副屏”时丢失记录。
    fn apply_default_zoom(&mut self, native_ppp: f32) {
        if let Some(prev_ppp) = self.zoom_applied_for {
            if native_key(prev_ppp) == native_key(native_ppp) {
                return;
            }
            let current = self.egui_ctx.zoom_factor();
            self.zoom_prefs.sync_from_ui(prev_ppp, current);
        }
        self.zoom_applied_for = Some(native_ppp);
        let zoom = self.zoom_prefs.desired_for(native_ppp);
        self.egui_ctx.set_zoom_factor(zoom);
    }

    /// 记录用户手动改过的缩放（`Ctrl +/-/0`），供跨显示器恢复。
    fn sync_user_zoom(&mut self) {
        let Some(native_ppp) = self.zoom_applied_for else {
            return;
        };
        let current = self.egui_ctx.zoom_factor();
        self.zoom_prefs.sync_from_ui(native_ppp, current);
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

        let active_native_ppp = self
            .zoom_applied_for
            .unwrap_or_else(|| window.scale_factor() as f32);
        let display_scale = DisplayScaleContext {
            native_key: native_key(active_native_ppp),
            base_zoom: default_zoom_for(active_native_ppp),
        };

        let raw_input = egui_state.take_egui_input(&window);
        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            app.update_ui(ctx, display_scale);
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

        // `egui::gui_zoom::zoom_with_keyboard` runs inside `ctx.run()` -> `end_pass()`;
        // sync right after the pass so manual `Ctrl +/-/0` is captured immediately.
        self.sync_user_zoom();
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
        // Seed per-monitor ZoomPrefs from saved user display scale preferences (if any)
        for (&key, &scale) in app.saved_monitor_ui_scales() {
            let native_ppp = (key as f32) / 1000.0;
            let eff = (default_zoom_for(native_ppp) * scale.clamp(0.6, 2.5)).clamp(0.5, 3.0);
            self.zoom_prefs.remember_key(key, eff);
        }

        self.backend = Some(backend);
        self.egui_state = Some(egui_state);
        self.app = Some(app);

        // 低 DPI 屏默认放大（见 LOW_DPI_THRESHOLD 的说明）
        self.apply_default_zoom(window.scale_factor() as f32);

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
        let mut new_scale_factor: Option<f32> = None;

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
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                should_redraw = true;
                new_scale_factor = Some(*scale_factor as f32);
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

        // 窗口被拖到另一块缩放比例不同的显示器上：同步旧屏手动倍率并按新屏重新计算缩放
        if let Some(sf) = new_scale_factor {
            self.apply_default_zoom(sf);
            if let Some(backend) = self.backend.as_ref() {
                backend.window().request_redraw();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_zoom_depends_on_native_scale() {
        assert_eq!(default_zoom_for(1.0), LOW_DPI_ZOOM);
        // 125% 及以上屏幕有效像素密度已 >= 1.25x，保持 1:1 避免在 1080p 笔记本上过度放大
        assert_eq!(default_zoom_for(1.25), 1.0);
        assert_eq!(default_zoom_for(1.5), 1.0);
        assert_eq!(default_zoom_for(2.0), 1.0);
    }

    /// 回归测试：在主屏用 Ctrl- 缩到 0.5 后，拖到副屏再拖回主屏，
    /// 必须恢复到 0.5，而不是被默认值 1.0 冲掉。
    #[test]
    fn manual_zoom_survives_monitor_round_trip() {
        let mut prefs = ZoomPrefs::new();

        // 首次进入主屏（200%）：默认 1.0
        assert_eq!(prefs.desired_for(2.0), 1.0);
        // 用户在主屏上按 Ctrl- 缩到 0.5
        prefs.sync_from_ui(2.0, 0.5);
        // 拖到副屏（100%）：首次进入，用低 DPI 默认值
        assert_eq!(prefs.desired_for(1.0), LOW_DPI_ZOOM);
        // 拖回主屏：恢复用户此前手动设置的 0.5
        assert_eq!(prefs.desired_for(2.0), 0.5);
    }

    #[test]
    fn per_monitor_zoom_is_independent() {
        let mut prefs = ZoomPrefs::new();

        prefs.desired_for(2.0);
        prefs.sync_from_ui(2.0, 0.6);
        prefs.desired_for(1.0);
        prefs.sync_from_ui(1.0, 1.5);

        assert_eq!(prefs.desired_for(2.0), 0.6);
        assert_eq!(prefs.desired_for(1.0), 1.5);
    }

    #[test]
    fn no_record_when_ui_matches_applied_value() {
        let mut prefs = ZoomPrefs::new();
        let zoom = prefs.desired_for(1.0);
        // 用户没有手动改动：不应产生记录
        prefs.sync_from_ui(1.0, zoom);
        assert!(prefs.per_native.is_empty());
    }
}
