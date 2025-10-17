use crate::config::{
    DurationOverride, FRIDAY_6_PRESET_NAME, PresetDefinition, SATURDAY_PRESET_NAME,
    TUESDAY_7_PRESET_NAME, TUESDAY_8_PRESET_NAME, ThemePreference,
    default_presets as config_default_presets,
};
use crate::openrouter;
use chrono::{Datelike, Duration, Local, NaiveDate};
use chrono_tz;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvocationsConfig {
    pub last: u32,
    pub dry_run: bool,
    pub infile: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub rsm7: bool,
    pub rsm8: bool,
    pub tp6: bool,
    pub one_hour: bool,
    pub two_hours: bool,
    pub process_file: Option<String>,
    pub format_dialogue: bool,
    pub cleanup: bool,
    pub use_llm: bool,
    pub keep_orig: bool,
    pub no_diff: bool,
    pub outfile: Option<String>,
    #[serde(default = "default_active_preset")]
    pub active_preset: String,
    #[serde(default = "runtime_presets_default")]
    pub presets: Vec<PresetDefinition>,
    #[serde(default)]
    pub duration_override: DurationOverride,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub show_technical_log: bool,
    #[serde(default = "default_follow_technical_log")]
    pub follow_technical_log: bool,
}

fn default_active_preset() -> String {
    SATURDAY_PRESET_NAME.to_string()
}

fn runtime_presets_default() -> Vec<PresetDefinition> {
    config_default_presets()
}

const fn default_follow_technical_log() -> bool {
    true
}

impl Default for ConvocationsConfig {
    fn default() -> Self {
        Self {
            last: 0,
            dry_run: false,
            infile: "~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log".to_string(),
            start: None,
            end: None,
            rsm7: false,
            rsm8: false,
            tp6: false,
            one_hour: false,
            two_hours: false,
            process_file: None,
            format_dialogue: true,
            cleanup: true,
            use_llm: true,
            keep_orig: false,
            no_diff: false,
            outfile: None,
            active_preset: default_active_preset(),
            presets: runtime_presets_default(),
            duration_override: DurationOverride::default(),
            theme: ThemePreference::default(),
            show_technical_log: false,
            follow_technical_log: default_follow_technical_log(),
        }
    }
}

/// Describes the resolved output file paths for the current configuration.
#[derive(Debug, Clone)]
pub struct OutfileResolution {
    /// Absolute (or user-specified) path that will be used during processing.
    pub effective: String,
    /// Generated default path based on the working directory and preset context.
    pub default: String,
    /// Indicates whether the effective path originates from a user override.
    pub was_overridden: bool,
}

/// Resolve the effective and default output paths for a configuration.
///
/// `working_dir` allows callers (e.g. wrappers or tests) to inject the desired base directory.
/// When `None`, the helper falls back to the `CONVOCATIONS_WORKING_DIR` environment variable
/// if it is present.
pub fn resolve_outfile_paths(
    config: &ConvocationsConfig,
    working_dir: Option<&Path>,
    today: Option<NaiveDate>,
) -> Result<OutfileResolution, String> {
    let working_dir_buf = resolve_working_dir(working_dir);
    let working_dir_ref = working_dir_buf.as_deref();

    let default = derive_default_outfile(config, working_dir_ref, today)?;

    let override_path = config
        .outfile
        .as_ref()
        .map(|value| value.trim())
        .filter(|trimmed| !trimmed.is_empty())
        .map(|trimmed| trimmed.to_string());
    let was_overridden = override_path.is_some();

    let effective = match override_path {
        Some(ref value) => qualify_outfile_path(value, working_dir_ref),
        None => default.clone(),
    };

    Ok(OutfileResolution {
        effective,
        default,
        was_overridden,
    })
}

fn derive_default_outfile(
    config: &ConvocationsConfig,
    working_dir: Option<&Path>,
    today: Option<NaiveDate>,
) -> Result<String, String> {
    if config
        .process_file
        .as_ref()
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .is_some()
    {
        return Ok(qualify_outfile_path("processed-output.txt", working_dir));
    }

    let event_type = if config.rsm7 {
        EventType::Rsm7
    } else if config.rsm8 {
        EventType::Rsm8
    } else if config.tp6 {
        EventType::Tp6
    } else {
        EventType::Saturday
    };

    let default_duration_minutes = resolve_default_duration_minutes(config, &event_type);
    let duration_minutes = if config.duration_override.enabled {
        hours_to_minutes(config.duration_override.hours)?
    } else if config.one_hour {
        60
    } else if config.two_hours {
        120
    } else {
        default_duration_minutes as i64
    };

    // Determine effective weeks_ago value: use preset's default_weeks_ago if config.last is 0
    let effective_weeks_ago = if config.last == 0 {
        if let Some(preset) = find_active_preset(config) {
            preset.default_weeks_ago
        } else {
            0
        }
    } else {
        config.last
    };

    let today = today.unwrap_or_else(|| Local::now().date_naive());
    let (calculated_start, calculated_end, file_date) =
        calculate_dates_for_event(today, effective_weeks_ago, &event_type, duration_minutes);

    let user_provided_start = config.start.is_some();
    let user_provided_end = config.end.is_some();

    let start_effective = config
        .start
        .as_deref()
        .unwrap_or_else(|| calculated_start.as_str());
    let end_effective = config
        .end
        .as_deref()
        .unwrap_or_else(|| calculated_end.as_str());

    let outfile_name = if user_provided_start || user_provided_end {
        let start_component = sanitize_for_filename(start_effective);
        let end_component = sanitize_for_filename(end_effective);
        format!("event-{}-{}.txt", start_component, end_component)
    } else {
        let prefix = derive_file_prefix(config, &event_type);
        format!("{}-{}.txt", prefix, file_date)
    };

    Ok(qualify_outfile_path(&outfile_name, working_dir))
}

fn derive_file_prefix(config: &ConvocationsConfig, event_type: &EventType) -> String {
    if let Some(preset) = find_active_preset(config) {
        let trimmed = preset.file_prefix.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    event_type.prefix().to_string()
}

fn sanitize_for_filename(value: &str) -> String {
    value.replace(':', "-").replace('T', "_")
}

fn resolve_working_dir(provided: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = provided {
        return Some(path.to_path_buf());
    }
    if let Ok(env_dir) = std::env::var("CONVOCATIONS_WORKING_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    None
}

fn qualify_outfile_path(outfile: &str, working_dir: Option<&Path>) -> String {
    let is_absolute = Path::new(outfile).is_absolute() || outfile.starts_with('~');
    if is_absolute {
        return outfile.to_string();
    }

    if let Some(base) = working_dir {
        let mut path = PathBuf::from(base);
        path.push(outfile);
        return path.to_string_lossy().to_string();
    }

    outfile.to_string()
}

#[derive(Clone, Copy, Debug)]
enum RunOrigin {
    CliArgs,
    ProvidedConfig,
}

#[derive(Clone, Debug)]
struct Pending {
    msgid: usize,
    value: String,
    first_channel: String,
    name: String,
}

#[derive(Debug)]
enum EventType {
    Saturday, // Default: Saturday 22:00-00:25
    Rsm7,     // Tuesday 7pm Eastern, 1 hour
    Rsm8,     // Tuesday 8pm Eastern, 1 hour
    Tp6,      // Friday 6pm Eastern, 1 hour
}

impl EventType {
    fn prefix(&self) -> &str {
        match self {
            EventType::Saturday => "conv",
            EventType::Rsm7 => "rsm7",
            EventType::Rsm8 => "rsm8",
            EventType::Tp6 => "tp6",
        }
    }
}

fn find_active_preset<'a>(config: &'a ConvocationsConfig) -> Option<&'a PresetDefinition> {
    config
        .presets
        .iter()
        .find(|preset| preset.name == config.active_preset)
}

