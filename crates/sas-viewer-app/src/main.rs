#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod panels;
mod sample_gen;
mod state;
mod theme;

use app::SasViewerApp;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let initial_paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SAS 数据集查看器 (XPT / SAS7BDAT)")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([920.0, 560.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "SAS Dataset Viewer",
        native_options,
        Box::new(move |cc| Ok(Box::new(SasViewerApp::new(cc, initial_paths)))),
    )
}
