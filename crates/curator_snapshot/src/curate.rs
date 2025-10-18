use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::alias::{AliasResolver, MatchResult, MatchStrategy};
use crate::config::Tunables;
use crate::fetch::{AaModel, OpenRouterModel};

const CHEAP_PROVIDER_ORDER: [&str; 4] = ["openai", "x-ai", "google", "anthropic"];
const CHEAP_PROVIDER_PREFS: [(&str, &[&str]); 4] = [
    ("openai", &["openai/gpt-5-mini", "openai/o3"]),
    ("x-ai", &["x-ai/grok-4-fast", "x-ai/grok-4"]),
    ("google", &["google/gemini-2.5-flash"]),
    (
        "anthropic",
        &["anthropic/claude-haiku-4.5", "anthropic/claude-3.5-haiku"],
    ),
];
const SCORE_EPSILON: f64 = 1e-6;
const PRICE_EPSILON: f64 = 1e-9;
const AAII_EPSILON: f32 = 1e-3;

#[derive(Debug, Clone, Copy)]
struct FreeSeriesSpec {
    key: &'static str,
    slug_terms: &'static [&'static str],
    prefer_instruct: bool,
}

const FREE_SERIES_SPECS: [FreeSeriesSpec; 5] = [
    FreeSeriesSpec {
        key: "meta-llama",
        slug_terms: &["meta-llama"],
        prefer_instruct: false,
    },
    FreeSeriesSpec {
        key: "deepseek",
        slug_terms: &["deepseek"],
        prefer_instruct: false,
    },
    FreeSeriesSpec {
        key: "qwen",
        slug_terms: &["qwen"],
        prefer_instruct: true,
    },
    FreeSeriesSpec {
        key: "kimi",
        slug_terms: &["moonshotai/kimi", "kimi"],
        prefer_instruct: false,
    },
    FreeSeriesSpec {
        key: "zai-glm",
        slug_terms: &["z-ai/glm", "glm"],
        prefer_instruct: false,
    },
];

const FREE_TARGET_BASE: usize = 3;
const FREE_TARGET_COUNT: usize = FREE_TARGET_BASE + FREE_SERIES_SPECS.len();

impl FreeSeriesSpec {
    fn matches_slug(&self, slug: &str) -> bool {
        if self.slug_terms.is_empty() {
            return false;
        }
        contains_any(slug, self.slug_terms)
    }

    fn matches_openrouter(&self, model: &OpenRouterModel) -> bool {
        self.matches_slug(&model.slug) || contains_any(&model.name, self.slug_terms)
    }
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceSource {
    Aa,
    OpenRouter,
}

#[derive(Debug, Clone)]
pub struct CuratedComputation {
    pub free: Vec<CuratedEntry>,
    pub cheap: Vec<CuratedEntry>,
    pub unmatched: Vec<UnmatchedModel>,
    pub discarded: Vec<DiscardedModel>,
}

#[derive(Debug, Clone)]
pub struct UnmatchedModel {
    pub name: String,
    pub provider: Option<String>,
    pub slug: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscardedModel {
    pub slug: String,
    pub reason: DiscardReason,
}

#[derive(Debug, Clone)]
pub enum DiscardReason {
    MissingAaii,
    NonTextModalities,
    InsufficientContext {
        min_required: u32,
        actual: Option<u32>,
    },
    MissingPricing,
    AaiiThreshold {
        minimum: f32,
        actual: f32,
    },
    PricingThreshold {
        max_in: f64,
        max_out: f64,
        actual_in: Option<f64>,
        actual_out: Option<f64>,
    },
}

pub fn curate_models<'a>(
    aliases: HashMap<String, String>,
    openrouter: &'a [OpenRouterModel],
    aa_models: &'a [AaModel],
    tunables: &Tunables,
) -> CuratedComputation {
    let resolver = AliasResolver::new(aliases, openrouter, tunables.fuzzy_match_threshold);
    let mut free = Vec::new();
    let mut cheap = Vec::new();
    let mut unmatched = Vec::new();
    let mut discard_log = Vec::new();
    let mut free_aaii_rejects = Vec::new();
    let mut cheap_aaii_rejects = Vec::new();
    let mut cheap_price_rejects = Vec::new();
    let mut free_low_rejects = Vec::new();
    let mut cheap_low_rejects = Vec::new();
    let free_fallback_min = (tunables.min_free_aaii - 32.0).max(25.0);
    let cheap_fallback_min = (tunables.min_paid_aaii - 8.0).max(50.0);

    for aa in aa_models {
        let match_result = resolver.resolve(aa);
        let slug = match &match_result.slug {
            Some(slug) => slug.clone(),
            None => {
                unmatched.push(UnmatchedModel {
                    name: aa.name.clone(),
                    provider: aa.provider_slug.clone(),
                    slug: aa.raw_slug.clone(),
                });
                eprintln!(
                    "[curator] unmatched AA entry name=\"{}\" provider={:?} slug={:?}",
                    aa.name, aa.provider_slug, aa.raw_slug
                );
                continue;
            }
        };

        let Some(openrouter_model) = resolver.get_model(&slug) else {
            unmatched.push(UnmatchedModel {
                name: aa.name.clone(),
                provider: aa.provider_slug.clone(),
                slug: Some(slug.clone()),
            });
            eprintln!(
                "[curator] matched slug {} missing in OpenRouter dataset",
                slug
            );
            continue;
        };

        if !is_text_model(&aa.modalities) {
            discard_log.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::NonTextModalities,
            });
            continue;
        }

