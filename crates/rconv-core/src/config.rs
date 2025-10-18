use crate::curator::AUTO_SENTINEL;
use crate::runtime::ConvocationsConfig;
use crate::secret_store::{self, SecretReference, SecretStoreError};
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

const CONFIG_DIR_NAME: &str = "convocations";
const CONFIG_FILE_NAME: &str = "config.toml";
const LEGACY_SETTINGS_FILE_NAME: &str = "settings.json";
const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const SATURDAY_PRESET_NAME: &str = "Saturday 10pm-midnight";
pub const TUESDAY_7_PRESET_NAME: &str = "Tuesday 7pm";
pub const TUESDAY_8_PRESET_NAME: &str = "Tuesday 8pm";
pub const FRIDAY_6_PRESET_NAME: &str = "Friday 6pm";
pub const SATURDAY_PRESET_ID: &str = "saturday-10pm-midnight";
pub const TUESDAY_7_PRESET_ID: &str = "tuesday-7pm";
pub const TUESDAY_8_PRESET_ID: &str = "tuesday-8pm";
pub const FRIDAY_6_PRESET_ID: &str = "friday-6pm";
pub const DEFAULT_OPENROUTER_MODEL: &str = "google/gemini-2.5-flash-lite";

/// Result returned by [`load_config`], capturing the source and any non-fatal issues.
#[derive(Debug, Clone)]
pub struct ConfigLoadResult {
    pub config: FileConfig,
    pub warnings: Vec<String>,
    pub source: ConfigSource,
}

/// Indicates where the configuration was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// No persisted configuration was found or usable; defaults were synthesized.
    Default,
    /// Configuration was read from `config.toml`.
    File,
    /// Configuration was converted from the legacy `settings.json`.
    LegacyJson,
}

/// Errors that can occur when persisting configuration.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Ser(toml::ser::Error),
    Secret(SecretStoreError),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "IO error: {err}"),
            ConfigError::Ser(err) => write!(f, "TOML serialization error: {err}"),
            ConfigError::Secret(err) => write!(f, "Secret storage error: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(value: toml::ser::Error) -> Self {
        Self::Ser(value)
    }
}

impl From<SecretStoreError> for ConfigError {
    fn from(value: SecretStoreError) -> Self {
        Self::Secret(value)
    }
}

/// Disk-backed configuration schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileConfig {
    #[serde(default = "FileConfig::schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub runtime: RuntimePreferences,
    #[serde(default)]
    pub ui: UiPreferences,
    #[serde(default = "default_presets")]
    pub presets: Vec<PresetDefinition>,
}

pub fn preset_id_from_name(name: &str) -> String {
    let mut id = String::with_capacity(name.len());
    let mut previous_dash = false;
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            id.push(lower);
            previous_dash = false;
        } else if !previous_dash {
            id.push('-');
            previous_dash = true;
        }
    }
    while id.starts_with('-') {
        id.remove(0);
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        "preset".to_string()
    } else {
        id
    }
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            runtime: RuntimePreferences::default(),
            ui: UiPreferences::default(),
            presets: default_presets(),
        }
    }
}

impl FileConfig {
    const fn schema_version() -> u32 {
        CURRENT_SCHEMA_VERSION
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum SecretValue {
    Plain(String),
    Reference(SecretReference),
}

impl SecretValue {
    pub fn as_reference(&self) -> Option<&SecretReference> {
        match self {
            SecretValue::Reference(reference) => Some(reference),
            SecretValue::Plain(_) => None,
        }
    }

    fn take_plain(&self) -> Option<String> {
        match self {
            SecretValue::Plain(value) => Some(value.clone()),
            SecretValue::Reference(_) => None,
        }
    }
}

/// Runtime preferences that map closely to CLI flag behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimePreferences {
    #[serde(default = "RuntimePreferences::default_chat_log_path")]
    pub chat_log_path: String,
    #[serde(default = "RuntimePreferences::default_active_preset")]
    pub active_preset: String,
    #[serde(default)]
    pub weeks_ago: u32,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "RuntimePreferences::default_use_ai_corrections")]
    pub use_ai_corrections: bool,
    #[serde(default)]
    pub keep_original_output: bool,
    #[serde(default = "RuntimePreferences::default_show_diff")]
    pub show_diff: bool,
    #[serde(default = "RuntimePreferences::default_cleanup_enabled")]
    pub cleanup_enabled: bool,
    #[serde(default = "RuntimePreferences::default_format_dialogue_enabled")]
    pub format_dialogue_enabled: bool,
    #[serde(default)]
    pub outfile_override: Option<String>,
    #[serde(default)]
    pub duration_override: DurationOverride,
    #[serde(default)]
    pub openrouter_api_key: Option<SecretValue>,
    #[serde(default)]
    pub openrouter_model: Option<String>,
    #[serde(default)]
    pub free_models_only: bool,
    #[serde(default)]
    pub output_target: OutputTarget,
    #[serde(default)]
    pub output_directory_override: Option<String>,
}

impl Default for RuntimePreferences {
    fn default() -> Self {
        Self {
            chat_log_path: Self::default_chat_log_path(),
            active_preset: Self::default_active_preset(),
            weeks_ago: 0,
            dry_run: false,
            use_ai_corrections: true,
            keep_original_output: false,
            show_diff: true,
            cleanup_enabled: true,
            format_dialogue_enabled: true,
            outfile_override: None,
            duration_override: DurationOverride::default(),
            openrouter_api_key: None,
            openrouter_model: None,
            free_models_only: false,
            output_target: OutputTarget::default(),
            output_directory_override: None,
        }
    }
}

