#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Error, anyhow};
use chrono::Local;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rconv_core::{
    ConvocationsConfig, FileConfig, PresetDefinition, StageProgressCallback, StageProgressEvent,
    StageProgressEventKind, load_config, resolve_outfile_paths, run_with_config_with_progress,
    runtime_preferences_to_convocations, save_presets_and_ui_only,
};
use serde::Serialize;
use tauri::async_runtime::RwLock;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager, State as TauriState};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

#[derive(Debug, Default)]
struct ApiServerState {
    base_url: RwLock<Option<String>>,
}

impl ApiServerState {
    async fn set_base_url(&self, base: String) {
        let mut guard = self.base_url.write().await;
        *guard = Some(base);
    }

    async fn get_base_url(&self) -> Option<String> {
        self.base_url.read().await.clone()
    }
}

#[derive(Clone, Default)]
struct ProcessManager {
    active: Arc<RwLock<Option<Uuid>>>,
}

impl ProcessManager {
    async fn start(&self, job_id: Uuid) -> Result<(), ()> {
        let mut guard = self.active.write().await;
        if guard.is_some() {
            return Err(());
        }
        *guard = Some(job_id);
        Ok(())
    }

    async fn finish(&self, job_id: Uuid) {
        let mut guard = self.active.write().await;
        if guard
            .as_ref()
            .map(|current| *current == job_id)
            .unwrap_or(false)
        {
            *guard = None;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ProcessEventKind {
    Queued,
    StageBegin,
    StageEnd,
    Info,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
struct ProcessEventPayload {
    job_id: String,
    kind: ProcessEventKind,
    stage: Option<String>,
    elapsed_ms: Option<f64>,
    stage_elapsed_ms: Option<f64>,
    message: Option<String>,
    error: Option<String>,
    #[serde(default = "default_origin")]
    origin: String,
}

#[allow(dead_code)]
fn default_origin() -> String {
    "backend".to_string()
}

impl ProcessEventPayload {
    fn queued(job_id: &str) -> Self {
        Self {
            job_id: job_id.to_string(),
            kind: ProcessEventKind::Queued,
            stage: None,
            elapsed_ms: None,
            stage_elapsed_ms: None,
            message: Some("Job queued".to_string()),
            error: None,
            origin: "backend".to_string(),
        }
    }

    fn from_stage(job_id: &str, event: &StageProgressEvent) -> Self {
        match event.kind {
            StageProgressEventKind::Begin => Self {
                job_id: job_id.to_string(),
                kind: ProcessEventKind::StageBegin,
                stage: event.stage.clone(),
                elapsed_ms: Some(event.elapsed_ms),
                stage_elapsed_ms: event.stage_elapsed_ms,
                message: event.message.clone(),
                error: None,
                origin: "backend".to_string(),
            },
            StageProgressEventKind::End => Self {
                job_id: job_id.to_string(),
                kind: ProcessEventKind::StageEnd,
                stage: event.stage.clone(),
                elapsed_ms: Some(event.elapsed_ms),
                stage_elapsed_ms: event.stage_elapsed_ms,
                message: event.message.clone(),
                error: None,
                origin: "backend".to_string(),
            },
            StageProgressEventKind::Note | StageProgressEventKind::Progress => Self {
                job_id: job_id.to_string(),
                kind: ProcessEventKind::Info,
                stage: event.stage.clone(),
                elapsed_ms: Some(event.elapsed_ms),
                stage_elapsed_ms: event.stage_elapsed_ms,
                message: event.message.clone(),
                error: None,
                origin: "backend".to_string(),
            },
        }
    }

    fn completed(job_id: &str) -> Self {
        Self {
            job_id: job_id.to_string(),
            kind: ProcessEventKind::Completed,
            stage: None,
            elapsed_ms: None,
            stage_elapsed_ms: None,
            message: Some("Processing completed".to_string()),
            error: None,
            origin: "backend".to_string(),
        }
    }

    fn failed(job_id: &str, error: impl Into<String>) -> Self {
        Self {
            job_id: job_id.to_string(),
            kind: ProcessEventKind::Failed,
            stage: None,
            elapsed_ms: None,
            stage_elapsed_ms: None,
            message: None,
            error: Some(error.into()),
            origin: "backend".to_string(),
        }
    }

    fn from_frontend_log(level: &str, message: String) -> Self {
        let kind = match level {
            "error" => ProcessEventKind::Failed,
            "warn" => ProcessEventKind::Info,
            _ => ProcessEventKind::Info,
        };

        let error = if level == "error" {
            Some(message.clone())
        } else {
            None
        };

        Self {
            job_id: "frontend".to_string(),
            kind,
            stage: None,
            elapsed_ms: None,
            stage_elapsed_ms: None,
            message: Some(message),
            error,
            origin: "frontend".to_string(),
        }
    }
}

impl ProcessEventKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::StageBegin => "stage-begin",
            Self::StageEnd => "stage-end",
            Self::Info => "info",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConsolePipeMode {
    InfoOnly,
    ErrorsOnly,
    All,
}

impl ConsolePipeMode {
    fn should_emit(self, kind: &ProcessEventKind) -> bool {
        match self {
            Self::InfoOnly => matches!(kind, ProcessEventKind::Info),
            Self::ErrorsOnly => matches!(kind, ProcessEventKind::Failed),
            Self::All => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConsolePipeTarget {
    Stdout,
    Stderr,
}

impl ConsolePipeTarget {
    fn write_line(self, line: &str) -> io::Result<()> {
        match self {
            Self::Stdout => {
                let mut handle = io::stdout();
                writeln!(handle, "{line}")?;
                handle.flush()
            }
            Self::Stderr => {
                let mut handle = io::stderr();
                writeln!(handle, "{line}")?;
                handle.flush()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ConsolePipe {
    target: ConsolePipeTarget,
    mode: ConsolePipeMode,
}

impl ConsolePipe {
    fn parse(spec: &str) -> Option<Self> {
        if spec.trim().is_empty() {
            return None;
        }

        let mut parts = spec.split(':');
        let target_part = parts.next()?.trim().to_ascii_lowercase();
        let target = match target_part.as_str() {
            "stdout" => ConsolePipeTarget::Stdout,
            "stderr" => ConsolePipeTarget::Stderr,
            _ => return None,
        };

        let mode = match parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => {
                let lower = value.to_ascii_lowercase();
                match lower.as_str() {
                    "all" => ConsolePipeMode::All,
                    "error" | "errors" => ConsolePipeMode::ErrorsOnly,
                    "info" => ConsolePipeMode::InfoOnly,
                    _ => return None,
                }
            }
            None => ConsolePipeMode::InfoOnly,
        };

        if parts.next().is_some() {
            return None;
        }

        Some(Self { target, mode })
    }

    fn emit(self, payload: &ProcessEventPayload) {
        if !self.mode.should_emit(&payload.kind) {
            return;
        }

        let line = format_console_line(payload);
        if let Err(err) = self.target.write_line(&line) {
            eprintln!("[Convocations] Failed to pipe progress event: {err}");
        }
    }
}

fn format_console_line(payload: &ProcessEventPayload) -> String {
    let mut parts = Vec::with_capacity(8);
    parts.push(format!("[{}]", payload.origin));
    parts.push(format!("job {}", payload.job_id));
    parts.push(format!("kind {}", payload.kind.as_str()));
    if let Some(stage) = &payload.stage {
        parts.push(format!("stage {}", stage));
    }
    if let Some(elapsed) = payload.elapsed_ms {
        parts.push(format!("t {:.3}ms", elapsed));
    }
    if let Some(stage_elapsed) = payload.stage_elapsed_ms {
        parts.push(format!("Δ {:.3}ms", stage_elapsed));
    }
    if let Some(message) = &payload.message {
        parts.push(format!("msg {}", message));
    }
    if let Some(error) = &payload.error {
        parts.push(format!("err {}", error));
    }
    parts.join(" | ")
}

fn detect_console_pipe_from_env() -> Option<ConsolePipe> {
    std::env::var("CONVOCATIONS_PROGRESS_PIPE")
        .ok()
        .and_then(|value| ConsolePipe::parse(value.trim()))
}

type ProgressEmitter = Arc<dyn Fn(ProcessEventPayload) + Send + Sync>;

#[derive(Clone)]
struct HttpContext {
    process_manager: ProcessManager,
    emitter: ProgressEmitter,
}

impl HttpContext {
    fn emit(&self, payload: ProcessEventPayload) {
        (self.emitter)(payload);
    }

    fn emit_info<S: Into<String>>(&self, job_id: &str, message: S) {
        self.emit(ProcessEventPayload {
            job_id: job_id.to_string(),
            kind: ProcessEventKind::Info,
            stage: None,
            elapsed_ms: None,
            stage_elapsed_ms: None,
            message: Some(message.into()),
            error: None,
            origin: "backend".to_string(),
        });
    }
}

fn emit_progress(app: &AppHandle, payload: ProcessEventPayload) {
    if let Err(err) = app.emit("process-progress", payload) {
        eprintln!("[Convocations] Failed to emit process event: {err}");
    }
}

#[tauri::command]
async fn get_api_base_url(state: TauriState<'_, Arc<ApiServerState>>) -> Result<String, String> {
    let state = state.inner().clone();
    let mut attempts = 0;
    loop {
        if let Some(url) = state.get_base_url().await {
            return Ok(url);
        }

        attempts += 1;
        if attempts > 40 {
            return Err("HTTP API failed to start".into());
        }

        sleep(Duration::from_millis(100)).await;
    }
}

#[tauri::command]
async fn open_file_dialog(_app: AppHandle, title: Option<String>) -> Result<Option<String>, String> {
    // Spawn a blocking task to avoid blocking the async runtime
    let result = tauri::async_runtime::spawn_blocking(move || {
        use rfd::FileDialog;

        let mut dialog = FileDialog::new();
        if let Some(title_str) = title {
            dialog = dialog.set_title(&title_str);
        }

        dialog.pick_file()
    })
    .await
    .map_err(|e| format!("Failed to open file dialog: {}", e))?;

    match result {
        Some(path) => Ok(Some(path.to_string_lossy().to_string())),
        None => Ok(None),
    }
}

#[tauri::command]
async fn get_default_chatlog_path() -> Result<String, String> {
    // Use the same default path as defined in rconv-core
    Ok("~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log".to_string())
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn get_settings_handler() -> Result<Json<SettingsResponse>, ApiError> {
    let load_result = load_config();
    if !load_result.warnings.is_empty() {
        eprintln!(
            "[Convocations] Configuration warnings: {}",
            load_result.warnings.join("; ")
        );
    }
    let config = load_result.config;
    let outfile = compute_outfile_summary_from_file_config(&config);
    Ok(Json(SettingsResponse { config, outfile }))
}

async fn save_settings_handler(Json(config): Json<FileConfig>) -> Result<StatusCode, ApiError> {
    // Only persist presets and UI preferences; runtime preferences are session-only
    save_presets_and_ui_only(&config.presets, &config.ui)
        .map_err(|err| ApiError::from(format!("{}", err)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_handler(
    Json(file_config): Json<FileConfig>,
) -> Result<Json<ValidationResult>, ApiError> {
    let (convocations_config, warnings) =
        runtime_preferences_to_convocations(&file_config.runtime, &file_config.presets);
    let validation = validate_config(&convocations_config);

    // Merge warnings from config conversion
    let mut all_warnings = warnings;
    all_warnings.extend(validation.warnings.iter().cloned());

    Ok(Json(ValidationResult {
        warnings: all_warnings,
        ..validation
    }))
}

async fn process_handler(
    State(ctx): State<HttpContext>,
    Json(file_config): Json<FileConfig>,
) -> Result<(StatusCode, Json<ProcessStartResponse>), ApiError> {
    // Convert to ConvocationsConfig and validate
    let (job_config, conversion_warnings) =
        runtime_preferences_to_convocations(&file_config.runtime, &file_config.presets);
    let validation = validate_config(&job_config);

    if !validation.valid {
        let joined = validation.errors.join("; ");
        return Err(ApiError::bad_request(format!(
            "Configuration invalid: {joined}"
        )));
    }

    // Note: We do NOT auto-save here - GUI interactions override for the session only.
    // Only explicit saves (via /api/settings) and preset CRUD operations persist config.

    let job_id = Uuid::new_v4();
    ctx.process_manager
        .start(job_id)
        .await
        .map_err(|_| ApiError::conflict("A processing job is already running"))?;

    let manager = ctx.process_manager.clone();
    let emitter_for_progress = ctx.emitter.clone();
    let emitter_for_completion = ctx.emitter.clone();
    let job_id_arc = Arc::new(job_id.to_string());
    let response_id = job_id_arc.as_ref().clone();
    let progress_id = job_id_arc.clone();
    let completion_id = job_id_arc.clone();

    ctx.emit(ProcessEventPayload::queued(job_id_arc.as_ref()));
    ctx.emit_info(job_id_arc.as_ref(), "Job accepted; awaiting scheduler");

    // Log any conversion warnings
    if !conversion_warnings.is_empty() {
        let warnings_str = conversion_warnings.join("; ");
        eprintln!(
            "[Convocations] Configuration conversion warnings: {}",
            warnings_str
        );
        ctx.emit_info(
            job_id_arc.as_ref(),
            format!("Config warnings: {}", warnings_str),
        );
    }

    tauri::async_runtime::spawn(async move {
        let progress_callback: StageProgressCallback =
            Arc::new(move |event: StageProgressEvent| {
                emitter_for_progress(ProcessEventPayload::from_stage(
                    progress_id.as_ref(),
                    &event,
                ));
            });

        let result = run_with_config_with_progress(job_config, progress_callback).await;

        match result {
            Ok(_) => emitter_for_completion(ProcessEventPayload::completed(completion_id.as_ref())),
            Err(err_msg) => {
                emitter_for_completion(ProcessEventPayload::failed(completion_id.as_ref(), err_msg))
            }
        }

        manager.finish(job_id).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ProcessStartResponse {
            job_id: response_id,
        }),
    ))
}

#[derive(Serialize)]
struct ProcessStartResponse {
    job_id: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    inner: Error,
}

impl ApiError {
    fn new(status: StatusCode, inner: Error) -> Self {
        Self { status, inner }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, anyhow!(message.into()))
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, anyhow!(message.into()))
    }
}

impl From<Error> for ApiError {
    fn from(value: Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, value)
    }
}

impl From<String> for ApiError {
    fn from(value: String) -> Self {
        Self::from(anyhow!(value))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let payload = Json(ErrorBody {
            error: self.inner.to_string(),
        });
        (self.status, payload).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Clone, Serialize)]
struct OutfileSummary {
    default: String,
    effective: String,
    overridden: bool,
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
    config: FileConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    outfile: Option<OutfileSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct ValidationResult {
    valid: bool,
    errors: Vec<String>,
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    field_errors: HashMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    field_warnings: HashMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outfile: Option<OutfileSummary>,
}

#[derive(Clone, Copy, Debug)]
enum ConfigContradiction {
    MultipleEvents,
    ConflictingDurations,
    CustomDatesWithOverrides,
}

impl ConfigContradiction {
    fn message(&self) -> &'static str {
        match self {
            Self::MultipleEvents => "Cannot specify more than one event type (RSM7, RSM8, TP6)",
            Self::ConflictingDurations => "Cannot specify both 1-hour and 2-hour durations",
            Self::CustomDatesWithOverrides => {
                "Cannot use event flags (RSM7, RSM8, TP6, 1h, 2h) with custom start/end dates"
            }
        }
    }

    fn fields(&self) -> &'static [&'static str] {
        match self {
            Self::MultipleEvents => &["rsm7", "rsm8", "tp6"],
            Self::ConflictingDurations => &["one_hour", "two_hours"],
            Self::CustomDatesWithOverrides => &[
                "start",
                "end",
                "rsm7",
                "rsm8",
                "tp6",
                "one_hour",
                "two_hours",
            ],
        }
    }
}

fn detect_contradictions(config: &ConvocationsConfig) -> Vec<ConfigContradiction> {
    let mut contradictions = Vec::new();

    let event_count = [config.rsm7, config.rsm8, config.tp6]
        .iter()
        .filter(|&&x| x)
        .count();
    if event_count > 1 {
        contradictions.push(ConfigContradiction::MultipleEvents);
    }

    if config.one_hour && config.two_hours {
        contradictions.push(ConfigContradiction::ConflictingDurations);
    }

    let has_event_or_duration_flags =
        config.rsm7 || config.rsm8 || config.tp6 || config.one_hour || config.two_hours;
    let has_custom_dates = config.start.is_some() || config.end.is_some();

    if has_event_or_duration_flags && has_custom_dates {
        contradictions.push(ConfigContradiction::CustomDatesWithOverrides);
    }

    contradictions
}

fn record_field_message(map: &mut HashMap<String, Vec<String>>, field: &str, message: &str) {
    map.entry(field.to_string())
        .or_default()
        .push(message.to_string());
}

fn record_field_messages(map: &mut HashMap<String, Vec<String>>, fields: &[&str], message: &str) {
    for field in fields {
        record_field_message(map, field, message);
    }
}

fn validate_config(config: &ConvocationsConfig) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut field_errors: HashMap<String, Vec<String>> = HashMap::new();
    let mut field_warnings: HashMap<String, Vec<String>> = HashMap::new();

    for contradiction in detect_contradictions(config) {
        let message = contradiction.message();
        record_field_messages(&mut field_errors, contradiction.fields(), message);
        errors.push(message.to_string());
    }

    if config.process_file.is_none() {
        let expanded_path = shellexpand::tilde(&config.infile).to_string();
        if !Path::new(&expanded_path).exists() {
            let message = format!("Input file does not exist: {}", config.infile);
            record_field_message(&mut field_errors, "infile", &message);
            errors.push(message);
        }
    } else if let Some(ref process_file) = config.process_file {
        let expanded_path = shellexpand::tilde(process_file).to_string();
        if !Path::new(&expanded_path).exists() {
            let message = format!("Process file does not exist: {}", process_file);
            record_field_message(&mut field_errors, "process_file", &message);
            errors.push(message);
        }
    }

    if let Some(ref outfile) = config.outfile {
        let path = Path::new(outfile);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                let message = format!("Output directory does not exist: {}", parent.display());
                record_field_message(&mut field_errors, "outfile", &message);
                errors.push(message);
            }
        }
    }

    if config.use_llm && std::env::var("OPENROUTER_API_KEY").is_err() {
        let message = "LLM corrections enabled but OPENROUTER_API_KEY environment variable not set. Processing will continue without AI corrections.";
        record_field_message(&mut field_warnings, "use_llm", message);
        warnings.push(message.to_string());
    }

    if config.keep_orig && !config.use_llm {
        let message = "keep_orig flag has no effect when LLM corrections are disabled";
        record_field_message(&mut field_warnings, "keep_orig", message);
        warnings.push(message.to_string());
    }

    if config.no_diff && !config.use_llm {
        let message = "no_diff flag has no effect when LLM corrections are disabled";
        record_field_message(&mut field_warnings, "no_diff", message);
        warnings.push(message.to_string());
    }

    ValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings,
        field_errors,
        field_warnings,
        outfile: compute_outfile_summary(config),
    }
}

fn infer_working_dir() -> Option<PathBuf> {
    if let Ok(env_dir) = std::env::var("CONVOCATIONS_WORKING_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    std::env::current_dir().ok()
}

fn compute_outfile_summary_from_file_config(file_config: &FileConfig) -> Option<OutfileSummary> {
    let (config, _) =
        runtime_preferences_to_convocations(&file_config.runtime, &file_config.presets);
    compute_outfile_summary_from_convocations_config(&config)
}

fn compute_outfile_summary(config: &ConvocationsConfig) -> Option<OutfileSummary> {
    compute_outfile_summary_from_convocations_config(config)
}

fn compute_outfile_summary_from_convocations_config(
    config: &ConvocationsConfig,
) -> Option<OutfileSummary> {
    let working_dir = infer_working_dir();
    resolve_outfile_paths(config, working_dir.as_deref(), None)
        .map(|resolution| OutfileSummary {
            default: resolution.default,
            effective: resolution.effective,
            overridden: resolution.was_overridden,
        })
        .ok()
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct CreatePresetRequest {
    preset: PresetDefinition,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct UpdatePresetRequest {
    name: String,
    preset: PresetDefinition,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct DeletePresetRequest {
    name: String,
}

async fn create_preset_handler(
    Json(request): Json<CreatePresetRequest>,
) -> Result<(StatusCode, Json<PresetDefinition>), ApiError> {
    let load_result = load_config();
    let mut config = load_result.config;

    // Validate the preset
    if request.preset.name.trim().is_empty() {
        return Err(ApiError::bad_request("Preset name cannot be empty"));
    }

    if request.preset.file_prefix.trim().is_empty() {
        return Err(ApiError::bad_request("File prefix cannot be empty"));
    }

    // Check for duplicate name
    if config.presets.iter().any(|p| p.name == request.preset.name) {
        return Err(ApiError::conflict(format!(
            "A preset with name '{}' already exists",
            request.preset.name
        )));
    }

    // Force builtin to false for user-created presets
    let mut new_preset = request.preset.clone();
    new_preset.builtin = false;

    // Add the preset
    config.presets.push(new_preset.clone());

    // Save only presets and UI preferences
    save_presets_and_ui_only(&config.presets, &config.ui)
        .map_err(|err| ApiError::from(format!("{}", err)))?;

    Ok((StatusCode::CREATED, Json(new_preset)))
}

async fn update_preset_handler(
    Json(request): Json<UpdatePresetRequest>,
) -> Result<StatusCode, ApiError> {
    let load_result = load_config();
    let mut config = load_result.config;

    // Find the preset
    let preset_index = config
        .presets
        .iter()
        .position(|p| p.name == request.name)
        .ok_or_else(|| ApiError::bad_request(format!("Preset '{}' not found", request.name)))?;

    // Block modification of built-in presets
    if config.presets[preset_index].builtin {
        return Err(ApiError::bad_request(format!(
            "Cannot modify built-in preset '{}'",
            request.name
        )));
    }

    // Validate the preset
    if request.preset.name.trim().is_empty() {
        return Err(ApiError::bad_request("Preset name cannot be empty"));
    }

    if request.preset.file_prefix.trim().is_empty() {
        return Err(ApiError::bad_request("File prefix cannot be empty"));
    }

    // Check if name is being changed and if so, check for duplicate
    if request.preset.name != request.name {
        if config.presets.iter().any(|p| p.name == request.preset.name) {
            return Err(ApiError::conflict(format!(
                "A preset with name '{}' already exists",
                request.preset.name
            )));
        }
    }

    // Update the preset (keeping builtin flag)
    let mut updated_preset = request.preset.clone();
    updated_preset.builtin = false; // User presets are never builtin

    config.presets[preset_index] = updated_preset;

    // Save only presets and UI preferences
    save_presets_and_ui_only(&config.presets, &config.ui)
        .map_err(|err| ApiError::from(format!("{}", err)))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_preset_handler(
    Json(request): Json<DeletePresetRequest>,
) -> Result<StatusCode, ApiError> {
    let load_result = load_config();
    let mut config = load_result.config;

    // Find the preset
    let preset_index = config
        .presets
        .iter()
        .position(|p| p.name == request.name)
        .ok_or_else(|| ApiError::bad_request(format!("Preset '{}' not found", request.name)))?;

    // Block deletion of built-in presets
    if config.presets[preset_index].builtin {
        return Err(ApiError::bad_request(format!(
            "Cannot delete built-in preset '{}'",
            request.name
        )));
    }

    // Remove the preset
    config.presets.remove(preset_index);

    // Save only presets and UI preferences
    // Note: If the deleted preset was active, sanitize_config will reset it on next load
    save_presets_and_ui_only(&config.presets, &config.ui)
        .map_err(|err| ApiError::from(format!("{}", err)))?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, Serialize)]
struct PkcePairResponse {
    verifier: String,
    challenge: String,
}

async fn generate_pkce_handler() -> Json<PkcePairResponse> {
    let (verifier, challenge) = rconv_core::openrouter::generate_pkce_pair();
    Json(PkcePairResponse {
        verifier,
        challenge,
    })
}

#[derive(Debug, Clone, serde::Deserialize)]
struct OAuthUrlRequest {
    code_challenge: String,
    redirect_uri: String,
}

#[derive(Debug, Clone, Serialize)]
struct OAuthUrlResponse {
    url: String,
}

async fn build_oauth_url_handler(Json(request): Json<OAuthUrlRequest>) -> Json<OAuthUrlResponse> {
    let url =
        rconv_core::openrouter::build_oauth_url(&request.code_challenge, &request.redirect_uri);
    Json(OAuthUrlResponse { url })
}

#[derive(Debug, Clone, Serialize)]
struct ModelsResponse {
    models: Vec<rconv_core::openrouter::ModelInfo>,
}

async fn fetch_models_handler() -> Result<Json<ModelsResponse>, ApiError> {
    let models = rconv_core::openrouter::fetch_models()
        .await
        .map_err(|e| ApiError::from(format!("Failed to fetch models: {}", e)))?;
    Ok(Json(ModelsResponse { models }))
}

#[derive(Debug, Clone, serde::Deserialize)]
struct FilterModelsRequest {
    free_only: bool,
}

async fn recommended_models_handler(
    Json(request): Json<FilterModelsRequest>,
) -> Json<Vec<(String, String)>> {
    let models = rconv_core::openrouter::get_recommended_models(request.free_only);
    Json(models)
}

#[derive(Debug, Clone, serde::Deserialize)]
struct CalculateDatesRequest {
    rsm7: bool,
    rsm8: bool,
    tp6: bool,
    one_hour: bool,
    two_hours: bool,
    last: u32,
}

#[derive(Debug, Clone, Serialize)]
struct CalculateDatesResponse {
    start: String,
    end: String,
}

async fn calculate_dates_handler(
    Json(request): Json<CalculateDatesRequest>,
) -> Result<Json<CalculateDatesResponse>, ApiError> {
    use rconv_core::runtime::calculate_event_dates;

    let event_type = if request.rsm7 {
        "rsm7"
    } else if request.rsm8 {
        "rsm8"
    } else if request.tp6 {
        "tp6"
    } else {
        "saturday"
    };

    let duration_minutes = if request.rsm7 || request.rsm8 || request.tp6 {
        if request.one_hour {
            60
        } else if request.two_hours {
            120
        } else {
            60 // default for non-Saturday events
        }
    } else {
        // Saturday
        if request.one_hour {
            60
        } else if request.two_hours {
            120
        } else {
            145 // default for Saturday (2h 25m)
        }
    };

    let today = Local::now().date_naive();
    let (start, end) = calculate_event_dates(today, request.last, event_type, duration_minutes)
        .map_err(|e| ApiError::from(format!("Failed to calculate dates: {}", e)))?;

    Ok(Json(CalculateDatesResponse { start, end }))
}

async fn launch_http_server(
    shared_state: Arc<ApiServerState>,
    app_handle: AppHandle,
) -> Result<(), Error> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;

    shared_state.set_base_url(format!("http://{}", addr)).await;

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<ProcessEventPayload>();
    let console_pipe = detect_console_pipe_from_env();
    if let Some(pipe) = console_pipe {
        eprintln!(
            "[Convocations] Progress console piping enabled ({:?}, {:?})",
            pipe.target, pipe.mode
        );
    }

    let app_handle_for_events = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let pipe = console_pipe;
        while let Some(payload) = progress_rx.recv().await {
            if let Some(console) = pipe {
                console.emit(&payload);
            }
            emit_progress(&app_handle_for_events, payload);
        }
    });

    let emitter: ProgressEmitter = Arc::new(move |payload: ProcessEventPayload| {
        if let Err(err) = progress_tx.send(payload) {
            eprintln!("[Convocations] Dropped progress event: {err}");
        }
    });

    let context = HttpContext {
        process_manager: ProcessManager::default(),
        emitter,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let router = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/settings", get(get_settings_handler))
        .route("/api/settings", post(save_settings_handler))
        .route("/api/validate", post(validate_handler))
        .route("/api/process", post(process_handler))
        .route("/api/presets/create", post(create_preset_handler))
        .route("/api/presets/update", post(update_preset_handler))
        .route("/api/presets/delete", post(delete_preset_handler))
        .route("/api/openrouter/pkce", get(generate_pkce_handler))
        .route("/api/openrouter/oauth-url", post(build_oauth_url_handler))
        .route("/api/openrouter/models", get(fetch_models_handler))
        .route(
            "/api/openrouter/recommended",
            post(recommended_models_handler),
        )
        .route("/api/calculate-dates", post(calculate_dates_handler))
        .with_state(context)
        .layer(cors);

    axum::serve(listener, router).await?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(Arc::new(ApiServerState::default()))
        .setup(|app| {
            // Set up tray icon
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show_item = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            // Load tray icon explicitly for reliable dev mode support
            // Use PNG for all platforms as it's universally supported
            let icon_bytes: &[u8] = include_bytes!("../icons/icon.png");
            let tray_icon = tauri::image::Image::from_bytes(icon_bytes)?;

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Set up frontend log listener
            let app_handle_for_logs = app.handle().clone();
            app.listen("frontend-log", move |event| {
                #[derive(serde::Deserialize)]
                #[allow(dead_code)]
                struct FrontendLog {
                    origin: String,
                    level: String,
                    message: String,
                    timestamp: String,
                }

                if let Ok(log) = serde_json::from_str::<FrontendLog>(event.payload()) {
                    let payload = ProcessEventPayload::from_frontend_log(&log.level, log.message);
                    emit_progress(&app_handle_for_logs, payload);
                }
            });

            // Launch HTTP server
            let shared = app.state::<Arc<ApiServerState>>().inner().clone();
            let task_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = launch_http_server(shared, task_handle).await {
                    eprintln!("[Convocations] HTTP server error: {err}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_api_base_url,
            open_file_dialog,
            get_default_chatlog_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::extract::State;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio::time::{Duration, Instant, timeout};

    #[tokio::test]
    async fn process_handler_emits_info_and_completion() {
        let (tx, mut rx) = mpsc::unbounded_channel::<ProcessEventPayload>();
        let emitter: ProgressEmitter = {
            let tx = tx.clone();
            Arc::new(move |payload: ProcessEventPayload| {
                let _ = tx.send(payload);
            })
        };
        drop(tx);

        let ctx = HttpContext {
            process_manager: ProcessManager::default(),
            emitter,
        };

        let infile_path = std::env::current_dir().expect("cwd").join("Cargo.toml");
        let mut file_config = FileConfig::default();
        file_config.runtime.dry_run = true;
        file_config.runtime.use_ai_corrections = false;
        file_config.runtime.chat_log_path = infile_path.to_string_lossy().into_owned();

        let (status, Json(response)) = process_handler(State(ctx.clone()), Json(file_config))
            .await
            .expect("process should start");

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(response.job_id.len(), 36);

        let mut saw_info = false;
        let mut saw_stage_begin = false;
        let mut saw_completed = false;
        let deadline = Instant::now() + Duration::from_secs(5);

        while Instant::now() < deadline {
            let event = timeout(Duration::from_millis(200), rx.recv())
                .await
                .ok()
                .and_then(|e| e);

            let Some(event) = event else {
                continue;
            };

            match event.kind {
                ProcessEventKind::Info => {
                    if let Some(message) = event.message {
                        if message.contains("Output") {
                            saw_info = true;
                        }
                    }
                }
                ProcessEventKind::StageBegin => {
                    saw_stage_begin = true;
                }
                ProcessEventKind::Completed => {
                    saw_completed = true;
                    break;
                }
                ProcessEventKind::Failed => {
                    panic!("processing failed unexpectedly: {:?}", event.error);
                }
                _ => {}
            }
        }

        assert!(
            saw_stage_begin,
            "expected at least one stage-begin event to be emitted"
        );
        assert!(
            saw_info,
            "expected at least one info event containing the output path"
        );
        assert!(saw_completed, "expected job to complete successfully");
    }

    #[test]
    fn console_pipe_parse_variants() {
        let stdout_default = ConsolePipe::parse("stdout").expect("stdout pipe");
        assert_eq!(stdout_default.target, ConsolePipeTarget::Stdout);
        assert_eq!(stdout_default.mode, ConsolePipeMode::InfoOnly);

        let stderr_all = ConsolePipe::parse("stderr:all").expect("stderr all pipe");
        assert_eq!(stderr_all.target, ConsolePipeTarget::Stderr);
        assert_eq!(stderr_all.mode, ConsolePipeMode::All);

        let stdout_errors = ConsolePipe::parse("stdout:errors").expect("stdout errors pipe");
        assert_eq!(stdout_errors.mode, ConsolePipeMode::ErrorsOnly);

        assert!(
            ConsolePipe::parse("").is_none(),
            "empty string should be rejected"
        );
        assert!(
            ConsolePipe::parse("stdout:oops").is_none(),
            "unknown mode should be rejected"
        );
        assert!(
            ConsolePipe::parse("stdout:info:extra").is_none(),
            "extra segments should be rejected"
        );
    }
}
