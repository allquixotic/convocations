//! Main application structure for rconv GUI

use crate::async_bridge::{AsyncBridge, ProgressKind, ProgressUpdate};
use crate::oauth::OAuthFlow;
use crate::processor;
use crate::state::{AppState, ProcessorState, ProgressInfo};
use crate::tray::TrayManager;
use crate::ui_state::{LogEntry, LogLevel, Theme, UiState};
use crate::widgets;
use crate::widgets::preset_selector::PresetEditorState;
use chrono::Local;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Main application struct implementing eframe::App
pub struct RconvApp {
    /// Domain state
    state: AppState,

    /// UI state
    ui_state: UiState,

    /// Async runtime bridge
    async_bridge: AsyncBridge,

    /// Preset editor state
    preset_editor: PresetEditorState,

    /// System tray manager
    tray_manager: Option<TrayManager>,

    /// OAuth flow (when in progress)
    oauth_flow: Option<OAuthFlow>,

    /// Last config save time
    last_save: std::time::Instant,

    /// Config dirty flag
    config_dirty: bool,

    /// Should quit flag
    should_quit: bool,
}

impl RconvApp {
    /// Create a new RconvApp
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Try to create tray manager
        let tray_manager = match TrayManager::new() {
            Ok(tray) => Some(tray),
            Err(e) => {
                eprintln!("Failed to create system tray: {}", e);
                None
            }
        };

        let mut app = Self {
            state: AppState::new(),
            ui_state: UiState::new(),
            async_bridge: AsyncBridge::new(),
            preset_editor: PresetEditorState::default(),
            tray_manager,
            oauth_flow: None,
            last_save: std::time::Instant::now(),
            config_dirty: false,
            should_quit: false,
        };

        // Add initial log entry
        app.add_log(LogLevel::Info, "Application started");

