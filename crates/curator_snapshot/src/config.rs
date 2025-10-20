use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::error::CuratorError;

/// Default schema version emitted into the snapshot.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;

/// CLI surface for the snapshot generator.
#[derive(Debug, Parser, Clone)]
#[command(author, version, about = "Generate curated OpenRouter model snapshots")]
pub struct CliArgs {
    /// Output path for the curated snapshot JSON.
    #[arg(
        long = "out",
        value_name = "FILE",
        default_value = "static/model_snapshot.json"
    )]
    pub out: PathBuf,

    /// Alias map used to map AA entries onto OpenRouter slugs.
    #[arg(
        long = "aliases",
        value_name = "FILE",
        default_value = "static/aliases.json"
    )]
    pub aliases: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Tunables {
    pub openrouter_models_url: String,
    pub openrouter_api_key: Option<String>,
    pub aa_models_url: String,
    pub aa_api_key: Option<String>,
    pub min_free_aaii: f32,
    pub min_paid_aaii: f32,
    pub cheap_in_max: f64,
    pub cheap_out_max: f64,
    pub min_context_length: u32,
    pub fuzzy_match_threshold: f64,
    pub max_retries: usize,
    pub retry_backoff_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub snapshot: PathBuf,
    pub aliases: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub paths: Paths,
    pub tunables: Tunables,
}

impl CliArgs {
    pub fn resolve(self) -> Result<AppConfig, CuratorError> {
        let snapshot = resolve_path(&self.out)?;
        let aliases = resolve_path(&self.aliases)?;

        ensure_parent_directory(&snapshot)?;
        ensure_parent_directory(&aliases)?;

        if !aliases.exists() {
            fs::write(&aliases, "{}\n")?;
        }

        let tunables = Tunables::from_env()?;

        Ok(AppConfig {
            paths: Paths { snapshot, aliases },
            tunables,
        })
    }
}

impl Tunables {
    pub fn from_env() -> Result<Self, CuratorError> {
        let openrouter_models_url = env::var("OPENROUTER_MODELS_URL")
            .unwrap_or_else(|_| "https://openrouter.ai/api/v1/models".to_string());
        let openrouter_api_key = env::var("OPENROUTER_API_KEY").ok();
        let aa_models_url = env::var("AA_MODELS_URL").unwrap_or_else(|_| {
            "https://artificialanalysis.ai/api/v2/data/llms/models".to_string()
        });
        let aa_api_key = env::var("AA_API_KEY").ok();

        let min_free_aaii = parse_f32_env("MIN_FREE_AAII", 60.0)?;
        let min_paid_aaii = parse_f32_env("MIN_PAID_AAII", 65.0)?;
        let cheap_in_max = parse_f64_env("CHEAP_IN_MAX_USD_PER_1M", 1.5)?;
        let cheap_out_max = parse_f64_env("CHEAP_OUT_MAX_USD_PER_1M", 6.0)?;
        let min_context_length = parse_u32_env("MIN_CONTEXT_LENGTH", 8_192)?;
        let fuzzy_match_threshold = parse_f64_env("FUZZY_MATCH_THRESHOLD", 0.94)?;

        let max_retries = env::var("CURATOR_MAX_RETRIES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3usize);
        let retry_backoff_ms = env::var("CURATOR_RETRY_BACKOFF_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1_000u64);

        Ok(Self {
            openrouter_models_url,
            openrouter_api_key,
            aa_models_url,
            aa_api_key,
            min_free_aaii,
            min_paid_aaii,
            cheap_in_max,
            cheap_out_max,
            min_context_length,
            fuzzy_match_threshold,
            max_retries,
            retry_backoff_ms,
        })
    }
}

fn parse_f32_env(var: &str, default: f32) -> Result<f32, CuratorError> {
    parse_env(var, default, |s| s.parse::<f32>())
}

fn parse_f64_env(var: &str, default: f64) -> Result<f64, CuratorError> {
    parse_env(var, default, |s| s.parse::<f64>())
}

fn parse_u32_env(var: &str, default: u32) -> Result<u32, CuratorError> {
    parse_env(var, default, |s| s.parse::<u32>())
}

fn parse_env<T, F, E>(var: &str, default: T, mut parser: F) -> Result<T, CuratorError>
where
    F: FnMut(&str) -> Result<T, E>,
    T: Copy,
    E: std::fmt::Display,
{
    match env::var(var) {
        Ok(value) => match parser(&value) {
            Ok(parsed) => Ok(parsed),
            Err(err) => Err(CuratorError::Config(format!(
                "invalid value for {}: {}",
                var, err
            ))),
        },
        Err(_) => Ok(default),
    }
}

fn resolve_path(path: &Path) -> Result<PathBuf, CuratorError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(workspace_root().join(path))
    }
}

fn ensure_parent_directory(path: &Path) -> Result<(), CuratorError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
