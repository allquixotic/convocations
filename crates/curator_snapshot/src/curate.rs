use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::alias::{AliasResolver, MatchResult, MatchStrategy};
use crate::config::Tunables;
use crate::fetch::{AaModel, OpenRouterModel};

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
    let mut discarded = Vec::new();

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
            discarded.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::NonTextModalities,
            });
            continue;
        }

        let Some(aaii) = aa.aaii else {
            discarded.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::MissingAaii,
            });
            continue;
        };

        if !aaii.is_finite() {
            discarded.push(DiscardedModel {
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
                discarded.push(DiscardedModel {
                    slug: slug.clone(),
                    reason: DiscardReason::InsufficientContext {
                        min_required: tunables.min_context_length,
                        actual: Some(context_length),
                    },
                });
                continue;
            }
        } else {
            discarded.push(DiscardedModel {
                slug: slug.clone(),
                reason: DiscardReason::InsufficientContext {
                    min_required: tunables.min_context_length,
                    actual: None,
                },
            });
            continue;
        }

        let (price_in, price_out, price_source) = resolve_pricing(aa, openrouter_model);

        if matches!(price_source, PriceSource::OpenRouter)
            && price_in.is_none()
            && price_out.is_none()
        {
            discarded.push(DiscardedModel {
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

        if is_free_model(price_in, price_out) {
            if aaii < tunables.min_free_aaii {
                discarded.push(DiscardedModel {
                    slug: slug.clone(),
                    reason: DiscardReason::AaiiThreshold {
                        minimum: tunables.min_free_aaii,
                        actual: aaii,
                    },
                });
                continue;
            }
            free.push(entry);
        } else {
            if aaii < tunables.min_paid_aaii {
                discarded.push(DiscardedModel {
                    slug: slug.clone(),
                    reason: DiscardReason::AaiiThreshold {
                        minimum: tunables.min_paid_aaii,
                        actual: aaii,
                    },
                });
                continue;
            }

            if !meets_pricing_thresholds(price_in, price_out, tunables) {
                discarded.push(DiscardedModel {
                    slug: slug.clone(),
                    reason: DiscardReason::PricingThreshold {
                        max_in: tunables.cheap_in_max,
                        max_out: tunables.cheap_out_max,
                        actual_in: price_in,
                        actual_out: price_out,
                    },
                });
                continue;
            }

            cheap.push(entry);
        }
    }

    sort_and_truncate(&mut free);
    sort_and_truncate(&mut cheap);

    CuratedComputation {
        free,
        cheap,
        unmatched,
        discarded,
    }
}

fn sort_and_truncate(entries: &mut Vec<CuratedEntry>) {
    entries.sort_by(
        |a, b| match b.aaii.partial_cmp(&a.aaii).unwrap_or(Ordering::Equal) {
            Ordering::Equal => a.slug.cmp(&b.slug),
            other => other,
        },
    );
    if entries.len() > 3 {
        entries.truncate(3);
    }
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
    let aa_in = sanitize_price(aa.price_in_per_million);
    let aa_out = sanitize_price(aa.price_out_per_million);

    if aa_in.is_some() || aa_out.is_some() {
        return (aa_in, aa_out, PriceSource::Aa);
    }

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

fn is_free_model(price_in: Option<f64>, price_out: Option<f64>) -> bool {
    let zero = |value: Option<f64>| value.map(|v| v <= 1e-9).unwrap_or(true);
    zero(price_in) && zero(price_out)
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

fn provider_from_slug(slug: &str) -> String {
    slug.split('/').next().unwrap_or("unknown").to_string()
}

fn match_strategy_label(result: &MatchResult) -> Option<String> {
    match &result.strategy {
        Some(MatchStrategy::ProvidedSlug) => Some("provided-slug".to_string()),
        Some(MatchStrategy::Alias { alias_key }) => Some(format!("alias:{}", alias_key)),
        Some(MatchStrategy::Fuzzy { candidate_name }) => Some(format!("fuzzy:{}", candidate_name)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_is_free_model() {
        assert!(is_free_model(Some(0.0), Some(0.0)));
        assert!(is_free_model(Some(0.0), None));
        assert!(is_free_model(None, None));
        assert!(!is_free_model(Some(0.01), Some(0.0)));
    }

    #[test]
    fn pricing_falls_back_to_openrouter_when_aa_missing() {
        let tunables = sample_tunables();
        let openrouter = vec![OpenRouterModel {
            slug: "provider/test-model".to_string(),
            name: "Test Model".to_string(),
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
        assert_eq!(computation.free.len(), 3);
        assert!(computation.free[0].aaii >= computation.free[1].aaii);
        assert!(computation.free[1].aaii >= computation.free[2].aaii);
        assert_eq!(computation.free[0].slug, "provider/free-4");
        assert_eq!(computation.free[1].slug, "provider/free-3");
        assert_eq!(computation.free[2].slug, "provider/free-2");
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