fn resolve_default_duration_minutes(config: &ConvocationsConfig, event_type: &EventType) -> u32 {
    if let Some(preset) = find_active_preset(config) {
        if preset.duration_minutes > 0 {
            return preset.duration_minutes;
        }
    }

    match event_type {
        EventType::Saturday => 145,
        EventType::Rsm7 | EventType::Rsm8 | EventType::Tp6 => 60,
    }
}

fn hours_to_minutes(hours: f32) -> Result<i64, String> {
    if !hours.is_finite() {
        return Err("Duration override hours must be a finite number.".to_string());
    }
    if hours < 1.0 {
        return Err("Duration override hours must be at least 1.0.".to_string());
    }

    let minutes = (hours * 60.0).round();
    if minutes <= 0.0 {
        return Err("Duration override must resolve to at least one minute.".to_string());
    }

    Ok(minutes as i64)
}

pub(crate) fn validate_config(config: &ConvocationsConfig) -> Result<(), String> {
    // Check for multiple event types
    let event_count = [config.rsm7, config.rsm8, config.tp6]
        .iter()
        .filter(|&&x| x)
        .count();
    if event_count > 1 {
        return Err("Cannot specify more than one event type (--rsm7, --rsm8, --tp6)".to_string());
    }

    // Check for multiple duration flags
    if config.one_hour && config.two_hours {
        return Err("Cannot specify both --1h and --2h".to_string());
    }

    if config.duration_override.enabled {
        let hours = config.duration_override.hours;
        if !hours.is_finite() {
            return Err("Duration override hours must be a finite number.".to_string());
        }
        if hours < 1.0 {
            return Err("Duration override hours must be at least 1.0.".to_string());
        }
    }

    // Check if event/duration flags conflict with custom start/end dates
    let has_duration_override =
        config.one_hour || config.two_hours || config.duration_override.enabled;
    let has_event_or_duration_flags =
        config.rsm7 || config.rsm8 || config.tp6 || has_duration_override;
    let has_custom_dates = config.start.is_some() || config.end.is_some();

    if has_event_or_duration_flags && has_custom_dates {
        return Err(
            "Cannot use event flags (--rsm7, --rsm8, --tp6) or duration overrides (--1h, --2h, --duration-hours) with custom start/end dates (--start, --end)"
                .to_string(),
        );
    }

    if let Some(preset) = find_active_preset(config) {
        if preset.duration_minutes == 0 {
            return Err(format!(
                "Preset '{}' duration_minutes must be greater than zero.",
                preset.name
            ));
        }
    }

    Ok(())
}

pub type StageProgressCallback = Arc<dyn Fn(StageProgressEvent) + Send + Sync + 'static>;

#[derive(Debug, Clone, Serialize)]
pub struct StageProgressEvent {
    pub kind: StageProgressEventKind,
    pub stage: Option<String>,
    pub elapsed_ms: f64,
    pub stage_elapsed_ms: Option<f64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageProgressEventKind {
    Begin,
    End,
    Note,
    Progress,
}

#[derive(Clone)]
struct StageLogger {
    program_start: Instant,
    stage_start: Instant,
    current_stage: Option<String>,
    callback: Option<StageProgressCallback>,
}

impl StageLogger {
    fn new(start: Instant, callback: Option<StageProgressCallback>) -> Self {
        Self {
            program_start: start,
            stage_start: start,
            current_stage: None,
            callback,
        }
    }

    fn begin(&mut self, name: &str) {
        let since_start = self.program_start.elapsed();
        println!("[+{} ms] BEGIN: {}", format_ms(since_start), name);
        self.stage_start = Instant::now();
        self.current_stage = Some(name.to_string());
        if let Some(cb) = &self.callback {
            cb(StageProgressEvent {
                kind: StageProgressEventKind::Begin,
                stage: Some(name.to_string()),
                elapsed_ms: since_start.as_secs_f64() * 1_000.0,
                stage_elapsed_ms: None,
                message: Some(format!("Starting {name}")),
            });
        }
    }

    fn end(&mut self, name: &str) {
        let stage_elapsed = self.stage_start.elapsed();
        let total_elapsed = self.program_start.elapsed();
        println!(
            "[+{} ms] END: {} (Δ {} ms)",
            format_ms(total_elapsed),
            name,
            format_ms(stage_elapsed)
        );
        if let Some(cb) = &self.callback {
            cb(StageProgressEvent {
                kind: StageProgressEventKind::End,
                stage: Some(name.to_string()),
                elapsed_ms: total_elapsed.as_secs_f64() * 1_000.0,
                stage_elapsed_ms: Some(stage_elapsed.as_secs_f64() * 1_000.0),
                message: Some(format!(
                    "Finished {name} (Δ {} ms)",
                    format_ms(stage_elapsed)
                )),
            });
        }
        self.current_stage = None;
    }

    fn note(&mut self, message: impl Into<String>) {
        let text = message.into();
        println!("{text}");
        if let Some(cb) = &self.callback {
            cb(StageProgressEvent {
                kind: StageProgressEventKind::Note,
                stage: self.current_stage.clone(),
                elapsed_ms: self.program_start.elapsed().as_secs_f64() * 1_000.0,
                stage_elapsed_ms: Some(self.stage_start.elapsed().as_secs_f64() * 1_000.0),
                message: Some(text),
            });
        }
    }

