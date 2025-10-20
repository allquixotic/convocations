use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::alias::{AliasResolver, MatchResult, MatchStrategy};
use crate::config::Tunables;
use crate::fetch::{AaModel, CheapestEndpoint, OpenRouterModel};

const CHEAP_PROVIDER_ORDER: [&str; 4] = ["openai", "x-ai", "google", "anthropic"];
const PRICE_EPSILON: f64 = 1e-9;

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
    pub openrouter_created_at: Option<DateTime<Utc>>,
    pub cheapest_endpoint: Option<CheapestEndpoint>,
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

        let (price_in, price_out, price_source) = resolve_pricing(openrouter_model);
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
            openrouter_created_at: openrouter_model.created_at,
            cheapest_endpoint: openrouter_model.cheapest_endpoint.clone(),
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
        openrouter,
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
            openrouter_created_at: candidate.created_at,
            cheapest_endpoint: candidate.cheapest_endpoint.clone(),
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
    accepted: Vec<CuratedEntry>,
    provider_order: &[&str],
    fallback_aaii: &mut Vec<(CuratedEntry, DiscardReason)>,
    fallback_price: &mut Vec<(CuratedEntry, DiscardReason)>,
    fallback_low: &mut Vec<(CuratedEntry, DiscardReason)>,
    openrouter: &[OpenRouterModel],
) -> Vec<CuratedEntry> {
    let mut provider_pool: HashMap<String, Vec<CuratedEntry>> = HashMap::new();

    for entry in accepted {
        provider_pool
            .entry(entry.provider.clone())
            .or_default()
            .push(entry);
    }

    for (entry, _) in fallback_aaii
        .iter()
        .chain(fallback_price.iter())
        .chain(fallback_low.iter())
    {
        provider_pool
            .entry(entry.provider.clone())
            .or_default()
            .push(entry.clone());
    }

    let openrouter_by_slug: HashMap<&str, &OpenRouterModel> = openrouter
        .iter()
        .map(|model| (model.slug.as_str(), model))
        .collect();

    let mut winners = Vec::with_capacity(provider_order.len());

    for provider in provider_order {
        let candidates = provider_pool.remove(*provider).unwrap_or_default();
        let candidates = dedupe_entries(candidates);
        if let Some(entry) =
            select_paid_candidate(provider, &candidates, openrouter, &openrouter_by_slug)
        {
            winners.push(entry);
            continue;
        }

        if let Some(entry) = fallback_first_model(provider, openrouter) {
            winners.push(entry);
        }
    }

    winners
}

fn dedupe_entries(entries: Vec<CuratedEntry>) -> Vec<CuratedEntry> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for entry in entries {
        if seen.insert(entry.slug.clone()) {
            deduped.push(entry);
        }
    }
    deduped
}

fn select_paid_candidate(
    provider: &str,
    candidates: &[CuratedEntry],
    openrouter: &[OpenRouterModel],
    openrouter_by_slug: &HashMap<&str, &OpenRouterModel>,
) -> Option<CuratedEntry> {
    match provider {
        "openai" => select_openai_paid(candidates, openrouter, openrouter_by_slug),
        "google" => select_google_paid(candidates, openrouter, openrouter_by_slug),
        "x-ai" => select_xai_paid(candidates, openrouter, openrouter_by_slug),
        "anthropic" => select_anthropic_paid(candidates, openrouter, openrouter_by_slug),
        _ => select_default_paid(candidates),
    }
}

fn select_openai_paid(
    candidates: &[CuratedEntry],
    openrouter: &[OpenRouterModel],
    _openrouter_by_slug: &HashMap<&str, &OpenRouterModel>,
) -> Option<CuratedEntry> {
    let mut minis: Vec<CuratedEntry> = candidates
        .iter()
        .filter(|entry| has_term(&entry.slug, "mini") || has_term(&entry.display_name, "mini"))
        .filter(|entry| has_valid_price(entry))
        .cloned()
        .collect();

    minis.sort_by(compare_paid_entries);
    if let Some(entry) = minis.into_iter().next() {
        return Some(entry);
    }

    let mut fallback: Vec<&OpenRouterModel> = openrouter
        .iter()
        .filter(|model| provider_from_slug(&model.slug) == "openai")
        .filter(|model| has_term(&model.slug, "mini") || has_term(&model.name, "mini"))
        .filter(|model| has_valid_model_price(model))
        .collect();

    fallback.sort_by(|left, right| compare_models_by_price(*left, *right));
    fallback
        .into_iter()
        .next()
        .map(|model| curated_entry_from_model(model, "provider-heuristic:openai-mini"))
}

