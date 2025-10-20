#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::openrouter;
use crate::openrouter::ModelInfo;

const SNAPSHOT_SCHEMA_VERSION: u32 = 2;
const SNAPSHOT_ENV: &str = "CONVOCATIONS_MODEL_SNAPSHOT";
const REMOTE_SNAPSHOT_URL: &str = "https://raw.githubusercontent.com/allquixotic/convocations/refs/heads/main/static/model_snapshot.json";
#[cfg(not(test))]
const REMOTE_FETCH_TIMEOUT_SECS: u64 = 5;
#[cfg(not(test))]
const REMOTE_USER_AGENT: &str = concat!("rconv-core/", env!("CARGO_PKG_VERSION"));
const EMBEDDED_SNAPSHOT: &str = include_str!("../../../static/model_snapshot.json");

#[cfg(test)]
type CatalogMock = Arc<dyn Fn() -> Result<CuratedCatalog, CuratorError> + Send + Sync + 'static>;
#[cfg(test)]
thread_local! {
    static LOAD_CATALOG_MOCK: RefCell<Option<CatalogMock>> = RefCell::new(None);
}

#[cfg(test)]
type FetchModelsMock =
    Arc<dyn Fn() -> Result<Vec<ModelInfo>, openrouter::OpenRouterError> + Send + Sync + 'static>;
#[cfg(test)]
thread_local! {
    static FETCH_MODELS_MOCK: RefCell<Option<FetchModelsMock>> = RefCell::new(None);
}

#[cfg(test)]
thread_local! {
    static REMOTE_SNAPSHOT_MOCK: RefCell<Option<String>> = RefCell::new(None);
}

#[cfg(test)]
fn load_catalog_mock() -> Option<CatalogMock> {
    LOAD_CATALOG_MOCK.with(|slot| slot.borrow().as_ref().map(Arc::clone))
}

#[cfg(test)]
pub(super) fn set_load_catalog_mock<F>(mock: F)
where
    F: Fn() -> Result<CuratedCatalog, CuratorError> + Send + Sync + 'static,
{
    LOAD_CATALOG_MOCK.with(|slot| {
        *slot.borrow_mut() = Some(Arc::new(mock));
    });
}