        app
    }

    /// Add a log entry
    fn add_log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.ui_state.add_log_entry(LogEntry {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            level,
            message: message.into(),
        });
    }

    /// Apply theme to egui context
    fn apply_theme(&self, ctx: &egui::Context) {
        let visuals = match self.ui_state.theme {
            Theme::Dark => egui::Visuals::dark(),
            Theme::Light => egui::Visuals::light(),
        };
        ctx.set_visuals(visuals);
    }

    /// Auto-save configuration if dirty and enough time has passed
    fn handle_auto_save(&mut self) {
        if self.config_dirty && self.last_save.elapsed() > Duration::from_millis(300) {
            if let Err(e) = self.state.save_config() {
                self.add_log(LogLevel::Error, format!("Failed to save config: {}", e));
            } else {
                self.config_dirty = false;
                self.last_save = std::time::Instant::now();
            }
        }
    }

    /// Mark configuration as dirty
    fn mark_dirty(&mut self) {
        self.config_dirty = true;
    }

    /// Start loading all OpenRouter models
    fn start_loading_all_models(&mut self) {
        if self.state.loading_all_models {
            return; // Already loading
        }

        self.state.loading_all_models = true;
        self.add_log(LogLevel::Info, "Loading all OpenRouter models...");

        // Clone the result channel for the async task
        let result_channel = Arc::clone(&self.state.model_load_result);

        // Spawn async task to fetch models
        let runtime = self.async_bridge.runtime();
        runtime.spawn(async move {
            let result = match rconv_core::openrouter::fetch_models().await {
                Ok(mut models) => {
                    // Sort alphabetically by id
                    models.sort_by(|a, b| a.id.cmp(&b.id));
                    Ok(models)
                }
                Err(e) => Err(e.to_string()),
            };

            // Send result back to main thread
            if let Ok(mut guard) = result_channel.lock() {
                *guard = Some(result);
            }
        });
    }

    /// Handle progress updates from async tasks
    fn handle_progress_update(&mut self, update: ProgressUpdate) {
        match update.kind {
            ProgressKind::Started { ref job_id } => {
                self.add_log(LogLevel::Info, format!("Started job: {}", job_id));
                self.state.processor_state = ProcessorState::Running {
                    progress: Arc::new(Mutex::new(ProgressInfo::default())),
                    job_id: job_id.clone(),
                };
            }
            ProgressKind::StageBegin { ref stage } => {
                self.add_log(LogLevel::Info, format!("Stage: {}", stage));
                if let ProcessorState::Running { ref progress, .. } = self.state.processor_state {
                    if let Ok(mut info) = progress.lock() {
                        info.stage = Some(stage.clone());
                        info.message = update.message.clone();
                        info.elapsed_ms = update.elapsed_ms;
                    }
                }
            }
            ProgressKind::StageEnd { ref stage } => {
                self.add_log(LogLevel::Info, format!("Completed: {}", stage));
            }
            ProgressKind::Info { ref message } => {
                self.add_log(LogLevel::Info, message);
                if let ProcessorState::Running { ref progress, .. } = self.state.processor_state {
                    if let Ok(mut info) = progress.lock() {
                        info.message = Some(message.clone());
                        info.elapsed_ms = update.elapsed_ms;
                    }
                }
            }
            ProgressKind::Completed { ref summary, ref diff } => {
                self.add_log(LogLevel::Info, "Processing completed successfully");
                self.state.processor_state = ProcessorState::Completed {
                    summary: summary.clone(),
                    diff: diff.clone(),
                };
                self.ui_state.diff_preview_expanded = diff.is_some();
            }
            ProgressKind::Failed { ref error } => {
                self.add_log(LogLevel::Error, format!("Processing failed: {}", error));
                self.state.processor_state = ProcessorState::Error {
                    message: error.clone(),
                };
            }
        }
    }

    /// Start processing
    fn start_processing(&mut self) {
        match processor::start_processing(&self.async_bridge, &self.state) {
            Ok(rx) => {
                self.async_bridge.register_progress_receiver(rx);
                self.add_log(LogLevel::Info, "Processing started");
            }
            Err(e) => {
                self.add_log(LogLevel::Error, format!("Failed to start processing: {}", e));
                self.state.processor_state = ProcessorState::Error {
                    message: format!("Failed to start: {}", e),
                };
            }
        }
    }

    /// Render the top panel with title and theme toggle
    fn render_top_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Convocations");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let theme_label = match self.ui_state.theme {
                        Theme::Dark => "☀ Light",
                        Theme::Light => "🌙 Dark",
                    };
                    if ui.button(theme_label).clicked() {
                        self.ui_state.theme = match self.ui_state.theme {
                            Theme::Dark => Theme::Light,
                            Theme::Light => Theme::Dark,
                        };
                    }

                    // Save button
                    if self.config_dirty {
                        if ui.button("💾 Save").clicked() {
                            if let Err(e) = self.state.save_config() {
                                self.add_log(LogLevel::Error, format!("Failed to save: {}", e));
                            } else {
                                self.add_log(LogLevel::Info, "Configuration saved");
                                self.config_dirty = false;
                            }
                        }
                    }
                });
            });
        });
    }

    /// Render the main UI content
    fn render_main_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Configuration section
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    ui.heading("Configuration").on_hover_text("Main processing configuration");
                    if widgets::config_form::render(ui, &mut self.state) {
                        self.mark_dirty();
                    }
                });

                ui.add_space(8.0);

                // Processing options
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    ui.heading("Processing Options");
                    if widgets::processing_options::render(ui, &mut self.state) {
                        self.mark_dirty();
                    }
                });

                ui.add_space(8.0);

                // OpenRouter API section
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    let mut oauth_triggered = false;
                    widgets::api_key_section::render(ui, &mut self.state, &mut self.ui_state, &mut || {
                        oauth_triggered = true;
                    });

                    if oauth_triggered {
                        self.start_oauth_flow();
                    }
                });

                ui.add_space(8.0);

                // Model selection
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    if widgets::model_selector::render(ui, &mut self.state) {
                        // "Load Models" button was clicked
                        self.start_loading_all_models();
                    }
                });

                ui.add_space(8.0);

                // Preset management
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    widgets::preset_selector::render(ui, &mut self.state, &mut self.preset_editor);
                });

                ui.add_space(16.0);

                // Run button
                ui.separator();
                self.render_run_section(ui);
                ui.separator();

                ui.add_space(8.0);

                // Progress display (when running)
                if !matches!(self.state.processor_state, ProcessorState::Idle) {
                    ui.group(|ui| {
                        ui.set_min_width(ui.available_width());
                        widgets::progress_display::render(ui, &self.state.processor_state);
                    });

                    ui.add_space(8.0);
                }

                // Technical log
                let log_response = egui::CollapsingHeader::new("Technical Log")
                    .default_open(self.ui_state.technical_log_expanded)
                    .show(ui, |ui| {
                        widgets::technical_log::render(ui, &mut self.ui_state);
                    });
                if log_response.header_response.clicked() {
                    self.ui_state.technical_log_expanded = !self.ui_state.technical_log_expanded;
                }

                ui.add_space(8.0);

                // Diff preview (when available)
                if let ProcessorState::Completed { diff: Some(ref diff_text), .. } = self.state.processor_state {
                    let diff_response = egui::CollapsingHeader::new("Diff Preview")
                        .default_open(self.ui_state.diff_preview_expanded)
                        .show(ui, |ui| {
                            widgets::diff_preview::render(ui, diff_text);
                        });
                    if diff_response.header_response.clicked() {
                        self.ui_state.diff_preview_expanded = !self.ui_state.diff_preview_expanded;
                    }
                }
            });
    }

    /// Render run section
    fn render_run_section(&mut self, ui: &mut egui::Ui) {
        match &self.state.processor_state {
            ProcessorState::Idle => {
                // Large, prominent green button for running the processor
                let button_size = egui::vec2(ui.available_width(), 60.0);
                let button = egui::Button::new(egui::RichText::new("▶ Run Processor").size(24.0))
                    .fill(egui::Color32::from_rgb(0, 150, 0))
                    .min_size(button_size);

                if ui.add(button).clicked() {
                    self.start_processing();
                }
            }
            ProcessorState::Running { .. } => {
                ui.label(egui::RichText::new("⏳ Processing...").size(18.0));
                // TODO: Add cancel button
            }
            ProcessorState::Completed { .. } | ProcessorState::Error { .. } => {
                let button_size = egui::vec2(ui.available_width(), 50.0);
                let button = egui::Button::new(egui::RichText::new("Run Again").size(20.0))
                    .fill(egui::Color32::from_rgb(0, 120, 200))
                    .min_size(button_size);

                if ui.add(button).clicked() {
                    self.state.processor_state = ProcessorState::Idle;
                }
            }
        }
    }

    /// Start OAuth flow
    fn start_oauth_flow(&mut self) {
        match OAuthFlow::start() {
            Ok(flow) => {
                self.oauth_flow = Some(flow);
                self.ui_state.oauth_pending = true;
                self.add_log(LogLevel::Info, "OAuth flow started. Check your browser.");
            }
            Err(e) => {
                self.add_log(LogLevel::Error, format!("Failed to start OAuth: {}", e));
            }
        }
    }
}