    fn progress(&self, message: impl Into<String>) {
        let text = message.into();
        // Only print progress to console if explicitly enabled
        if std::env::var("CONVOCATIONS_PROGRESS_CONSOLE").is_ok() {
            println!("{text}");
        }
        if let Some(cb) = &self.callback {
            cb(StageProgressEvent {
                kind: StageProgressEventKind::Progress,
                stage: self.current_stage.clone(),
                elapsed_ms: self.program_start.elapsed().as_secs_f64() * 1_000.0,
                stage_elapsed_ms: Some(self.stage_start.elapsed().as_secs_f64() * 1_000.0),
                message: Some(text),
            });
        }
    }
}

fn format_ms(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1_000.0; // fractional milliseconds
    format!("{:.3}", ms)
}

pub async fn run_cli(config: ConvocationsConfig) -> Result<(), String> {
    run(config, RunOrigin::CliArgs, None).await
}

pub async fn run_with_config(config: ConvocationsConfig) -> Result<(), String> {
    run(config, RunOrigin::ProvidedConfig, None).await
}

pub async fn run_with_config_with_progress(
    config: ConvocationsConfig,
    callback: StageProgressCallback,
) -> Result<(), String> {
    run(config, RunOrigin::ProvidedConfig, Some(callback)).await
}

/// Calculate start and end dates for an event type given current date, weeks ago, and duration.
///
/// This is a public API for calculating dates that the GUI and other consumers can use.
///
/// # Parameters
/// - `today`: The reference date to calculate from
/// - `weeks_ago`: Number of weeks to go back (0 = most recent occurrence)
/// - `event_type_str`: The event type as a string ("saturday", "rsm7", "rsm8", "tp6")
/// - `duration_minutes`: Duration of the event in minutes
///
/// # Returns
/// A tuple of (start_datetime, end_datetime) in ISO format (YYYY-MM-DDTHH:MM)
pub fn calculate_event_dates(
    today: NaiveDate,
    weeks_ago: u32,
    event_type_str: &str,
    duration_minutes: i64,
) -> Result<(String, String), String> {
    let event_type = match event_type_str.to_lowercase().as_str() {
        "saturday" => EventType::Saturday,
        "rsm7" => EventType::Rsm7,
        "rsm8" => EventType::Rsm8,
        "tp6" => EventType::Tp6,
        _ => return Err(format!("Unknown event type: {}", event_type_str)),
    };

    let (start, end, _file_date) =
        calculate_dates_for_event(today, weeks_ago, &event_type, duration_minutes);
    Ok((start, end))
}

async fn run(
    mut config: ConvocationsConfig,
    origin: RunOrigin,
    callback: Option<StageProgressCallback>,
) -> Result<(), String> {
    // High-precision start timestamps
    let program_start = Instant::now();
    let start_wall = Local::now();
    println!(
        "Program start (local): {}",
        start_wall.format("%Y-%m-%dT%H:%M:%S%.6f %z")
    );
    let today = start_wall.date_naive();
    let mut logger = StageLogger::new(program_start, callback.clone());

    let stage_label = match origin {
        RunOrigin::CliArgs => "Parse CLI arguments",
        RunOrigin::ProvidedConfig => "Load configuration",
    };

    logger.begin(stage_label);
    // Validate argument combinations
    if let Err(e) = validate_config(&config) {
        logger.end(stage_label);
        return Err(e);
    }

    // Normalize preset flags and duration toggles so downstream logic can rely on booleans
    if !config.active_preset.is_empty() {
        match config.active_preset.as_str() {
            TUESDAY_7_PRESET_NAME => {
                config.rsm7 = true;
                config.rsm8 = false;
                config.tp6 = false;
            }
            TUESDAY_8_PRESET_NAME => {
                config.rsm7 = false;
                config.rsm8 = true;
                config.tp6 = false;
            }
            FRIDAY_6_PRESET_NAME => {
                config.rsm7 = false;
                config.rsm8 = false;
                config.tp6 = true;
            }
            SATURDAY_PRESET_NAME => {
                config.rsm7 = false;
                config.rsm8 = false;
                config.tp6 = false;
            }
            _ => { /* leave legacy flags as-is for custom presets */ }
        }
    }

    if config.duration_override.enabled {
        let hours = config.duration_override.hours;
        if (hours - 1.0).abs() < f32::EPSILON {
            config.one_hour = true;
            config.two_hours = false;
        } else if (hours - 2.0).abs() < f32::EPSILON {
            config.one_hour = false;
            config.two_hours = true;
        } else {
            config.one_hour = false;
            config.two_hours = false;
        }
    } else {
        config.one_hour = false;
        config.two_hours = false;
    }

    logger.end(stage_label);

    // Check if we're in pre-filtered file mode
    if let Some(ref process_file) = config.process_file {
        logger.note("Mode: Pre-filtered file processing (--process-file)");
        // Process a pre-filtered file with configurable processing stages
        let outfile_resolution = resolve_outfile_paths(&config, None, Some(today))?;
        let outfile = outfile_resolution.effective.clone();

        if outfile_resolution.was_overridden {
            logger.note(format!(
                "Pre-filtered output target: {} -> {} (override; default would be {})",
                process_file, outfile, outfile_resolution.default
            ));
        } else {
            logger.note(format!(
                "Pre-filtered output target: {} -> {}",
                process_file, outfile
            ));
        }

        if config.dry_run {
            logger.note(format!(
                "Dry run: would process pre-filtered file {} -> {}",
                process_file, outfile
            ));
            logger.note(format!("  Format dialogue: {}", config.format_dialogue));
            logger.note(format!("  Apply cleanup: {}", config.cleanup));
            logger.note(format!("  Use LLM: {}", config.use_llm));
            logger.note(format!(
                "[+{} ms] Program complete (dry run)",
                format_ms(program_start.elapsed())
            ));
            return Ok(());
        }

        logger.begin("Process pre-filtered file");
        process_filtered_file(
            &mut logger,
            process_file,
            &outfile,
            config.format_dialogue,
            config.cleanup,
            config.use_llm,
            config.keep_orig,
            config.no_diff,
        )
        .await;
        logger.end("Process pre-filtered file");
        logger.note(format!(
            "Finished processing pre-filtered file. Output at {}",
            outfile
        ));
    } else {
        logger.note("Mode: Standard processing (ChatLog.log)");
        // Standard mode: process ChatLog.log with date filtering
        // Determine dates based on precedence: explicit -s/-e override computed values.
        logger.begin("Calculate date filters");
        let mut start_opt = config.start.clone();
        let mut end_opt = config.end.clone();

        // Determine event type
        let event_type = if config.rsm7 {
            EventType::Rsm7
        } else if config.rsm8 {
            EventType::Rsm8
        } else if config.tp6 {
            EventType::Tp6
        } else {
            EventType::Saturday
        };

        // Determine duration (in minutes)
        let default_duration_minutes = resolve_default_duration_minutes(&config, &event_type);
        let duration_minutes = if config.duration_override.enabled {
            hours_to_minutes(config.duration_override.hours)?
        } else if config.one_hour {
            60
        } else if config.two_hours {
            120
        } else {
            default_duration_minutes as i64
        };

        let duration_source = if config.duration_override.enabled {
            format!("override {:.2}h", config.duration_override.hours)
        } else if let Some(preset) = find_active_preset(&config) {
            format!("preset {}", preset.name)
        } else {
            "default event duration".to_string()
        };
        logger.note(format!(
            "Resolved duration: {} minutes ({})",
            duration_minutes, duration_source
        ));

        // Determine effective weeks_ago value: use preset's default_weeks_ago if config.last is 0
        let effective_weeks_ago = if config.last == 0 {
            if let Some(preset) = find_active_preset(&config) {
                preset.default_weeks_ago
            } else {
                0
            }
        } else {
            config.last
        };

        if effective_weeks_ago != config.last {
            logger.note(format!(
                "Using preset default_weeks_ago: {} (config.last was {})",
                effective_weeks_ago, config.last
            ));
        }

        // Always calculate dates to get the file_date for default filename
        let (saturday_date, sunday_date, file_date) =
            calculate_dates_for_event(today, effective_weeks_ago, &event_type, duration_minutes);

        // Always log the calculated dates (matching Python's logging.info)
        logger.note(format!("Calculated Saturday Date: {}", saturday_date));
        logger.note(format!("Calculated Sunday Date: {}", sunday_date));
        logger.note(format!("Calculated File Date: {}", file_date));

        // ALWAYS set date filters if not explicitly provided (matching Python behavior)
        // Python always passes -s and -e to Node.js, regardless of --last value
        if start_opt.is_none() {
            start_opt = Some(saturday_date.clone());
        }
        if end_opt.is_none() {
            end_opt = Some(sunday_date.clone());
        }

        let outfile_resolution = resolve_outfile_paths(&config, None, Some(today))?;
        let outfile = outfile_resolution.effective.clone();
        logger.end("Calculate date filters");

        match (start_opt.as_ref(), end_opt.as_ref()) {
            (Some(s), Some(e)) => logger.note(format!("Processing window: {} → {}", s, e)),
            _ => logger.note("Processing entire log (no start/end filter)"),
        }
        if outfile_resolution.was_overridden {
            logger.note(format!(
                "Output file: {} (override; default would be {})",
                outfile, outfile_resolution.default
            ));
        } else {
            logger.note(format!("Output file: {}", outfile));
        }

        if config.dry_run {
            match (&start_opt, &end_opt) {
                (Some(s), Some(e)) => logger.note(format!(
                    "Dry run: would process {} from {} to {} -> {}",
                    config.infile, s, e, outfile
                )),
                _ => logger.note(format!(
                    "Dry run: would process entire file {} (no start/end filter) -> {}",
                    config.infile, outfile
                )),
            }
            logger.note(format!(
                "[+{} ms] Program complete (dry run)",
                format_ms(program_start.elapsed())
            ));
            return Ok(());
        }

        logger.begin("Process log file");
        process_log_file(
            &mut logger,
            &config.infile,
            &outfile,
            start_opt.as_deref(),
            end_opt.as_deref(),
            config.use_llm,
            config.keep_orig,
            config.no_diff,
        )
        .await;
        logger.end("Process log file");
        logger.note(format!("Finished processing log. Output at {}", outfile));
    }

    println!(
        "[+{} ms] Program complete",
        format_ms(program_start.elapsed())
    );
    Ok(())
}

fn calculate_dates_for_event(
    today: chrono::NaiveDate,
    last_occurrences: u32,
    event_type: &EventType,
    duration_minutes: i64,
) -> (String, String, String) {
    use chrono::Weekday;

    match event_type {
        EventType::Saturday => {
            // Determine the last relevant Saturday and start at 22:00 Eastern
            let adjusted_date = today - Duration::weeks(last_occurrences as i64);
            let days_since_saturday = (adjusted_date.weekday().num_days_from_monday() + 2) % 7;
            let last_saturday = adjusted_date - Duration::days(days_since_saturday as i64);

            calculate_event_times(last_saturday, 22, duration_minutes)
        }
        EventType::Rsm7 | EventType::Rsm8 => {
            // Find the most recent Tuesday (or N Tuesdays ago)
            let target_weekday = Weekday::Tue;
            let event_date = find_weekday_occurrence(today, target_weekday, last_occurrences);

            // Determine start hour based on event type
            let start_hour = match event_type {
                EventType::Rsm7 => 19, // 7pm
                EventType::Rsm8 => 20, // 8pm
                _ => unreachable!(),
            };

            calculate_event_times(event_date, start_hour, duration_minutes)
        }
        EventType::Tp6 => {
            // Find the most recent Friday (or N Fridays ago)
            let target_weekday = Weekday::Fri;
            let event_date = find_weekday_occurrence(today, target_weekday, last_occurrences);

            calculate_event_times(event_date, 18, duration_minutes) // 6pm
        }
    }
}

fn find_weekday_occurrence(
    from_date: chrono::NaiveDate,
    target_weekday: chrono::Weekday,
    occurrences_ago: u32,
) -> chrono::NaiveDate {
    // Find the most recent occurrence of target_weekday from from_date
    let days_since_target = {
        let today_weekday_num = from_date.weekday().num_days_from_monday();
        let target_weekday_num = target_weekday.num_days_from_monday();

        if today_weekday_num >= target_weekday_num {
            today_weekday_num - target_weekday_num
        } else {
            7 - (target_weekday_num - today_weekday_num)
        }
    };

    let most_recent = from_date - Duration::days(days_since_target as i64);

    // Go back N weeks if occurrences_ago > 0
    most_recent - Duration::weeks(occurrences_ago as i64)
}

fn calculate_event_times(
    event_date: chrono::NaiveDate,
    start_hour_eastern: u32,
    duration_minutes: i64,
) -> (String, String, String) {
    use chrono::TimeZone;
    use chrono_tz::America::New_York;

    // Create datetime in Eastern timezone
    let start_eastern = New_York
        .with_ymd_and_hms(
            event_date.year(),
            event_date.month(),
            event_date.day(),
            start_hour_eastern,
            0,
            0,
        )
        .unwrap();

    // Calculate end time
    let end_eastern = start_eastern + Duration::minutes(duration_minutes);

    // Convert to local timezone for the log format
    let start_local = start_eastern.with_timezone(&Local);
    let end_local = end_eastern.with_timezone(&Local);

    let start_date = start_local.format("%Y-%m-%dT%H:%M").to_string();
    let end_date = end_local.format("%Y-%m-%dT%H:%M").to_string();
    let file_date = event_date.format("%m%d%y").to_string();

    (start_date, end_date, file_date)
}

fn get_unedited_filename(outfile: &str) -> String {
    // Extract filename without extension and add _unedited.txt
    let path = std::path::Path::new(outfile);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or("");

    if parent.is_empty() {
        format!("{}_unedited.txt", stem)
    } else {
        format!("{}/{}_unedited.txt", parent, stem)
    }
}

fn display_diff_and_cleanup(
    logger: &mut StageLogger,
    unedited_file: &str,
    edited_file: &str,
    keep_orig: bool,
) {
    logger.begin("Generate and display diff");

    // Read both files
    let unedited_content = match fs::read_to_string(unedited_file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Warning: Could not read unedited file for diff: {}", e);
            logger.end("Generate and display diff");
            return;
        }
    };

