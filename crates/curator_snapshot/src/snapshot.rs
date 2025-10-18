use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;

use crate::config::{SNAPSHOT_SCHEMA_VERSION, Tunables};
use crate::curate::{CuratedComputation, CuratedEntry, DiscardReason, PriceSource};
use crate::error::CuratorError;

#[derive(Debug, Serialize)]
pub struct SnapshotFile {
    pub schema_version: u32,
    pub generated_at: String,
    pub metadata: SnapshotMetadata,
    pub free: Vec<SnapshotEntry>,
    pub cheap: Vec<SnapshotEntry>,
    pub unmatched: Vec<UnmatchedEntry>,
    pub discarded: Vec<DiscardedEntry>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotMetadata {
    pub thresholds: ThresholdMetadata,
    pub sources: SourceMetadata,
    pub counts: SnapshotCounts,
}

#[derive(Debug, Serialize)]
pub struct ThresholdMetadata {
    pub min_free_aaii: f32,
    pub min_paid_aaii: f32,
    pub cheap_in_max: f64,
    pub cheap_out_max: f64,
    pub min_context_length: u32,
    pub fuzzy_match_threshold: f64,
}

#[derive(Debug, Serialize)]
pub struct SourceMetadata {
    pub openrouter_models_url: String,
    pub aa_models_url: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotCounts {
    pub curated_free: usize,
    pub curated_cheap: usize,
    pub unmatched: usize,
    pub discarded: usize,
}

#[derive(Debug, Serialize)]
pub struct SnapshotEntry {
    pub slug: String,
    pub display_name: String,
    pub provider: String,
    pub aaii: f32,
    pub price_in_per_million: Option<f64>,
    pub price_out_per_million: Option<f64>,
    pub price_source: String,
    pub context_length: Option<u32>,
    pub modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aa_last_updated: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UnmatchedEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiscardedEntry {
    pub slug: String,
    pub reason: String,
}

pub fn materialize_snapshot(computation: CuratedComputation, tunables: &Tunables) -> SnapshotFile {
    let metadata = SnapshotMetadata {
        thresholds: ThresholdMetadata {
            min_free_aaii: tunables.min_free_aaii,
            min_paid_aaii: tunables.min_paid_aaii,
            cheap_in_max: tunables.cheap_in_max,
            cheap_out_max: tunables.cheap_out_max,
            min_context_length: tunables.min_context_length,
            fuzzy_match_threshold: tunables.fuzzy_match_threshold,
        },
        sources: SourceMetadata {
            openrouter_models_url: tunables.openrouter_models_url.clone(),
            aa_models_url: tunables.aa_models_url.clone(),
        },
        counts: SnapshotCounts {
            curated_free: computation.free.len(),
            curated_cheap: computation.cheap.len(),
            unmatched: computation.unmatched.len(),
            discarded: computation.discarded.len(),
        },
    };

    SnapshotFile {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        metadata,
        free: computation
            .free
            .into_iter()
            .map(SnapshotEntry::from)
            .collect(),
        cheap: computation
            .cheap
            .into_iter()
            .map(SnapshotEntry::from)
            .collect(),
        unmatched: computation
            .unmatched
            .into_iter()
            .map(|entry| UnmatchedEntry {
                name: entry.name,
                provider: entry.provider,
                slug: entry.slug,
            })
            .collect(),
        discarded: computation
            .discarded
            .into_iter()
            .map(|entry| DiscardedEntry {
                slug: entry.slug,
                reason: format_discard_reason(entry.reason),
            })
            .collect(),
    }
}

impl From<CuratedEntry> for SnapshotEntry {
    fn from(entry: CuratedEntry) -> Self {
        SnapshotEntry {
            slug: entry.slug,
            display_name: entry.display_name,
            provider: entry.provider,
            aaii: entry.aaii,
            price_in_per_million: entry.price_in_per_million,
            price_out_per_million: entry.price_out_per_million,
            price_source: match entry.price_source {
                PriceSource::Aa => "aa".to_string(),
                PriceSource::OpenRouter => "openrouter".to_string(),
            },
            context_length: entry.context_length,
            modalities: entry.modalities,
            match_strategy: entry.match_strategy,
            aa_last_updated: entry.aa_last_updated.map(|dt| dt.to_rfc3339()),
        }
    }
}

pub fn write_snapshot(path: &Path, snapshot: &SnapshotFile) -> Result<(), CuratorError> {
    let serialized = serde_json::to_string_pretty(snapshot)?;
    let temp_path = build_temp_path(path);
    fs::write(&temp_path, format!("{serialized}\n"))?;
    fs::rename(&temp_path, path)?;
    Ok(())
}

fn build_temp_path(path: &Path) -> PathBuf {
    let mut temp_path = path.to_path_buf();
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if !ext.is_empty() => {
            temp_path.set_extension(format!("{ext}.tmp"));
        }
        _ => {
            temp_path.set_extension("tmp");
        }
    }
    temp_path
}

fn format_discard_reason(reason: DiscardReason) -> String {
    match reason {
        DiscardReason::MissingAaii => "missing-aaii".to_string(),
        DiscardReason::NonTextModalities => "non-text-modalities".to_string(),
        DiscardReason::InsufficientContext {
            min_required,
            actual,
        } => format!(
            "insufficient-context:min={min_required},actual={}",
            actual
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
        DiscardReason::MissingPricing => "missing-pricing".to_string(),
        DiscardReason::AaiiThreshold { minimum, actual } => {
            format!("aaii-threshold:min={minimum:.2},actual={actual:.2}")
        }
        DiscardReason::PricingThreshold {
            max_in,
            max_out,
            actual_in,
            actual_out,
        } => format!(
            "pricing-threshold:prompt<= {max_in:.2},completion<= {max_out:.2},actual=({},{})",
            actual_in
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "n/a".to_string()),
            actual_out
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "n/a".to_string())
        ),
    }
}
