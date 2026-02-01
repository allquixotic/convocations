//! Application state management for rconv GUI

use rconv_core::config::FileConfig;
use rconv_core::curator::CuratedModelSummary;
use rconv_core::openrouter::ModelInfo;
use std::sync::{Arc, Mutex};

/// Model selection mode (separate from the actual selected model)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelSelectionMode {
    Auto,
    Curated,
    AllModels,
    Manual,
}

/// Main application state (domain/persistent)
#[derive(Clone)]
pub struct AppState {
    /// Configuration from rconv-core
    pub config: FileConfig,

    /// Current processor state
    pub processor_state: ProcessorState,

    /// Model selection mode (determines which UI to show)
    pub model_selection_mode: ModelSelectionMode,

    /// Cached model catalog (loaded once on startup, not on every frame)
    pub model_catalog_cache: Option<Vec<CuratedModelSummary>>,

    /// All OpenRouter models (loaded on demand)
    pub all_models_cache: Option<Vec<ModelInfo>>,

    /// Loading state for all models
    pub loading_all_models: bool,

    /// Result channel for model loading (shared with async task)
    pub model_load_result: Arc<Mutex<Option<Result<Vec<ModelInfo>, String>>>>,
}

impl AppState {
    pub fn new() -> Self {
        let mut load = rconv_core::load_config();

        // Log warnings to console
        for warning in &load.warnings {
            eprintln!("Warning: {}", warning);
        }

        // Load model catalog once on startup (not on every frame!)
        let model_catalog_cache = rconv_core::curator::catalog_summaries().ok();

        // Determine initial mode from persisted UI preference
        let model_selection_mode = match load.config.ui.model_selection_mode.as_str() {
            "auto" => ModelSelectionMode::Auto,
            "curated" => ModelSelectionMode::Curated,
            "all_models" => ModelSelectionMode::AllModels,
            "manual" => ModelSelectionMode::Manual,
            _ => {
                // Fallback to inferring from openrouter_model if persisted mode is invalid
                if let Some(ref model) = load.config.runtime.openrouter_model {
                    if model == "auto" {
                        ModelSelectionMode::Auto
                    } else if model == "all" {
                        load.config.runtime.openrouter_model = None;
                        ModelSelectionMode::AllModels
                    } else if model.is_empty() {
                        ModelSelectionMode::Manual
                    } else {
                        ModelSelectionMode::Curated
                    }
                } else {
                    ModelSelectionMode::Curated
                }
            }
        };

        Self {
            config: load.config,
            processor_state: ProcessorState::Idle,
            model_selection_mode,
            model_catalog_cache,
            all_models_cache: None,
            loading_all_models: false,
            model_load_result: Arc::new(Mutex::new(None)),
        }
    }

    /// Save configuration to disk
    pub fn save_config(&self) -> Result<(), String> {
        rconv_core::save_config(&self.config).map_err(|e| e.to_string())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Current state of the processor
#[derive(Clone)]
pub enum ProcessorState {
    /// Not processing
    Idle,

    /// Currently processing
    Running {
        /// Progress information
        progress: Arc<Mutex<ProgressInfo>>,

        /// Job ID
        job_id: String,
    },

    /// Processing completed successfully
    Completed {
        /// Summary of processing
        summary: String,

        /// Optional diff preview
        diff: Option<String>,
    },

    /// Processing failed
    Error {
        /// Error message
        message: String,
    },
}

/// Progress information for active processing
#[derive(Clone, Default)]
pub struct ProgressInfo {
    /// Current stage
    pub stage: Option<String>,

    /// Progress message
    pub message: Option<String>,

    /// Elapsed time in milliseconds
    pub elapsed_ms: Option<f64>,
}
