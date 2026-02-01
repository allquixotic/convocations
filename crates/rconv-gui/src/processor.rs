//! Processor integration for running rconv-core processing tasks

use crate::async_bridge::{AsyncBridge, ProgressKind, ProgressUpdate};
use crate::state::AppState;
use rconv_core::{
    runtime_preferences_to_convocations, run_with_config_with_progress,
    StageProgressEvent, StageProgressEventKind,
};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Start a processing job
pub fn start_processing(
    bridge: &AsyncBridge,
    state: &AppState,
) -> Result<mpsc::UnboundedReceiver<ProgressUpdate>, String> {
    // Convert FileConfig to ConvocationsConfig
    let (runtime_config, warnings) =
        runtime_preferences_to_convocations(&state.config.runtime, &state.config.presets);

    // Log warnings
    for warning in warnings {
        eprintln!("Warning: {}", warning);
    }

    // Create progress channel
    let (tx, rx) = mpsc::unbounded_channel();

    // Generate job ID
    let job_id = format!("job-{}", chrono::Local::now().timestamp());

    // Send started event
    let _ = tx.send(ProgressUpdate {
        kind: ProgressKind::Started {
            job_id: job_id.clone(),
        },
        message: Some("Processing started".to_string()),
        stage: None,
        elapsed_ms: None,
    });

    // Clone for async task
    let tx_clone = tx.clone();

    // Spawn processing task
    bridge.runtime().spawn(async move {
        // Track diff for final completion event
        let diff_content = Arc::new(std::sync::Mutex::new(None));
        let diff_clone = diff_content.clone();

        // Create progress callback
        let progress_callback = Arc::new(move |event: StageProgressEvent| {
            let update = match event.kind {
                StageProgressEventKind::Begin => ProgressUpdate {
                    kind: ProgressKind::StageBegin {
                        stage: event.stage.clone().unwrap_or_default(),
                    },
                    message: event.message.clone().or_else(|| {
                        event.stage.as_ref().map(|s| format!("Starting: {}", s))
                    }),
                    stage: event.stage.clone(),
                    elapsed_ms: Some(event.elapsed_ms),
                },
                StageProgressEventKind::End => ProgressUpdate {
                    kind: ProgressKind::StageEnd {
                        stage: event.stage.clone().unwrap_or_default(),
                    },
                    message: event.message.clone().or_else(|| {
                        event.stage.as_ref().map(|s| format!("Completed: {}", s))
                    }),
                    stage: event.stage.clone(),
                    elapsed_ms: Some(event.elapsed_ms),
                },
                StageProgressEventKind::Note | StageProgressEventKind::Progress => ProgressUpdate {
                    kind: ProgressKind::Info {
                        message: event.message.clone().unwrap_or_default(),
                    },
                    message: event.message.clone(),
                    stage: event.stage.clone(),
                    elapsed_ms: Some(event.elapsed_ms),
                },
                StageProgressEventKind::Diff => {
                    // Store diff for final completion event
                    if let Some(ref diff) = event.diff {
                        *diff_clone.lock().unwrap() = Some(diff.clone());
                    }
                    ProgressUpdate {
                        kind: ProgressKind::Info {
                            message: "Generated diff".to_string(),
                        },
                        message: Some("Generated diff".to_string()),
                        stage: None,
                        elapsed_ms: Some(event.elapsed_ms),
                    }
                }
            };

            let _ = tx_clone.send(update);
        }) as Arc<dyn Fn(StageProgressEvent) + Send + Sync + 'static>;

        // Run processing
        match run_with_config_with_progress(runtime_config, progress_callback).await {
            Ok(()) => {
                let diff = diff_content.lock().unwrap().clone();
                let _ = tx.send(ProgressUpdate {
                    kind: ProgressKind::Completed {
                        summary: "Processing completed successfully".to_string(),
                        diff,
                    },
                    message: Some("Processing completed successfully".to_string()),
                    stage: None,
                    elapsed_ms: None,
                });
            }
            Err(e) => {
                let error_msg = format!("Processing failed: {}", e);
                let _ = tx.send(ProgressUpdate {
                    kind: ProgressKind::Failed {
                        error: error_msg.clone(),
                    },
                    message: Some(error_msg),
                    stage: None,
                    elapsed_ms: None,
                });
            }
        }
    });

    Ok(rx)
}
