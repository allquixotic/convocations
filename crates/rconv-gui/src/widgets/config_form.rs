//! Configuration form widget

use crate::dialogs;
use crate::state::AppState;
use rconv_core::config::OutputTarget;

/// Render the configuration form
/// Returns true if any value was changed
pub fn render(ui: &mut egui::Ui, state: &mut AppState) -> bool {
    ui.vertical(|ui| {
        let mut changed = false;
        // Preset selection
        ui.horizontal(|ui| {
            ui.label("Event Preset:")
                .on_hover_text("Select a preset for recurring events (e.g., RSM7, WVM)");

            let current_preset = &state.config.runtime.active_preset;

            let response = egui::ComboBox::from_id_salt("preset_selector")
                .selected_text(current_preset)
                .show_ui(ui, |ui| {
                    let mut preset_changed = false;
                    if ui.selectable_value(&mut state.config.runtime.active_preset, String::new(), "None").changed() {
                        preset_changed = true;
                    }

                    for preset in &state.config.presets {
                        if ui.selectable_value(
                            &mut state.config.runtime.active_preset,
                            preset.name.clone(),
                            &preset.name
                        ).changed() {
                            preset_changed = true;
                        }
                    }
                    preset_changed
                });

            if response.inner.unwrap_or(false) {
                changed = true;
            }
        });

        ui.add_space(8.0);

        // Weeks ago
        if ui.horizontal(|ui| {
            ui.label("Weeks Ago:")
                .on_hover_text("How many weeks in the past to process (0 = current week)");
            ui.add(egui::DragValue::new(&mut state.config.runtime.weeks_ago).range(0..=52))
        }).inner.changed() {
            changed = true;
        }

        ui.add_space(8.0);

        // Show calculated date/time range when a preset is selected
        if !state.config.runtime.active_preset.is_empty() {
            // Find the selected preset
            if let Some(preset) = state.config.presets.iter().find(|p| p.name == state.config.runtime.active_preset) {
                // Calculate duration to use
                let duration_minutes = if state.config.runtime.duration_override.enabled {
                    (state.config.runtime.duration_override.hours * 60.0) as i64
                } else {
                    preset.duration_minutes as i64
                };

                // Determine event type from preset name
                let event_type = if preset.name == rconv_core::TUESDAY_7_PRESET_NAME {
                    "rsm7"
                } else if preset.name == rconv_core::TUESDAY_8_PRESET_NAME {
                    "rsm8"
                } else if preset.name == rconv_core::FRIDAY_6_PRESET_NAME {
                    "tp6"
                } else {
                    "saturday"
                };

                // Calculate dates
                if let Ok((start, end)) = rconv_core::calculate_event_dates(
                    chrono::Local::now().date_naive(),
                    state.config.runtime.weeks_ago,
                    event_type,
                    duration_minutes,
                ) {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("Event Date Range:").strong());
                        ui.horizontal(|ui| {
                            ui.label("Start:");
                            ui.label(egui::RichText::new(&start).monospace());
                        });
                        ui.horizontal(|ui| {
                            ui.label("End:");
                            ui.label(egui::RichText::new(&end).monospace());
                        });
                    });
                }
            }
        }

        ui.add_space(8.0);

        // Duration override
        if ui.horizontal(|ui| {
            ui.label("Duration Override:")
                .on_hover_text("Override the default event duration from the preset");

            let mut duration_changed = ui.checkbox(&mut state.config.runtime.duration_override.enabled, "Enable").changed();

            if state.config.runtime.duration_override.enabled {
                if ui.add(egui::DragValue::new(&mut state.config.runtime.duration_override.hours)
                    .range(0.1..=24.0)
                    .suffix(" hours")).changed() {
                    duration_changed = true;
                }

                if ui.button("1h").clicked() {
                    state.config.runtime.duration_override.hours = 1.0;
                    duration_changed = true;
                }
                if ui.button("2h").clicked() {
                    state.config.runtime.duration_override.hours = 2.0;
                    duration_changed = true;
                }
            }
            duration_changed
        }).inner {
            changed = true;
        }

        ui.add_space(8.0);

        // File paths
        if ui.horizontal(|ui| {
            ui.label("ChatLog Path:")
                .on_hover_text("Path to the raw chat log file to process");
            let mut path_changed = ui.text_edit_singleline(&mut state.config.runtime.chat_log_path).changed();
            if ui.button("Browse...")
                .on_hover_text("Select a chat log file")
                .clicked() {
                if let Some(path) = dialogs::pick_chatlog() {
                    state.config.runtime.chat_log_path = path.to_string_lossy().to_string();
                    path_changed = true;
                }
            }
            path_changed
        }).inner {
            changed = true;
        }

        ui.add_space(8.0);

        // Output target
        if ui.horizontal(|ui| {
            ui.label("Output Target:");

            let is_file = matches!(state.config.runtime.output_target, OutputTarget::File);
            let mut target_changed = false;
            if ui.radio(is_file, "File").clicked() {
                state.config.runtime.output_target = OutputTarget::File;
                target_changed = true;
            }
            if ui.radio(!is_file, "Directory").clicked() {
                state.config.runtime.output_target = OutputTarget::Directory;
                target_changed = true;
            }
            target_changed
        }).inner {
            changed = true;
        }

        if matches!(state.config.runtime.output_target, OutputTarget::Directory) {
            if ui.horizontal(|ui| {
                ui.label("Output Directory:");
                let mut output_dir = state.config.runtime.output_directory_override.clone().unwrap_or_default();
                let mut dir_changed = false;
                if ui.text_edit_singleline(&mut output_dir).changed() {
                    state.config.runtime.output_directory_override = Some(output_dir);
                    dir_changed = true;
                }
                if ui.button("Browse...").clicked() {
                    if let Some(path) = dialogs::pick_output_directory() {
                        state.config.runtime.output_directory_override = Some(path.to_string_lossy().to_string());
                        dir_changed = true;
                    }
                }
                dir_changed
            }).inner {
                changed = true;
            }
        } else {
            if ui.horizontal(|ui| {
                ui.label("Output File:");
                let mut output_file = state.config.runtime.outfile_override.clone().unwrap_or_default();
                let mut file_changed = ui.text_edit_singleline(&mut output_file).changed();
                if ui.button("Browse...").clicked() {
                    if let Some(path) = dialogs::save_output_file() {
                        state.config.runtime.outfile_override = Some(path.to_string_lossy().to_string());
                        file_changed = true;
                    }
                }
                if file_changed {
                    state.config.runtime.outfile_override = if output_file.is_empty() {
                        None
                    } else {
                        Some(output_file)
                    };
                }
                file_changed
            }).inner {
                changed = true;
            }

            // Show the default output file name that will be used
            if state.config.runtime.outfile_override.is_none() || state.config.runtime.outfile_override.as_ref().map(|s| s.is_empty()).unwrap_or(false) {
                // Calculate default output file name
                let (convocations_config, _warnings) = rconv_core::config::runtime_preferences_to_convocations(
                    &state.config.runtime,
                    &state.config.presets,
                );

                if let Ok(outfile_resolution) = rconv_core::runtime::resolve_outfile_paths(
                    &convocations_config,
                    None,
                    Some(chrono::Local::now().date_naive()),
                ) {
                    ui.label(egui::RichText::new(format!("  (default: {})", outfile_resolution.default)).italics().weak());
                }
            }
        }

        ui.add_space(8.0);

        // Dry run
        if ui.checkbox(&mut state.config.runtime.dry_run, "Dry Run (preview only, don't create files)").changed() {
            changed = true;
        }

        changed
    }).inner
}
