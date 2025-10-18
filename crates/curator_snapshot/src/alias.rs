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
        let openrouter_by_slug = models
            .iter()
            .map(|model| (model.slug.as_str(), model))
            .collect::<HashMap<_, _>>();

        let fuzzy_candidates = models
            .iter()
            .map(|model| FuzzyCandidate {
                slug: model.slug.as_str(),
                name: model.name.as_str(),
                normalized_name: normalize(&model.name),
                normalized_slug: normalize(&model.slug),
            })
            .collect();

        Self {
            aliases,
            openrouter_by_slug,
            fuzzy_candidates,
            threshold,
        }
    }

    pub fn resolve(&self, aa: &AaModel) -> MatchResult {
        if let Some(slug) = aa.openrouter_slug.as_deref() {
            if let Some(model) = self.openrouter_by_slug.get(slug) {
                return MatchResult::direct(model.slug.clone());
            }
        }

        if let Some(raw) = aa.raw_slug.as_deref() {
            if let Some(model) = self.openrouter_by_slug.get(raw) {
                return MatchResult::direct(model.slug.clone());
            }
        }

        let alias_candidates = build_alias_candidates(aa);
        for candidate in alias_candidates {
            let normalized_key = normalize(&candidate);
            if let Some(slug) = self.aliases.get(&normalized_key) {
                if let Some(model) = self.openrouter_by_slug.get(slug.as_str()) {
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

    fn sample_openrouter_models() -> Vec<OpenRouterModel> {
        vec![
            OpenRouterModel {
                slug: "provider/direct-model".to_string(),
                name: "Direct Match".to_string(),
                context_length: Some(8_192),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
            OpenRouterModel {
                slug: "provider/alias-model".to_string(),
                name: "Alias Model".to_string(),
                context_length: Some(8_192),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
            OpenRouterModel {
                slug: "provider/fuzzy-match".to_string(),
                name: "Fuzzi Modell".to_string(),
                context_length: Some(8_192),
                prompt_price_per_million: Some(0.0),
                completion_price_per_million: Some(0.0),
            },
        ]
    }
}