fn select_google_paid(
    candidates: &[CuratedEntry],
    openrouter: &[OpenRouterModel],
    _openrouter_by_slug: &HashMap<&str, &OpenRouterModel>,
) -> Option<CuratedEntry> {
    let mut flash: Vec<(CuratedEntry, f64)> = candidates
        .iter()
        .filter_map(|entry| {
            parse_gemini_flash_version(&entry.slug)
                .or_else(|| parse_gemini_flash_version(&entry.display_name))
                .map(|version| (entry.clone(), version))
        })
        .filter(|(entry, _)| has_valid_price(entry))
        .collect();

    flash.sort_by(|(left_entry, left_version), (right_entry, right_version)| {
        match right_version
            .partial_cmp(left_version)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => compare_paid_entries(left_entry, right_entry),
            other => other,
        }
    });

    if let Some((entry, _)) = flash.into_iter().next() {
        return Some(entry);
    }

    let mut fallback: Vec<(&OpenRouterModel, f64)> = openrouter
        .iter()
        .filter(|model| provider_from_slug(&model.slug) == "google")
        .filter_map(|model| {
            parse_gemini_flash_version(&model.slug)
                .or_else(|| parse_gemini_flash_version(&model.name))
                .map(|version| (model, version))
        })
        .filter(|(model, _)| has_valid_model_price(model))
        .collect();

    fallback.sort_by(
        |(left_model, left_version), (right_model, right_version)| match right_version
            .partial_cmp(left_version)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => compare_models_by_price(left_model, right_model),
            other => other,
        },
    );

    fallback
        .into_iter()
        .next()
        .map(|(model, _)| curated_entry_from_model(model, "provider-heuristic:google-gemini-flash"))
        .or_else(|| match fallback_first_model("google", openrouter) {
            Some(entry) => Some(entry),
            None => None,
        })
}

fn select_xai_paid(
    candidates: &[CuratedEntry],
    openrouter: &[OpenRouterModel],
    _openrouter_by_slug: &HashMap<&str, &OpenRouterModel>,
) -> Option<CuratedEntry> {
    let mut grok_fast: Vec<(CuratedEntry, f64)> = candidates
        .iter()
        .filter_map(|entry| {
            parse_grok_fast_version(&entry.slug)
                .or_else(|| parse_grok_fast_version(&entry.display_name))
                .map(|version| (entry.clone(), version))
        })
        .filter(|(entry, _)| has_valid_price(entry))
        .collect();

    grok_fast.sort_by(
        |(left_entry, left_version), (right_entry, right_version)| match right_version
            .partial_cmp(left_version)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => compare_paid_entries(left_entry, right_entry),
            other => other,
        },
    );

    if let Some((entry, _)) = grok_fast.into_iter().next() {
        return Some(entry);
    }

    let mut fallback: Vec<(&OpenRouterModel, f64)> = openrouter
        .iter()
        .filter(|model| provider_from_slug(&model.slug) == "x-ai")
        .filter_map(|model| {
            parse_grok_fast_version(&model.slug)
                .or_else(|| parse_grok_fast_version(&model.name))
                .map(|version| (model, version))
        })
        .filter(|(model, _)| has_valid_model_price(model))
        .collect();

    fallback.sort_by(
        |(left_model, left_version), (right_model, right_version)| match right_version
            .partial_cmp(left_version)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => compare_models_by_price(left_model, right_model),
            other => other,
        },
    );

    fallback
        .into_iter()
        .next()
        .map(|(model, _)| curated_entry_from_model(model, "provider-heuristic:xai-grok-fast"))
}

fn select_anthropic_paid(
    candidates: &[CuratedEntry],
    openrouter: &[OpenRouterModel],
    _openrouter_by_slug: &HashMap<&str, &OpenRouterModel>,
) -> Option<CuratedEntry> {
    let mut haiku: Vec<(CuratedEntry, f64)> = candidates
        .iter()
        .filter_map(|entry| {
            parse_haiku_version(&entry.slug)
                .or_else(|| parse_haiku_version(&entry.display_name))
                .map(|version| (entry.clone(), version))
        })
        .filter(|(entry, _)| has_valid_price(entry))
        .collect();

    haiku.sort_by(|(left_entry, left_version), (right_entry, right_version)| {
        match right_version
            .partial_cmp(left_version)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => compare_paid_entries(left_entry, right_entry),
            other => other,
        }
    });

    if let Some((entry, _)) = haiku.into_iter().next() {
        return Some(entry);
    }

    let mut fallback: Vec<(&OpenRouterModel, f64)> = openrouter
        .iter()
        .filter(|model| provider_from_slug(&model.slug) == "anthropic")
        .filter_map(|model| {
            parse_haiku_version(&model.slug)
                .or_else(|| parse_haiku_version(&model.name))
                .map(|version| (model, version))
        })
        .filter(|(model, _)| has_valid_model_price(model))
        .collect();

    fallback.sort_by(
        |(left_model, left_version), (right_model, right_version)| match right_version
            .partial_cmp(left_version)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => compare_models_by_price(left_model, right_model),
            other => other,
        },
    );

    fallback
        .into_iter()
        .next()
        .map(|(model, _)| curated_entry_from_model(model, "provider-heuristic:anthropic-haiku"))
}

