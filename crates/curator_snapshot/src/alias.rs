use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use strsim::jaro_winkler;

use crate::error::CuratorError;
use crate::fetch::{AaModel, OpenRouterModel};

pub fn load_alias_map(path: &Path) -> Result<HashMap<String, String>, CuratorError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let parsed: HashMap<String, String> = serde_json::from_str(&raw)?;
    let mut normalized = HashMap::new();
    for (key, value) in parsed {
        let normalized_key = normalize(&key);
        let slug = value.trim();
        if !normalized_key.is_empty() && !slug.is_empty() {
            normalized.insert(normalized_key, slug.to_string());
        }
    }

    Ok(normalized)
}

#[derive(Debug, Clone)]
pub struct AliasResolver<'a> {
    aliases: HashMap<String, String>,
    openrouter_by_slug: HashMap<&'a str, &'a OpenRouterModel>,
    openrouter_by_lower: HashMap<String, &'a OpenRouterModel>,
    openrouter_by_suffix: HashMap<String, Vec<&'a OpenRouterModel>>,
    fuzzy_candidates: Vec<FuzzyCandidate<'a>>,
    threshold: f64,
}

#[derive(Debug, Clone)]
struct FuzzyCandidate<'a> {
    slug: &'a str,
    name: &'a str,
    normalized_name: String,
    normalized_slug: String,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub slug: Option<String>,
    pub strategy: Option<MatchStrategy>,
    pub score: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum MatchStrategy {
    ProvidedSlug,
    Alias { alias_key: String },
    Derived { source: String },
    Fuzzy { candidate_name: String },
}

impl MatchResult {
    pub fn none() -> Self {
        Self {
            slug: None,
            strategy: None,
            score: None,
        }
    }

    pub fn direct(slug: String) -> Self {
        Self {
            slug: Some(slug),
            strategy: Some(MatchStrategy::ProvidedSlug),
            score: None,
        }
    }

    pub fn alias(slug: String, alias_key: String) -> Self {
        Self {
            slug: Some(slug),
            strategy: Some(MatchStrategy::Alias { alias_key }),
            score: None,
        }
    }

    pub fn derived(slug: String, source: String) -> Self {
        Self {
            slug: Some(slug),
            strategy: Some(MatchStrategy::Derived { source }),
            score: None,
        }
    }

    pub fn fuzzy(slug: String, candidate_name: String, score: f64) -> Self {
        Self {
            slug: Some(slug),
            strategy: Some(MatchStrategy::Fuzzy { candidate_name }),
            score: Some(score),
        }
    }
}

impl<'a> AliasResolver<'a> {
    pub fn new(
        aliases: HashMap<String, String>,
        models: &'a [OpenRouterModel],
        threshold: f64,
    ) -> Self {
        let mut openrouter_by_slug = HashMap::new();
        let mut openrouter_by_lower = HashMap::new();
        let mut openrouter_by_suffix: HashMap<String, Vec<&OpenRouterModel>> = HashMap::new();
        let mut fuzzy_candidates = Vec::with_capacity(models.len());

        for model in models {
            openrouter_by_slug.insert(model.slug.as_str(), model);
            openrouter_by_lower.insert(model.slug.to_ascii_lowercase(), model);

            let suffix_key = model
                .slug
                .rsplit('/')
                .next()
                .unwrap_or(model.slug.as_str())
                .to_ascii_lowercase();
            openrouter_by_suffix
                .entry(suffix_key)
                .or_default()
                .push(model);

            fuzzy_candidates.push(FuzzyCandidate {
                slug: model.slug.as_str(),
                name: model.name.as_str(),
                normalized_name: normalize(&model.name),
                normalized_slug: normalize(&model.slug),
            });
        }

        Self {
            aliases,
            openrouter_by_slug,
            openrouter_by_lower,
            openrouter_by_suffix,
            fuzzy_candidates,
            threshold,
        }
    }

    pub fn resolve(&self, aa: &AaModel) -> MatchResult {
        if let Some(slug) = aa.openrouter_slug.as_deref() {
            if let Some(model) = self.lookup_slug(slug) {
                return MatchResult::direct(model.slug.clone());
            }
        }

        if let Some(raw) = aa.raw_slug.as_deref() {
            if let Some(model) = self.lookup_slug(raw) {
                return MatchResult::direct(model.slug.clone());
            }

            if let Some(result) = self.match_by_suffix(raw, aa.provider_slug.as_deref()) {
                return result;
            }
        }

        let alias_candidates = build_alias_candidates(aa);
        for candidate in alias_candidates {
            if let Some(model) = self.lookup_slug(&candidate) {
                return MatchResult::derived(model.slug.clone(), candidate);
            }

            let normalized_key = normalize(&candidate);
            if let Some(slug) = self.aliases.get(&normalized_key) {
                if let Some(model) = self.lookup_slug(slug) {
                    return MatchResult::alias(model.slug.clone(), candidate);
                }
            }
        }

        let aa_normalized = normalize(&aa.name);
        let mut best: Option<(f64, &FuzzyCandidate)> = None;

        for candidate in &self.fuzzy_candidates {
            let score_name = jaro_winkler(&aa_normalized, &candidate.normalized_name);
            let score_slug = jaro_winkler(&aa_normalized, &candidate.normalized_slug);
            let score = score_name.max(score_slug);

            if score < self.threshold {
                continue;
            }

            match &mut best {
                Some((best_score, best_candidate)) => {
                    if score > *best_score + 1e-6
                        || ((score - *best_score).abs() <= 1e-6
                            && candidate.slug < best_candidate.slug)
                    {
                        *best_score = score;
                        *best_candidate = candidate;
                    }
                }
                None => best = Some((score, candidate)),
            }
        }

        if let Some((score, candidate)) = best {
            return MatchResult::fuzzy(
                candidate.slug.to_string(),
                candidate.name.to_string(),
                score,
            );
        }

        MatchResult::none()
    }

