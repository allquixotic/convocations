//! rconv GUI module using eframe/egui
//!
//! This module provides the graphical user interface for rconv using eframe 0.33.3.

pub mod app;
pub mod async_bridge;
pub mod dialogs;
pub mod oauth;
pub mod processor;
pub mod state;
pub mod tray;
pub mod ui_state;
pub mod widgets;

/// Main entry point for the GUI
pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 900.0])
            .with_min_inner_size([960.0, 700.0])
            .with_resizable(true)
            .with_title("Convocations"),
        ..Default::default()
    };

    eframe::run_native(
        "Convocations",
        native_options,
        Box::new(|cc| {
            Ok(Box::new(app::RconvApp::new(cc)))
        }),
    )
    .map_err(|e| format!("{:?}", e))
    .map_err(|e| Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)) as Box<dyn std::error::Error + Send + Sync>)?;

    Ok(())
}
