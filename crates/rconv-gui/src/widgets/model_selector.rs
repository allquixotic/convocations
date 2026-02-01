//! Model selector widget

use crate::state::AppState;
use rconv_core::curator;

/// Render model selector
/// Returns true if "Load Models" button was clicked
pub fn render(ui: &mut egui::Ui, state: &mut AppState) -> bool {
    ui.vertical(|ui| {
        let mut load_all_models = false;

        ui.heading("Model Selection");

        // Model selection mode
        ui.horizontal(|ui| {
            ui.label("Mode:");

            use crate::state::ModelSelectionMode;

            if ui.radio_value(&mut state.model_selection_mode, ModelSelectionMode::Auto, "Automatic").clicked() {
                state.config.runtime.openrouter_model = Some("auto".to_string());
                state.config.ui.model_selection_mode = "auto".to_string();
            }
            if ui.radio_value(&mut state.model_selection_mode, ModelSelectionMode::Curated, "Curated List").clicked() {
                state.config.runtime.openrouter_model = None;
                state.config.ui.model_selection_mode = "curated".to_string();
            }
            if ui.radio_value(&mut state.model_selection_mode, ModelSelectionMode::AllModels, "All Models").clicked() {
                // Don't set openrouter_model yet - let user select from dropdown
                // Keep existing selection if any, or clear it
                if state.all_models_cache.is_some() {
                    // Keep existing selection if valid, otherwise clear
                    if let Some(ref model_id) = state.config.runtime.openrouter_model {
                        if model_id == "auto" || model_id.is_empty() {
                            state.config.runtime.openrouter_model = None;
                        }
                    }
                } else {
                    state.config.runtime.openrouter_model = None;
                }
                state.config.ui.model_selection_mode = "all_models".to_string();
            }
            if ui.radio_value(&mut state.model_selection_mode, ModelSelectionMode::Manual, "Manual Entry").clicked() {
                state.config.runtime.openrouter_model = Some(String::new());
                state.config.ui.model_selection_mode = "manual".to_string();
            }
        });

        ui.add_space(8.0);

        // Free models only filter
        ui.checkbox(&mut state.config.runtime.free_models_only, "Show Free Models Only");

        ui.add_space(8.0);

        // Show appropriate UI based on mode
        use crate::state::ModelSelectionMode;
        match state.model_selection_mode {
            ModelSelectionMode::AllModels => {
                // All models mode - show dropdown with all OpenRouter models
                ui.horizontal(|ui| {
                    ui.label("All OpenRouter Models:");

                    if state.all_models_cache.is_none() && !state.loading_all_models {
                        if ui.button("Load Models").clicked() {
                            load_all_models = true;
                        }
                    } else if state.loading_all_models {
                        ui.spinner();
                        ui.label("Loading models...");
                    }
                });

                if let Some(ref models) = state.all_models_cache {
                    ui.add_space(8.0);

                    // Filter and sort models
                    let mut filtered_models: Vec<_> = if state.config.runtime.free_models_only {
                        models.iter().filter(|m| m.is_free()).cloned().collect()
                    } else {
                        models.clone()
                    };

                    // Sort alphabetically by id (slug)
                    filtered_models.sort_by(|a, b| a.id.cmp(&b.id));

                    let current_selection = state.config.runtime.openrouter_model.as_deref().unwrap_or("");

                    egui::ComboBox::from_id_salt("all_models_selector")
                        .selected_text(if current_selection.is_empty() {
                            "Select a model..."
                        } else {
                            current_selection
                        })
                        .show_ui(ui, |ui| {
                            for model in filtered_models {
                                // Parse pricing strings and multiply by 1M to get per-million-tokens price
                                let prompt_price = model.pricing.prompt.parse::<f64>().unwrap_or(0.0) * 1_000_000.0;
                                let completion_price = model.pricing.completion.parse::<f64>().unwrap_or(0.0) * 1_000_000.0;

                                let display_text = format!(
                                    "{} - {} (${:.2}/M, ${:.2}/M tokens)",
                                    model.id,
                                    model.name,
                                    prompt_price,
                                    completion_price
                                );

                                if ui.selectable_value(
                                    &mut state.config.runtime.openrouter_model,
                                    Some(model.id.clone()),
                                    display_text
                                ).clicked() {
                                    // Model changed
                                }
                            }
                        });

                    // Show details of selected model
                    if let Some(ref model_id) = state.config.runtime.openrouter_model {
                        if !model_id.is_empty() {
                            if let Some(model) = models.iter().find(|m| m.id == *model_id) {
                                ui.add_space(4.0);
                                ui.label(format!("Model: {}", model.name));

                                // Parse pricing strings and multiply by 1M to get per-million-tokens price
                                let prompt_price = model.pricing.prompt.parse::<f64>().unwrap_or(0.0) * 1_000_000.0;
                                let completion_price = model.pricing.completion.parse::<f64>().unwrap_or(0.0) * 1_000_000.0;

                                ui.label(format!(
                                    "Pricing: ${:.2}/M tokens (prompt), ${:.2}/M tokens (completion)",
                                    prompt_price, completion_price
                                ));
                                if let Some(ctx) = model.context_length {
                                    ui.label(format!("Context: {} tokens", ctx));
                                }
                                if model.is_free() {
                                    ui.colored_label(egui::Color32::GREEN, "FREE");
                                }
                            }
                        }
                    }

                    ui.add_space(4.0);
                    ui.label(format!("Total models: {}", models.len()));
                }
            }
            ModelSelectionMode::Auto => {
                // Auto mode - show what will be selected
                ui.label("Model will be selected automatically based on availability and quality.");

                ui.add_space(4.0);

                // Preview which model will be selected based on current settings
                if let Some(ref models) = state.model_catalog_cache {
                    let free_models: Vec<_> = models.iter()
                        .filter(|m| matches!(m.tier, curator::CuratedTier::Free))
                        .collect();
                    let cheap_models: Vec<_> = models.iter()
                        .filter(|m| matches!(m.tier, curator::CuratedTier::Cheap))
                        .collect();

                    // Apply the same logic as curator::select_auto
                    let selected = if state.config.runtime.free_models_only {
                        free_models.first().or_else(|| cheap_models.first())
                    } else {
                        cheap_models.first().or_else(|| free_models.first())
                    };

                    if let Some(model) = selected {
                        ui.group(|ui| {
                            ui.label(egui::RichText::new("Auto-Selected Model:").strong());
                            ui.label(format!("Model: {}", model.display_name));
                            ui.label(format!("Slug: {}", model.slug));
                            ui.label(format!("Provider: {}", model.provider));
                            ui.label(format!("Quality (AAII): {:.1}", model.aaii));
                            if let Some(price_in) = model.price_in_per_million {
                                let price_display = price_in * 1_000_000.0;
                                ui.label(format!("Input: ${:.2}/M tokens", price_display));
                            }
                            if let Some(price_out) = model.price_out_per_million {
                                let price_display = price_out * 1_000_000.0;
                                ui.label(format!("Output: ${:.2}/M tokens", price_display));
                            }
                            if matches!(model.tier, curator::CuratedTier::Free) {
                                ui.colored_label(egui::Color32::GREEN, "FREE");
                            }
                        });
                    } else {
                        ui.colored_label(egui::Color32::YELLOW, "No models available for automatic selection");
                    }
                } else {
                    ui.colored_label(egui::Color32::RED, "Failed to load curated models for preview");
                }
            }
            ModelSelectionMode::Curated => {
                // Show curated models dropdown
                ui.label("Select Model:");

                // Get curated models from cache (already loaded on startup)
                if let Some(ref models) = state.model_catalog_cache {
                    let filtered_models: Vec<_> = if state.config.runtime.free_models_only {
                        models.iter().filter(|m| matches!(m.tier, curator::CuratedTier::Free)).cloned().collect()
                    } else {
                        models.clone()
                    };

                    let current_selection = state.config.runtime.openrouter_model.as_deref().unwrap_or("");

                    egui::ComboBox::from_id_salt("model_selector")
                        .selected_text(if current_selection.is_empty() {
                            "Select a model..."
                        } else {
                            current_selection
                        })
                        .show_ui(ui, |ui| {
                            for model in filtered_models {
                                // Prices are stored per-token, multiply by 1M for display
                                let price_in_display = model.price_in_per_million.unwrap_or(0.0) * 1_000_000.0;
                                let price_out_display = model.price_out_per_million.unwrap_or(0.0) * 1_000_000.0;

                                let display_text = format!(
                                    "{} - {} (AAII: {:.1}, ${:.2}/${:.2}/M)",
                                    model.slug,
                                    model.display_name,
                                    model.aaii,
                                    price_in_display,
                                    price_out_display
                                );

                                if ui.selectable_value(
                                    &mut state.config.runtime.openrouter_model,
                                    Some(model.slug.clone()),
                                    display_text
                                ).clicked() {
                                    // Model changed
                                }
                            }
                        });

                    // Show details of selected model
                    if let Some(ref model_slug) = state.config.runtime.openrouter_model {
                        if !model_slug.is_empty() && model_slug != "auto" {
                            // Find and display model details from cache
                            if let Some(ref models) = state.model_catalog_cache {
                                if let Some(model) = models.iter().find(|m| m.slug == *model_slug) {
                                    ui.add_space(4.0);
                                    ui.group(|ui| {
                                        ui.label(egui::RichText::new(format!("Selected: {}", model.display_name)).strong());
                                        ui.label(format!("Slug: {}", model.slug));
                                        ui.label(format!("Provider: {}", model.provider));
                                        ui.label(format!("Quality (AAII): {:.1}", model.aaii));
                                        if let Some(price_in) = model.price_in_per_million {
                                            let price_display = price_in * 1_000_000.0;
                                            ui.label(format!("Input: ${:.2}/M tokens", price_display));
                                        }
                                        if let Some(price_out) = model.price_out_per_million {
                                            let price_display = price_out * 1_000_000.0;
                                            ui.label(format!("Output: ${:.2}/M tokens", price_display));
                                        }
                                        if matches!(model.tier, curator::CuratedTier::Free) {
                                            ui.colored_label(egui::Color32::GREEN, "FREE");
                                        }
                                    });
                                }
                            }
                        }
                    }
                } else {
                    ui.colored_label(egui::Color32::RED, "Failed to load curated models");
                }
            }
            ModelSelectionMode::Manual => {
                // Manual entry mode
                ui.horizontal(|ui| {
                    ui.label("Model ID:");
                    let mut model_id = state.config.runtime.openrouter_model.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut model_id).changed() {
                        state.config.runtime.openrouter_model = Some(model_id);
                    }
                });
            }
        }

        load_all_models
    }).inner
}