    pub fn get_model(&self, slug: &str) -> Option<&'a OpenRouterModel> {
        self.openrouter_by_slug.get(slug).copied()
    }

    fn lookup_slug(&self, slug: &str) -> Option<&'a OpenRouterModel> {
        if let Some(model) = self.openrouter_by_slug.get(slug) {
            return Some(*model);
        }
        let lower = slug.to_ascii_lowercase();
        self.openrouter_by_lower.get(&lower).copied()
    }

    fn match_by_suffix(&self, slug: &str, provider_slug: Option<&str>) -> Option<MatchResult> {
        let key = slug.to_ascii_lowercase();
        let candidates = self.openrouter_by_suffix.get(&key)?;
        if candidates.is_empty() {
            return None;
        }

        if candidates.len() == 1 {
            let model = candidates[0];
            return Some(MatchResult::derived(
                model.slug.clone(),
                format!("suffix:{slug}"),
            ));
        }

        if let Some(provider) = provider_slug {
            let provider_lower = provider.to_ascii_lowercase();
            let mut selected: Option<&OpenRouterModel> = None;

            for candidate in candidates {
                let prefix = candidate
                    .slug
                    .split('/')
                    .next()
                    .unwrap_or(candidate.slug.as_str());
                if provider_matches(&provider_lower, prefix) {
                    if selected.is_some() {
                        return None;
                    }
                    selected = Some(*candidate);
                }
            }

            if let Some(model) = selected {
                return Some(MatchResult::derived(
                    model.slug.clone(),
                    format!("suffix:{slug}"),
                ));
            }
        }

        None
    }
}

fn build_alias_candidates(aa: &AaModel) -> Vec<String> {
    let mut candidates = HashSet::new();
    candidates.insert(aa.name.clone());
    if let Some(raw_slug) = aa.raw_slug.as_ref() {
        candidates.insert(raw_slug.clone());
    }
    if let Some(provider) = aa.provider_slug.as_ref() {
        candidates.insert(format!("{provider}/{}", aa.name));
        if let Some(raw_slug) = aa.raw_slug.as_ref() {
            candidates.insert(format!("{provider}/{}", raw_slug));
        }
    }
    candidates.into_iter().collect()
}

fn provider_matches(provider_lower: &str, candidate_prefix: &str) -> bool {
    let candidate_lower = candidate_prefix.to_ascii_lowercase();
    if provider_lower == candidate_lower
        || candidate_lower.starts_with(provider_lower)
        || provider_lower.starts_with(candidate_lower.as_str())
    {
        return true;
    }

    match provider_lower {
        "meta" | "meta-llama" => candidate_lower == "meta-llama",
        "alibaba" => candidate_lower == "qwen" || candidate_lower == "alibaba",
        "qwen" => candidate_lower == "qwen",
        "xai" | "x-ai" => candidate_lower == "x-ai",
        "ai21-labs" | "ai21" => candidate_lower == "ai21",
        "nous-research" | "nousresearch" => candidate_lower == "nousresearch",
        "mistral" | "mistralai" => candidate_lower == "mistralai",
        "aws" | "amazon" => candidate_lower == "amazon",
        "azure" | "microsoft" => candidate_lower == "microsoft",
        "bytedance_seed" | "bytedance" => candidate_lower == "bytedance",
        "liquidai" | "liquid" => candidate_lower == "liquid",
        "moonshotai" => candidate_lower == "moonshotai",
        "zai" | "z-ai" => candidate_lower == "z-ai",
        "ai2" | "allenai" => candidate_lower == "allenai",
        _ => false,
    }
}