impl eframe::App for RconvApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme
        self.apply_theme(ctx);

        // Poll for progress updates - collect first to avoid borrow checker issues
        let mut updates = Vec::new();
        self.async_bridge.poll_progress(|update| {
            updates.push(update);
        });

        // Process updates
        for update in updates {
            self.handle_progress_update(update);
        }

        // Poll OAuth flow
        if let Some(ref flow) = self.oauth_flow {
            if let Some(result) = flow.poll() {
                match result {
                    Ok(api_key) => {
                        if let Err(e) = self.state.config.runtime.set_openrouter_api_key(&api_key) {
                            self.add_log(LogLevel::Error, format!("Failed to save API key: {}", e));
                        } else {
                            self.add_log(LogLevel::Info, "API key saved successfully");
                        }
                    }
                    Err(e) => {
                        self.add_log(LogLevel::Error, format!("OAuth failed: {}", e));
                    }
                }
                self.oauth_flow = None;
                self.ui_state.oauth_pending = false;
            }
        }

        // Poll model loading result
        if self.state.loading_all_models {
            let result = {
                let mut guard = self.state.model_load_result.lock().unwrap();
                guard.take()
            };

            if let Some(result) = result {
                match result {
                    Ok(models) => {
                        self.add_log(LogLevel::Info, format!("Loaded {} models", models.len()));
                        self.state.all_models_cache = Some(models);
                    }
                    Err(e) => {
                        self.add_log(LogLevel::Error, format!("Failed to load models: {}", e));
                    }
                }
                self.state.loading_all_models = false;
            }
        }

        // Request continuous repaint for smooth UI
        ctx.request_repaint();

        // Top panel
        self.render_top_panel(ctx);

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_main_ui(ui);
        });

        // Handle tray events
        if let Some(ref tray) = self.tray_manager {
            let mut window_visible = true;
            if tray.handle_events(&mut window_visible) {
                self.should_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Auto-save
        self.handle_auto_save();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Cleanup on exit
        self.should_quit = true;
    }
}