    let edited_content = match fs::read_to_string(edited_file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Warning: Could not read edited file for diff: {}", e);
            logger.end("Generate and display diff");
            return;
        }
    };

    // Print the diff header
    println!("\n{}", "=".repeat(80));
    println!("Diff between unedited and LLM-edited versions:");
    println!("{}", "=".repeat(80));

    // Generate and write unified diff directly to stdout using termdiff
    use std::io::Write;
    let mut stdout = std::io::stdout();
    let theme = termdiff::SignsColorTheme {};
    if let Err(e) = termdiff::diff(&mut stdout, &unedited_content, &edited_content, &theme) {
        eprintln!("Warning: Error generating diff: {}", e);
    }
    let _ = stdout.flush(); // Ensure output is flushed

    println!("{}", "=".repeat(80));

    logger.end("Generate and display diff");

    // Clean up unedited file if not keeping it
    if !keep_orig {
        if let Err(e) = fs::remove_file(unedited_file) {
            eprintln!("Warning: Could not remove temporary unedited file: {}", e);
        } else {
            println!("Removed temporary file: {}", unedited_file);
        }
    } else {
        println!("Kept unedited file: {}", unedited_file);
    }
}

async fn process_log_file(
    logger: &mut StageLogger,
    infile: &str,
    outfile: &str,
    start_date: Option<&str>,
    end_date: Option<&str>,
    use_llm: bool,
    keep_orig: bool,
    no_diff: bool,
) {
    // Expand the tilde in the infile path
    logger.begin("Read input file");
    let expanded_infile = shellexpand::tilde(infile).to_string();
    let data = match fs::read_to_string(&expanded_infile) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error reading file {}: {}", expanded_infile, e);
            logger.end("Read input file");
            return;
        }
    };
    logger.end("Read input file");

    logger.begin("Parse and filter lines");
    let mut in_progress: HashMap<String, Pending> = HashMap::new();
    let mut output: Vec<String> = Vec::new();

    let time_regex =
        Regex::new(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.\d{3}-\d{2}:\d{2}) ").unwrap();
    let line_regex = Regex::new(r"(\d+),(.+?),(.+)").unwrap();
    let whtspc = Regex::new(r"\s+").unwrap();
    let strip_ooc = Regex::new(r"(\(\(|\[\[).*?(\)\)|\]\])").unwrap();

    for raw_line in data.lines() {
        if raw_line.is_empty() {
            continue;
        }

        let mut line = raw_line.to_string();
        let datetime = match time_regex.captures(&line) {
            Some(caps) => caps.get(1).map_or("", |m| m.as_str()).to_string(),
            None => continue,
        };

        line = time_regex.replace(&line, "").to_string();

        // Apply optional date filters
        // Extract just the datetime portion without milliseconds and timezone for comparison
        // Original format: 2025-09-09T21:04:27.785-05:00
        // Need to compare: 2025-09-09T21:04 against filter like 2025-08-30T22:00
        let datetime_comparable = if datetime.len() >= 16 {
            &datetime[..16] // Takes "2025-09-09T21:04"
        } else {
            datetime.as_str()
        };

        if let Some(s) = start_date {
            if datetime_comparable < s {
                continue;
            }
        }
        if let Some(e) = end_date {
            if datetime_comparable > e {
                continue;
            }
        }

        let caps = match line_regex.captures(&line) {
            Some(caps) => caps,
            None => continue,
        };

        let channel = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let name = caps.get(2).map_or("", |m| m.as_str()).to_string();
        let mut msg = caps.get(3).map_or("", |m| m.as_str()).to_string();

        // Only include channels 0 (say) and 6 (emote) to match Node behavior
        if channel != "0" && channel != "6" {
            continue;
        }

        if is_encapsulated(&msg) {
            continue;
        }

        // Normalize punctuation
        msg = msg.replace('‘', "'").replace('’', "'").trim().to_string();
        msg = msg.replace('“', "\"").replace('”', "\"");
        msg = msg.replace('…', "...");
        msg = strip_ooc.replace_all(&msg, "").to_string();

        // Spell step (placeholder: no-op but preserves structure and proper-noun skip)
        msg = spell_check_and_correct(&msg);

        if msg.ends_with('>') || msg.ends_with('+') {
            if !in_progress.contains_key(&name) {
                in_progress.insert(
                    name.clone(),
                    Pending {
                        msgid: output.len(),
                        value: msg.clone(),
                        first_channel: channel.clone(),
                        name: name.clone(),
                    },
                );
            } else {
                // Smash continuation into existing pending
                if let Some(entry) = in_progress.get_mut(&name) {
                    smash(entry, &msg);
                }
            }
            continue;
        } else if in_progress.contains_key(&name) {
            // Final line in a series for this person
            if let Some(entry) = in_progress.get_mut(&name) {
                smash(entry, &msg);
                ensure_end_punc(&mut entry.value);
                let formatted = fmt_start(&entry.name, &entry.value, &entry.first_channel, &whtspc)
                    .replace("\"\"", "\"");
                let idx = entry.msgid.min(output.len());
                output.insert(idx, formatted);
            }
            in_progress.remove(&name);
            continue;
        }

        // Finish a single-line message
        ensure_end_punc(&mut msg);
        let formatted = fmt_start(&name, &msg, &channel, &whtspc).replace("\"\"", "\"");
        output.push(formatted);
    }

    // Drain any remaining pending entries; insert in ascending msgid order
    let mut drained: Vec<Pending> = in_progress.into_values().collect();
    drained.sort_by_key(|p| p.msgid);
    for entry in drained.into_iter() {
        let formatted = fmt_start(&entry.name, &entry.value, &entry.first_channel, &whtspc)
            .replace("\"\"", "\"");
        let idx = entry.msgid.min(output.len());
        output.insert(idx, formatted);
    }

    // Concatenate like the Node script (each element already includes a trailing \n)
    let mut final_output = output.join("");

    // Check if we found any data
    if final_output.is_empty() {
        eprintln!("Warning: No log data found for the specified date range!");
        if let (Some(start), Some(end)) = (start_date, end_date) {
            eprintln!("  Searched for entries between {} and {}", start, end);
        } else if let Some(start) = start_date {
            eprintln!("  Searched for entries after {}", start);
        } else if let Some(end) = end_date {
            eprintln!("  Searched for entries before {}", end);
        }
        eprintln!("  Input file: {}", expanded_infile);
        eprintln!("  The log file may not contain data for this time period.");
        logger.end("Parse and filter lines");
        return;
    }
    logger.end("Parse and filter lines");

    // Apply LLM spelling and grammar correction if enabled
    if use_llm {
        if no_diff {
            // Old behavior: apply LLM and write directly to output file
            logger.begin("Apply LLM corrections");
            final_output = apply_llm_correction(&logger, final_output).await;
            logger.end("Apply LLM corrections");

            logger.begin("Write output file");
            match fs::write(outfile, &final_output) {
                Ok(_) => println!("Successfully wrote to {}", outfile),
                Err(e) => eprintln!("Error writing to file {}: {}", outfile, e),
            }
            logger.end("Write output file");
        } else {
            // New behavior: save unedited, apply LLM, save edited, show diff
            let unedited_file = get_unedited_filename(outfile);

            // Save unedited version
            logger.begin("Write unedited file");
            match fs::write(&unedited_file, &final_output) {
                Ok(_) => println!("Saved unedited version to {}", unedited_file),
                Err(e) => {
                    eprintln!("Error writing unedited file {}: {}", unedited_file, e);
                    logger.end("Write unedited file");
                    return;
                }
            }
            logger.end("Write unedited file");

            // Apply LLM corrections
            logger.begin("Apply LLM corrections");
            final_output = apply_llm_correction(&logger, final_output).await;
            logger.end("Apply LLM corrections");

            // Save edited version
            logger.begin("Write output file");
            match fs::write(outfile, &final_output) {
                Ok(_) => println!("Successfully wrote to {}", outfile),
                Err(e) => {
                    eprintln!("Error writing to file {}: {}", outfile, e);
                    logger.end("Write output file");
                    return;
                }
            }
            logger.end("Write output file");

            // Display diff and cleanup
            display_diff_and_cleanup(logger, &unedited_file, outfile, keep_orig);
        }
    } else {
        println!("LLM corrections disabled; skipping stage");

        logger.begin("Write output file");
        match fs::write(outfile, final_output) {
            Ok(_) => println!("Successfully wrote to {}", outfile),
            Err(e) => eprintln!("Error writing to file {}: {}", outfile, e),
        }
        logger.end("Write output file");
    }
}

