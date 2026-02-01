//! Async runtime bridge for running background tasks in egui

use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Bridge between async runtime and egui
pub struct AsyncBridge {
    /// Tokio runtime for async operations (wrapped in Option for clean shutdown)
    runtime: Option<Runtime>,

    /// Channel for receiving progress updates
    progress_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<ProgressUpdate>>>>,
}

/// Progress update from background tasks
#[derive(Clone, Debug)]
pub struct ProgressUpdate {
    pub kind: ProgressKind,
    pub message: Option<String>,
    pub stage: Option<String>,
    pub elapsed_ms: Option<f64>,
}

/// Type of progress update
#[derive(Clone, Debug)]
pub enum ProgressKind {
    Started { job_id: String },
    StageBegin { stage: String },
    StageEnd { stage: String },
    Info { message: String },
    Completed { summary: String, diff: Option<String> },
    Failed { error: String },
}

impl AsyncBridge {
    /// Create a new async bridge
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("Failed to create tokio runtime");

        Self {
            runtime: Some(runtime),
            progress_rx: Arc::new(Mutex::new(None)),
        }
    }

    /// Get the runtime handle for spawning tasks
    pub fn runtime(&self) -> &Runtime {
        self.runtime.as_ref().expect("Runtime has been shut down")
    }

    /// Register a progress receiver
    pub fn register_progress_receiver(&self, rx: mpsc::UnboundedReceiver<ProgressUpdate>) {
        let mut guard = self.progress_rx.lock().unwrap();
        *guard = Some(rx);
    }

    /// Clear the progress receiver
    pub fn clear_progress_receiver(&self) {
        let mut guard = self.progress_rx.lock().unwrap();
        *guard = None;
    }

    /// Poll for progress updates and call the handler
    pub fn poll_progress<F>(&self, mut handler: F)
    where
        F: FnMut(ProgressUpdate),
    {
        let mut guard = self.progress_rx.lock().unwrap();
        if let Some(rx) = guard.as_mut() {
            while let Ok(update) = rx.try_recv() {
                handler(update);
            }
        }
    }
}

impl Default for AsyncBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AsyncBridge {
    fn drop(&mut self) {
        // Shutdown the runtime without blocking
        // This prevents the "Cannot drop a runtime in a context where blocking is not allowed" panic
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
