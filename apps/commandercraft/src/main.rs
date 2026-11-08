//! CommanderCraft desktop app: a two-panel, keyboard-driven file manager.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use commandercraft_ui_egui::CommanderApp;

fn main() -> eframe::Result {
    let settings = commandercraft_ui_egui::Settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CommanderCraft")
            .with_app_id("commandercraft")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("CommanderCraft", options, Box::new(move |_cc| Ok(Box::new(CommanderApp::new(settings)))))
}
