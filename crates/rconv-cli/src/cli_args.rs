use std::collections::HashSet;

use clap::{ArgAction, Args, Parser, Subcommand, ValueHint};
use rconv_core::config::{
    DurationOverride, FRIDAY_6_PRESET_NAME, RuntimeOverrides, TUESDAY_7_PRESET_NAME,
    TUESDAY_8_PRESET_NAME,
};
use rconv_core::curator::AUTO_SENTINEL;

/// Top-level CLI entrypoint.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(flatten)]
    pub process: ProcessArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Supported subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(subcommand)]
    Preset(PresetCommand),
    #[command(subcommand)]
    Secret(SecretCommand),
}

/// Preset management subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum PresetCommand {
    /// Create a new preset stored in config.toml.
    #[command(alias = "add")]
    Create(PresetCreateArgs),
    /// Update an existing preset by ID.
    Update(PresetUpdateArgs),
    /// Delete a preset by ID (builtin presets cannot be removed).
    #[command(alias = "remove")]
    Delete(PresetDeleteArgs),
}

/// Secret management commands.
#[derive(Debug, Clone, Subcommand)]
pub enum SecretCommand {
    /// Store or update the OpenRouter API key used for AI corrections.
    SetOpenRouterKey {
        /// API key value. If omitted, you will be prompted securely.
        #[arg(short, long, value_name = "KEY")]
        key: Option<String>,
    },
    /// Remove any saved OpenRouter API key from secure storage.
    ClearOpenRouterKey,
}

/// Arguments for the main processing flow (default command).
#[derive(Debug, Clone, Args, Default)]
pub struct ProcessArgs {
    /// Weeks ago to look back when determining the event date.
    #[arg(
        long = "last",
        num_args = 0..=1,
        value_parser = clap::value_parser!(u32),
        value_name = "WEEKS"
    )]
    pub last: Option<u32>,

    /// Print actions without writing files.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Chat log file path.
    #[arg(short, long, value_hint = ValueHint::FilePath)]
    pub infile: Option<String>,

    /// Override the start timestamp (ISO 8601).
    #[arg(long = "start")]
    pub start: Option<String>,

    /// Override the end timestamp (ISO 8601).
    #[arg(long = "end")]
    pub end: Option<String>,

    /// Select preset by ID.
    #[arg(long = "preset", value_name = "ID")]
    pub preset: Option<String>,

    /// Shortcut for the Tuesday 7pm preset.
    #[arg(long, action = ArgAction::SetTrue)]
    pub rsm7: bool,

    /// Shortcut for the Tuesday 8pm preset.
    #[arg(long, action = ArgAction::SetTrue)]
    pub rsm8: bool,

    /// Shortcut for the Friday 6pm preset.
    #[arg(long, action = ArgAction::SetTrue)]
    pub tp6: bool,

    /// Force duration override to 1 hour.
    #[arg(long = "1h", action = ArgAction::SetTrue)]
    pub one_hour: bool,

    /// Force duration override to 2 hours.
    #[arg(long = "2h", action = ArgAction::SetTrue)]
    pub two_hours: bool,

    /// Custom duration override in hours (e.g. 1.5).
    #[arg(long = "duration-hours", value_name = "HOURS")]
    pub duration_hours: Option<f32>,

    /// Disable any duration override.
    #[arg(long = "duration-disable", action = ArgAction::SetTrue)]
    pub duration_disable: bool,

    /// Process a pre-filtered file instead of the raw chat log.
    #[arg(short = 'p', long = "process-file", value_hint = ValueHint::FilePath)]
    pub process_file: Option<String>,

    /// Toggle the cleanup stage (defaults to config value).
    #[arg(
        long = "cleanup",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::value_parser!(bool)
    )]
    pub cleanup: Option<bool>,

    /// Toggle AI corrections (defaults to config value).
    #[arg(
        long = "llm",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::value_parser!(bool)
    )]
    pub use_llm: Option<bool>,

    /// Keep the original file when AI corrections run.
    #[arg(long = "keep-orig", action = ArgAction::SetTrue)]
    pub keep_orig: bool,

    /// Skip diff generation when AI corrections run.
    #[arg(long = "no-diff", action = ArgAction::SetTrue)]
    pub no_diff: bool,

    /// Override the output file name.
    #[arg(value_name = "OUTFILE")]
    pub outfile: Option<String>,

    /// Select a curated model by ID (use "auto" for automatic selection).
    #[arg(long = "model", value_name = "ID")]
    pub model: Option<String>,

    /// List curated model IDs and exit.
    #[arg(long = "list-curated", action = ArgAction::SetTrue)]
    pub list_curated: bool,
}