#[cfg(test)]
pub(super) fn reset_load_catalog_mock() {
    LOAD_CATALOG_MOCK.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

#[cfg(test)]
fn fetch_models_mock() -> Option<FetchModelsMock> {
    FETCH_MODELS_MOCK.with(|slot| slot.borrow().as_ref().map(Arc::clone))
}

#[cfg(test)]
pub(super) fn set_fetch_models_mock<F>(mock: F)
where
    F: Fn() -> Result<Vec<ModelInfo>, openrouter::OpenRouterError> + Send + Sync + 'static,
{
    FETCH_MODELS_MOCK.with(|slot| {
        *slot.borrow_mut() = Some(Arc::new(mock));
    });
}

#[cfg(test)]
pub(super) fn reset_fetch_models_mock() {
    FETCH_MODELS_MOCK.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

#[cfg(test)]
pub(super) fn set_remote_snapshot_mock(snapshot: Option<String>) {
    REMOTE_SNAPSHOT_MOCK.with(|slot| {
        *slot.borrow_mut() = snapshot;
    });
}

#[cfg(test)]
pub(super) fn reset_remote_snapshot_mock() {
    REMOTE_SNAPSHOT_MOCK.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// Preference encoded in configuration/CLI for curated model selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPreference {
    Auto,
    Explicit(String),
}

impl ModelPreference {
    pub fn from_str(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("auto") {
            ModelPreference::Auto
        } else {
            ModelPreference::Explicit(value.trim().to_string())
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            ModelPreference::Auto => AUTO_SENTINEL,
            ModelPreference::Explicit(value) => value.as_str(),
        }
    }
}

pub const AUTO_SENTINEL: &str = "auto";

#[derive(Debug, Error)]
pub enum CuratorError {
    #[error("failed to read snapshot: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported snapshot schema version {found}; expected {expected}")]
    SchemaVersion { expected: u32, found: u32 },
    #[error("snapshot missing required fields: {0}")]
    InvalidSnapshot(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct CuratedModelSummary {
    pub slug: String,
    pub display_name: String,
    pub provider: String,
    pub tier: CuratedTier,
    pub aaii: f32,
    pub price_in_per_million: Option<f64>,
    pub price_out_per_million: Option<f64>,
    pub context_length: Option<u32>,
    pub price_source: PriceSource,
    pub match_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter_created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cheapest_endpoint: Option<CheapestEndpoint>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CuratedTier {
    Free,
    Cheap,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PriceSource {
    Aa,
    Openrouter,
}

#[derive(Debug, Clone)]
pub struct CuratedEntry {
    pub slug: String,
    pub display_name: String,
    pub provider: String,
    pub aaii: f32,
    pub price_in_per_million: Option<f64>,
    pub price_out_per_million: Option<f64>,
    pub price_source: PriceSource,
    pub context_length: Option<u32>,
    pub modalities: Vec<String>,
    pub match_strategy: Option<String>,
    pub aa_last_updated: Option<DateTime<Utc>>,
    pub tier: CuratedTier,
    pub openrouter_created_at: Option<DateTime<Utc>>,
    pub cheapest_endpoint: Option<CheapestEndpoint>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CheapestEndpoint {
    pub name: String,
    pub provider: String,
    pub prompt_price_per_million: Option<f64>,
    pub completion_price_per_million: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CuratedCatalog {
    pub free: Vec<CuratedEntry>,
    pub cheap: Vec<CuratedEntry>,
    pub generated_at: DateTime<Utc>,
    pub metadata: CatalogMetadata,
    pub source: CatalogSource,
}

impl CuratedCatalog {
    fn reconcile(self, live_models: &[ModelInfo]) -> Self {
        let mut permitted = HashSet::new();
        for model in live_models {
            permitted.insert(model.id.as_str());
        }

        let filter = |entry: &CuratedEntry| permitted.contains(entry.slug.as_str());

        let free = self.free.into_iter().filter(filter).collect::<Vec<_>>();
        let cheap = self.cheap.into_iter().filter(filter).collect::<Vec<_>>();

        CuratedCatalog {
            free,
            cheap,
            ..self
        }
    }

    pub fn summaries(&self) -> Vec<CuratedModelSummary> {
        let mut summaries = Vec::new();
        for entry in &self.free {
            summaries.push(entry.to_summary());
        }
        for entry in &self.cheap {
            summaries.push(entry.to_summary());
        }
        summaries
    }

    fn find(&self, slug: &str) -> Option<CuratedEntry> {
        self.free
            .iter()
            .chain(self.cheap.iter())
            .find(|entry| entry.slug.eq_ignore_ascii_case(slug))
            .cloned()
    }
}

impl CuratedEntry {
    fn to_summary(&self) -> CuratedModelSummary {
        CuratedModelSummary {
            slug: self.slug.clone(),
            display_name: self.display_name.clone(),
            provider: self.provider.clone(),
            tier: self.tier,
            aaii: self.aaii,
            price_in_per_million: self.price_in_per_million,
            price_out_per_million: self.price_out_per_million,
            context_length: self.context_length,
            price_source: self.price_source,
            match_strategy: self.match_strategy.clone(),
            openrouter_created_at: self.openrouter_created_at,
            cheapest_endpoint: self.cheapest_endpoint.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CatalogMetadata {
    pub thresholds: ThresholdMetadata,
    pub sources: SourceMetadata,
}

#[derive(Debug, Clone)]
pub struct ThresholdMetadata {
    pub min_free_aaii: f32,
    pub min_paid_aaii: f32,
    pub cheap_in_max: f64,
    pub cheap_out_max: f64,
    pub min_context_length: u32,
    pub fuzzy_match_threshold: f64,
}

#[derive(Debug, Clone)]
pub struct SourceMetadata {
    pub openrouter_models_url: String,
    pub aa_models_url: String,
}

#[derive(Debug, Clone)]
pub enum CatalogSource {
    Remote(&'static str),
    File(PathBuf),
    Embedded,
}

#[derive(Debug, Clone)]
pub struct CuratedResolution {
    pub model_slug: String,
    pub entry: Option<CuratedEntry>,
    pub source: ResolutionSource,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    CuratedAuto,
    CuratedExplicit,
    FallbackNoSnapshot,
    FallbackEmpty,
    FallbackMissingEntry,
}

impl CuratedResolution {
    fn fallback(reason: ResolutionSource, details: impl Into<String>) -> Self {
        CuratedResolution {
            model_slug: crate::config::DEFAULT_OPENROUTER_MODEL.to_string(),
            entry: None,
            source: reason,
            message: details.into(),
        }
    }
}

pub fn load_catalog() -> Result<CuratedCatalog, CuratorError> {
    #[cfg(test)]
    if let Some(mock) = load_catalog_mock() {
        return mock();
    }

    if let Some(raw) = fetch_remote_snapshot() {
        match parse_snapshot(&raw) {
            Ok(snapshot) => {
                match convert_snapshot(snapshot, CatalogSource::Remote(REMOTE_SNAPSHOT_URL)) {
                    Ok(catalog) => return Ok(catalog),
                    Err(err) => eprintln!(
                        "[curator] remote snapshot at {} invalid: {}",
                        REMOTE_SNAPSHOT_URL, err
                    ),
                }
            }
            Err(err) => eprintln!(
                "[curator] failed to parse remote snapshot at {}: {}",
                REMOTE_SNAPSHOT_URL, err
            ),
        }
    }

    for path in candidate_paths() {
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(raw) => match parse_snapshot(&raw) {
                    Ok(snapshot) => {
                        return convert_snapshot(snapshot, CatalogSource::File(path.clone()));
                    }
                    Err(err) => {
                        eprintln!(
                            "[curator] failed to parse snapshot at {}: {}",
                            path.display(),
                            err
                        );
                        continue;
                    }
                },
                Err(err) => {
                    eprintln!(
                        "[curator] unable to read snapshot at {}: {}",
                        path.display(),
                        err
                    );
                }
            }
        }
    }

    let snapshot = parse_snapshot(EMBEDDED_SNAPSHOT)?;
    convert_snapshot(snapshot, CatalogSource::Embedded)
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = env::var(SNAPSHOT_ENV) {
        candidates.push(PathBuf::from(path));
    }

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("static/model_snapshot.json"));
    }

    candidates
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../static/model_snapshot.json"));

    candidates
}

#[cfg(not(test))]
fn fetch_remote_snapshot() -> Option<String> {
    std::thread::spawn(fetch_remote_snapshot_thread)
        .join()
        .ok()
        .flatten()
}

#[cfg(not(test))]
fn fetch_remote_snapshot_thread() -> Option<String> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(REMOTE_FETCH_TIMEOUT_SECS))
        .user_agent(REMOTE_USER_AGENT)
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            eprintln!(
                "[curator] failed to build HTTP client for snapshot fetch: {}",
                err
            );
            return None;
        }
    };

    let response = match client.get(REMOTE_SNAPSHOT_URL).send() {
        Ok(response) => response,
        Err(err) => {
            eprintln!(
                "[curator] failed to fetch remote snapshot from {}: {}",
                REMOTE_SNAPSHOT_URL, err
            );
            return None;
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "[curator] remote snapshot request to {} returned status {}",
            REMOTE_SNAPSHOT_URL,
            response.status()
        );
        return None;
    }

    match response.text() {
        Ok(body) => {
            if body.trim().is_empty() {
                eprintln!(
                    "[curator] remote snapshot at {} returned an empty body",
                    REMOTE_SNAPSHOT_URL
                );
                None
            } else {
                Some(body)
            }
        }
        Err(err) => {
            eprintln!(
                "[curator] failed to read remote snapshot body from {}: {}",
                REMOTE_SNAPSHOT_URL, err
            );
            None
        }
    }
}

#[cfg(test)]
fn fetch_remote_snapshot() -> Option<String> {
    REMOTE_SNAPSHOT_MOCK.with(|slot| slot.borrow().clone())
}

async fn fetch_live_models() -> Result<Vec<ModelInfo>, openrouter::OpenRouterError> {
    #[cfg(test)]
    if let Some(mock) = fetch_models_mock() {
        return mock();
    }

    openrouter::fetch_models().await
}

pub async fn resolve_preference(
    preference: &ModelPreference,
    free_only: bool,
    _openrouter_api_key: Option<&str>,
) -> CuratedResolution {
    let catalog = match load_catalog() {
        Ok(catalog) => catalog,
        Err(err) => {
            return CuratedResolution::fallback(
                ResolutionSource::FallbackNoSnapshot,
                format!("no snapshot available ({err})"),
            );
        }
    };

    let live_models = match fetch_live_models().await {
        Ok(models) => Some(models),
        Err(err) => {
            eprintln!("[curator] failed to fetch live OpenRouter models: {}", err);
            None
        }
    };

    let reconciled = if let Some(models) = live_models {
        catalog.reconcile(&models)
    } else {
        catalog
    };

    let selected = match preference {
        ModelPreference::Explicit(slug) => reconciled.find(slug),
        ModelPreference::Auto => select_auto(&reconciled, free_only),
    };

    match selected {
        Some(entry) => {
            let source = match preference {
                ModelPreference::Auto => ResolutionSource::CuratedAuto,
                ModelPreference::Explicit(_) => ResolutionSource::CuratedExplicit,
            };
            CuratedResolution {
                model_slug: entry.slug.clone(),
                entry: Some(entry),
                source,
                message: String::new(),
            }
        }
        None => {
            let reason = match preference {
                ModelPreference::Auto => ResolutionSource::FallbackEmpty,
                ModelPreference::Explicit(_) => ResolutionSource::FallbackMissingEntry,
            };
            CuratedResolution::fallback(reason, "no curated entry matched the request")
        }
    }
}

fn select_auto(catalog: &CuratedCatalog, free_only: bool) -> Option<CuratedEntry> {
    if free_only {
        catalog
            .free
            .first()
            .cloned()
            .or_else(|| catalog.cheap.first().cloned())
    } else {
        catalog
            .cheap
            .first()
            .cloned()
            .or_else(|| catalog.free.first().cloned())
    }
}

fn parse_snapshot(raw: &str) -> Result<SnapshotFile, CuratorError> {
    let snapshot: SnapshotFile = serde_json::from_str(raw)?;
    if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(CuratorError::SchemaVersion {
            expected: SNAPSHOT_SCHEMA_VERSION,
            found: snapshot.schema_version,
        });
    }
    Ok(snapshot)
}

fn convert_snapshot(
    snapshot: SnapshotFile,
    source: CatalogSource,
) -> Result<CuratedCatalog, CuratorError> {
    let generated_at = snapshot
        .generated_at
        .parse::<DateTime<Utc>>()
        .map_err(|err| CuratorError::InvalidSnapshot(format!("invalid generated_at: {err}")))?;

    let metadata = CatalogMetadata {
        thresholds: ThresholdMetadata {
            min_free_aaii: snapshot.metadata.thresholds.min_free_aaii,
            min_paid_aaii: snapshot.metadata.thresholds.min_paid_aaii,
            cheap_in_max: snapshot.metadata.thresholds.cheap_in_max,
            cheap_out_max: snapshot.metadata.thresholds.cheap_out_max,
            min_context_length: snapshot.metadata.thresholds.min_context_length,
            fuzzy_match_threshold: snapshot.metadata.thresholds.fuzzy_match_threshold,
        },
        sources: SourceMetadata {
            openrouter_models_url: snapshot.metadata.sources.openrouter_models_url,
            aa_models_url: snapshot.metadata.sources.aa_models_url,
        },
    };

    let mut free_entries = Vec::new();
    for entry in snapshot.free {
        free_entries.push(convert_entry(entry, CuratedTier::Free)?);
    }

    let mut cheap_entries = Vec::new();
    for entry in snapshot.cheap {
        cheap_entries.push(convert_entry(entry, CuratedTier::Cheap)?);
    }

    Ok(CuratedCatalog {
        free: free_entries,
        cheap: cheap_entries,
        generated_at,
        metadata,
        source,
    })
}

fn convert_entry(entry: SnapshotEntry, tier: CuratedTier) -> Result<CuratedEntry, CuratorError> {
    let SnapshotEntry {
        slug,
        display_name,
        provider,
        aaii,
        price_in_per_million,
        price_out_per_million,
        price_source,
        context_length,
        modalities,
        match_strategy,
        aa_last_updated,
        openrouter_created_at,
        cheapest_endpoint,
    } = entry;

    if slug.trim().is_empty() {
        return Err(CuratorError::InvalidSnapshot(
            "curated entry missing slug".to_string(),
        ));
    }

    let price_source = match price_source {
        SnapshotPriceSource::Aa => PriceSource::Aa,
        SnapshotPriceSource::Openrouter => PriceSource::Openrouter,
    };

    let aa_last_updated = match aa_last_updated.as_deref() {
        Some(value) => Some(value.parse::<DateTime<Utc>>().map_err(|err| {
            CuratorError::InvalidSnapshot(format!("invalid aa_last_updated: {err}"))
        })?),
        None => None,
    };

    let openrouter_created_at = match openrouter_created_at.as_deref() {
        Some(value) => Some(value.parse::<DateTime<Utc>>().map_err(|err| {
            CuratorError::InvalidSnapshot(format!("invalid openrouter_created_at: {err}"))
        })?),
        None => None,
    };

    let cheapest_endpoint = cheapest_endpoint.map(|endpoint| CheapestEndpoint {
        name: endpoint.name,
        provider: endpoint.provider,
        prompt_price_per_million: endpoint.prompt_price_per_million,
        completion_price_per_million: endpoint.completion_price_per_million,
    });

    let curated_entry = CuratedEntry {
        slug,
        display_name,
        provider,
        aaii,
        price_in_per_million,
        price_out_per_million,
        price_source,
        context_length,
        modalities,
        match_strategy,
        aa_last_updated,
        tier,
        openrouter_created_at,
        cheapest_endpoint,
    };

    Ok(curated_entry)
}

#[derive(Debug, Deserialize)]
struct SnapshotFile {
    schema_version: u32,
    generated_at: String,
    metadata: SnapshotMetadata,
    free: Vec<SnapshotEntry>,
    cheap: Vec<SnapshotEntry>,
}

#[derive(Debug, Deserialize)]
struct SnapshotMetadata {
    thresholds: SnapshotThresholds,
    sources: SnapshotSources,
}

#[derive(Debug, Deserialize)]
struct SnapshotThresholds {
    min_free_aaii: f32,
    min_paid_aaii: f32,
    cheap_in_max: f64,
    cheap_out_max: f64,
    min_context_length: u32,
    fuzzy_match_threshold: f64,
}

#[derive(Debug, Deserialize)]
struct SnapshotSources {
    openrouter_models_url: String,
    aa_models_url: String,
}

#[derive(Debug, Deserialize)]
struct SnapshotEntry {
    slug: String,
    display_name: String,
    provider: String,
    aaii: f32,
    price_in_per_million: Option<f64>,
    price_out_per_million: Option<f64>,
    price_source: SnapshotPriceSource,
    context_length: Option<u32>,
    modalities: Vec<String>,
    #[serde(default)]
    match_strategy: Option<String>,
    #[serde(default)]
    aa_last_updated: Option<String>,
    #[serde(default)]
    openrouter_created_at: Option<String>,
    #[serde(default)]
    cheapest_endpoint: Option<SnapshotEndpoint>,
}

#[derive(Debug, Deserialize)]
struct SnapshotEndpoint {
    name: String,
    provider: String,
    #[serde(default)]
    prompt_price_per_million: Option<f64>,
    #[serde(default)]
    completion_price_per_million: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SnapshotPriceSource {
    Aa,
    Openrouter,
}

pub fn catalog_summaries() -> Result<Vec<CuratedModelSummary>, CuratorError> {
    let catalog = load_catalog()?;
    Ok(catalog.summaries())
}

pub fn catalog_for_testing(raw: &str) -> Result<CuratedCatalog, CuratorError> {
    let snapshot = parse_snapshot(raw)?;
    convert_snapshot(snapshot, CatalogSource::Embedded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::Mutex;

    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    struct MockGuard;

    impl Drop for MockGuard {
        fn drop(&mut self) {
            reset_load_catalog_mock();
            reset_fetch_models_mock();
            reset_remote_snapshot_mock();
        }
    }

    const SAMPLE_SNAPSHOT: &str = r#"{
      "schema_version": 2,
      "generated_at": "2025-01-01T00:00:00Z",
      "metadata": {
        "thresholds": {
          "min_free_aaii": 60.0,
          "min_paid_aaii": 65.0,
          "cheap_in_max": 1.5,
          "cheap_out_max": 6.0,
          "min_context_length": 8192,
          "fuzzy_match_threshold": 0.94
        },
        "sources": {
          "openrouter_models_url": "https://example.com/models",
          "aa_models_url": "https://example.com/aa"
        }
      },
      "free": [
        {
          "slug": "provider/pro-free",
          "display_name": "Free Model",
          "provider": "provider",
          "aaii": 72.4,
          "price_in_per_million": 0.0,
          "price_out_per_million": 0.0,
          "price_source": "aa",
          "context_length": 8192,
          "modalities": ["text"],
          "match_strategy": "provided-slug",
          "aa_last_updated": "2025-01-01T00:00:00Z",
          "openrouter_created_at": "2024-12-30T00:00:00Z",
          "cheapest_endpoint": null
        }
      ],
      "cheap": [
        {
          "slug": "provider/pro-cheap",
          "display_name": "Cheap Model",
          "provider": "provider",
          "aaii": 68.1,
          "price_in_per_million": 0.75,
          "price_out_per_million": 3.0,
          "price_source": "openrouter",
          "context_length": 16000,
          "modalities": ["text"],
          "match_strategy": "alias:cheap",
          "aa_last_updated": null,
          "openrouter_created_at": "2024-12-29T00:00:00Z",
          "cheapest_endpoint": {
            "name": "Provider | pro-cheap",
            "provider": "Provider",
            "prompt_price_per_million": 0.75,
            "completion_price_per_million": 3.0
          }
        }
      ]
    }"#;

    #[test]
    fn parses_sample_snapshot() {
        let catalog = catalog_for_testing(SAMPLE_SNAPSHOT).expect("catalog");
        assert_eq!(catalog.free.len(), 1);
        assert_eq!(catalog.cheap.len(), 1);

        let summaries = catalog.summaries();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].tier, CuratedTier::Free);
        assert_eq!(summaries[1].tier, CuratedTier::Cheap);
    }

    #[test]
    fn find_returns_matching_entry() {
        let catalog = catalog_for_testing(SAMPLE_SNAPSHOT).expect("catalog");
        let entry = catalog.find("provider/pro-cheap").expect("entry");
        assert_eq!(entry.display_name, "Cheap Model");
    }

    #[test]
    fn load_catalog_prefers_remote_snapshot_when_available() {
        let _test_guard = TEST_MUTEX.lock().unwrap();
        let _mock_guard = MockGuard;

        set_remote_snapshot_mock(Some(SAMPLE_SNAPSHOT.to_string()));

        let catalog = load_catalog().expect("catalog");
        match catalog.source {
            CatalogSource::Remote(url) => assert_eq!(url, REMOTE_SNAPSHOT_URL),
            other => panic!("expected remote catalog source, got {:?}", other),
        }
        assert_eq!(catalog.free.len(), 1);
        assert_eq!(catalog.cheap.len(), 1);
    }

    #[test]
    fn load_catalog_falls_back_when_remote_invalid() {
        let _test_guard = TEST_MUTEX.lock().unwrap();
        let _mock_guard = MockGuard;

        set_remote_snapshot_mock(Some("{\"not\":\"valid\"}".to_string()));

        let catalog = load_catalog().expect("catalog");
        assert!(
            matches!(catalog.source, CatalogSource::File(_)),
            "expected file source fallback, got {:?}",
            catalog.source
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_preference_falls_back_when_snapshot_missing() {
        let _test_guard = TEST_MUTEX.lock().unwrap();
        let _mock_guard = MockGuard;

        set_load_catalog_mock(|| {
            Err(CuratorError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                "missing snapshot",
            )))
        });
        set_fetch_models_mock(|| panic!("fetch_models should not be called without a snapshot"));

        let result = resolve_preference(&ModelPreference::Auto, false, None).await;

        assert_eq!(result.source, ResolutionSource::FallbackNoSnapshot);
        assert!(result.entry.is_none());
        assert_eq!(result.model_slug, crate::config::DEFAULT_OPENROUTER_MODEL);
        assert!(
            result.message.contains("no snapshot available"),
            "unexpected message: {}",
            result.message
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_preference_handles_fetch_failure() {
        let _test_guard = TEST_MUTEX.lock().unwrap();
        let _mock_guard = MockGuard;

        set_load_catalog_mock(|| catalog_for_testing(SAMPLE_SNAPSHOT));
        set_fetch_models_mock(|| Err(openrouter::OpenRouterError::from("network failure")));

        let result = resolve_preference(&ModelPreference::Auto, false, None).await;

        assert_eq!(result.source, ResolutionSource::CuratedAuto);
        let entry = result.entry.expect("expected curated entry");
        assert_eq!(entry.slug, "provider/pro-cheap");
        assert_eq!(result.model_slug, entry.slug);
        assert!(result.message.is_empty());
    }
}
