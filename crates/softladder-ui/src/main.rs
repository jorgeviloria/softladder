//! SoftLadder editor binary: opens the `eframe` window running
//! [`softladder_ui::SoftLadderApp`].

#![forbid(unsafe_code)]

use eframe::egui;
use softladder_ui::SoftLadderApp;

#[rustfmt::skip]
fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0])
        .with_title("SoftLadder");
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("SoftLadder", options, Box::new(|cc| Ok(Box::new(SoftLadderApp::new(cc)))))
}