        let Some(aaii) = aa.aaii else {
            discard_log.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::MissingAaii,
            });
            continue;
        };

        if !aaii.is_finite() {
            discard_log.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::MissingAaii,
            });
            continue;
        }

        let aa_context = aa.context_length;
        let or_context = openrouter_model.context_length;
        let context_length = aa_context.or(or_context);
        if let Some(context_length) = context_length {
            if context_length < tunables.min_context_length {
                discard_log.push(DiscardedModel {
                    slug: slug.clone(),
                    reason: DiscardReason::InsufficientContext {
                        min_required: tunables.min_context_length,
                        actual: Some(context_length),
                    },
                });
                continue;
            }
        } else {
            discard_log.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::InsufficientContext {
                    min_required: tunables.min_context_length,
                    actual: None,
                },
            });
            continue;
        }

        let (price_in, price_out, price_source) = resolve_pricing(aa, openrouter_model);
        let openrouter_free = openrouter_free_status(openrouter_model);

        if matches!(price_source, PriceSource::OpenRouter)
            && price_in.is_none()
            && price_out.is_none()
        {
            discard_log.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::MissingPricing,
            });
            continue;
        }

        let entry = CuratedEntry {
            slug: slug.clone(),
            display_name: openrouter_model.name.clone(),
            provider: provider_from_slug(&slug),
            aaii,
            price_in_per_million: price_in,
            price_out_per_million: price_out,
            price_source,
            context_length,
            modalities: aa.modalities.clone(),
            match_strategy: match_strategy_label(&match_result),
            aa_last_updated: aa.last_updated,
        };

        if matches!(openrouter_free, Some(true)) {
            if aaii < tunables.min_free_aaii {
                let reason = DiscardReason::AaiiThreshold {
                    minimum: tunables.min_free_aaii,
                    actual: aaii,
                };
                if aaii >= free_fallback_min {
                    free_aaii_rejects.push((entry, reason));
                } else {
                    free_low_rejects.push((entry, reason));
                }
                continue;
            }
            free.push(entry);
        } else {
            if aaii < tunables.min_paid_aaii {
                let reason = DiscardReason::AaiiThreshold {
                    minimum: tunables.min_paid_aaii,
                    actual: aaii,
                };
                if aaii >= cheap_fallback_min {
                    cheap_aaii_rejects.push((entry, reason));
                } else {
                    cheap_low_rejects.push((entry, reason));
                }
                continue;
            }

            if !meets_pricing_thresholds(price_in, price_out, tunables) {
                let reason = DiscardReason::PricingThreshold {
                    max_in: tunables.cheap_in_max,
                    max_out: tunables.cheap_out_max,
                    actual_in: price_in,
                    actual_out: price_out,
                };
                if aaii >= cheap_fallback_min {
                    cheap_price_rejects.push((entry, reason));
                } else {
                    cheap_low_rejects.push((entry, reason));
                }
                continue;
            }

            cheap.push(entry);
        }
    }

    promote_candidates(&mut free, &mut free_aaii_rejects, FREE_TARGET_COUNT);
    promote_candidates(&mut free, &mut free_low_rejects, FREE_TARGET_COUNT);
    let cheap_target = CHEAP_PROVIDER_ORDER.len();
    promote_candidates(&mut cheap, &mut cheap_aaii_rejects, cheap_target);
    promote_candidates(&mut cheap, &mut cheap_price_rejects, cheap_target);
    promote_candidates(&mut cheap, &mut cheap_low_rejects, cheap_target);

    apply_series_fallbacks(&mut free, openrouter);
    finalize_free(&mut free);
    let cheap = finalize_cheap(
        cheap,
        &CHEAP_PROVIDER_ORDER,
        &mut cheap_aaii_rejects,
        &mut cheap_price_rejects,
        &mut cheap_low_rejects,
    );

    let mut discarded = discard_log;
    discarded.extend(
        free_aaii_rejects
            .into_iter()
            .chain(cheap_aaii_rejects.into_iter())
            .chain(cheap_price_rejects.into_iter())
            .chain(free_low_rejects.into_iter())
            .chain(cheap_low_rejects.into_iter())
            .map(|(entry, reason)| DiscardedModel {
                slug: entry.slug,
                reason,
            }),
    );

    CuratedComputation {
        free,
        cheap,
        unmatched,
        discarded,
    }
}

