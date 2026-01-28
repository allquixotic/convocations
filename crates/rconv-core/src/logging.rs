use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use thiserror::Error;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::ParseError;
use tracing_subscriber::fmt::time::{LocalTime, UtcTime};
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::config_directory;

/// Controls where structured logs are published.
#[derive(Debug, Clone, Copy)]
pub enum LoggingDestination {
    /// Emit logs to both the persistent file and stderr for interactive CLIs.
    FileAndStderr,
    /// Emit logs only to the persistent file (GUI background services).
    FileOnly,
    /// Emit logs only to stderr (primarily for tests or ad-hoc tools).
    StderrOnly,
}

#[derive(Debug)]
struct LoggingGuards {
    _guard: Option<WorkerGuard>,
    log_path: Option<PathBuf>,
}

static LOGGING_STATE: OnceLock<LoggingGuards> = OnceLock::new();

/// Errors that can arise while standing up structured logging.
#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("failed to prepare log directory: {0}")]
    Io(#[from] io::Error),
    #[error("invalid logging filter: {0}")]
    Filter(#[from] ParseError),
    #[error("failed to install logging subscriber: {0}")]
    Subscriber(#[from] tracing_subscriber::util::TryInitError),
}

/// Install the global structured logging subscriber.
///
/// The first call wins; subsequent calls are no-ops that return the resolved log file path.
pub fn init_logging(
    destination: LoggingDestination,
) -> Result<Option<&'static PathBuf>, LoggingError> {
    if LOGGING_STATE.get().is_none() {
        let guards = install_logging(destination)?;
        if let Err(guards) = LOGGING_STATE.set(guards) {
            drop(guards);
        }
    }

    Ok(LOGGING_STATE
        .get()
        .and_then(|guards| guards.log_path.as_ref()))
}

/// Returns the log file path selected during logging initialization (if any).
pub fn current_log_path() -> Option<&'static PathBuf> {
    LOGGING_STATE
        .get()
        .and_then(|guards| guards.log_path.as_ref())
}

fn install_logging(destination: LoggingDestination) -> Result<LoggingGuards, LoggingError> {
    let filter = build_filter()?;
    let registry = tracing_subscriber::registry().with(filter);

    let (file_layer, guard, log_path) = match destination {
        LoggingDestination::FileAndStderr | LoggingDestination::FileOnly => {
            let dir = config_directory().join("logs");
            fs::create_dir_all(&dir)?;
            let path = dir.join("convocations.log");
            let file_appender = tracing_appender::rolling::never(&dir, "convocations.log");
            let (writer, worker_guard) = tracing_appender::non_blocking(file_appender);
            let layer = tracing_subscriber::fmt::layer()
                .event_format(
                    tracing_subscriber::fmt::format()
                        .json()
                        .with_timer(UtcTime::rfc_3339())
                        .with_level(true)
                        .with_target(true)
                        .with_file(true)
                        .with_line_number(true),
                )
                .with_writer(writer)
                .with_ansi(false)
                .boxed();
            (layer, Some(worker_guard), Some(path))
        }
        LoggingDestination::StderrOnly => {
            let layer = tracing_subscriber::fmt::layer()
                .event_format(
                    tracing_subscriber::fmt::format()
                        .json()
                        .with_timer(UtcTime::rfc_3339())
                        .with_level(true)
                        .with_target(true)
                        .with_file(true)
                        .with_line_number(true),
                )
                .with_writer(io::sink)
                .with_ansi(false)
                .boxed();
            (layer, None, None)
        }
    };

    let stderr_layer = match destination {
        LoggingDestination::FileOnly => tracing_subscriber::fmt::layer()
            .event_format(
                tracing_subscriber::fmt::format()
                    .with_timer(LocalTime::rfc_3339())
                    .with_level(true)
                    .with_target(true)
                    .with_ansi(false),
            )
            .with_writer(io::sink)
            .with_ansi(false)
            .boxed(),
        _ => tracing_subscriber::fmt::layer()
            .event_format(
                tracing_subscriber::fmt::format()
                    .with_timer(LocalTime::rfc_3339())
                    .with_level(true)
                    .with_target(true)
                    .with_ansi(false),
            )
            .with_writer(io::stderr)
            .with_ansi(false)
            .boxed(),
    };

    let subscriber = registry.with(file_layer).with(stderr_layer);

    subscriber.try_init()?;

    if let Some(path) = log_path.as_ref() {
        info!(path = %path.display(), "Structured logging enabled");
    }

    Ok(LoggingGuards {
        _guard: guard,
        log_path,
    })
}

fn build_filter() -> Result<EnvFilter, ParseError> {
    if let Ok(spec) = env::var("CONVOCATIONS_LOG") {
        if !spec.trim().is_empty() {
            return EnvFilter::try_new(spec);
        }
    }

    match EnvFilter::try_from_default_env() {
        Ok(filter) => Ok(filter),
        Err(_) => EnvFilter::try_new("info"),
    }
}