impl ProcessArgs {
    /// Returns true when no overrides were provided.
    pub fn is_empty(&self) -> bool {
        self.last.is_none()
            && !self.dry_run
            && self.infile.is_none()
            && self.start.is_none()
            && self.end.is_none()
            && self.preset.is_none()
            && !self.rsm7
            && !self.rsm8
            && !self.tp6
            && !self.one_hour
            && !self.two_hours
            && self.duration_hours.is_none()
            && !self.duration_disable
            && self.process_file.is_none()
            && self.cleanup.is_none()
            && self.use_llm.is_none()
            && !self.keep_orig
            && !self.no_diff
            && self.outfile.is_none()
            && self.model.is_none()
            && !self.list_curated
    }

    /// Convert CLI flags into runtime overrides plus any advisory warnings.
    pub fn to_runtime_overrides(&self) -> Result<(RuntimeOverrides, Vec<String>), String> {
        let mut overrides = RuntimeOverrides::default();
        let warnings = Vec::new();

        if let Some(weeks) = self.last {
            overrides.last = Some(weeks);
        }

        if self.dry_run {
            overrides.dry_run = Some(true);
        }

        if let Some(ref infile) = self.infile {
            overrides.infile = Some(infile.clone());
        }

        if let Some(ref start) = self.start {
            overrides.start = Some(parse_optional_field(start));
        }

        if let Some(ref end) = self.end {
            overrides.end = Some(parse_optional_field(end));
        }

        if let Some(process_file) = self.process_file.as_ref() {
            overrides.process_file = Some(parse_optional_field(process_file));
        }

        if let Some(outfile) = self.outfile.as_ref() {
            overrides.outfile = Some(parse_optional_field(outfile));
        }

        if let Some(model) = self.model.as_ref() {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                overrides.openrouter_model = Some(AUTO_SENTINEL.to_string());
            } else if trimmed.eq_ignore_ascii_case(AUTO_SENTINEL) {
                overrides.openrouter_model = Some(AUTO_SENTINEL.to_string());
            } else {
                overrides.openrouter_model = Some(trimmed.to_string());
            }
        }

        let mut preset_ids = HashSet::new();
        if let Some(ref preset) = self.preset {
            preset_ids.insert(preset.clone());
        }
        if self.rsm7 {
            preset_ids.insert(TUESDAY_7_PRESET_NAME.to_string());
        }
        if self.rsm8 {
            preset_ids.insert(TUESDAY_8_PRESET_NAME.to_string());
        }
        if self.tp6 {
            preset_ids.insert(FRIDAY_6_PRESET_NAME.to_string());
        }

        if preset_ids.len() > 1 {
            return Err(
                "Multiple preset flags detected (--preset/--rsm7/--rsm8/--tp6). Choose one.".into(),
            );
        } else if let Some(id) = preset_ids.into_iter().next() {
            overrides.active_preset = Some(id);
        }

        let mut duration_specified: Option<f32> = None;
        if self.one_hour {
            duration_specified = Some(1.0);
        }
        if self.two_hours {
            duration_specified = combine_duration(duration_specified, 2.0)?;
        }
        if let Some(hours) = self.duration_hours {
            duration_specified = combine_duration(duration_specified, hours)?;
        }

