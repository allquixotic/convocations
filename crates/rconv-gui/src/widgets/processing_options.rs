//! Processing options widget

use crate::state::AppState;

/// Render processing options
/// Returns true if any value was changed
pub fn render(ui: &mut egui::Ui, state: &mut AppState) -> bool {
    ui.vertical(|ui| {
        let mut changed = false;

        // Use AI corrections
        if ui.checkbox(&mut state.config.runtime.use_ai_corrections, "Use AI Corrections")
            .on_hover_text("Apply AI-powered corrections to the chat log")
            .changed() {
            changed = true;
        }

        // Keep original output
        if ui.checkbox(&mut state.config.runtime.keep_original_output, "Keep Original Output")
            .on_hover_text("Preserve the original processed file before AI corrections")
            .changed() {
            changed = true;
        }

        // Show diff
        if ui.checkbox(&mut state.config.runtime.show_diff, "Show Diff After Processing")
            .on_hover_text("Display a diff showing changes made during processing")
            .changed() {
            changed = true;
        }

        // Enable cleanup
        if ui.checkbox(&mut state.config.runtime.cleanup_enabled, "Enable Cleanup (remove OOC markers, normalize text)")
            .on_hover_text("Remove out-of-character markers and normalize text formatting")
            .changed() {
            changed = true;
        }

        // Format dialogue
        if ui.checkbox(&mut state.config.runtime.format_dialogue_enabled, "Format Dialogue")
            .on_hover_text("Apply dialogue formatting to improve readability")
            .changed() {
            changed = true;
        }

        changed
    }).inner
}
