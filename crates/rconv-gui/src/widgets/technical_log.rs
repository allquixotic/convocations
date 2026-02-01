//! Technical log widget

use crate::ui_state::{LogLevel, UiState};

/// Render technical log
pub fn render(ui: &mut egui::Ui, ui_state: &mut UiState) {
    ui.vertical(|ui| {
        ui.heading("Technical Log");

        // Auto-scroll checkbox
        ui.horizontal(|ui| {
            ui.label("Entries:");
            ui.label(format!("{} / 200", ui_state.technical_log.len()));

            if ui.button("Clear").clicked() {
                ui_state.technical_log.clear();
            }
        });

        ui.separator();

        // Scrollable log area - bigger when expanded
        egui::ScrollArea::vertical()
            .max_height(600.0)
            .auto_shrink([false, false])  // Don't shrink - always take full space
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &ui_state.technical_log {
                    ui.horizontal(|ui| {
                        ui.label(&entry.timestamp);

                        let (color, prefix) = match entry.level {
                            LogLevel::Info => (egui::Color32::GRAY, "INFO"),
                            LogLevel::Warning => (egui::Color32::YELLOW, "WARN"),
                            LogLevel::Error => (egui::Color32::RED, "ERROR"),
                        };

                        ui.colored_label(color, prefix);
                        ui.label(&entry.message);
                    });
                }
            });
    });
}
