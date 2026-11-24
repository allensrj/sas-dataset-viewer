//! Pure-CPU `egui` software rasterizer over `softbuffer` (Win32 GDI `BitBlt` on Windows,
//! CoreGraphics on macOS, X11/Wayland shm on Linux) with multi-core `rayon` band parallelism.
//!
//! Zero GPU, zero OpenGL 2.0+, zero DirectX 12 requirements — guarantees instant single-process
//! startup on AWS EC2 Xeon servers, Windows Server, RDP, and virtual display adapters.

use crate::backends::RenderBackend;
use egui::{ClippedPrimitive, Color32, ImageData, Mesh, Rect, TextureId, TexturesDelta};
use rayon::prelude::*;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

struct SoftTexture {
    width: usize,
    height: usize,
    pixels: Vec<Color32>,
}

impl SoftTexture {
    #[inline]
    fn sample_bilinear(&self, u: f32, v: f32) -> Color32 {
        if self.width == 0 || self.height == 0 {
            return Color32::WHITE;
        }
        let fx = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let fy = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        if tx < 1e-3 && ty < 1e-3 {
            return self.pixels[y0 * self.width + x0];
        }

        let c00 = self.pixels[y0 * self.width + x0];
        let c10 = self.pixels[y0 * self.width + x1];
        let c01 = self.pixels[y1 * self.width + x0];
        let c11 = self.pixels[y1 * self.width + x1];

        let wx1 = (tx * 256.0) as u32;
        let wx0 = 256 - wx1;
        let wy1 = (ty * 256.0) as u32;
        let wy0 = 256 - wy1;

        let w00 = wx0 * wy0;
        let w10 = wx1 * wy0;
        let w01 = wx0 * wy1;
        let w11 = wx1 * wy1;

        let blend_ch = |a: u8, b: u8, c: u8, d: u8| -> u8 {
            ((a as u32 * w00 + b as u32 * w10 + c as u32 * w01 + d as u32 * w11 + 32768) >> 16)
                as u8
        };

        Color32::from_rgba_premultiplied(
            blend_ch(c00.r(), c10.r(), c01.r(), c11.r()),
            blend_ch(c00.g(), c10.g(), c01.g(), c11.g()),
            blend_ch(c00.b(), c10.b(), c01.b(), c11.b()),
            blend_ch(c00.a(), c10.a(), c01.a(), c11.a()),
        )
    }
}

pub struct CpuRasterizer {
    textures: HashMap<TextureId, SoftTexture>,
}