async fn process_filtered_file(
    logger: &mut StageLogger,
    infile: &str,
    outfile: &str,
    format_dialogue: bool,
    cleanup: bool,
    use_llm: bool,
    keep_orig: bool,
    no_diff: bool,
) {
    // Expand the tilde in the infile path
    logger.begin("Read input file");
    let expanded_infile = shellexpand::tilde(infile).to_string();
    let data = match fs::read_to_string(&expanded_infile) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error reading file {}: {}", expanded_infile, e);
            logger.end("Read input file");
            return;
        }
    };
    logger.end("Read input file");

    let time_regex =
        Regex::new(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.\d{3}-\d{2}:\d{2}) ").unwrap();
    let line_regex = Regex::new(r"(\d+),(.+?),(.+)").unwrap();
    let whtspc = Regex::new(r"\s+").unwrap();
    let strip_ooc = Regex::new(r"(\(\(|\[\[).*?(\)\)|\]\])").unwrap();

    let mut final_output: String;

    let stage_name = if format_dialogue {
        format!("Process lines (format_dialogue=true, cleanup={})", cleanup)
    } else {
        format!("Process lines (format_dialogue=false, cleanup={})", cleanup)
    };
    logger.begin(&stage_name);

    if format_dialogue {
        // Full conversion to human-readable dialogue
        let mut in_progress: HashMap<String, Pending> = HashMap::new();
        let mut output: Vec<String> = Vec::new();

        for raw_line in data.lines() {
            if raw_line.is_empty() {
                continue;
            }

            let mut line = raw_line.to_string();
            let _datetime = match time_regex.captures(&line) {
                Some(caps) => caps.get(1).map_or("", |m| m.as_str()).to_string(),
                None => continue,
            };

            // Remove timestamp
            line = time_regex.replace(&line, "").to_string();

            let caps = match line_regex.captures(&line) {
                Some(caps) => caps,
                None => continue,
            };

            let channel = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let name = caps.get(2).map_or("", |m| m.as_str()).to_string();
            let mut msg = caps.get(3).map_or("", |m| m.as_str()).to_string();

            // Only include channels 0 (say) and 6 (emote)
            if channel != "0" && channel != "6" {
                continue;
            }

            // Cleanup stage (optional)
            if cleanup {
                if is_encapsulated(&msg) {
                    continue;
                }
                // Normalize punctuation
                msg = msg.replace('‘', "'").replace('’', "'").trim().to_string();
                msg = msg.replace('“', "\"").replace('”', "\"");
                msg = msg.replace('…', "...");
                msg = strip_ooc.replace_all(&msg, "").to_string();
                // Placeholder spell check
                msg = spell_check_and_correct(&msg);
            }

            if msg.ends_with('>') || msg.ends_with('+') {
                if !in_progress.contains_key(&name) {
                    in_progress.insert(
                        name.clone(),
                        Pending {
                            msgid: output.len(),
                            value: msg.clone(),
                            first_channel: channel.clone(),
                            name: name.clone(),
                        },
                    );
                } else {
                    // Smash continuation into existing pending
                    if let Some(entry) = in_progress.get_mut(&name) {
                        smash(entry, &msg);
                    }
                }
                continue;
            } else if in_progress.contains_key(&name) {
                // Final line in a series for this person
                if let Some(entry) = in_progress.get_mut(&name) {
                    smash(entry, &msg);
                    ensure_end_punc(&mut entry.value);
                    let formatted =
                        fmt_start(&entry.name, &entry.value, &entry.first_channel, &whtspc)
                            .replace("\"\"", "\"");
                    let idx = entry.msgid.min(output.len());
                    output.insert(idx, formatted);
                }
                in_progress.remove(&name);
                continue;
            }

            // Finish a single-line message
            if cleanup {
                ensure_end_punc(&mut msg);
            }
            let formatted = fmt_start(&name, &msg, &channel, &whtspc).replace("\"\"", "\"");
            output.push(formatted);
        }

        // Drain any remaining pending entries; insert in ascending msgid order
        let mut drained: Vec<Pending> = in_progress.into_values().collect();
        drained.sort_by_key(|p| p.msgid);
        for entry in drained.into_iter() {
            let formatted = fmt_start(&entry.name, &entry.value, &entry.first_channel, &whtspc)
                .replace("\"\"", "\"");
            let idx = entry.msgid.min(output.len());
            output.insert(idx, formatted);
        }

        final_output = output.join("");
    } else {
        // No formatting; optionally cleanup and just output message text per line
        let mut lines_out: Vec<String> = Vec::new();
        for raw_line in data.lines() {
            if raw_line.is_empty() {
                continue;
            }
            let mut line = raw_line.to_string();
            if time_regex.captures(&line).is_none() {
                continue;
            }
            line = time_regex.replace(&line, "").to_string();
            let caps = match line_regex.captures(&line) {
                Some(c) => c,
                None => continue,
            };
            let channel = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let _name = caps.get(2).map_or("", |m| m.as_str()).to_string();
            let mut msg = caps.get(3).map_or("", |m| m.as_str()).to_string();
            if channel != "0" && channel != "6" {
                continue;
            }
            if cleanup {
                if is_encapsulated(&msg) {
                    continue;
                }
                msg = msg.replace('‘', "'").replace('’', "'").trim().to_string();
                msg = msg.replace('“', "\"").replace('”', "\"");
                msg = msg.replace('…', "...");
                msg = strip_ooc.replace_all(&msg, "").to_string();
                msg = spell_check_and_correct(&msg);
            }
            lines_out.push(msg);
        }
        final_output = lines_out.join("\n");
    }
    logger.end(&stage_name);

    // Warn if empty
    if final_output.is_empty() {
        eprintln!("Warning: No log data produced from pre-filtered file!");
        eprintln!("  Input file: {}", expanded_infile);
        eprintln!(
            "  Check flags (format={}, cleanup={}) and input content.",
            format_dialogue, cleanup
        );
        return;
    }

    // Apply LLM corrections if enabled
    if use_llm {
        if no_diff {
            // Old behavior: apply LLM and write directly to output file
            logger.begin("Apply LLM corrections");
            final_output = apply_llm_correction(&logger, final_output).await;
            logger.end("Apply LLM corrections");

            logger.begin("Write output file");
            match fs::write(outfile, &final_output) {
                Ok(_) => println!("Successfully wrote to {}", outfile),
                Err(e) => eprintln!("Error writing to file {}: {}", outfile, e),
            }
            logger.end("Write output file");
        } else {
            // New behavior: save unedited, apply LLM, save edited, show diff
            let unedited_file = get_unedited_filename(outfile);

            // Save unedited version
            logger.begin("Write unedited file");
            match fs::write(&unedited_file, &final_output) {
                Ok(_) => println!("Saved unedited version to {}", unedited_file),
                Err(e) => {
                    eprintln!("Error writing unedited file {}: {}", unedited_file, e);
                    logger.end("Write unedited file");
                    return;
                }
            }
            logger.end("Write unedited file");

            // Apply LLM corrections
            logger.begin("Apply LLM corrections");
            final_output = apply_llm_correction(&logger, final_output).await;
            logger.end("Apply LLM corrections");

            // Save edited version
            logger.begin("Write output file");
            match fs::write(outfile, &final_output) {
                Ok(_) => println!("Successfully wrote to {}", outfile),
                Err(e) => {
                    eprintln!("Error writing to file {}: {}", outfile, e);
                    logger.end("Write output file");
                    return;
                }
            }
            logger.end("Write output file");

            // Display diff and cleanup
            display_diff_and_cleanup(logger, &unedited_file, outfile, keep_orig);
        }
    } else {
        println!("LLM corrections disabled; skipping stage");

        logger.begin("Write output file");
        match fs::write(outfile, final_output) {
            Ok(_) => println!("Successfully wrote to {}", outfile),
            Err(e) => eprintln!("Error writing to file {}: {}", outfile, e),
        }
        logger.end("Write output file");
    }
}