fn select_default_paid(candidates: &[CuratedEntry]) -> Option<CuratedEntry> {
    let mut filtered: Vec<CuratedEntry> = candidates
        .iter()
        .filter(|entry| has_valid_price(entry))
        .cloned()
        .collect();
    filtered.sort_by(compare_paid_entries);
    filtered.into_iter().next()
}

fn fallback_first_model(provider: &str, openrouter: &[OpenRouterModel]) -> Option<CuratedEntry> {
    openrouter
        .iter()
        .find(|model| provider_from_slug(&model.slug) == provider)
        .map(|model| {
            curated_entry_from_model(model, &format!("provider-fallback:first:{provider}"))
        })
}

fn curated_entry_from_model(model: &OpenRouterModel, strategy: &str) -> CuratedEntry {
    CuratedEntry {
        slug: model.slug.clone(),
        display_name: model.name.clone(),
        provider: provider_from_slug(&model.slug),
        aaii: 0.0,
        price_in_per_million: sanitize_price(model.prompt_price_per_million),
        price_out_per_million: sanitize_price(model.completion_price_per_million),
        price_source: PriceSource::OpenRouter,
        context_length: model.context_length,
        modalities: Vec::new(),
        match_strategy: Some(strategy.to_string()),
        aa_last_updated: None,
        openrouter_created_at: model.created_at,
        cheapest_endpoint: model.cheapest_endpoint.clone(),
    }
}

fn has_valid_price(entry: &CuratedEntry) -> bool {
    entry.price_in_per_million.is_some() || entry.price_out_per_million.is_some()
}

fn has_valid_model_price(model: &OpenRouterModel) -> bool {
    model.prompt_price_per_million.is_some() || model.completion_price_per_million.is_some()
}

fn compare_paid_entries(left: &CuratedEntry, right: &CuratedEntry) -> Ordering {
    let price_order = compare_price_pairs(
        left.price_in_per_million,
        left.price_out_per_million,
        right.price_in_per_million,
        right.price_out_per_million,
    );
    if price_order != Ordering::Equal {
        return price_order;
    }
    let created_order =
        compare_created_desc(left.openrouter_created_at, right.openrouter_created_at);
    if created_order != Ordering::Equal {
        return created_order;
    }
    left.slug.cmp(&right.slug)
}

fn compare_models_by_price(left: &OpenRouterModel, right: &OpenRouterModel) -> Ordering {
    let price_order = compare_price_pairs(
        sanitize_price(left.prompt_price_per_million),
        sanitize_price(left.completion_price_per_million),
        sanitize_price(right.prompt_price_per_million),
        sanitize_price(right.completion_price_per_million),
    );
    if price_order != Ordering::Equal {
        return price_order;
    }
    let created_order = compare_created_desc(left.created_at, right.created_at);
    if created_order != Ordering::Equal {
        return created_order;
    }
    left.slug.cmp(&right.slug)
}

fn compare_price_pairs(
    left_in: Option<f64>,
    left_out: Option<f64>,
    right_in: Option<f64>,
    right_out: Option<f64>,
) -> Ordering {
    let left_prompt = left_in.unwrap_or(f64::INFINITY);
    let left_completion = left_out.unwrap_or(f64::INFINITY);
    let right_prompt = right_in.unwrap_or(f64::INFINITY);
    let right_completion = right_out.unwrap_or(f64::INFINITY);

    let left_total = left_prompt + left_completion;
    let right_total = right_prompt + right_completion;

    match left_total
        .partial_cmp(&right_total)
        .unwrap_or(Ordering::Equal)
    {
        Ordering::Equal => match left_prompt
            .partial_cmp(&right_prompt)
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => left_completion
                .partial_cmp(&right_completion)
                .unwrap_or(Ordering::Equal),
            other => other,
        },
        other => other,
    }
}

