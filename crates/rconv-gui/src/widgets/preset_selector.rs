//! Preset management widget

use crate::state::AppState;
use rconv_core::config::PresetDefinition;

/// State for preset creation/editing
pub struct PresetEditorState {
    pub editing: bool,
    pub editing_name: Option<String>,
    pub form: PresetForm,
}

impl Default for PresetEditorState {
    fn default() -> Self {
        Self {
            editing: false,
            editing_name: None,
            form: PresetForm::default(),
        }
    }
}

#[derive(Default, Clone)]
pub struct PresetForm {
    pub name: String,
    pub weekday: String,
    pub timezone: String,
    pub start_time: String,
    pub duration_minutes: u32,
    pub file_prefix: String,
    pub default_weeks_ago: u32,
}

impl PresetForm {
    fn from_preset(preset: &PresetDefinition) -> Self {
        Self {
            name: preset.name.clone(),
            weekday: preset.weekday.clone(),
            timezone: preset.timezone.clone(),
            start_time: preset.start_time.clone(),
            duration_minutes: preset.duration_minutes,
            file_prefix: preset.file_prefix.clone(),
            default_weeks_ago: preset.default_weeks_ago,
        }
    }

    fn to_preset(&self, builtin: bool) -> PresetDefinition {
        PresetDefinition {
            name: self.name.clone(),
            weekday: self.weekday.clone(),
            timezone: self.timezone.clone(),
            start_time: self.start_time.clone(),
            duration_minutes: self.duration_minutes,
            file_prefix: self.file_prefix.clone(),
            default_weeks_ago: self.default_weeks_ago,
            builtin,
        }
    }
}

/// Render preset management section
pub fn render(ui: &mut egui::Ui, state: &mut AppState, editor_state: &mut PresetEditorState) {
    ui.vertical(|ui| {
        ui.heading("Preset Management");

        // List existing presets
        ui.label("Custom Presets:");

        let custom_presets: Vec<_> = state.config.presets.iter()
            .filter(|p| !p.builtin)
            .cloned()
            .collect();

        if custom_presets.is_empty() {
            ui.label("No custom presets defined.");
        } else {
            for preset in &custom_presets {
                ui.horizontal(|ui| {
                    ui.label(&preset.name);
                    ui.label(format!("({}, {} @ {})", preset.weekday, preset.start_time, preset.timezone));

                    if ui.button("Edit").clicked() {
                        editor_state.editing = true;
                        editor_state.editing_name = Some(preset.name.clone());
                        editor_state.form = PresetForm::from_preset(preset);
                    }

                    if ui.button("Delete").clicked() {
                        // Delete preset
                        state.config.presets.retain(|p| p.name != preset.name);
                    }
                });
            }
        }

        ui.add_space(8.0);

        // Create new preset button
        if !editor_state.editing {
            if ui.button("+ Create New Preset").clicked() {
                editor_state.editing = true;
                editor_state.editing_name = None;
                editor_state.form = PresetForm {
                    name: String::new(),
                    weekday: "saturday".to_string(),
                    timezone: "America/New_York".to_string(),
                    start_time: "22:00".to_string(),
                    duration_minutes: 120,
                    file_prefix: "conv".to_string(),
                    default_weeks_ago: 0,
                };
            }
        }

        // Preset editor form
        if editor_state.editing {
            ui.separator();
            ui.heading(if editor_state.editing_name.is_some() {
                "Edit Preset"
            } else {
                "Create Preset"
            });

            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut editor_state.form.name);
            });

            ui.horizontal(|ui| {
                ui.label("Weekday:");
                egui::ComboBox::from_id_salt("preset_weekday")
                    .selected_text(&editor_state.form.weekday)
                    .show_ui(ui, |ui| {
                        for day in &["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"] {
                            ui.selectable_value(&mut editor_state.form.weekday, day.to_string(), *day);
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.label("Timezone:");
                ui.text_edit_singleline(&mut editor_state.form.timezone);
            });

            ui.horizontal(|ui| {
                ui.label("Start Time (HH:MM):");
                ui.text_edit_singleline(&mut editor_state.form.start_time);
            });

            ui.horizontal(|ui| {
                ui.label("Duration (minutes):");
                ui.add(egui::DragValue::new(&mut editor_state.form.duration_minutes).range(1..=1440));
            });

            ui.horizontal(|ui| {
                ui.label("File Prefix:");
                ui.text_edit_singleline(&mut editor_state.form.file_prefix);
            });

            ui.horizontal(|ui| {
                ui.label("Default Weeks Ago:");
                ui.add(egui::DragValue::new(&mut editor_state.form.default_weeks_ago).range(0..=52));
            });

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    let preset = editor_state.form.to_preset(false);

                    if let Some(ref edit_name) = editor_state.editing_name {
                        // Update existing preset
                        if let Some(existing) = state.config.presets.iter_mut().find(|p| p.name == *edit_name) {
                            *existing = preset;
                        }
                    } else {
                        // Create new preset
                        state.config.presets.push(preset);
                    }

                    // Save to disk
                    if let Err(e) = rconv_core::save_presets_and_ui_only(&state.config.presets, &state.config.ui) {
                        eprintln!("Error saving presets: {}", e);
                    }

                    editor_state.editing = false;
                }

                if ui.button("Cancel").clicked() {
                    editor_state.editing = false;
                }
            });
        }
    });
}
