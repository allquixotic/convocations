//! Diff preview widget

/// Render diff preview
pub fn render(ui: &mut egui::Ui, diff_text: &str) {
    ui.vertical(|ui| {
        ui.heading("Diff Preview");

        ui.separator();

        // Scrollable diff area - much bigger when expanded
        egui::ScrollArea::vertical()
            .max_height(800.0)
            .auto_shrink([false, false])  // Don't shrink - always take full space
            .show(ui, |ui| {
                // Set monospace font for the entire diff area
                ui.style_mut().override_font_id = Some(egui::FontId::monospace(12.0));

                // Parse and colorize diff
                for line in diff_text.lines() {
                    let (color, text) = if line.starts_with('+') && !line.starts_with("+++") {
                        (egui::Color32::from_rgb(0, 255, 0), line) // Green for additions
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        (egui::Color32::from_rgb(255, 0, 0), line) // Red for deletions
                    } else if line.starts_with("@@") {
                        (egui::Color32::from_rgb(0, 200, 255), line) // Cyan for hunks
                    } else {
                        (egui::Color32::GRAY, line) // Gray for context
                    };

                    ui.colored_label(color, text);
                }
            });

        ui.add_space(8.0);

        if ui.button("Copy to Clipboard").clicked() {
            ui.ctx().copy_text(diff_text.to_string());
        }
    });
}