fn apply_series_fallbacks(free: &mut Vec<CuratedEntry>, openrouter: &[OpenRouterModel]) {
    for spec in FREE_SERIES_SPECS.iter().copied() {
        if free.iter().any(|entry| spec.matches_slug(&entry.slug)) {
            continue;
        }

        let Some(candidate) = select_latest_free_series_model(spec, openrouter) else {
            continue;
        };

        let entry = CuratedEntry {
            slug: candidate.slug.clone(),
            display_name: candidate.name.clone(),
            provider: provider_from_slug(&candidate.slug),
            aaii: 0.0,
            price_in_per_million: sanitize_price(candidate.prompt_price_per_million),
            price_out_per_million: sanitize_price(candidate.completion_price_per_million),
            price_source: PriceSource::OpenRouter,
            context_length: candidate.context_length,
            modalities: Vec::new(),
            match_strategy: Some(format!("manual-series:{}", spec.key)),
            aa_last_updated: candidate.created_at,
        };

        free.push(entry);
    }
}

fn select_latest_free_series_model<'a>(
    spec: FreeSeriesSpec,
    openrouter: &'a [OpenRouterModel],
) -> Option<&'a OpenRouterModel> {
    let mut candidates: Vec<&OpenRouterModel> = openrouter
        .iter()
        .filter(|model| spec.matches_openrouter(model))
        .filter(|model| openrouter_free_status(model) == Some(true))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    if spec.prefer_instruct {
        let instruct: Vec<&OpenRouterModel> = candidates
            .iter()
            .copied()
            .filter(|model| contains_instruct(&model.slug) || contains_instruct(&model.name))
            .collect();
        if !instruct.is_empty() {
            candidates = instruct;
        }
    }

    candidates
        .into_iter()
        .max_by(|left, right| compare_by_created_then_slug(left, right))
}

fn contains_instruct(value: &str) -> bool {
    contains_any(value, &["instruct"])
}

fn compare_by_created_then_slug(left: &OpenRouterModel, right: &OpenRouterModel) -> Ordering {
    match (left.created_at, right.created_at) {
        (Some(left_dt), Some(right_dt)) => match left_dt.cmp(&right_dt) {
            Ordering::Equal => left.slug.cmp(&right.slug),
            other => other,
        },
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => left.slug.cmp(&right.slug),
    }
}

fn promote_candidates(
    target: &mut Vec<CuratedEntry>,
    source: &mut Vec<(CuratedEntry, DiscardReason)>,
    limit: usize,
) {
    if target.len() >= limit || source.is_empty() {
        return;
    }

    source.sort_by(|a, b| b.0.aaii.partial_cmp(&a.0.aaii).unwrap_or(Ordering::Equal));

    let mut retained = Vec::new();
    for (entry, reason) in source.drain(..) {
        if target.len() < limit && !target.iter().any(|existing| existing.slug == entry.slug) {
            target.push(entry);
        } else {
            retained.push((entry, reason));
        }
    }

    *source = retained;
}