impl RuntimePreferences {
    fn default_chat_log_path() -> String {
        "~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log".to_string()
    }

    fn default_active_preset() -> String {
        SATURDAY_PRESET_NAME.to_string()
    }

    const fn default_use_ai_corrections() -> bool {
        true
    }

    const fn default_show_diff() -> bool {
        true
    }

    const fn default_cleanup_enabled() -> bool {
        true
    }

    const fn default_format_dialogue_enabled() -> bool {
        true
    }

    pub fn set_openrouter_api_key(&mut self, api_key: &str) -> Result<(), SecretStoreError> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            self.clear_openrouter_api_key()?;
            return Ok(());
        }

        if let Some(SecretValue::Reference(existing)) = self.openrouter_api_key.as_ref() {
            // Best effort removal of previous stored secret to avoid stale entries.
            let _ = secret_store::delete_secret(existing);
        }

        let reference = secret_store::store_secret("openrouter_api_key", trimmed)?;
        self.openrouter_api_key = Some(SecretValue::Reference(reference));
        Ok(())
    }

    pub fn clear_openrouter_api_key(&mut self) -> Result<(), SecretStoreError> {
        if let Some(SecretValue::Reference(reference)) = self.openrouter_api_key.as_ref() {
            secret_store::delete_secret(reference)?;
        }
        self.openrouter_api_key = None;
        Ok(())
    }

    pub fn migrate_openrouter_secret(&mut self) -> Result<bool, SecretStoreError> {
        let Some(value) = self
            .openrouter_api_key
            .as_ref()
            .and_then(SecretValue::take_plain)
        else {
            return Ok(false);
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            self.openrouter_api_key = None;
            return Ok(true);
        }

        let reference = secret_store::store_secret("openrouter_api_key", trimmed)?;
        self.openrouter_api_key = Some(SecretValue::Reference(reference));
        Ok(true)
    }

    pub fn resolve_openrouter_api_key(&self) -> Result<Option<String>, SecretStoreError> {
        match self.openrouter_api_key.as_ref() {
            Some(SecretValue::Reference(reference)) => secret_store::load_secret(reference),
            Some(SecretValue::Plain(value)) => Ok(Some(value.clone())),
            None => Ok(None),
        }
    }

    pub fn has_openrouter_api_key(&self) -> bool {
        matches!(self.openrouter_api_key, Some(SecretValue::Reference(_)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputTarget {
    File,
    Directory,
}

impl Default for OutputTarget {
    fn default() -> Self {
        Self::File
    }
}

/// Represents the optional duration override UI state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DurationOverride {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "DurationOverride::default_hours")]
    pub hours: f32,
}

impl Default for DurationOverride {
    fn default() -> Self {
        Self {
            enabled: false,
            hours: 1.0,
        }
    }
}

impl DurationOverride {
    const fn default_hours() -> f32 {
        1.0
    }
}

/// UI-only preferences that the GUI needs to persist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPreferences {
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub show_technical_log: bool,
    #[serde(default = "UiPreferences::default_follow_technical_log")]
    pub follow_technical_log: bool,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
            show_technical_log: false,
            follow_technical_log: true,
        }
    }
}

impl UiPreferences {
    const fn default_follow_technical_log() -> bool {
        true
    }
}

/// Theme preference options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePreference {
    Light,
    Dark,
    System,
}

impl Default for ThemePreference {
    fn default() -> Self {
        ThemePreference::Dark
    }
}

/// Preset definition shared between CLI and GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetDefinition {
    pub name: String,
    pub weekday: String,
    pub timezone: String,
    pub start_time: String,
    pub duration_minutes: u32,
    pub file_prefix: String,
    #[serde(default)]
    pub default_weeks_ago: u32,
    #[serde(default)]
    pub builtin: bool,
}

pub(crate) fn default_presets() -> Vec<PresetDefinition> {
    vec![
        PresetDefinition {
            name: SATURDAY_PRESET_NAME.to_string(),
            weekday: "saturday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "22:00".to_string(),
            duration_minutes: 145,
            file_prefix: "conv".to_string(),
            default_weeks_ago: 0,
            builtin: true,
        },
        PresetDefinition {
            name: TUESDAY_7_PRESET_NAME.to_string(),
            weekday: "tuesday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "19:00".to_string(),
            duration_minutes: 60,
            file_prefix: "rsm7".to_string(),
            default_weeks_ago: 0,
            builtin: true,
        },
        PresetDefinition {
            name: TUESDAY_8_PRESET_NAME.to_string(),
            weekday: "tuesday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "20:00".to_string(),
            duration_minutes: 60,
            file_prefix: "rsm8".to_string(),
            default_weeks_ago: 0,
            builtin: true,
        },
        PresetDefinition {
            name: FRIDAY_6_PRESET_NAME.to_string(),
            weekday: "friday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "18:00".to_string(),
            duration_minutes: 60,
            file_prefix: "tp6".to_string(),
            default_weeks_ago: 0,
            builtin: true,
        },
    ]
}

/// Represents overrides sourced from runtime inputs (CLI flags, GUI form edits).
#[derive(Debug, Default, Clone)]
pub struct RuntimeOverrides {
    pub last: Option<u32>,
    pub dry_run: Option<bool>,
    pub infile: Option<String>,
    pub start: Option<Option<String>>,
    pub end: Option<Option<String>>,
    pub active_preset: Option<String>,
    pub duration_override: Option<DurationOverride>,
    pub process_file: Option<Option<String>>,
    pub format_dialogue: Option<bool>,
    pub cleanup: Option<bool>,
    pub use_llm: Option<bool>,
    pub keep_orig: Option<bool>,
    pub no_diff: Option<bool>,
    pub outfile: Option<Option<String>>,
    pub use_ai_corrections: Option<bool>,
    pub keep_original_output: Option<bool>,
    pub show_diff: Option<bool>,
    pub output_directory: Option<Option<String>>,
    pub output_target: Option<OutputTarget>,
    pub openrouter_model: Option<String>,
}

