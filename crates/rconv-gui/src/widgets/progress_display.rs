//! Progress display widget

use crate::state::ProcessorState;

/// Render progress display
pub fn render(ui: &mut egui::Ui, state: &ProcessorState) {
    match state {
        ProcessorState::Running { progress, job_id } => {
            ui.vertical(|ui| {
                ui.heading("Processing...");

                if let Ok(info) = progress.lock() {
                    if let Some(ref stage) = info.stage {
                        ui.label(format!("Stage: {}", stage));
                    }

                    if let Some(ref message) = info.message {
                        ui.label(message);
                    }

                    if let Some(elapsed_ms) = info.elapsed_ms {
                        let elapsed_secs = elapsed_ms / 1000.0;
                        ui.label(format!("Elapsed: {:.1}s", elapsed_secs));
                    }

                    // Indeterminate progress bar
                    ui.add(egui::ProgressBar::new(f32::NAN));
                }

                ui.label(format!("Job ID: {}", job_id));
            });
        }
        ProcessorState::Completed { summary, .. } => {
            ui.vertical(|ui| {
                ui.colored_label(egui::Color32::GREEN, "✓ Processing Completed");
                ui.label(summary);
            });
        }
        ProcessorState::Error { message } => {
            ui.vertical(|ui| {
                ui.colored_label(egui::Color32::RED, "✗ Processing Failed");
                ui.label(message);
            });
        }
        ProcessorState::Idle => {
            // Nothing to show
        }
    }
}