fn finalize_free(entries: &mut Vec<CuratedEntry>) {
    entries.sort_by(
        |a, b| match b.aaii.partial_cmp(&a.aaii).unwrap_or(Ordering::Equal) {
            Ordering::Equal => a.slug.cmp(&b.slug),
            other => other,
        },
    );
    if entries.len() > FREE_TARGET_COUNT {
        entries.truncate(FREE_TARGET_COUNT);
    }
}

fn finalize_cheap(
    mut accepted: Vec<CuratedEntry>,
    provider_order: &[&str],
    fallback_aaii: &mut Vec<(CuratedEntry, DiscardReason)>,
    fallback_price: &mut Vec<(CuratedEntry, DiscardReason)>,
    fallback_low: &mut Vec<(CuratedEntry, DiscardReason)>,
) -> Vec<CuratedEntry> {
    let mut winners = Vec::with_capacity(provider_order.len());

    for provider in provider_order {
        if let Some(entry) = extract_preferred_candidate(
            provider,
            &mut accepted,
            fallback_aaii,
            fallback_price,
            fallback_low,
        ) {
            winners.push(entry);
            continue;
        }

        if let Some(entry) = select_best_by_provider(provider, &mut accepted) {
            winners.push(entry);
        }
    }

    for provider in provider_order {
        if winners.iter().any(|entry| &entry.provider == provider) {
            continue;
        }

        if let Some(entry) = extract_best_from_pool(provider, fallback_aaii) {
            winners.push(entry);
            continue;
        }

        if let Some(entry) = extract_best_from_pool(provider, fallback_price) {
            winners.push(entry);
            continue;
        }

        if let Some(entry) = extract_best_from_pool(provider, fallback_low) {
            winners.push(entry);
            continue;
        }
    }

    if winners.len() < provider_order.len() {
        accepted.sort_by(compare_candidate);
        for entry in accepted.into_iter() {
            if winners
                .iter()
                .any(|existing| existing.provider == entry.provider)
            {
                continue;
            }
            winners.push(entry);
            if winners.len() >= provider_order.len() {
                break;
            }
        }
    }

    winners.sort_by(|a, b| {
        let idx_a = provider_order_index(&a.provider, provider_order);
        let idx_b = provider_order_index(&b.provider, provider_order);
        match idx_a.cmp(&idx_b) {
            Ordering::Equal => compare_candidate(a, b),
            other => other,
        }
    });

    winners
}

fn extract_preferred_candidate(
    provider: &str,
    accepted: &mut Vec<CuratedEntry>,
    fallback_aaii: &mut Vec<(CuratedEntry, DiscardReason)>,
    fallback_price: &mut Vec<(CuratedEntry, DiscardReason)>,
    fallback_low: &mut Vec<(CuratedEntry, DiscardReason)>,
) -> Option<CuratedEntry> {
    let Some(slugs) = preferred_slugs(provider) else {
        return None;
    };

    for slug in slugs {
        if let Some(entry) = extract_by_slug(accepted, slug) {
            return Some(entry);
        }
        if let Some(entry) = extract_by_slug_from_pool(fallback_aaii, slug) {
            return Some(entry);
        }
        if let Some(entry) = extract_by_slug_from_pool(fallback_price, slug) {
            return Some(entry);
        }
        if let Some(entry) = extract_by_slug_from_pool(fallback_low, slug) {
            return Some(entry);
        }
    }

    None
}

fn preferred_slugs(provider: &str) -> Option<&'static [&'static str]> {
    CHEAP_PROVIDER_PREFS
        .iter()
        .find(|(candidate, _)| *candidate == provider)
        .map(|(_, slugs)| *slugs)
}

fn extract_by_slug(entries: &mut Vec<CuratedEntry>, slug: &str) -> Option<CuratedEntry> {
    entries
        .iter()
        .position(|entry| entry.slug == slug)
        .map(|index| entries.swap_remove(index))
}

fn extract_by_slug_from_pool(
    pool: &mut Vec<(CuratedEntry, DiscardReason)>,
    slug: &str,
) -> Option<CuratedEntry> {
    pool.iter()
        .position(|(entry, _)| entry.slug == slug)
        .map(|index| pool.swap_remove(index).0)
}