fn is_quote(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '‘' | '’' | '“' | '”')
}

fn is_punctuation_char(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?')
}

fn ends_with_punctuation(s: &str) -> bool {
    let mut tmp = s.trim().to_string();
    while tmp.ends_with('>') || tmp.ends_with('+') {
        tmp.pop();
        tmp = tmp.trim_end().to_string();
    }
    // If it ends with a quote, check the preceding char
    let mut chars = tmp.chars().rev();
    let mut last = chars.next();
    while let Some(ch) = last {
        if is_quote(ch) {
            last = chars.next();
            continue;
        }
        // Found last non-quote char
        return is_punctuation_char(ch) || tmp.ends_with("...");
    }
    false
}

fn is_quoted(s: &str) -> bool {
    let mut tmp = s.trim().to_string();
    while tmp.ends_with('>') || tmp.ends_with('+') {
        tmp.pop();
        tmp = tmp.trim_end().to_string();
    }
    let first = tmp.chars().next().unwrap_or('\0');
    let last = tmp.chars().last().unwrap_or('\0');
    is_quote(first) && (is_quote(last) || is_punctuation_char(last))
}

fn is_encapsulated(msg: &str) -> bool {
    (msg.starts_with("((") && msg.ends_with("))")) || (msg.starts_with("[[") && msg.ends_with("]]"))
}

