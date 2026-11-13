#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod highlight;
mod markdown;
mod store;
mod theme;

use app::Notan;

const WINDOW_ICON: &[u8] = include_bytes!("../assets/logo-128.rgba");

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("notan")
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([680.0, 440.0])
            .with_icon(eframe::egui::IconData {
                rgba: WINDOW_ICON.to_vec(),
                width: 128,
                height: 128,
            }),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "notan",
        options,
        Box::new(|cc| Ok(Box::new(Notan::new(cc)))),
    )
}