fn select_best_by_provider(
    provider: &str,
    entries: &mut Vec<CuratedEntry>,
) -> Option<CuratedEntry> {
    let mut best_idx: Option<usize> = None;
    let mut best_score = f64::MIN;
    let mut best_aaii = f32::MIN;
    let mut best_price = f64::MAX;
    let mut best_slug = String::new();

    for (idx, entry) in entries.iter().enumerate() {
        if entry.provider != provider {
            continue;
        }

        let score = price_efficiency(entry);
        let price = effective_price(entry).unwrap_or(f64::MAX);

        let better = match best_idx {
            None => true,
            Some(_) => {
                if score > best_score + SCORE_EPSILON {
                    true
                } else if score + SCORE_EPSILON < best_score {
                    false
                } else if entry.aaii > best_aaii + AAII_EPSILON {
                    true
                } else if entry.aaii + AAII_EPSILON < best_aaii {
                    false
                } else if price < best_price - PRICE_EPSILON {
                    true
                } else if price > best_price + PRICE_EPSILON {
                    false
                } else {
                    entry.slug < best_slug
                }
            }
        };

        if better {
            best_idx = Some(idx);
            best_score = score;
            best_aaii = entry.aaii;
            best_price = price;
            best_slug = entry.slug.clone();
        }
    }

    best_idx.map(|idx| entries.swap_remove(idx))
}

fn extract_best_from_pool(
    provider: &str,
    pool: &mut Vec<(CuratedEntry, DiscardReason)>,
) -> Option<CuratedEntry> {
    let mut best_idx: Option<usize> = None;
    let mut best_score = f64::MIN;
    let mut best_aaii = f32::MIN;
    let mut best_price = f64::MAX;
    let mut best_slug = String::new();

    for (idx, (entry, _)) in pool.iter().enumerate() {
        if entry.provider != provider {
            continue;
        }

        let score = price_efficiency(entry);
        let price = effective_price(entry).unwrap_or(f64::MAX);

        let better = match best_idx {
            None => true,
            Some(_) => {
                if score > best_score + SCORE_EPSILON {
                    true
                } else if score + SCORE_EPSILON < best_score {
                    false
                } else if entry.aaii > best_aaii + AAII_EPSILON {
                    true
                } else if entry.aaii + AAII_EPSILON < best_aaii {
                    false
                } else if price < best_price - PRICE_EPSILON {
                    true
                } else if price > best_price + PRICE_EPSILON {
                    false
                } else {
                    entry.slug < best_slug
                }
            }
        };

        if better {
            best_idx = Some(idx);
            best_score = score;
            best_aaii = entry.aaii;
            best_price = price;
            best_slug = entry.slug.clone();
        }
    }

    best_idx.map(|idx| pool.swap_remove(idx).0)
}

