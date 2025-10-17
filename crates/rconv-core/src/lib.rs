//! Core library crate exposing shared Convocations processing logic.

pub mod config;
pub mod openrouter;
pub mod runtime;

pub use config::{
    ConfigError, ConfigLoadResult, ConfigSource, DurationOverride, FRIDAY_6_PRESET_NAME,
    FileConfig, PresetDefinition, RuntimeOverrides, RuntimePreferences, SATURDAY_PRESET_NAME,
    TUESDAY_7_PRESET_NAME, TUESDAY_8_PRESET_NAME, ThemePreference, UiPreferences,
    apply_runtime_overrides, config_directory, config_path, load_config,
    runtime_overrides_from_convocations, runtime_preferences_to_convocations, save_config,
    save_presets_and_ui_only,
};
pub use runtime::{
    ConvocationsConfig, OutfileResolution, StageProgressCallback, StageProgressEvent,
    StageProgressEventKind, calculate_event_dates, resolve_outfile_paths, run_cli, run_with_config,
    run_with_config_with_progress,
};