fn compare_created_desc(left: Option<DateTime<Utc>>, right: Option<DateTime<Utc>>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn has_term(value: &str, term: &str) -> bool {
    value
        .to_ascii_lowercase()
        .contains(&term.to_ascii_lowercase())
}

fn parse_gemini_flash_version(value: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    if !lower.contains("gemini") || !lower.contains("flash") {
        return None;
    }

    if let Some(start) = lower.find("gemini-") {
        let rest = &lower[start + "gemini-".len()..];
        if let Some(end) = rest.find("-flash") {
            let version = &rest[..end];
            return version.parse::<f64>().ok();
        }
    }

    if let Some(start) = lower.find("flash-") {
        let rest = &lower[start + "flash-".len()..];
        let token = rest
            .split(|c: char| c == '-' || c == '/' || c == ' ')
            .next()
            .unwrap_or("");
        return token.parse::<f64>().ok();
    }

    None
}

fn parse_grok_fast_version(value: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    if !lower.contains("grok") || !lower.contains("fast") {
        return None;
    }

    if let Some(start) = lower.find("grok-") {
        let rest = &lower[start + "grok-".len()..];
        if let Some(end) = rest.find("-fast") {
            let version = &rest[..end];
            return version.parse::<f64>().ok();
        }
    }

    if let Some(start) = lower.find("grok ") {
        let rest = &lower[start + "grok ".len()..];
        if let Some(end) = rest.find(" fast") {
            let version = &rest[..end];
            return version.parse::<f64>().ok();
        }
    }

    None
}

fn parse_haiku_version(value: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    if !lower.contains("haiku") {
        return None;
    }

    if let Some(start) = lower.find("haiku-") {
        let rest = &lower[start + "haiku-".len()..];
        let token = rest
            .split(|c: char| c == '-' || c == '/' || c == ' ')
            .next()
            .unwrap_or("");
        return token.parse::<f64>().ok();
    }

    if let Some(start) = lower.find("haiku ") {
        let rest = &lower[start + "haiku ".len()..];
        let token = rest
            .split(|c: char| c == '-' || c == '/' || c == ' ')
            .next()
            .unwrap_or("");
        return token.parse::<f64>().ok();
    }

    None
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

fn resolve_pricing(openrouter: &OpenRouterModel) -> (Option<f64>, Option<f64>, PriceSource) {
    let prompt = sanitize_price(openrouter.prompt_price_per_million);
    let completion = sanitize_price(openrouter.completion_price_per_million);

    (prompt, completion, PriceSource::OpenRouter)
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
            cheapest_endpoint: None,
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
                "openai/gpt-7-lite",
                "OpenAI GPT-7 Lite",
                "openai",
                78.0,
                Some(2e-7),
                Some(1.5e-6),
            ),
            (
                "openai/gpt-5-mini",
                "OpenAI GPT-5 Mini",
                "openai",
                72.0,
                Some(2.5e-7),
                Some(2.0e-6),
            ),
            (
                "x-ai/grok-4-fast",
                "Grok 4 Fast",
                "x-ai",
                60.0,
                Some(2e-7),
                Some(6e-7),
            ),
            (
                "x-ai/grok-4",
                "Grok 4",
                "x-ai",
                58.0,
                Some(2e-7),
                Some(7e-7),
            ),
            (
                "google/gemini-3-flash",
                "Gemini 3 Flash",
                "google",
                70.0,
                Some(3e-7),
                Some(1.6e-6),
            ),
            (
                "google/gemini-2.5-flash",
                "Gemini 2.5 Flash",
                "google",
                60.0,
                Some(3e-7),
                Some(2.5e-6),
            ),
            (
                "anthropic/claude-haiku-5",
                "Claude Haiku 5",
                "anthropic",
                68.0,
                Some(1e-6),
                Some(4.5e-6),
            ),
            (
                "anthropic/claude-haiku-4.5",
                "Claude Haiku 4.5",
                "anthropic",
                58.0,
                Some(1e-6),
                Some(5e-6),
            ),
            (
                "mistralai/mistral-small",
                "Mistral Small",
                "mistralai",
                62.0,
                Some(4e-1),
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
                cheapest_endpoint: None,
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

        let expected = vec![
            ("openai", "openai/gpt-7-lite"),
            ("x-ai", "x-ai/grok-4-fast"),
            ("google", "google/gemini-3-flash"),
            ("anthropic", "anthropic/claude-haiku-5"),
        ];

        for (provider, expected_slug) in expected {
            let entry = computation
                .cheap
                .iter()
                .find(|entry| entry.provider == provider)
                .unwrap_or_else(|| panic!("missing provider {provider} in cheap list"));
            assert_eq!(
                entry.slug, expected_slug,
                "expected latest/best model for provider {} to be {}, got {}",
                provider, expected_slug, entry.slug
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
            cheapest_endpoint: None,
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
                cheapest_endpoint: None,
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
                cheapest_endpoint: None,
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
                cheapest_endpoint: None,
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
                cheapest_endpoint: None,
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
                cheapest_endpoint: None,
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