fn compare_candidate(a: &CuratedEntry, b: &CuratedEntry) -> Ordering {
    let score_a = price_efficiency(a);
    let score_b = price_efficiency(b);

    if (score_a - score_b).abs() > SCORE_EPSILON {
        return if score_a > score_b {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    match b.aaii.partial_cmp(&a.aaii).unwrap_or(Ordering::Equal) {
        Ordering::Equal => {}
        other => return other,
    }

    let price_a = effective_price(a).unwrap_or(f64::MAX);
    let price_b = effective_price(b).unwrap_or(f64::MAX);
    if (price_a - price_b).abs() > PRICE_EPSILON {
        return if price_a < price_b {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    a.slug.cmp(&b.slug)
}

fn price_efficiency(entry: &CuratedEntry) -> f64 {
    let mut values = Vec::new();
    if let Some(value) = entry.price_in_per_million {
        values.push(value);
    }
    if let Some(value) = entry.price_out_per_million {
        values.push(value);
    }

    if values.is_empty() {
        return f64::NEG_INFINITY;
    }

    let cost = values.iter().sum::<f64>() / values.len() as f64;
    if cost <= 0.0 {
        return f64::INFINITY;
    }

    f64::from(entry.aaii) / cost
}

fn effective_price(entry: &CuratedEntry) -> Option<f64> {
    match (entry.price_in_per_million, entry.price_out_per_million) {
        (Some(input), Some(output)) => Some((input + output) / 2.0),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    }
}

fn provider_order_index(provider: &str, order: &[&str]) -> usize {
    order
        .iter()
        .position(|candidate| candidate == &provider)
        .unwrap_or(order.len())
}

fn is_text_model(modalities: &[String]) -> bool {
    if modalities.is_empty() {
        return true;
    }
    modalities.iter().any(|modality| {
        let lower = modality.to_ascii_lowercase();
        lower.contains("text") || lower.contains("chat")
    })
}

fn resolve_pricing(
    aa: &AaModel,
    openrouter: &OpenRouterModel,
) -> (Option<f64>, Option<f64>, PriceSource) {
    let prompt = sanitize_price(openrouter.prompt_price_per_million);
    let completion = sanitize_price(openrouter.completion_price_per_million);

    if prompt.is_some() || completion.is_some() {
        return (prompt, completion, PriceSource::OpenRouter);
    }

    let aa_in = sanitize_price(aa.price_in_per_million);
    let aa_out = sanitize_price(aa.price_out_per_million);

    (aa_in, aa_out, PriceSource::Aa)
}

fn sanitize_price(price: Option<f64>) -> Option<f64> {
    price.and_then(|value| {
        if value.is_finite() && value >= 0.0 {
            Some(value)
        } else {
            None
        }
    })
}

fn openrouter_free_status(model: &OpenRouterModel) -> Option<bool> {
    let prompt = sanitize_price(model.prompt_price_per_million);
    let completion = sanitize_price(model.completion_price_per_million);

    if prompt.is_none() && completion.is_none() {
        return None;
    }

    Some(is_zero_price(prompt) && is_zero_price(completion))
}

fn meets_pricing_thresholds(
    price_in: Option<f64>,
    price_out: Option<f64>,
    tunables: &Tunables,
) -> bool {
    match (price_in, price_out) {
        (Some(in_price), Some(out_price)) => {
            in_price <= tunables.cheap_in_max && out_price <= tunables.cheap_out_max
        }
        (Some(in_price), None) => in_price <= tunables.cheap_in_max,
        (None, Some(out_price)) => out_price <= tunables.cheap_out_max,
        (None, None) => false,
    }
}

fn is_zero_price(value: Option<f64>) -> bool {
    value.map(|v| v <= PRICE_EPSILON).unwrap_or(true)
}

fn provider_from_slug(slug: &str) -> String {
    slug.split('/').next().unwrap_or("unknown").to_string()
}

fn match_strategy_label(result: &MatchResult) -> Option<String> {
    match &result.strategy {
        Some(MatchStrategy::ProvidedSlug) => Some("provided-slug".to_string()),
        Some(MatchStrategy::Alias { alias_key }) => Some(format!("alias:{}", alias_key)),
        Some(MatchStrategy::Derived { source }) => Some(format!("derived:{}", source)),
        Some(MatchStrategy::Fuzzy { candidate_name }) => Some(format!("fuzzy:{}", candidate_name)),
        None => None,
    }
}

fn contains_any(haystack: &str, terms: &[&str]) -> bool {
    if terms.is_empty() {
        return false;
    }
    let lower = haystack.to_ascii_lowercase();
    terms.iter().any(|term| lower.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_meets_pricing_thresholds_requires_values() {
        let mut tunables = sample_tunables();
        tunables.cheap_in_max = 2.0;
        tunables.cheap_out_max = 4.0;

        assert!(!meets_pricing_thresholds(None, None, &tunables));
        assert!(meets_pricing_thresholds(Some(1.5), Some(3.5), &tunables));
        assert!(!meets_pricing_thresholds(Some(2.5), Some(3.5), &tunables));
        assert!(!meets_pricing_thresholds(Some(1.5), Some(4.5), &tunables));
    }

    #[test]
    fn test_openrouter_free_status_requires_pricing() {
        let mut model = OpenRouterModel {
            slug: "provider/free-model".to_string(),
            name: "Free Model".to_string(),
            created_at: None,
            context_length: Some(16_384),
            prompt_price_per_million: Some(0.0),
            completion_price_per_million: Some(0.0),
        };

        assert_eq!(openrouter_free_status(&model), Some(true));

        model.prompt_price_per_million = Some(0.0001);
        assert_eq!(openrouter_free_status(&model), Some(false));

        model.prompt_price_per_million = None;
        model.completion_price_per_million = None;
        assert_eq!(openrouter_free_status(&model), None);
    }

    #[test]
    fn cheap_list_prioritizes_target_providers() {
        let mut tunables = sample_tunables();
        tunables.min_paid_aaii = 40.0;
        tunables.cheap_in_max = 2.0;
        tunables.cheap_out_max = 6.0;

        let candidate_data = vec![
            (
                "openai/gpt-5-mini",
                "OpenAI GPT-5 Mini",
                "openai",
                72.0,
                Some(0.25),
                Some(2.0),
            ),
            (
                "openai/other",
                "OpenAI Other",
                "openai",
                70.0,
                Some(1.6),
                Some(6.5),
            ),
            (
                "x-ai/grok-4-fast",
                "Grok 4 Fast",
                "x-ai",
                55.0,
                Some(0.2),
                Some(0.5),
            ),
            (
                "google/gemini-2.5-flash",
                "Gemini 2.5 Flash",
                "google",
                60.0,
                Some(0.3),
                Some(2.5),
            ),
            (
                "anthropic/claude-haiku-4.5",
                "Claude Haiku 4.5",
                "anthropic",
                58.0,
                Some(0.5),
                Some(1.5),
            ),
            (
                "mistralai/mistral-small",
                "Mistral Small",
                "mistralai",
                62.0,
                Some(0.4),
                Some(1.2),
            ),
        ];

        let openrouter = candidate_data
            .iter()
            .map(|(slug, name, _, _, price_in, price_out)| OpenRouterModel {
                slug: slug.to_string(),
                name: name.to_string(),
                created_at: None,
                context_length: Some(32_768),
                prompt_price_per_million: *price_in,
                completion_price_per_million: *price_out,
            })
            .collect::<Vec<_>>();

        let aa_models = candidate_data
            .iter()
            .map(|(slug, name, provider, aaii, price_in, price_out)| {
                build_aa_model(
                    name,
                    Some(provider),
                    Some(slug),
                    *aaii,
                    *price_in,
                    *price_out,
                    Some(32_768),
                )
            })
            .collect::<Vec<_>>();

        let computation = curate_models(
            std::collections::HashMap::<String, String>::new(),
            &openrouter,
            &aa_models,
            &tunables,
        );

        assert_eq!(computation.cheap.len(), CHEAP_PROVIDER_ORDER.len());
        for provider in CHEAP_PROVIDER_ORDER {
            assert!(
                computation
                    .cheap
                    .iter()
                    .any(|entry| entry.provider == provider),
                "expected cheap list to include provider {}",
                provider
            );
        }

        let expected_slugs = vec![
            "openai/gpt-5-mini",
            "x-ai/grok-4-fast",
            "google/gemini-2.5-flash",
            "anthropic/claude-haiku-4.5",
        ];

        for (entry, expected_slug) in computation.cheap.iter().zip(expected_slugs.iter()) {
            assert_eq!(
                entry.slug, *expected_slug,
                "expected {} to be selected for provider {}",
                expected_slug, entry.provider
            );
        }
    }

    #[test]
    fn pricing_falls_back_to_openrouter_when_aa_missing() {
        let tunables = sample_tunables();
        let openrouter = vec![OpenRouterModel {
            slug: "provider/test-model".to_string(),
            name: "Test Model".to_string(),
            created_at: None,
            context_length: Some(16_384),
            prompt_price_per_million: Some(1.0),
            completion_price_per_million: Some(5.0),
        }];
        let aa_models = vec![build_aa_model(
            "Test Model",
            Some("provider"),
            Some("provider/test-model"),
            80.0,
            None,
            None,
            Some(16_384),
        )];

        let computation = curate_models(
            std::collections::HashMap::<String, String>::new(),
            &openrouter,
            &aa_models,
            &tunables,
        );
        assert_eq!(computation.free.len(), 0);
        assert_eq!(computation.cheap.len(), 1);
        assert!(matches!(
            computation.cheap[0].price_source,
            PriceSource::OpenRouter
        ));
        assert_eq!(computation.cheap[0].price_in_per_million, Some(1.0));
        assert_eq!(computation.cheap[0].price_out_per_million, Some(5.0));
    }

    #[test]
    fn free_bucket_sorted_and_limited() {
        let mut tunables = sample_tunables();
        tunables.min_free_aaii = 50.0;
        let openrouter = (0..5)
            .map(|index| OpenRouterModel {
                slug: format!("provider/free-{index}"),
                name: format!("Free Model {index}"),
                created_at: None,
                context_length: Some(10_000),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            })
            .collect::<Vec<_>>();

        let aa_models = (0..5)
            .map(|index| {
                let aaii = 50.0 + (index as f32) * 5.0;
                build_aa_model(
                    &format!("Free Model {index}"),
                    Some("provider"),
                    Some(&format!("provider/free-{index}")),
                    aaii,
                    Some(0.0),
                    Some(0.0),
                    Some(10_000),
                )
            })
            .collect::<Vec<_>>();

        let computation = curate_models(
            std::collections::HashMap::<String, String>::new(),
            &openrouter,
            &aa_models,
            &tunables,
        );
        assert_eq!(computation.free.len(), std::cmp::min(FREE_TARGET_COUNT, 5));
        for index in 0..(computation.free.len() - 1) {
            assert!(
                computation.free[index].aaii >= computation.free[index + 1].aaii,
                "free bucket not sorted by AAII descending"
            );
        }
        assert_eq!(computation.free[0].slug, "provider/free-4");
        assert_eq!(computation.free[1].slug, "provider/free-3");
        assert_eq!(computation.free[2].slug, "provider/free-2");
    }

    #[test]
    fn series_fallback_adds_latest_free_model_when_no_scores() {
        let tunables = sample_tunables();
        let openrouter = vec![
            OpenRouterModel {
                slug: "meta-llama/llama-3:free".to_string(),
                name: "Meta Llama 3".to_string(),
                created_at: Some(
                    Utc.timestamp_opt(1_700_000_000, 0)
                        .single()
                        .expect("valid timestamp"),
                ),
                context_length: Some(128_000),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
            OpenRouterModel {
                slug: "meta-llama/llama-4:free".to_string(),
                name: "Meta Llama 4".to_string(),
                created_at: Some(
                    Utc.timestamp_opt(1_800_000_000, 0)
                        .single()
                        .expect("valid timestamp"),
                ),
                context_length: Some(256_000),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
        ];

        let computation = curate_models(
            std::collections::HashMap::<String, String>::new(),
            &openrouter,
            &[],
            &tunables,
        );

        assert!(
            computation
                .free
                .iter()
                .any(|entry| entry.slug == "meta-llama/llama-4:free"),
            "expected latest Meta Llama free model to be included"
        );
    }

    #[test]
    fn series_fallback_prefers_instruct_variants() {
        let tunables = sample_tunables();
        let openrouter = vec![
            OpenRouterModel {
                slug: "qwen/qwen4:free".to_string(),
                name: "Qwen 4 Free".to_string(),
                created_at: Some(
                    Utc.timestamp_opt(1_900_000_000, 0)
                        .single()
                        .expect("valid timestamp"),
                ),
                context_length: Some(128_000),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
            OpenRouterModel {
                slug: "qwen/qwen3-instruct:free".to_string(),
                name: "Qwen 3 Instruct Free".to_string(),
                created_at: Some(
                    Utc.timestamp_opt(1_850_000_000, 0)
                        .single()
                        .expect("valid timestamp"),
                ),
                context_length: Some(128_000),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
        ];

        let computation = curate_models(
            std::collections::HashMap::<String, String>::new(),
            &openrouter,
            &[],
            &tunables,
        );

        assert!(
            computation
                .free
                .iter()
                .any(|entry| entry.slug == "qwen/qwen3-instruct:free"),
            "expected instruct variant to be selected when available"
        );
    }

    fn sample_tunables() -> Tunables {
        Tunables {
            openrouter_models_url: "https://example.com".to_string(),
            openrouter_api_key: None,
            aa_models_url: "https://example.com".to_string(),
            aa_api_key: None,
            min_free_aaii: 60.0,
            min_paid_aaii: 65.0,
            cheap_in_max: 1.5,
            cheap_out_max: 6.0,
            min_context_length: 8_192,
            fuzzy_match_threshold: 0.94,
            max_retries: 3,
            retry_backoff_ms: 1_000,
        }
    }

    fn build_aa_model(
        name: &str,
        provider: Option<&str>,
        openrouter_slug: Option<&str>,
        aaii: f32,
        price_in: Option<f64>,
        price_out: Option<f64>,
        context: Option<u32>,
    ) -> AaModel {
        AaModel {
            raw_slug: None,
            openrouter_slug: openrouter_slug.map(|value| value.to_string()),
            name: name.to_string(),
            provider_slug: provider.map(|value| value.to_string()),
            modalities: vec!["text".to_string()],
            context_length: context,
            aaii: Some(aaii),
            price_in_per_million: price_in,
            price_out_per_million: price_out,
            last_updated: None,
        }
    }
}
