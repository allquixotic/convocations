//! API key management widget

use crate::state::AppState;
use crate::ui_state::UiState;

/// Render API key section
pub fn render(ui: &mut egui::Ui, state: &mut AppState, ui_state: &mut UiState, oauth_callback: &mut dyn FnMut()) {
    ui.vertical(|ui| {
        ui.heading("OpenRouter API");

        // Check if API key is stored
        let has_key = state.config.runtime.has_openrouter_api_key();

        if has_key {
            ui.colored_label(egui::Color32::GREEN, "✓ API Key is saved");

            if ui.button("Clear API Key").clicked() {
                if let Err(e) = state.config.runtime.clear_openrouter_api_key() {
                    eprintln!("Error clearing API key: {}", e);
                }
            }
        } else {
            ui.colored_label(egui::Color32::YELLOW, "⚠ No API Key saved");

            ui.add_space(8.0);

            // OAuth login button
            if ui_state.oauth_pending {
                ui.label("⏳ Waiting for OAuth authorization...");
                ui.label("Check your browser to complete login.");
            } else {
                if ui.button("🔐 Login with OAuth").clicked() {
                    oauth_callback();
                }
            }

            ui.label("— or —");

            // Direct key entry
            ui.horizontal(|ui| {
                ui.label("API Key:");
                let _response = ui.add(egui::TextEdit::singleline(&mut ui_state.api_key_input).password(true));

                if ui.button("Save").clicked() && !ui_state.api_key_input.is_empty() {
                    if let Err(e) = state.config.runtime.set_openrouter_api_key(&ui_state.api_key_input) {
                        eprintln!("Error saving API key: {}", e);
                    } else {
                        // Clear the input after successful save
                        ui_state.api_key_input.clear();
                    }
                }
            });
        }
    });
}