impl CpuRasterizer {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
        }
    }

    pub fn apply_textures_delta(&mut self, delta: &TexturesDelta) {
        for (id, image_delta) in &delta.set {
            let (img_w, img_h, new_pixels): (usize, usize, Vec<Color32>) = match &image_delta.image {
                ImageData::Color(c) => (c.width(), c.height(), c.pixels.clone()),
                ImageData::Font(f) => (f.width(), f.height(), f.srgba_pixels(None).collect()),
            };

            if let Some([px, py]) = image_delta.pos {
                if let Some(existing) = self.textures.get_mut(id) {
                    for y in 0..img_h {
                        let dst_y = py + y;
                        if dst_y >= existing.height {
                            break;
                        }
                        let src_row = y * img_w;
                        let dst_row = dst_y * existing.width + px;
                        let copy_w = img_w.min(existing.width.saturating_sub(px));
                        existing.pixels[dst_row..dst_row + copy_w]
                            .copy_from_slice(&new_pixels[src_row..src_row + copy_w]);
                    }
                    continue;
                }
            }

            self.textures.insert(
                *id,
                SoftTexture {
                    width: img_w,
                    height: img_h,
                    pixels: new_pixels,
                },
            );
        }

        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// Render all egui clipped primitives into an `0x00RRGGBB` framebuffer (`softbuffer` format).
    /// Uses lock-free horizontal band parallelism on large frames (>250k px) to leverage Xeon multi-core CPUs.
    pub fn paint(
        &self,
        fb: &mut [u32],
        fb_w: usize,
        fb_h: usize,
        pixels_per_point: f32,
        clear_rgb: u32,
        primitives: &[ClippedPrimitive],
    ) {
        if fb_w == 0 || fb_h == 0 {
            return;
        }

        if fb_w * fb_h >= 250_000 && fb_h >= 128 {
            // Split framebuffer into horizontal bands across CPU cores
            let band_rows = (fb_h / 8).max(64);
            let band_pixels = band_rows * fb_w;
            fb.par_chunks_mut(band_pixels)
                .enumerate()
                .for_each(|(band_idx, band_slice)| {
                    band_slice.fill(clear_rgb);
                    let y_offset = (band_idx * band_rows) as i32;
                    let actual_rows = (band_slice.len() / fb_w) as i32;
                    let band_y1 = y_offset + actual_rows;

                    for prim in primitives {
                        if let egui::epaint::Primitive::Mesh(mesh) = &prim.primitive {
                            self.paint_mesh_band(
                                band_slice,
                                fb_w,
                                y_offset,
                                band_y1,
                                pixels_per_point,
                                prim.clip_rect,
                                mesh,
                            );
                        }
                    }
                });
        } else {
            fb.fill(clear_rgb);
            for prim in primitives {
                if let egui::epaint::Primitive::Mesh(mesh) = &prim.primitive {
                    self.paint_mesh_band(
                        fb,
                        fb_w,
                        0,
                        fb_h as i32,
                        pixels_per_point,
                        prim.clip_rect,
                        mesh,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_mesh_band(
        &self,
        band_fb: &mut [u32],
        fb_w: usize,
        band_y0: i32,
        band_y1: i32,
        ppp: f32,
        clip_rect: Rect,
        mesh: &Mesh,
    ) {
        if mesh.indices.is_empty() || mesh.vertices.is_empty() {
            return;
        }

        let clip_x0 = (clip_rect.min.x * ppp).floor().max(0.0) as i32;
        let clip_y0 = ((clip_rect.min.y * ppp).floor() as i32).max(band_y0);
        let clip_x1 = (clip_rect.max.x * ppp).ceil().min(fb_w as f32) as i32;
        let clip_y1 = ((clip_rect.max.y * ppp).ceil() as i32).min(band_y1);
        if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
            return;
        }

        let tex = self.textures.get(&mesh.texture_id);
        let idx = &mesh.indices;
        let verts = &mesh.vertices;
        let n_idx = idx.len();
        let mut i = 0;

        while i + 2 < n_idx {
            if i + 5 < n_idx {
                if let Some(quad) = detect_axis_aligned_quad(verts, &idx[i..i + 6], ppp) {
                    if quad.y1 <= clip_y0 || quad.y0 >= clip_y1 {
                        i += 6;
                        continue;
                    }
                    if quad.is_untextured {
                        fill_solid_rect(
                            band_fb,
                            fb_w,
                            band_y0,
                            quad.x0.max(clip_x0),
                            quad.y0.max(clip_y0),
                            quad.x1.min(clip_x1),
                            quad.y1.min(clip_y1),
                            quad.color,
                        );
                        i += 6;
                        continue;
                    } else if let Some(t) = tex {
                        fill_textured_rect(
                            band_fb, fb_w, band_y0, &quad, clip_x0, clip_y0, clip_x1, clip_y1, t,
                        );
                        i += 6;
                        continue;
                    }
                }
            }

            let v0 = verts[idx[i] as usize];
            let v1 = verts[idx[i + 1] as usize];
            let v2 = verts[idx[i + 2] as usize];
            rasterize_triangle(
                band_fb, fb_w, band_y0, ppp, clip_x0, clip_y0, clip_x1, clip_y1, &v0, &v1, &v2, tex,
            );
            i += 3;
        }
    }
}

struct AxisQuad {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    fx0: f32,
    fy0: f32,
    inv_w: f32,
    inv_h: f32,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    color: Color32,
    is_untextured: bool,
}

#[inline]
fn detect_axis_aligned_quad(
    verts: &[egui::epaint::Vertex],
    tri6: &[u32],
    ppp: f32,
) -> Option<AxisQuad> {
    let a = verts[tri6[0] as usize];
    let b = verts[tri6[1] as usize];
    let c = verts[tri6[2] as usize];
    let d = verts[tri6[3] as usize];
    let e = verts[tri6[4] as usize];
    let f = verts[tri6[5] as usize];

    let col = a.color;
    if b.color != col || c.color != col || d.color != col || e.color != col || f.color != col {
        return None;
    }

    let pts = [a, b, c, d, e, f];
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut min_u = f32::INFINITY;
    let mut max_u = f32::NEG_INFINITY;
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;

    for v in &pts {
        min_x = min_x.min(v.pos.x);
        max_x = max_x.max(v.pos.x);
        min_y = min_y.min(v.pos.y);
        max_y = max_y.max(v.pos.y);
        min_u = min_u.min(v.uv.x);
        max_u = max_u.max(v.uv.x);
        min_v = min_v.min(v.uv.y);
        max_v = max_v.max(v.uv.y);
    }

    const EPS: f32 = 1e-3;
    let mut corner_mask = 0u8;
    for v in &pts {
        let on_left = (v.pos.x - min_x).abs() <= EPS;
        let on_right = (v.pos.x - max_x).abs() <= EPS;
        let on_top = (v.pos.y - min_y).abs() <= EPS;
        let on_bot = (v.pos.y - max_y).abs() <= EPS;
        if !((on_left || on_right) && (on_top || on_bot)) {
            return None;
        }
        let bit = match (on_right, on_bot) {
            (false, false) => 1,
            (true, false) => 2,
            (false, true) => 4,
            (true, true) => 8,
        };
        corner_mask |= bit;
    }
    if corner_mask != 0b1111 {
        return None;
    }

    let is_untextured = max_u <= 1e-4 && max_v <= 1e-4;
    if !is_untextured {
        for v in &pts {
            let on_right = (v.pos.x - max_x).abs() <= EPS;
            let on_bot = (v.pos.y - max_y).abs() <= EPS;
            let exp_u = if on_right { max_u } else { min_u };
            let exp_v = if on_bot { max_v } else { min_v };
            if (v.uv.x - exp_u).abs() > 1e-4 || (v.uv.y - exp_v).abs() > 1e-4 {
                return None;
            }
        }
    }

    let fx0 = min_x * ppp;
    let fy0 = min_y * ppp;
    let fx1 = max_x * ppp;
    let fy1 = max_y * ppp;
    let w = fx1 - fx0;
    let h = fy1 - fy0;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }

    Some(AxisQuad {
        x0: fx0.round() as i32,
        y0: fy0.round() as i32,
        x1: fx1.round() as i32,
        y1: fy1.round() as i32,
        fx0,
        fy0,
        inv_w: 1.0 / w,
        inv_h: 1.0 / h,
        u0: min_u,
        v0: min_v,
        u1: max_u,
        v1: max_v,
        color: col,
        is_untextured,
    })
}

#[inline]
fn blend_premul(dst: u32, sr: u32, sg: u32, sb: u32, sa: u32) -> u32 {
    if sa == 0 {
        return dst;
    }
    if sa >= 255 {
        return (sr.min(255) << 16) | (sg.min(255) << 8) | sb.min(255);
    }
    let inv_a = 255 - sa;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let r = (sr + ((dr * inv_a + 127) / 255)).min(255);
    let g = (sg + ((dg * inv_a + 127) / 255)).min(255);
    let b = (sb + ((db * inv_a + 127) / 255)).min(255);
    (r << 16) | (g << 8) | b
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn fill_solid_rect(
    band_fb: &mut [u32],
    fb_w: usize,
    band_y0: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    c: Color32,
) {
    if x0 >= x1 || y0 >= y1 || c.a() == 0 {
        return;
    }
    let x0 = x0 as usize;
    let x1 = x1 as usize;
    let (sr, sg, sb, sa) = (c.r() as u32, c.g() as u32, c.b() as u32, c.a() as u32);
    if sa == 255 {
        let packed = (sr << 16) | (sg << 8) | sb;
        for y in y0..y1 {
            let row = ((y - band_y0) as usize) * fb_w;
            band_fb[row + x0..row + x1].fill(packed);
        }
    } else {
        for y in y0..y1 {
            let row = ((y - band_y0) as usize) * fb_w;
            for px in &mut band_fb[row + x0..row + x1] {
                *px = blend_premul(*px, sr, sg, sb, sa);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_textured_rect(
    band_fb: &mut [u32],
    fb_w: usize,
    band_y0: i32,
    quad: &AxisQuad,
    clip_x0: i32,
    clip_y0: i32,
    clip_x1: i32,
    clip_y1: i32,
    tex: &SoftTexture,
) {
    let x0 = quad.x0.max(clip_x0);
    let y0 = quad.y0.max(clip_y0);
    let x1 = quad.x1.min(clip_x1);
    let y1 = quad.y1.min(clip_y1);
    if x0 >= x1 || y0 >= y1 || quad.color.a() == 0 {
        return;
    }
    let (vr, vg, vb, va) = (
        quad.color.r() as u32,
        quad.color.g() as u32,
        quad.color.b() as u32,
        quad.color.a() as u32,
    );
    let du = quad.u1 - quad.u0;
    let dv = quad.v1 - quad.v0;

    for y in y0..y1 {
        let ty_norm = ((y as f32 + 0.5) - quad.fy0) * quad.inv_h;
        let v = quad.v0 + ty_norm * dv;
        let row = ((y - band_y0) as usize) * fb_w;
        for x in x0..x1 {
            let tx_norm = ((x as f32 + 0.5) - quad.fx0) * quad.inv_w;
            let u = quad.u0 + tx_norm * du;
            let tc = tex.sample_bilinear(u, v);
            let ta = tc.a() as u32;
            if ta == 0 {
                continue;
            }
            let sr = (vr * tc.r() as u32 + 127) / 255;
            let sg = (vg * tc.g() as u32 + 127) / 255;
            let sb = (vb * tc.b() as u32 + 127) / 255;
            let sa = (va * ta + 127) / 255;
            let idx = row + x as usize;
            band_fb[idx] = blend_premul(band_fb[idx], sr, sg, sb, sa);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rasterize_triangle(
    band_fb: &mut [u32],
    fb_w: usize,
    band_y0: i32,
    ppp: f32,
    clip_x0: i32,
    clip_y0: i32,
    clip_x1: i32,
    clip_y1: i32,
    v0: &egui::epaint::Vertex,
    v1: &egui::epaint::Vertex,
    v2: &egui::epaint::Vertex,
    tex: Option<&SoftTexture>,
) {
    let x0 = v0.pos.x * ppp;
    let y0 = v0.pos.y * ppp;
    let x1 = v1.pos.x * ppp;
    let y1 = v1.pos.y * ppp;
    let x2 = v2.pos.x * ppp;
    let y2 = v2.pos.y * ppp;

    let area = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
    if area.abs() < 1e-4 {
        return;
    }
    let inv_area = 1.0 / area;

    let min_x = (x0.min(x1).min(x2).floor() as i32).max(clip_x0);
    let max_x = (x0.max(x1).max(x2).ceil() as i32).min(clip_x1 - 1);
    let min_y = (y0.min(y1).min(y2).floor() as i32).max(clip_y0);
    let max_y = (y0.max(y1).max(y2).ceil() as i32).min(clip_y1 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }

    let untextured = v0.uv.x <= 1e-4
        && v0.uv.y <= 1e-4
        && v1.uv.x <= 1e-4
        && v1.uv.y <= 1e-4
        && v2.uv.x <= 1e-4
        && v2.uv.y <= 1e-4;

    let uniform_color = v0.color == v1.color && v1.color == v2.color;
    let sign = if area > 0.0 { 1.0f32 } else { -1.0f32 };

    for py in min_y..=max_y {
        let fy = py as f32 + 0.5;
        let row = ((py - band_y0) as usize) * fb_w;
        for px in min_x..=max_x {
            let fx = px as f32 + 0.5;
            let w0 = ((x1 - fx) * (y2 - fy) - (y1 - fy) * (x2 - fx)) * sign;
            if w0 < 0.0 {
                continue;
            }
            let w1 = ((x2 - fx) * (y0 - fy) - (y2 - fy) * (x0 - fx)) * sign;
            if w1 < 0.0 {
                continue;
            }
            let w2 = ((x0 - fx) * (y1 - fy) - (y0 - fy) * (x1 - fx)) * sign;
            if w2 < 0.0 {
                continue;
            }

            let b0 = w0 * inv_area * sign;
            let b1 = w1 * inv_area * sign;
            let b2 = 1.0 - b0 - b1;

            let (vr, vg, vb, va) = if uniform_color {
                (
                    v0.color.r() as u32,
                    v0.color.g() as u32,
                    v0.color.b() as u32,
                    v0.color.a() as u32,
                )
            } else {
                (
                    (b0 * v0.color.r() as f32 + b1 * v1.color.r() as f32 + b2 * v2.color.r() as f32)
                        .clamp(0.0, 255.0) as u32,
                    (b0 * v0.color.g() as f32 + b1 * v1.color.g() as f32 + b2 * v2.color.g() as f32)
                        .clamp(0.0, 255.0) as u32,
                    (b0 * v0.color.b() as f32 + b1 * v1.color.b() as f32 + b2 * v2.color.b() as f32)
                        .clamp(0.0, 255.0) as u32,
                    (b0 * v0.color.a() as f32 + b1 * v1.color.a() as f32 + b2 * v2.color.a() as f32)
                        .clamp(0.0, 255.0) as u32,
                )
            };

            let (sr, sg, sb, sa) = if untextured {
                (vr, vg, vb, va)
            } else if let Some(t) = tex {
                let u = b0 * v0.uv.x + b1 * v1.uv.x + b2 * v2.uv.x;
                let v = b0 * v0.uv.y + b1 * v1.uv.y + b2 * v2.uv.y;
                let tc = t.sample_bilinear(u, v);
                (
                    (vr * tc.r() as u32 + 127) / 255,
                    (vg * tc.g() as u32 + 127) / 255,
                    (vb * tc.b() as u32 + 127) / 255,
                    (va * tc.a() as u32 + 127) / 255,
                )
            } else {
                (vr, vg, vb, va)
            };

            let idx = row + px as usize;
            band_fb[idx] = blend_premul(band_fb[idx], sr, sg, sb, sa);
        }
    }
}

pub struct CpuBackend {
    window: Arc<Window>,
    _context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    rasterizer: CpuRasterizer,
    surface_size: (u32, u32),
}

impl CpuBackend {
    pub fn new(event_loop: &ActiveEventLoop, title_suffix: &str) -> Result<Self, String> {
        let title = if title_suffix.is_empty() {
            "SAS 数据集查看器 (XPT / SAS7BDAT)".to_string()
        } else {
            format!("SAS 数据集查看器 (XPT / SAS7BDAT) [{title_suffix}]")
        };

        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1440.0, 880.0))
            .with_min_inner_size(LogicalSize::new(920.0, 560.0))
            .with_visible(false);

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("创建软件渲染窗口失败: {e}"))?,
        );

        let context = softbuffer::Context::new(window.clone())
            .map_err(|e| format!("初始化 GDI/Softbuffer 上下文失败: {e}"))?;
        let surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|e| format!("初始化 GDI/Softbuffer 表面失败: {e}"))?;

        Ok(Self {
            window,
            _context: context,
            surface,
            rasterizer: CpuRasterizer::new(),
            surface_size: (0, 0),
        })
    }
}

impl RenderBackend for CpuBackend {
    fn window(&self) -> &Arc<Window> {
        &self.window
    }

    fn max_texture_side(&self) -> Option<usize> {
        Some(4096)
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if let (Some(nw), Some(nh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            if self.surface.resize(nw, nh).is_ok() {
                self.surface_size = (size.width, size.height);
            }
        }
    }

    fn paint(
        &mut self,
        pixels_per_point: f32,
        clear_color: Color32,
        clipped_primitives: &[ClippedPrimitive],
        textures_delta: &TexturesDelta,
    ) {
        self.rasterizer.apply_textures_delta(textures_delta);

        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.surface_size != (size.width, size.height) {
            self.resize(size);
        }

        let clear_rgb = ((clear_color.r() as u32) << 16)
            | ((clear_color.g() as u32) << 8)
            | (clear_color.b() as u32);

        if let Ok(mut buffer) = self.surface.buffer_mut() {
            self.rasterizer.paint(
                &mut buffer,
                size.width as usize,
                size.height as usize,
                pixels_per_point,
                clear_rgb,
                clipped_primitives,
            );
            let _ = buffer.present();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::{Primitive, Vertex};
    use egui::pos2;

    #[test]
    fn test_cpu_rasterizer_solid_quad_and_band_parallelism() {
        let rast = CpuRasterizer::new();
        // Test both small (single-threaded) and large (rayon band-parallel) framebuffers
        for (w, h) in [(20usize, 20usize), (800usize, 600usize)] {
            let mut fb = vec![0u32; w * h];
            let mut mesh = Mesh::default();
            let red = Color32::from_rgb(200, 40, 20);
            mesh.vertices = vec![
                Vertex { pos: pos2(2.0, 2.0), uv: pos2(0.0, 0.0), color: red },
                Vertex { pos: pos2((w - 2) as f32, 2.0), uv: pos2(0.0, 0.0), color: red },
                Vertex { pos: pos2((w - 2) as f32, (h - 2) as f32), uv: pos2(0.0, 0.0), color: red },
                Vertex { pos: pos2(2.0, (h - 2) as f32), uv: pos2(0.0, 0.0), color: red },
            ];
            mesh.indices = vec![0, 1, 2, 0, 2, 3];
            let clip_max = (h / 2) as f32;
            let prim = ClippedPrimitive {
                clip_rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(clip_max, clip_max)),
                primitive: Primitive::Mesh(mesh),
            };
            rast.paint(&mut fb, w, h, 1.0, 0x00112233, &[prim]);
            let expected_red = (200 << 16) | (40 << 8) | 20;
            assert_eq!(fb[3 * w + 3], expected_red);
            assert_eq!(fb[(h - 3) * w + (w - 3)], 0x00112233);
        }
    }
}
