//! UI-specific state (ephemeral)

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// UI-specific state that doesn't need to be persisted
#[derive(Clone)]
pub struct UiState {
    /// Current theme (dark/light)
    pub theme: Theme,

    /// Form validation errors (field name -> error message)
    pub validation_errors: HashMap<String, String>,

    /// Last edit time for debouncing
    pub last_edit: Instant,

    /// Technical log visibility
    pub technical_log_expanded: bool,

    /// Diff preview visibility
    pub diff_preview_expanded: bool,

    /// Technical log entries (max 200)
    pub technical_log: VecDeque<LogEntry>,

    /// OAuth state
    pub oauth_pending: bool,

    /// API key input buffer for direct entry
    pub api_key_input: String,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            theme: Theme::Dark,
            validation_errors: HashMap::new(),
            last_edit: Instant::now(),
            technical_log_expanded: false,
            diff_preview_expanded: false,
            technical_log: VecDeque::with_capacity(200),
            oauth_pending: false,
            api_key_input: String::new(),
        }
    }

    /// Add a log entry, maintaining max 200 entries
    pub fn add_log_entry(&mut self, entry: LogEntry) {
        if self.technical_log.len() >= 200 {
            self.technical_log.pop_front();
        }
        self.technical_log.push_back(entry);
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Theme selection
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

/// Technical log entry
#[derive(Clone)]
pub struct LogEntry {
    /// Timestamp
    pub timestamp: String,

    /// Log level
    pub level: LogLevel,

    /// Message
    pub message: String,
}

/// Log level for coloring
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}
