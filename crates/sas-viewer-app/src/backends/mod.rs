#[cfg(feature = "backend-cpu")]
pub mod backend_cpu;
#[cfg(feature = "backend-glow")]
pub mod backend_glow;
#[cfg(feature = "backend-wgpu")]
pub mod backend_wgpu;

use egui::{ClippedPrimitive, Color32, TexturesDelta};
use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    #[cfg(feature = "backend-wgpu")]
    Wgpu,
    #[cfg(feature = "backend-glow")]
    Glow,
    #[cfg(feature = "backend-cpu")]
    Cpu,
}

impl BackendKind {
    pub fn cli_flag(&self) -> &'static str {
        match *self {
            #[cfg(feature = "backend-wgpu")]
            Self::Wgpu => "--renderer=wgpu",
            #[cfg(feature = "backend-glow")]
            Self::Glow => "--renderer=glow",
            #[cfg(feature = "backend-cpu")]
            Self::Cpu => "--renderer=software",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match *self {
            #[cfg(feature = "backend-wgpu")]
            Self::Wgpu => "wgpu",
            #[cfg(feature = "backend-glow")]
            Self::Glow => "glow (OpenGL)",
            #[cfg(feature = "backend-cpu")]
            Self::Cpu => "CPU GDI 兼容模式",
        }
    }
}

/// Pluggable rendering backend driven by our unified `winit` window host.
pub trait RenderBackend {
    fn window(&self) -> &Arc<Window>;
    fn max_texture_side(&self) -> Option<usize>;
    fn resize(&mut self, size: PhysicalSize<u32>);
    fn paint(
        &mut self,
        pixels_per_point: f32,
        clear_color: Color32,
        clipped_primitives: &[ClippedPrimitive],
        textures_delta: &TexturesDelta,
    );
    fn destroy(&mut self) {}
}