fn spell_check_and_correct(msg: &str) -> String {
    // For now, do not perform local per-word corrections; rely on LM Studio at the end.
    msg.to_string()
}

fn ensure_end_punc(s: &mut String) {
    if ends_with_punctuation(s) {
        return;
    }
    // If ends with a quote, insert period before it
    if let Some(last) = s.chars().last() {
        if is_quote(last) {
            // Insert '.' before the final quote
            let len = s.len();
            s.insert(len - last.len_utf8(), '.');
            return;
        }
    }
    s.push('.');
}

fn smash(entry: &mut Pending, new_msg: &str) {
    // Remove '+' and '>' globally and trailing quotes from existing value
    entry.value = entry.value.replace(['+', '>'], "").trim_end().to_string();
    if let Some(last) = entry.value.chars().last() {
        if is_quote(last) {
            entry.value.pop();
        }
    }

    // Remove opening quote from new message if present
    let mut nm = new_msg.to_string();
    if let Some(first) = nm.chars().next() {
        if is_quote(first) {
            nm = nm.chars().skip(1).collect();
        }
    }

    if !entry.value.is_empty() {
        entry.value.push(' ');
    }
    entry.value.push_str(&nm);

    // Add closing quote if it started quoted but doesn't end quoted
    if let Some(first) = entry.value.chars().next() {
        if is_quote(first) {
            if let Some(last) = entry.value.chars().last() {
                if !is_quote(last) {
                    entry.value.push(first);
                }
            }
        }
    }
}

fn fmt_start(name: &str, value: &str, first_channel: &str, whtspc: &Regex) -> String {
    let mut mmsg = String::new();
    if first_channel == "0" {
        if is_quoted(value) {
            mmsg = format!("{} says, {}", name, value);
        } else {
            mmsg = format!("{} says, \"{}\"", name, value);
        }
    } else if first_channel == "6" {
        if is_quoted(value) {
            mmsg = format!("{} says, {}", name, value);
        } else {
            mmsg = format!("{} {}", name, value);
        }
    }
    let compact = whtspc.replace_all(&mmsg, " ").to_string();
    format!("{}\n", compact.trim())
}

async fn apply_llm_correction(logger: &StageLogger, text: String) -> String {
    // Try to use OpenRouter to apply corrections
    match perform_openrouter_correction(logger, text.clone()).await {
        Ok(corrected) => {
            println!("Applied OpenRouter grammar and spelling corrections");
            corrected
        }
        Err(e) => {
            eprintln!(
                "Warning: Could not apply OpenRouter corrections: {}. Using original text.",
                e
            );
            text
        }
    }
}

