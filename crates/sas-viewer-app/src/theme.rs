use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, Stroke, Style, Visuals};
use std::sync::Arc;

/// Palette tokens tuned for crisp clinical/data-table readability in both Light & Dark themes.
#[derive(Debug, Clone, Copy)]
pub struct AppPalette {
    pub bg_canvas: Color32,
    pub bg_panel: Color32,
    pub bg_header_name: Color32,
    pub bg_header_label: Color32,
    pub bg_row_even: Color32,
    pub bg_row_odd: Color32,
    pub bg_row_hover: Color32,
    pub bg_selected_cell: Color32,
    pub bg_selected_row: Color32,
    pub bg_selected_col_hdr: Color32,
    pub bg_chip: Color32,
    pub border_grid: Color32,
    pub border_header: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub text_missing: Color32,
    pub badge_sas: Color32,
    pub badge_xpt: Color32,
    pub badge_num: Color32,
    pub badge_char: Color32,
    pub bar_fill: Color32,
}

impl AppPalette {
    pub fn light() -> Self {
        Self {
            bg_canvas: Color32::from_rgb(248, 250, 252),
            bg_panel: Color32::from_rgb(244, 246, 249),
            bg_header_name: Color32::from_rgb(235, 240, 247),
            bg_header_label: Color32::from_rgb(245, 247, 251),
            bg_row_even: Color32::WHITE,
            bg_row_odd: Color32::from_rgb(249, 251, 253),
            bg_row_hover: Color32::from_rgb(237, 245, 255),
            bg_selected_cell: Color32::from_rgb(214, 232, 255),
            bg_selected_row: Color32::from_rgb(238, 246, 255),
            bg_selected_col_hdr: Color32::from_rgb(216, 231, 252),
            bg_chip: Color32::from_rgb(226, 238, 255),
            border_grid: Color32::from_rgb(228, 233, 240),
            border_header: Color32::from_rgb(203, 213, 225),
            accent: Color32::from_rgb(29, 102, 214),
            accent_soft: Color32::from_rgb(219, 234, 254),
            text_primary: Color32::from_rgb(24, 32, 44),
            text_muted: Color32::from_rgb(100, 114, 134),
            text_missing: Color32::from_rgb(156, 163, 175),
            badge_sas: Color32::from_rgb(37, 99, 235),
            badge_xpt: Color32::from_rgb(13, 148, 136),
            badge_num: Color32::from_rgb(37, 99, 235),
            badge_char: Color32::from_rgb(180, 83, 9),
            bar_fill: Color32::from_rgb(147, 197, 253),
        }
    }

    pub fn dark() -> Self {
        Self {
            bg_canvas: Color32::from_rgb(22, 25, 31),
            bg_panel: Color32::from_rgb(28, 32, 40),
            bg_header_name: Color32::from_rgb(36, 42, 54),
            bg_header_label: Color32::from_rgb(30, 35, 45),
            bg_row_even: Color32::from_rgb(24, 28, 35),
            bg_row_odd: Color32::from_rgb(28, 32, 40),
            bg_row_hover: Color32::from_rgb(38, 47, 62),
            bg_selected_cell: Color32::from_rgb(38, 72, 122),
            bg_selected_row: Color32::from_rgb(32, 44, 64),
            bg_selected_col_hdr: Color32::from_rgb(44, 62, 92),
            bg_chip: Color32::from_rgb(38, 56, 86),
            border_grid: Color32::from_rgb(44, 50, 62),
            border_header: Color32::from_rgb(62, 70, 86),
            accent: Color32::from_rgb(82, 152, 255),
            accent_soft: Color32::from_rgb(34, 58, 94),
            text_primary: Color32::from_rgb(232, 237, 245),
            text_muted: Color32::from_rgb(148, 160, 178),
            text_missing: Color32::from_rgb(112, 122, 138),
            badge_sas: Color32::from_rgb(96, 165, 250),
            badge_xpt: Color32::from_rgb(45, 212, 191),
            badge_num: Color32::from_rgb(96, 165, 250),
            badge_char: Color32::from_rgb(251, 191, 36),
            bar_fill: Color32::from_rgb(59, 130, 246),
        }
    }

    pub fn for_mode(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

pub fn apply_theme(ctx: &Context, dark_mode: bool) {
    let mut style = Style::default();
    let mut visuals = if dark_mode {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let pal = AppPalette::for_mode(dark_mode);
    visuals.panel_fill = pal.bg_panel;
    visuals.window_fill = pal.bg_panel;
    visuals.extreme_bg_color = pal.bg_row_even;
    visuals.selection.bg_fill = pal.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, pal.accent);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border_grid);

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(7.0, 3.0);
    ctx.set_style(style);
}

/// Load native OS CJK fonts so Chinese UI labels, variable labels, and dataset contents render crisply.
pub fn setup_cjk_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyh.ttf",
            r"C:\Windows\Fonts\simhei.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ]
    };

    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let font_name = "system_cjk".to_string();
            fonts
                .font_data
                .insert(font_name.clone(), Arc::new(FontData::from_owned(bytes)));

            if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                fam.insert(0, font_name.clone());
            }
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                fam.push(font_name);
            }
            break;
        }
    }

    ctx.set_fonts(fonts);
}