        if self.duration_disable {
            if duration_specified.is_some() {
                return Err(
                    "Cannot combine --duration-disable with other duration flags (--1h/--2h/--duration-hours)."
                        .into(),
                );
            }
            overrides.duration_override = Some(DurationOverride {
                enabled: false,
                ..DurationOverride::default()
            });
        } else if let Some(hours) = duration_specified {
            if !hours.is_finite() || hours <= 0.0 {
                return Err("Duration hours must be a positive finite number.".into());
            }
            overrides.duration_override = Some(DurationOverride {
                enabled: true,
                hours,
            });
        }

        if let Some(cleanup) = self.cleanup {
            overrides.cleanup = Some(cleanup);
        }

        if let Some(use_llm) = self.use_llm {
            overrides.use_llm = Some(use_llm);
            overrides.use_ai_corrections = Some(use_llm);
        }

        if self.keep_orig {
            overrides.keep_orig = Some(true);
            overrides.keep_original_output = Some(true);
        }

        if self.no_diff {
            overrides.no_diff = Some(true);
            overrides.show_diff = Some(false);
        }

        Ok((overrides, warnings))
    }
}

/// Arguments for creating a new preset.
#[derive(Debug, Clone, Args)]
pub struct PresetCreateArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub weekday: String,
    #[arg(long)]
    pub timezone: String,
    #[arg(long = "start-time")]
    pub start_time: String,
    #[arg(long = "duration-minutes")]
    pub duration_minutes: u32,
    #[arg(long = "file-prefix")]
    pub file_prefix: String,
    #[arg(long = "weeks-ago", default_value_t = 0)]
    pub default_weeks_ago: u32,
}

/// Arguments for updating an existing preset by name.
#[derive(Debug, Clone, Args)]
pub struct PresetUpdateArgs {
    /// The name of the preset to update
    #[arg(long)]
    pub name: String,
    /// New weekday (if changing)
    #[arg(long = "new-weekday")]
    pub weekday: Option<String>,
    /// New timezone (if changing)
    #[arg(long = "new-timezone")]
    pub timezone: Option<String>,
    /// New start time (if changing)
    #[arg(long = "new-start-time")]
    pub start_time: Option<String>,
    /// New duration in minutes (if changing)
    #[arg(long = "new-duration-minutes")]
    pub duration_minutes: Option<u32>,
    /// New file prefix (if changing)
    #[arg(long = "new-file-prefix")]
    pub file_prefix: Option<String>,
    /// New default weeks ago (if changing)
    #[arg(long = "new-weeks-ago")]
    pub default_weeks_ago: Option<u32>,
}

/// Arguments for deleting a preset by name.
#[derive(Debug, Clone, Args)]
pub struct PresetDeleteArgs {
    #[arg(long)]
    pub name: String,
}

fn parse_optional_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "none" | "null" | "unset"
    ) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_override_defaults_to_auto() {
        let args = ProcessArgs {
            model: Some("auto".to_string()),
            ..Default::default()
        };
        let (overrides, warnings) = args.to_runtime_overrides().expect("overrides");
        assert!(warnings.is_empty());
        assert_eq!(overrides.openrouter_model, Some(AUTO_SENTINEL.to_string()));

        let args = ProcessArgs {
            model: Some("provider/custom".to_string()),
            ..Default::default()
        };
        let (overrides, _) = args.to_runtime_overrides().expect("overrides");
        assert_eq!(
            overrides.openrouter_model,
            Some("provider/custom".to_string())
        );
    }
}

fn combine_duration(current: Option<f32>, next: f32) -> Result<Option<f32>, String> {
    if let Some(existing) = current {
        if (existing - next).abs() > f32::EPSILON {
            Err(
                "Duration flags conflict; specify only one of --1h, --2h, or --duration-hours."
                    .into(),
            )
        } else {
            Ok(Some(existing))
        }
    } else {
        Ok(Some(next))
    }
}