pub fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut prev_was_space = false;

    for ch in value.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            prev_was_space = false;
        } else if matches!(ch, '/' | '-' | '_' | '.') {
            normalized.push(ch);
            prev_was_space = false;
        } else if ch.is_ascii_whitespace() {
            if !prev_was_space {
                normalized.push(' ');
                prev_was_space = true;
            }
        }
    }

    normalized.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn resolves_provided_slug_before_alias_or_fuzzy() {
        let models = sample_openrouter_models();
        let mut aliases = HashMap::new();
        aliases.insert(normalize("Alias Model"), "provider/alias-model".to_string());
        let resolver = AliasResolver::new(aliases, &models, 0.8);

        let aa = AaModel {
            raw_slug: None,
            openrouter_slug: Some("provider/direct-model".to_string()),
            name: "Alias Model".to_string(),
            provider_slug: Some("provider".to_string()),
            modalities: vec!["text".to_string()],
            context_length: Some(8_192),
            aaii: Some(70.0),
            price_in_per_million: Some(0.0),
            price_out_per_million: Some(0.0),
            last_updated: Some(Utc::now()),
        };

        let result = resolver.resolve(&aa);
        assert_eq!(result.slug.as_deref(), Some("provider/direct-model"));
        assert!(matches!(result.strategy, Some(MatchStrategy::ProvidedSlug)));
    }

    #[test]
    fn resolves_alias_before_fuzzy() {
        let models = sample_openrouter_models();
        let mut aliases = HashMap::new();
        aliases.insert(normalize("Alias Model"), "provider/alias-model".to_string());
        let resolver = AliasResolver::new(aliases, &models, 0.8);

        let aa = AaModel {
            raw_slug: None,
            openrouter_slug: None,
            name: "Alias Model".to_string(),
            provider_slug: Some("provider".to_string()),
            modalities: vec!["text".to_string()],
            context_length: Some(8_192),
            aaii: Some(70.0),
            price_in_per_million: Some(0.0),
            price_out_per_million: Some(0.0),
            last_updated: Some(Utc::now()),
        };

        let result = resolver.resolve(&aa);
        assert_eq!(result.slug.as_deref(), Some("provider/alias-model"));
        assert!(matches!(result.strategy, Some(MatchStrategy::Alias { .. })));
    }

    #[test]
    fn falls_back_to_fuzzy_matching() {
        let models = sample_openrouter_models();
        let aliases = HashMap::new();
        let resolver = AliasResolver::new(aliases, &models, 0.8);

        let aa = AaModel {
            raw_slug: None,
            openrouter_slug: None,
            name: "Fuzzy Model".to_string(),
            provider_slug: Some("provider".to_string()),
            modalities: vec!["text".to_string()],
            context_length: Some(8_192),
            aaii: Some(70.0),
            price_in_per_million: Some(0.0),
            price_out_per_million: Some(0.0),
            last_updated: Some(Utc::now()),
        };

        let result = resolver.resolve(&aa);
        assert_eq!(result.slug.as_deref(), Some("provider/fuzzy-match"));
        assert!(matches!(result.strategy, Some(MatchStrategy::Fuzzy { .. })));
        assert!(result.score.unwrap() >= 0.8);
    }

    #[test]
    fn resolves_suffix_with_provider_hint() {
        let models = vec![OpenRouterModel {
            slug: "openai/gpt-5".to_string(),
            name: "OpenAI: GPT-5".to_string(),
            created_at: None,
            context_length: Some(128_000),
            prompt_price_per_million: Some(10.0),
            completion_price_per_million: Some(30.0),
            cheapest_endpoint: None,
        }];

        let resolver = AliasResolver::new(HashMap::new(), &models, 0.8);
        let aa = AaModel {
            raw_slug: Some("gpt-5".to_string()),
            openrouter_slug: None,
            name: "GPT-5 (high)".to_string(),
            provider_slug: Some("openai".to_string()),
            modalities: vec!["text".to_string()],
            context_length: Some(128_000),
            aaii: Some(90.0),
            price_in_per_million: Some(10.0),
            price_out_per_million: Some(30.0),
            last_updated: Some(Utc::now()),
        };

        let result = resolver.resolve(&aa);
        assert_eq!(result.slug.as_deref(), Some("openai/gpt-5"));
        assert!(matches!(
            result.strategy,
            Some(MatchStrategy::Derived { .. })
        ));
    }

    #[test]
    fn jaro_winkler_reference_score_is_low_for_variant_names() {
        let left = normalize("GPT-5 (high)");
        let right = normalize("OpenAI: GPT-5");
        let score = jaro_winkler(&left, &right);
        assert!(
            score < 0.6,
            "expected score below 0.6, got {score} between '{left}' and '{right}'"
        );
    }

    fn sample_openrouter_models() -> Vec<OpenRouterModel> {
        vec![
            OpenRouterModel {
                slug: "provider/direct-model".to_string(),
                name: "Direct Match".to_string(),
                created_at: None,
                context_length: Some(8_192),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
                cheapest_endpoint: None,
            },
            OpenRouterModel {
                slug: "provider/alias-model".to_string(),
                name: "Alias Model".to_string(),
                created_at: None,
                context_length: Some(8_192),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
                cheapest_endpoint: None,
            },
            OpenRouterModel {
                slug: "provider/fuzzy-match".to_string(),
                name: "Fuzzi Modell".to_string(),
                created_at: None,
                context_length: Some(8_192),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
                cheapest_endpoint: None,
            },
        ]
    }
}
