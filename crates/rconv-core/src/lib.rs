//! Core library crate exposing shared Convocations processing logic.

pub mod config;
pub mod runtime;

pub use config::{
    ConfigError, ConfigLoadResult, ConfigSource, DurationOverride, FRIDAY_6_PRESET_ID, FileConfig,
    PresetDefinition, RuntimeOverrides, RuntimePreferences, SATURDAY_PRESET_ID,
    TUESDAY_7_PRESET_ID, TUESDAY_8_PRESET_ID, ThemePreference, UiPreferences,
    apply_runtime_overrides, config_directory, config_path, load_config,
    runtime_overrides_from_convocations, runtime_preferences_to_convocations, save_config,
};
pub use runtime::{
    ConvocationsConfig, OutfileResolution, StageProgressCallback, StageProgressEvent,
    StageProgressEventKind, resolve_outfile_paths, run_cli, run_with_config,
    run_with_config_with_progress,
};