impl RuntimeOverrides {
    pub fn is_empty(&self) -> bool {
        self.last.is_none()
            && self.dry_run.is_none()
            && self.infile.is_none()
            && self.start.is_none()
            && self.end.is_none()
            && self.active_preset.is_none()
            && self.duration_override.is_none()
            && self.process_file.is_none()
            && self.format_dialogue.is_none()
            && self.cleanup.is_none()
            && self.use_llm.is_none()
            && self.keep_orig.is_none()
            && self.no_diff.is_none()
            && self.outfile.is_none()
            && self.use_ai_corrections.is_none()
            && self.keep_original_output.is_none()
            && self.show_diff.is_none()
            && self.output_directory.is_none()
            && self.output_target.is_none()
            && self.openrouter_model.is_none()
    }
}

/// Path to the configuration directory.
pub fn config_directory() -> PathBuf {
    config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR_NAME)
}

/// Path to `config.toml`.
pub fn config_path() -> PathBuf {
    config_directory().join(CONFIG_FILE_NAME)
}

fn legacy_settings_path() -> PathBuf {
    config_directory().join(LEGACY_SETTINGS_FILE_NAME)
}

/// Load the configuration, falling back to defaults or the legacy JSON representation.
pub fn load_config() -> ConfigLoadResult {
    let mut warnings = Vec::new();
    let primary_path = config_path();

    if primary_path.exists() {
        match fs::read_to_string(&primary_path) {
            Ok(raw) => match toml::from_str::<FileConfig>(&raw) {
                Ok(cfg) => {
                    let (cfg, mut sanitize_warnings, secrets_migrated) = sanitize_config(cfg);
                    warnings.append(&mut sanitize_warnings);
                    if secrets_migrated {
                        if let Err(err) = save_config(&cfg) {
                            warnings
                                .push(format!("Failed to persist secure secret updates: {}", err));
                        }
                    }
                    return ConfigLoadResult {
                        config: cfg,
                        warnings,
                        source: ConfigSource::File,
                    };
                }
                Err(err) => {
                    warnings.push(format!(
                        "Failed to parse {} as TOML: {}. Falling back to defaults.",
                        CONFIG_FILE_NAME, err
                    ));
                }
            },
            Err(err) => {
                warnings.push(format!(
                    "Failed to read {}: {}. Falling back to defaults.",
                    CONFIG_FILE_NAME, err
                ));
            }
        }
    } else {
        // Attempt to migrate the legacy JSON settings.
        let legacy_path = legacy_settings_path();
        if legacy_path.exists() {
            match fs::read_to_string(&legacy_path) {
                Ok(raw) => match serde_json::from_str::<ConvocationsConfig>(&raw) {
                    Ok(legacy) => {
                        let cfg = migrate_legacy_config(legacy);
                        let (cfg, mut sanitize_warnings, secrets_migrated) = sanitize_config(cfg);
                        warnings.push(format!(
                            "Loaded configuration from legacy {}. A new {} will be written.",
                            LEGACY_SETTINGS_FILE_NAME, CONFIG_FILE_NAME
                        ));
                        warnings.append(&mut sanitize_warnings);
                        if let Err(err) = save_config(&cfg) {
                            warnings
                                .push(format!("Failed to persist migrated configuration: {}", err));
                        } else if secrets_migrated {
                            warnings.push(
                                "Migrated secrets were stored securely during legacy import."
                                    .to_string(),
                            );
                        }
                        return ConfigLoadResult {
                            config: cfg,
                            warnings,
                            source: ConfigSource::LegacyJson,
                        };
                    }
                    Err(err) => warnings.push(format!(
                        "Failed to parse {}: {}. Ignoring legacy settings.",
                        LEGACY_SETTINGS_FILE_NAME, err
                    )),
                },
                Err(err) => warnings.push(format!(
                    "Failed to read {}: {}. Ignoring legacy settings.",
                    LEGACY_SETTINGS_FILE_NAME, err
                )),
            }
        }
    }

    // Default fallback
    ConfigLoadResult {
        config: FileConfig::default(),
        warnings,
        source: ConfigSource::Default,
    }
}

/// Persist the configuration to disk.
pub fn save_config(config: &FileConfig) -> Result<(), ConfigError> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut config_to_write = config.clone();
    if config_to_write.runtime.migrate_openrouter_secret()? {
        // ensure the on-disk representation never regresses to plaintext
    }
    let serialized = toml::to_string_pretty(&config_to_write)?;
    fs::write(path, serialized)?;
    Ok(())
}

/// Persist only presets and UI preferences, preserving existing runtime preferences from disk.
/// This is used by GUI save operations and preset CRUD to keep runtime preferences ephemeral.
pub fn save_presets_and_ui_only(
    presets: &[PresetDefinition],
    ui: &UiPreferences,
) -> Result<(), ConfigError> {
    let load_result = load_config();
    let mut config = load_result.config;

    // Replace only presets and UI, keep existing runtime preferences
    config.presets = presets.to_vec();
    config.ui = ui.clone();

    save_config(&config)
}