async fn perform_openrouter_correction(
    logger: &StageLogger,
    text: String,
) -> Result<String, Box<dyn std::error::Error>> {
    // Get API key from environment variable
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY environment variable not set")?;

    // Get model from environment variable or use default
    let model = std::env::var("OPENROUTER_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash-lite".to_string());

    // System instructions for grammar correction
    let system_prompt = r##"
    You are a grammar and spelling correction assistant for fantasy role-playing game chat logs.
    Your task is to correct spelling and grammar errors in the provided text.

    Rules:
    - Fix spelling mistakes
    - Correct grammar errors
    - Preserve the original meaning and tone
    - Keep character names exactly as they appear (do not change proper nouns)
    - Keep fantasy terms as they appear (e.g., names of races, places, items)
    - Maintain the dialogue format exactly (e.g., "Name says, ...")
    - Do not add or remove content
    - Do not add any explanations or commentary
    - Return ONLY the corrected text, nothing else
    "##;

    // Split text into manageable chunks if needed (to respect token limits)
    const MAX_CHUNK_SIZE: usize = 4000; // Conservative chunk size
    let chunks: Vec<String> = if text.len() > MAX_CHUNK_SIZE {
        text.split('\n')
            .collect::<Vec<&str>>()
            .chunks(50) // Process ~50 lines at a time
            .map(|chunk| chunk.join("\n"))
            .collect()
    } else {
        vec![text]
    };

    let total_chunks = chunks.len();
    if total_chunks > 1 {
        logger.progress(format!(
            "Processing {} chunks for LLM corrections",
            total_chunks
        ));
    }

    let mut corrected_chunks = Vec::new();

    for (index, chunk) in chunks.iter().enumerate() {
        if total_chunks > 1 {
            logger.progress(format!(
                "Processing chunk {}/{} ({} chars)",
                index + 1,
                total_chunks,
                chunk.len()
            ));
        }

        // Create the prompt with system instructions and the text to correct
        let prompt = format!(
            "{}

Text to correct:
{}

Corrected text:",
            system_prompt, chunk
        );

        // Send request to OpenRouter
        let corrected = openrouter::complete(&api_key, &model, &prompt, 0.3).await?;

        // Clean up the response - remove any potential markdown formatting
        let cleaned = corrected
            .trim()
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        corrected_chunks.push(cleaned.to_string());
    }

    if total_chunks > 1 {
        logger.progress(format!("Completed all {} chunks", total_chunks));
    }

    // Rejoin all corrected chunks
    Ok(corrected_chunks.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::path::Path;

    #[test]
    fn test_resolve_outfile_paths_uses_preset_prefix() {
        let mut config = ConvocationsConfig::default();
        config.active_preset = TUESDAY_7_PRESET_NAME.to_string();
        config.rsm7 = true;

        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap(); // Thursday
        let result = resolve_outfile_paths(&config, None, Some(today)).unwrap();

        // Should use "rsm7" prefix from the preset
        assert!(
            result.default.contains("rsm7-"),
            "Expected preset prefix 'rsm7' in filename: {}",
            result.default
        );
        assert!(!result.was_overridden);
    }

    #[test]
    fn test_resolve_outfile_paths_with_working_dir() {
        let config = ConvocationsConfig::default();
        let working_dir = Path::new("/tmp/test");
        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap();

        let result = resolve_outfile_paths(&config, Some(working_dir), Some(today)).unwrap();

        // Should use working directory
        assert!(
            result.default.starts_with("/tmp/test/"),
            "Expected working dir in path: {}",
            result.default
        );
    }

    #[test]
    fn test_resolve_outfile_paths_with_override() {
        let mut config = ConvocationsConfig::default();
        config.outfile = Some("custom-output.txt".to_string());
        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap();

        let result = resolve_outfile_paths(&config, None, Some(today)).unwrap();

        assert_eq!(result.effective, "custom-output.txt");
        assert!(result.was_overridden);
        // Default should still be calculated
        assert!(result.default.contains("conv-"));
    }

    #[test]
    fn test_calculate_dates_saturday() {
        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap(); // Thursday
        let (_start, _end, file_date) = calculate_dates_for_event(
            today,
            0, // last_occurrences
            &EventType::Saturday,
            145, // 2h 25m
        );

        // Should calculate most recent Saturday (Oct 11)
        assert!(
            file_date == "101125",
            "Expected file date 101125, got {}",
            file_date
        );
    }

    #[test]
    fn test_calculate_dates_weeks_ago() {
        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap(); // Thursday
        let (_, _, file_date) = calculate_dates_for_event(
            today,
            1, // 1 week ago
            &EventType::Saturday,
            145,
        );

        // Should calculate Saturday one week before most recent (Oct 4)
        assert!(
            file_date == "100425",
            "Expected file date 100425, got {}",
            file_date
        );
    }

    #[test]
    fn test_calculate_dates_tuesday_rsm7() {
        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap(); // Thursday
        let (_, _, file_date) = calculate_dates_for_event(today, 0, &EventType::Rsm7, 60);

        // Should calculate most recent Tuesday (Oct 14)
        assert!(
            file_date == "101425",
            "Expected file date 101425, got {}",
            file_date
        );
    }

    #[test]
    fn test_derive_file_prefix_uses_preset() {
        let mut config = ConvocationsConfig::default();
        // Set up a custom preset
        config.presets.push(PresetDefinition {
            name: "Custom Event".to_string(),
            weekday: "monday".to_string(),
            timezone: "America/New_York".to_string(),
            start_time: "20:00".to_string(),
            duration_minutes: 90,
            file_prefix: "custom-prefix".to_string(),
            default_weeks_ago: 0,
            builtin: false,
        });
        config.active_preset = "Custom Event".to_string();

        let prefix = derive_file_prefix(&config, &EventType::Saturday);
        assert_eq!(prefix, "custom-prefix");
    }

    #[test]
    fn test_derive_file_prefix_fallback_to_event_type() {
        let mut config = ConvocationsConfig::default();
        // Set active preset to something that doesn't exist in the preset list
        config.active_preset = "nonexistent".to_string();
        config.presets.clear(); // Remove all presets to ensure fallback

        let prefix = derive_file_prefix(&config, &EventType::Rsm8);
        assert_eq!(prefix, "rsm8");
    }

    #[test]
    fn test_find_active_preset() {
        let mut config = ConvocationsConfig::default();
        config.active_preset = TUESDAY_7_PRESET_NAME.to_string();

        let preset = find_active_preset(&config);
        assert!(preset.is_some());
        assert_eq!(preset.unwrap().name, TUESDAY_7_PRESET_NAME);
    }

    #[test]
    fn test_resolve_default_duration_from_preset() {
        let mut config = ConvocationsConfig::default();
        config.active_preset = SATURDAY_PRESET_NAME.to_string();

        let duration = resolve_default_duration_minutes(&config, &EventType::Saturday);
        assert_eq!(duration, 145); // Saturday preset has 145 minutes
    }

    #[test]
    fn test_hours_to_minutes_conversion() {
        assert_eq!(hours_to_minutes(1.0).unwrap(), 60);
        assert_eq!(hours_to_minutes(2.0).unwrap(), 120);
        assert_eq!(hours_to_minutes(1.5).unwrap(), 90);
        assert_eq!(hours_to_minutes(2.25).unwrap(), 135);
    }

    #[test]
    fn test_hours_to_minutes_validation() {
        assert!(hours_to_minutes(0.5).is_err());
        assert!(hours_to_minutes(f32::INFINITY).is_err());
        assert!(hours_to_minutes(f32::NAN).is_err());
    }

    #[test]
    fn test_find_weekday_occurrence() {
        let today = NaiveDate::from_ymd_opt(2025, 10, 16).unwrap(); // Thursday

        // Find most recent Tuesday
        let tuesday = find_weekday_occurrence(today, chrono::Weekday::Tue, 0);
        assert_eq!(tuesday, NaiveDate::from_ymd_opt(2025, 10, 14).unwrap());

        // Find Tuesday one week ago
        let last_tuesday = find_weekday_occurrence(today, chrono::Weekday::Tue, 1);
        assert_eq!(last_tuesday, NaiveDate::from_ymd_opt(2025, 10, 7).unwrap());

        // Find most recent Friday
        let friday = find_weekday_occurrence(today, chrono::Weekday::Fri, 0);
        assert_eq!(friday, NaiveDate::from_ymd_opt(2025, 10, 10).unwrap());
    }
}
