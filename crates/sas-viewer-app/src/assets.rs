use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::io::Cursor;
use winit::window::Icon;

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app_icon.png");
const WECHAT_ACCOUNT_PNG: &[u8] = include_bytes!("../assets/wechat_account.png");

struct DecodedRgba {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn decode_png_rgba(bytes: &[u8]) -> Option<DecodedRgba> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let buf_size = reader.output_buffer_size()?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).ok()?;
    let raw = &buf[..info.buffer_size()];

    let rgba = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.chunks_exact(2) {
                out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &g in raw {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::Indexed => return None,
    };

    Some(DecodedRgba {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Decode the embedded application logo into a `winit::window::Icon` (used for Windows/Linux
/// titlebar, taskbar, and Alt-Tab icons).
pub fn load_window_icon() -> Option<Icon> {
    let decoded = decode_png_rgba(APP_ICON_PNG)?;
    Icon::from_rgba(decoded.rgba, decoded.width, decoded.height).ok()
}

/// Lazily uploaded UI textures shared across the toolbar, welcome screen, and About modal.
#[derive(Default)]
pub struct AppTextures {
    app_logo: Option<TextureHandle>,
    app_logo_attempted: bool,
    wechat_qr: Option<TextureHandle>,
    wechat_qr_attempted: bool,
}

impl AppTextures {
    pub fn app_logo(&mut self, ctx: &Context) -> Option<&TextureHandle> {
        if !self.app_logo_attempted {
            self.app_logo_attempted = true;
            if let Some(decoded) = decode_png_rgba(APP_ICON_PNG) {
                let image = ColorImage::from_rgba_unmultiplied(
                    [decoded.width as usize, decoded.height as usize],
                    &decoded.rgba,
                );
                self.app_logo =
                    Some(ctx.load_texture("app_logo_icon", image, TextureOptions::LINEAR));
            }
        }
        self.app_logo.as_ref()
    }

    pub fn wechat_qr(&mut self, ctx: &Context) -> Option<&TextureHandle> {
        if !self.wechat_qr_attempted {
            self.wechat_qr_attempted = true;
            if let Some(decoded) = decode_png_rgba(WECHAT_ACCOUNT_PNG) {
                let image = ColorImage::from_rgba_unmultiplied(
                    [decoded.width as usize, decoded.height as usize],
                    &decoded.rgba,
                );
                self.wechat_qr =
                    Some(ctx.load_texture("about_wechat_qr", image, TextureOptions::LINEAR));
            }
        }
        self.wechat_qr.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_png_assets_decode_cleanly() {
        let logo = decode_png_rgba(APP_ICON_PNG).expect("app_icon.png must decode");
        assert_eq!(logo.width, 256);
        assert_eq!(logo.height, 256);
        assert_eq!(logo.rgba.len(), 256 * 256 * 4);
        assert!(load_window_icon().is_some());

        let wechat = decode_png_rgba(WECHAT_ACCOUNT_PNG).expect("wechat_account.png must decode");
        assert!(wechat.width > 500 && wechat.height > 150);
        assert_eq!(wechat.rgba.len(), (wechat.width * wechat.height * 4) as usize);
    }
}