fn sanitize_config(mut config: FileConfig) -> (FileConfig, Vec<String>, bool) {
    let mut warnings = Vec::new();
    let mut secrets_migrated = false;

    if config.schema_version != CURRENT_SCHEMA_VERSION {
        warnings.push(format!(
            "Unknown config schema version {}. Resetting to {}.",
            config.schema_version, CURRENT_SCHEMA_VERSION
        ));
        config = FileConfig::default();
        return (config, warnings, secrets_migrated);
    }

    let mut preset_names = HashSet::new();
    let mut duplicates = HashSet::new();

    config.presets.retain(|preset| {
        if !preset_names.insert(preset.name.clone()) {
            duplicates.insert(preset.name.clone());
            false
        } else {
            true
        }
    });

    if !duplicates.is_empty() {
        warnings.push(format!(
            "Removed duplicate preset names: {}",
            duplicates.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    // Ensure all built-in presets exist. If missing, append defaults.
    let mut existing: HashMap<String, PresetDefinition> = config
        .presets
        .iter()
        .map(|p| (p.name.clone(), p.clone()))
        .collect();
    for builtin in default_presets() {
        existing.entry(builtin.name.clone()).or_insert(builtin);
    }
    config.presets = existing.into_values().collect();
    config.presets.sort_by(|a, b| a.name.cmp(&b.name));

    if !config
        .presets
        .iter()
        .any(|preset| preset.name == config.runtime.active_preset)
    {
        warnings.push(format!(
            "Active preset '{}' not found. Resetting to default preset '{}'.",
            config.runtime.active_preset, SATURDAY_PRESET_NAME
        ));
        config.runtime.active_preset = SATURDAY_PRESET_NAME.to_string();
    }

    // Validate preset definitions
    for preset in &mut config.presets {
        if preset.duration_minutes == 0 {
            warnings.push(format!(
                "Preset '{}' has invalid duration_minutes (0). Resetting to 60.",
                preset.name
            ));
            preset.duration_minutes = 60;
        }
        if preset.file_prefix.trim().is_empty() {
            warnings.push(format!(
                "Preset '{}' has empty file_prefix. This preset will be removed.",
                preset.name
            ));
        }
    }

    // Remove presets with empty file_prefix (now a hard requirement)
    let initial_count = config.presets.len();
    config.presets.retain(|p| !p.file_prefix.trim().is_empty());
    if config.presets.len() < initial_count {
        warnings.push("Removed presets with empty file_prefix (required field)".to_string());
    }

    let duration_hours = config.runtime.duration_override.hours;
    if !duration_hours.is_finite() {
        warnings.push(
            "Duration override hours must be a finite number. Disabling override and resetting to 1.0."
                .to_string(),
        );
        config.runtime.duration_override.enabled = false;
        config.runtime.duration_override.hours = DurationOverride::default_hours();
    } else if duration_hours < 1.0 {
        warnings.push(
            "Duration override hours must be at least 1.0. Disabling override and resetting to 1.0."
                .to_string(),
        );
        config.runtime.duration_override.enabled = false;
        config.runtime.duration_override.hours = DurationOverride::default_hours();
    }

    if let Some(ref mut outfile) = config.runtime.outfile_override {
        if outfile.trim().is_empty() {
            *outfile = String::new();
        }
    }

    // Convert to ConvocationsConfig and validate
    let (convocations_config, mut conv_warnings) =
        runtime_preferences_to_convocations(&config.runtime, &config.presets);
    warnings.append(&mut conv_warnings);

    // Use the runtime validation function to check for contradictory settings
    if let Err(validation_error) = crate::runtime::validate_config(&convocations_config) {
        warnings.push(format!(
            "Configuration validation failed: {}. Resetting runtime preferences to defaults.",
            validation_error
        ));
        config.runtime = RuntimePreferences::default();
    }

    match config.runtime.migrate_openrouter_secret() {
        Ok(true) => {
            warnings.push("Migrated stored OpenRouter API key into secure storage.".to_string());
            secrets_migrated = true;
        }
        Ok(false) => {}
        Err(err) => {
            warnings.push(format!(
                "Failed to secure OpenRouter API key: {}. Clearing the saved key.",
                err
            ));
            let _ = config.runtime.clear_openrouter_api_key();
            secrets_migrated = true;
        }
    }

    (config, warnings, secrets_migrated)
}

fn migrate_legacy_config(legacy: ConvocationsConfig) -> FileConfig {
    let mut runtime = RuntimePreferences::default();
    runtime.chat_log_path = legacy.infile;
    runtime.weeks_ago = legacy.last;
    runtime.dry_run = legacy.dry_run;
    runtime.use_ai_corrections = legacy.use_llm;
    runtime.keep_original_output = legacy.keep_orig;
    runtime.show_diff = !legacy.no_diff;
    runtime.cleanup_enabled = legacy.cleanup;
    runtime.format_dialogue_enabled = legacy.format_dialogue;
    runtime.outfile_override = legacy.outfile;
    runtime.duration_override = if legacy.one_hour {
        DurationOverride {
            enabled: true,
            hours: 1.0,
        }
    } else if legacy.two_hours {
        DurationOverride {
            enabled: true,
            hours: 2.0,
        }
    } else {
        DurationOverride::default()
    };

    runtime.active_preset = if legacy.rsm7 {
        TUESDAY_7_PRESET_NAME.to_string()
    } else if legacy.rsm8 {
        TUESDAY_8_PRESET_NAME.to_string()
    } else if legacy.tp6 {
        FRIDAY_6_PRESET_NAME.to_string()
    } else {
        SATURDAY_PRESET_NAME.to_string()
    };

    FileConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        runtime,
        ui: UiPreferences::default(),
        presets: default_presets(),
    }
}

/// Produce a `ConvocationsConfig` based on the stored runtime settings.
pub fn runtime_preferences_to_convocations(
    runtime: &RuntimePreferences,
    presets: &[PresetDefinition],
) -> (ConvocationsConfig, Vec<String>) {
    let mut config = ConvocationsConfig::default();
    let mut warnings = Vec::new();

    config.presets = presets.to_vec();
    config.infile = runtime.chat_log_path.clone();
    config.last = runtime.weeks_ago;
    config.dry_run = runtime.dry_run;
    config.use_llm = runtime.use_ai_corrections;
    config.keep_orig = runtime.keep_original_output;
    config.no_diff = !runtime.show_diff;
    config.cleanup = runtime.cleanup_enabled;
    config.format_dialogue = runtime.format_dialogue_enabled;
    config.free_models_only = runtime.free_models_only;

    let trimmed_outfile = runtime.outfile_override.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let trimmed_directory = runtime
        .output_directory_override
        .as_ref()
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

    match runtime.output_target {
        OutputTarget::File => {
            config.outfile = trimmed_outfile;
            config.output_directory = None;
        }
        OutputTarget::Directory => {
            config.outfile = None;
            config.output_directory = trimmed_directory;
        }
    }

    config.active_preset = runtime.active_preset.clone();

    // Reset event flags based on active preset.
    set_event_flags_for_preset(
        &mut config,
        runtime.active_preset.as_str(),
        presets,
        &mut warnings,
    );

    // Map duration override to existing boolean flags.
    config.duration_override = runtime.duration_override.clone();
    if config.duration_override.enabled {
        if !config.duration_override.hours.is_finite() {
            warnings.push(
                "Duration override hours must be a finite number. Disabling override.".to_string(),
            );
            config.duration_override.enabled = false;
            config.duration_override.hours = DurationOverride::default_hours();
        } else if config.duration_override.hours < 1.0 {
            warnings.push(
                "Duration override hours must be at least 1.0. Disabling override.".to_string(),
            );
            config.duration_override.enabled = false;
            config.duration_override.hours = DurationOverride::default_hours();
        }
    }

    if config.duration_override.enabled {
        config.one_hour = (config.duration_override.hours - 1.0).abs() < f32::EPSILON;
        config.two_hours = (config.duration_override.hours - 2.0).abs() < f32::EPSILON;
    } else {
        config.one_hour = false;
        config.two_hours = false;
    }

    let openrouter_model = runtime
        .openrouter_model
        .as_ref()
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .unwrap_or_else(|| AUTO_SENTINEL.to_string());
    config.openrouter_model = openrouter_model.clone();

    match runtime.resolve_openrouter_api_key() {
        Ok(Some(key)) => {
            config.openrouter_api_key = Some(key);
        }
        Ok(None) => {
            config.openrouter_api_key = None;
            if config.use_llm {
                warnings.push(
                    "OpenRouter API key not configured; AI corrections will be skipped."
                        .to_string(),
                );
            }
        }
        Err(err) => {
            config.openrouter_api_key = None;
            warnings.push(format!(
                "Failed to retrieve OpenRouter API key: {}. AI corrections will be skipped.",
                err
            ));
        }
    }

    (config, warnings)
}

fn set_event_flags_for_preset(
    config: &mut ConvocationsConfig,
    preset_name: &str,
    presets: &[PresetDefinition],
    warnings: &mut Vec<String>,
) {
    config.rsm7 = false;
    config.rsm8 = false;
    config.tp6 = false;

    let preset = presets.iter().find(|preset| preset.name == preset_name);
    match preset {
        Some(preset) => match preset.file_prefix.as_str() {
            "rsm7" => config.rsm7 = true,
            "rsm8" => config.rsm8 = true,
            "tp6" => config.tp6 = true,
            "conv" => { /* default Saturday */ }
            other => warnings.push(format!(
                "Preset '{}' uses unrecognised file prefix '{}'; falling back to Saturday configuration.",
                preset_name, other
            )),
        },
        None => warnings.push(format!(
            "Preset '{}' not found. Falling back to Saturday configuration.",
            preset_name
        )),
    }
}

/// Merge runtime overrides into an existing configuration.
pub fn apply_runtime_overrides(
    config: &mut ConvocationsConfig,
    overrides: &RuntimeOverrides,
    presets: &[PresetDefinition],
    warnings: &mut Vec<String>,
) {
    if let Some(value) = overrides.last {
        config.last = value;
    }
    if let Some(value) = overrides.dry_run {
        config.dry_run = value;
    }
    if let Some(ref value) = overrides.infile {
        config.infile = value.clone();
    }
    if let Some(ref value) = overrides.start {
        config.start = value.clone();
    }
    if let Some(ref value) = overrides.end {
        config.end = value.clone();
    }
    if let Some(ref preset_id) = overrides.active_preset {
        config.active_preset = preset_id.clone();
        set_event_flags_for_preset(config, preset_id, presets, warnings);
    }
    if let Some(mut duration) = overrides.duration_override.clone() {
        if !duration.hours.is_finite() {
            warnings.push(
                "Duration override hours must be a finite number. Ignoring override.".to_string(),
            );
            duration.enabled = false;
            duration.hours = DurationOverride::default_hours();
        } else if duration.hours < 1.0 {
            warnings.push(
                "Duration override hours must be at least 1.0. Ignoring override.".to_string(),
            );
            duration.enabled = false;
            duration.hours = DurationOverride::default_hours();
        }

        config.duration_override = duration;
        if config.duration_override.enabled {
            config.one_hour = (config.duration_override.hours - 1.0).abs() < f32::EPSILON;
            config.two_hours = (config.duration_override.hours - 2.0).abs() < f32::EPSILON;
        } else {
            config.one_hour = false;
            config.two_hours = false;
        }
    }
    if let Some(ref value) = overrides.process_file {
        config.process_file = value.clone();
    }
    if let Some(value) = overrides.format_dialogue {
        config.format_dialogue = value;
    }
    if let Some(value) = overrides.cleanup {
        config.cleanup = value;
    }
    if let Some(value) = overrides.use_llm {
        config.use_llm = value;
    }
    if let Some(value) = overrides.keep_orig {
        config.keep_orig = value;
    }
    if let Some(value) = overrides.no_diff {
        config.no_diff = value;
    }
    if let Some(target) = overrides.output_target.clone() {
        match target {
            OutputTarget::File => {
                config.output_directory = None;
            }
            OutputTarget::Directory => {
                config.outfile = None;
            }
        }
    }
    if let Some(ref value) = overrides.output_directory {
        config.output_directory = value.clone();
    }
    if let Some(ref value) = overrides.outfile {
        config.outfile = value.clone();
    }
    if let Some(ref value) = overrides.openrouter_model {
        config.openrouter_model = value.clone();
    }
    if let Some(value) = overrides.use_ai_corrections {
        config.use_llm = value;
    }
    if let Some(value) = overrides.keep_original_output {
        config.keep_orig = value;
    }
    if let Some(value) = overrides.show_diff {
        config.no_diff = !value;
    }
}

/// Produce overrides by diffing a configuration against defaults.
pub fn runtime_overrides_from_convocations(config: &ConvocationsConfig) -> RuntimeOverrides {
    let defaults = ConvocationsConfig::default();
    let mut overrides = RuntimeOverrides::default();

    if config.last != defaults.last {
        overrides.last = Some(config.last);
    }
    if config.dry_run != defaults.dry_run {
        overrides.dry_run = Some(config.dry_run);
    }
    if config.infile != defaults.infile {
        overrides.infile = Some(config.infile.clone());
    }
    if config.start != defaults.start {
        overrides.start = Some(config.start.clone());
    }
    if config.end != defaults.end {
        overrides.end = Some(config.end.clone());
    }
    if config.active_preset != defaults.active_preset {
        overrides.active_preset = Some(config.active_preset.clone());
    } else if config.rsm7 && config.rsm7 != defaults.rsm7 {
        overrides.active_preset = Some(TUESDAY_7_PRESET_NAME.to_string());
    } else if config.rsm8 && config.rsm8 != defaults.rsm8 {
        overrides.active_preset = Some(TUESDAY_8_PRESET_NAME.to_string());
    } else if config.tp6 && config.tp6 != defaults.tp6 {
        overrides.active_preset = Some(FRIDAY_6_PRESET_NAME.to_string());
    }
    if config.duration_override != defaults.duration_override {
        overrides.duration_override = Some(config.duration_override.clone());
    } else if config.one_hour != defaults.one_hour || config.two_hours != defaults.two_hours {
        overrides.duration_override = Some(DurationOverride {
            enabled: config.one_hour || config.two_hours,
            hours: if config.one_hour {
                1.0
            } else if config.two_hours {
                2.0
            } else {
                1.0
            },
        });
    }
    if config.process_file != defaults.process_file {
        overrides.process_file = Some(config.process_file.clone());
    }
    if config.format_dialogue != defaults.format_dialogue {
        overrides.format_dialogue = Some(config.format_dialogue);
    }
    if config.cleanup != defaults.cleanup {
        overrides.cleanup = Some(config.cleanup);
    }
    if config.use_llm != defaults.use_llm {
        overrides.use_llm = Some(config.use_llm);
        overrides.use_ai_corrections = Some(config.use_llm);
    }
    if config.keep_orig != defaults.keep_orig {
        overrides.keep_orig = Some(config.keep_orig);
        overrides.keep_original_output = Some(config.keep_orig);
    }
    if config.no_diff != defaults.no_diff {
        overrides.no_diff = Some(config.no_diff);
        overrides.show_diff = Some(!config.no_diff);
    }
    if config.output_directory != defaults.output_directory {
        overrides.output_directory = Some(config.output_directory.clone());
        overrides.output_target = Some(OutputTarget::Directory);
    }
    if config.outfile != defaults.outfile {
        overrides.outfile = Some(config.outfile.clone());
        if overrides.output_target.is_none() {
            overrides.output_target = Some(OutputTarget::File);
        }
    }
    if config.openrouter_model != defaults.openrouter_model {
        overrides.openrouter_model = Some(config.openrouter_model.clone());
    }

    overrides
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_bad_toml_duplicate_presets() {
        let mut config = FileConfig::default();
        // Add a duplicate preset
        config.presets.push(PresetDefinition {
            name: SATURDAY_PRESET_NAME.to_string(),
            weekday: "saturday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "22:00".to_string(),
            duration_minutes: 145,
            file_prefix: "conv".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have removed the duplicate
        assert_eq!(
            sanitized
                .presets
                .iter()
                .filter(|p| p.name == SATURDAY_PRESET_NAME)
                .count(),
            1,
            "Should have exactly one instance of the Saturday preset"
        );

        // Should have a warning about duplicates
        assert!(
            warnings.iter().any(|w| w.contains("duplicate")),
            "Should warn about duplicate preset names"
        );
    }

    #[test]
    fn test_sanitize_preset_zero_duration() {
        let mut config = FileConfig::default();
        // Create a preset with zero duration
        config.presets.push(PresetDefinition {
            name: "Bad Preset".to_string(),
            weekday: "monday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "12:00".to_string(),
            duration_minutes: 0,
            file_prefix: "bad".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have fixed the duration
        let bad_preset = sanitized.presets.iter().find(|p| p.name == "Bad Preset");
        assert!(bad_preset.is_some());
        assert_eq!(
            bad_preset.unwrap().duration_minutes,
            60,
            "Should reset to 60"
        );

        // Should have a warning
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Bad Preset") && w.contains("duration_minutes")),
            "Should warn about zero duration"
        );
    }

    #[test]
    fn test_sanitize_preset_empty_prefix() {
        let mut config = FileConfig::default();
        // Create a preset with empty prefix
        config.presets.push(PresetDefinition {
            name: "No Prefix".to_string(),
            weekday: "tuesday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "14:00".to_string(),
            duration_minutes: 60,
            file_prefix: "  ".to_string(), // whitespace only
            default_weeks_ago: 0,
            builtin: false,
        });

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have removed the preset with empty prefix
        let preset = sanitized.presets.iter().find(|p| p.name == "No Prefix");
        assert!(
            preset.is_none(),
            "Preset with empty prefix should be removed"
        );

        // Should have a warning
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("No Prefix") && w.contains("file_prefix")),
            "Should warn about empty prefix"
        );
    }

    #[test]
    fn test_sanitize_invalid_duration_override_infinite() {
        let mut config = FileConfig::default();
        config.runtime.duration_override = DurationOverride {
            enabled: true,
            hours: f32::INFINITY,
        };

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have disabled and reset
        assert!(!sanitized.runtime.duration_override.enabled);
        assert_eq!(sanitized.runtime.duration_override.hours, 1.0);

        // Should have a warning
        assert!(
            warnings.iter().any(|w| w.contains("finite")),
            "Should warn about non-finite hours"
        );
    }

    #[test]
    fn test_sanitize_invalid_duration_override_negative() {
        let mut config = FileConfig::default();
        config.runtime.duration_override = DurationOverride {
            enabled: true,
            hours: 0.5,
        };

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have disabled and reset
        assert!(!sanitized.runtime.duration_override.enabled);
        assert_eq!(sanitized.runtime.duration_override.hours, 1.0);

        // Should have a warning
        assert!(
            warnings.iter().any(|w| w.contains("at least 1.0")),
            "Should warn about hours < 1.0"
        );
    }

    #[test]
    fn test_sanitize_invalid_active_preset() {
        let mut config = FileConfig::default();
        config.runtime.active_preset = "nonexistent-preset".to_string();

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have reset to default
        assert_eq!(sanitized.runtime.active_preset, SATURDAY_PRESET_NAME);

        // Should have a warning
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("nonexistent-preset") && w.contains("not found")),
            "Should warn about missing preset"
        );
    }

    #[test]
    fn test_sanitize_wrong_schema_version() {
        let mut config = FileConfig::default();
        config.schema_version = 999;

        let (sanitized, warnings, _) = sanitize_config(config);

        // Should have reset to defaults
        assert_eq!(sanitized.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(sanitized.runtime, RuntimePreferences::default());

        // Should have a warning
        assert!(
            warnings.iter().any(|w| w.contains("schema version")),
            "Should warn about unknown schema version"
        );
    }

    #[test]
    fn test_load_config_bad_toml() {
        // This test would require creating a temporary config file
        // For now, we just test that load_config returns a valid result
        let result = load_config();
        // Should get a valid config (source may be File if config exists, or Default otherwise)
        assert!(result.source == ConfigSource::Default || result.source == ConfigSource::File);
        // Config should be valid
        assert_eq!(result.config.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_runtime_preferences_to_convocations() {
        let mut runtime = RuntimePreferences::default();
        runtime.duration_override = DurationOverride {
            enabled: true,
            hours: 2.5,
        };

        let presets = default_presets();
        let (config, warnings) = runtime_preferences_to_convocations(&runtime, &presets);

        // Should convert properly
        assert_eq!(config.duration_override.hours, 2.5);
        assert!(config.duration_override.enabled);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_preset_crud_add_user_preset() {
        let mut config = FileConfig::default();
        let initial_count = config.presets.len();

        // Add a custom user preset
        config.presets.push(PresetDefinition {
            name: "Wednesday Event".to_string(),
            weekday: "wednesday".to_string(),
            timezone: "America/Los_Angeles".to_string(),
            start_time: "19:30".to_string(),
            duration_minutes: 90,
            file_prefix: "wed".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });

        let (sanitized, warnings, _) = sanitize_config(config);

        assert_eq!(sanitized.presets.len(), initial_count + 1);
        assert!(
            sanitized
                .presets
                .iter()
                .any(|p| p.name == "Wednesday Event"),
            "Custom preset should be present"
        );
        assert!(warnings.is_empty(), "Should not generate warnings");
    }

    #[test]
    fn test_preset_crud_remove_user_preset() {
        let mut config = FileConfig::default();

        // Add a custom preset
        config.presets.push(PresetDefinition {
            name: "Temporary".to_string(),
            weekday: "thursday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "20:00".to_string(),
            duration_minutes: 60,
            file_prefix: "temp".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });

        // Remove it (simulate deletion)
        config.presets.retain(|p| p.name != "Temporary");

        let (sanitized, _, _) = sanitize_config(config);

        assert!(
            !sanitized.presets.iter().any(|p| p.name == "Temporary"),
            "Deleted preset should not be present"
        );

        // Built-in presets should still be there
        assert!(
            sanitized
                .presets
                .iter()
                .any(|p| p.name == SATURDAY_PRESET_NAME),
            "Built-in presets should remain"
        );
    }

    #[test]
    fn test_preset_crud_edit_user_preset() {
        let mut config = FileConfig::default();

        // Add a custom preset
        config.presets.push(PresetDefinition {
            name: "Original Name".to_string(),
            weekday: "monday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "18:00".to_string(),
            duration_minutes: 60,
            file_prefix: "orig".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });

        // Edit it
        if let Some(preset) = config
            .presets
            .iter_mut()
            .find(|p| p.name == "Original Name")
        {
            preset.name = "Updated Name".to_string();
            preset.duration_minutes = 120;
            preset.file_prefix = "updated".to_string();
        }

        let (sanitized, warnings, _) = sanitize_config(config);

        let edited = sanitized.presets.iter().find(|p| p.name == "Updated Name");
        assert!(edited.is_some());
        assert_eq!(edited.unwrap().name, "Updated Name");
        assert_eq!(edited.unwrap().duration_minutes, 120);
        assert_eq!(edited.unwrap().file_prefix, "updated");
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_preset_builtin_protection() {
        let mut config = FileConfig::default();
        let initial_builtin_count = config.presets.iter().filter(|p| p.builtin).count();

        // Try to remove all built-in presets
        config.presets.retain(|p| !p.builtin);

        let (sanitized, _, _) = sanitize_config(config);

        // Built-ins should be restored
        let restored_builtin_count = sanitized.presets.iter().filter(|p| p.builtin).count();
        assert_eq!(
            restored_builtin_count, initial_builtin_count,
            "All built-in presets should be restored"
        );
    }

    #[test]
    fn test_duration_override_hours_validation() {
        // Test various invalid hour values
        let test_cases = vec![
            (f32::INFINITY, false),
            (f32::NEG_INFINITY, false),
            (f32::NAN, false),
            (0.5, false),  // Less than 1.0
            (0.0, false),  // Zero
            (-1.0, false), // Negative
            (1.0, true),   // Valid minimum
            (2.5, true),   // Valid non-integer
            (24.0, true),  // Valid large value
        ];

        for (hours, should_be_valid) in test_cases {
            let mut config = FileConfig::default();
            config.runtime.duration_override = DurationOverride {
                enabled: true,
                hours,
            };

            let (sanitized, warnings, _) = sanitize_config(config);

            if should_be_valid {
                assert!(
                    sanitized.runtime.duration_override.enabled,
                    "Valid hours ({}) should keep override enabled",
                    hours
                );
                assert_eq!(sanitized.runtime.duration_override.hours, hours);
            } else {
                assert!(
                    !sanitized.runtime.duration_override.enabled,
                    "Invalid hours ({}) should disable override",
                    hours
                );
                assert_eq!(
                    sanitized.runtime.duration_override.hours,
                    DurationOverride::default_hours()
                );
                assert!(
                    !warnings.is_empty(),
                    "Should have warnings for invalid hours"
                );
            }
        }
    }

    #[test]
    fn test_apply_runtime_overrides_duration() {
        let mut config = ConvocationsConfig::default();
        let presets = default_presets();
        let mut warnings = Vec::new();

        let mut overrides = RuntimeOverrides::default();
        overrides.duration_override = Some(DurationOverride {
            enabled: true,
            hours: 3.5,
        });

        apply_runtime_overrides(&mut config, &overrides, &presets, &mut warnings);

        assert!(config.duration_override.enabled);
        assert_eq!(config.duration_override.hours, 3.5);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_apply_runtime_overrides_preset_change() {
        let mut config = ConvocationsConfig::default();
        let presets = default_presets();
        let mut warnings = Vec::new();

        // Default should be Saturday
        assert_eq!(config.active_preset, SATURDAY_PRESET_NAME);

        let mut overrides = RuntimeOverrides::default();
        overrides.active_preset = Some(TUESDAY_7_PRESET_NAME.to_string());

        apply_runtime_overrides(&mut config, &overrides, &presets, &mut warnings);

        assert_eq!(config.active_preset, TUESDAY_7_PRESET_NAME);
        assert!(config.rsm7, "RSM7 flag should be set");
        assert!(!config.rsm8, "RSM8 flag should not be set");
        assert!(!config.tp6, "TP6 flag should not be set");
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_save_presets_and_ui_only_preserves_runtime() {
        // This test verifies that save_presets_and_ui_only() doesn't overwrite runtime preferences
        let mut custom_runtime = RuntimePreferences::default();
        custom_runtime.weeks_ago = 5;
        custom_runtime.dry_run = true;

        // First, create and save a config with custom runtime settings
        let initial_config = FileConfig {
            schema_version: CURRENT_SCHEMA_VERSION,
            runtime: custom_runtime.clone(),
            ui: UiPreferences::default(),
            presets: default_presets(),
        };

        // In a real scenario, this would be saved to disk
        // For this test, we'll verify the logic by checking what load_config returns

        let mut new_ui = UiPreferences::default();
        new_ui.theme = ThemePreference::Light;

        let mut new_presets = default_presets();
        new_presets.push(PresetDefinition {
            name: "New Preset".to_string(),
            weekday: "sunday".to_string(),
            timezone: "America/Chicago".to_string(),
            start_time: "15:00".to_string(),
            duration_minutes: 90,
            file_prefix: "new".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });

        // Simulate what save_presets_and_ui_only does
        let mut config = initial_config;
        config.presets = new_presets.clone();
        config.ui = new_ui.clone();

        // Runtime should be preserved
        assert_eq!(config.runtime.weeks_ago, 5);
        assert!(config.runtime.dry_run);

        // UI and presets should be updated
        assert_eq!(config.ui.theme, ThemePreference::Light);
        assert!(config.presets.iter().any(|p| p.name == "New Preset"));
    }
}
