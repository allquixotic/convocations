use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures_util::stream::{FuturesUnordered, StreamExt};
use openrouter_rs::{
    OpenRouterClient,
    api::models::{Endpoint, Model as OrModel},
};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::IgnoredAny;
use tokio::time::sleep;

use crate::config::Tunables;
use crate::error::CuratorError;

#[derive(Debug, Clone)]
pub struct OpenRouterModel {
    pub slug: String,
    pub name: String,
    pub created_at: Option<DateTime<Utc>>,
    pub context_length: Option<u32>,
    pub prompt_price_per_million: Option<f64>,
    pub completion_price_per_million: Option<f64>,
    pub cheapest_endpoint: Option<CheapestEndpoint>,
}

#[derive(Debug, Clone)]
pub struct CheapestEndpoint {
    pub endpoint_name: String,
    pub provider_name: String,
    pub prompt_price_per_million: Option<f64>,
    pub completion_price_per_million: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AaModel {
    pub raw_slug: Option<String>,
    pub openrouter_slug: Option<String>,
    pub name: String,
    pub provider_slug: Option<String>,
    pub modalities: Vec<String>,
    pub context_length: Option<u32>,
    pub aaii: Option<f32>,
    pub price_in_per_million: Option<f64>,
    pub price_out_per_million: Option<f64>,
    pub last_updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct FetchResults {
    pub openrouter: Vec<OpenRouterModel>,
    pub aa: Vec<AaModel>,
}

pub async fn fetch_datasets(
    client: &Client,
    tunables: &Tunables,
) -> Result<FetchResults, CuratorError> {
    let openrouter = fetch_openrouter_models(tunables).await?;
    let aa = fetch_aa_models(client, tunables).await?;
    Ok(FetchResults { openrouter, aa })
}

async fn fetch_openrouter_models(
    tunables: &Tunables,
) -> Result<Vec<OpenRouterModel>, CuratorError> {
    let api_key = tunables
        .openrouter_api_key
        .clone()
        .ok_or_else(|| CuratorError::Config("OPENROUTER_API_KEY is required".to_string()))?;

    let base_url = derive_openrouter_base_url(&tunables.openrouter_models_url)?;
    let client = OpenRouterClient::builder()
        .base_url(base_url.clone())
        .api_key(api_key.clone())
        .build()?;

    let models = client.list_models().await?;

    let shared_client = Arc::new(client);
    let mut enriched: Vec<(usize, OpenRouterModel)> = Vec::new();
    let mut pending = FuturesUnordered::new();
    let mut iter = models.into_iter().enumerate();

    const MAX_CONCURRENT_ENDPOINT_FETCHES: usize = 8;

    while pending.len() < MAX_CONCURRENT_ENDPOINT_FETCHES {
        if let Some((index, model)) = iter.next() {
            let client = Arc::clone(&shared_client);
            pending.push(enrich_future(client, model, index));
        } else {
            break;
        }
    }

    while let Some(result) = pending.next().await {
        match result {
            Ok((index, model)) => enriched.push((index, model)),
            Err(err) => {
                eprintln!("[curator] failed to enrich OpenRouter model: {}", err);
            }
        }

        if let Some((index, model)) = iter.next() {
            let client = Arc::clone(&shared_client);
            pending.push(enrich_future(client, model, index));
        }
    }

    enriched.sort_by_key(|(index, _)| *index);
    Ok(enriched.into_iter().map(|(_, model)| model).collect())
}

fn derive_openrouter_base_url(models_url: &str) -> Result<String, CuratorError> {
    let trimmed = models_url.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(CuratorError::Config(
            "OPENROUTER_MODELS_URL is empty".to_string(),
        ));
    }
    if let Some(stripped) = trimmed.strip_suffix("/models") {
        Ok(stripped.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

async fn enrich_openrouter_model(
    client: Arc<OpenRouterClient>,
    model: OrModel,
) -> Result<OpenRouterModel, CuratorError> {
    let slug = model.id.clone();
    let (author, slug_part) = match split_author_slug(&slug) {
        Some(parts) => parts,
        None => {
            return Ok(OpenRouterModel {
                slug,
                name: model.name,
                created_at: parse_created_timestamp(model.created),
                context_length: parse_context_length(model.context_length),
                prompt_price_per_million: parse_price_str(model.pricing.prompt.as_str()),
                completion_price_per_million: parse_price_str(model.pricing.completion.as_str()),
                cheapest_endpoint: None,
            });
        }
    };

    let endpoint_data = match client.list_model_endpoints(author, slug_part).await {
        Ok(data) => Some(data),
        Err(err) => {
            eprintln!(
                "[curator] failed to fetch endpoints for {}: {}",
                model.id, err
            );
            None
        }
    };

    let cheapest_endpoint = endpoint_data
        .as_ref()
        .and_then(|data| compute_cheapest_endpoint(&data.endpoints));

    let fallback_prompt = parse_price_str(model.pricing.prompt.as_str());
    let fallback_completion = parse_price_str(model.pricing.completion.as_str());
    let prompt_price = cheapest_endpoint
        .as_ref()
        .and_then(|meta| meta.prompt_price_per_million)
        .or(fallback_prompt);
    let completion_price = cheapest_endpoint
        .as_ref()
        .and_then(|meta| meta.completion_price_per_million)
        .or(fallback_completion);

    Ok(OpenRouterModel {
        slug,
        name: model.name,
        created_at: parse_created_timestamp(model.created),
        context_length: parse_context_length(model.context_length),
        prompt_price_per_million: prompt_price,
        completion_price_per_million: completion_price,
        cheapest_endpoint,
    })
}

fn enrich_future(
    client: Arc<OpenRouterClient>,
    model: OrModel,
    index: usize,
) -> impl std::future::Future<Output = Result<(usize, OpenRouterModel), CuratorError>> {
    async move {
        let enriched = enrich_openrouter_model(client, model).await;
        enriched.map(|model| (index, model))
    }
}

fn split_author_slug(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
}

fn compute_cheapest_endpoint(endpoints: &[Endpoint]) -> Option<CheapestEndpoint> {
    let mut best: Option<(f64, f64, f64, CheapestEndpoint)> = None;
    for endpoint in endpoints {
        let prompt = parse_price_str(endpoint.pricing.prompt.as_str());
        let completion = parse_price_str(endpoint.pricing.completion.as_str());
        if prompt.is_none() && completion.is_none() {
            continue;
        }

        let prompt_value = prompt.unwrap_or(f64::INFINITY);
        let completion_value = completion.unwrap_or(f64::INFINITY);
        let total = prompt_value + completion_value;

        let candidate = CheapestEndpoint {
            endpoint_name: endpoint.name.clone(),
            provider_name: endpoint.provider_name.clone(),
            prompt_price_per_million: prompt,
            completion_price_per_million: completion,
        };

        match &mut best {
            None => {
                best = Some((total, prompt_value, completion_value, candidate));
            }
            Some(current) => {
                let (best_total, best_prompt, best_completion, _) = current;
                let ordering = total
                    .partial_cmp(best_total)
                    .unwrap_or(std::cmp::Ordering::Greater);
                let should_replace = if ordering == std::cmp::Ordering::Less {
                    true
                } else if ordering == std::cmp::Ordering::Equal {
                    match prompt_value
                        .partial_cmp(best_prompt)
                        .unwrap_or(std::cmp::Ordering::Greater)
                    {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Equal => completion_value
                            .partial_cmp(best_completion)
                            .map(|ord| ord == std::cmp::Ordering::Less)
                            .unwrap_or(false),
                        _ => false,
                    }
                } else {
                    false
                };

                if should_replace {
                    *current = (total, prompt_value, completion_value, candidate);
                }
            }
        }
    }

    best.map(|(_, _, _, endpoint)| endpoint)
}

fn parse_price_str(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok().and_then(|value| {
        if value.is_finite() && value >= 0.0 {
            Some(value)
        } else {
            None
        }
    })
}

fn parse_context_length(value: f64) -> Option<u32> {
    if value.is_finite() && value > 0.0 {
        Some(value.round() as u32)
    } else {
        None
    }
}

fn parse_created_timestamp(value: f64) -> Option<DateTime<Utc>> {
    if value.is_finite() && value > 0.0 {
        let seconds = value.floor() as i64;
        let nanos = ((value - value.floor()) * 1_000_000_000.0) as u32;
        DateTime::<Utc>::from_timestamp(seconds, nanos)
            .or_else(|| DateTime::<Utc>::from_timestamp(seconds, 0))
    } else {
        None
    }
}

async fn fetch_aa_models(
    client: &Client,
    tunables: &Tunables,
) -> Result<Vec<AaModel>, CuratorError> {
    let request = || async {
        let mut builder = client.get(&tunables.aa_models_url);
        if let Some(key) = &tunables.aa_api_key {
            builder = builder.header("x-api-key", key.as_str()).bearer_auth(key);
        }
        builder.send().await
    };

    let response = fetch_with_retries("artificial-analysis", tunables, request).await?;
    if !response.status().is_success() {
        return Err(CuratorError::Message(format!(
            "AA responded with {}",
            response.status()
        )));
    }

    let payload: AaResponse = response.json().await?;
    Ok(payload.into_vec())
}

async fn fetch_with_retries<F, Fut>(
    label: &str,
    tunables: &Tunables,
    mut op: F,
) -> Result<reqwest::Response, CuratorError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let max = tunables.max_retries.max(1);
    let backoff = Duration::from_millis(tunables.retry_backoff_ms);
    let mut attempt = 0usize;

    loop {
        match op().await {
            Ok(response) => return Ok(response),
            Err(err) => {
                attempt += 1;
                let should_retry = match err.status() {
                    Some(status) => {
                        status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS
                    }
                    None => err.is_timeout() || err.is_connect() || err.is_request(),
                };

                if attempt >= max || !should_retry {
                    return Err(CuratorError::Message(format!(
                        "{} request failed after {} attempts: {}",
                        label, attempt, err
                    )));
                }

                sleep(backoff).await;
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AaResponse {
    Wrapped {
        models: Vec<AaPayload>,
    },
    Structured {
        #[serde(default)]
        _status: Option<u16>,
        #[serde(default)]
        _prompt_options: Option<IgnoredAny>,
        data: Vec<AaPayload>,
    },
    Direct(Vec<AaPayload>),
}

impl AaResponse {
    fn into_vec(self) -> Vec<AaModel> {
        match self {
            AaResponse::Wrapped { models } => models.into_iter().map(AaModel::from).collect(),
            AaResponse::Structured { data, .. } => data.into_iter().map(AaModel::from).collect(),
            AaResponse::Direct(models) => models.into_iter().map(AaModel::from).collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AaPayload {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    #[serde(rename = "modelSlug")]
    model_slug: Option<String>,
    #[serde(default)]
    #[serde(rename = "openrouterSlug")]
    openrouter_slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    provider: Option<AaProvider>,
    #[serde(default)]
    modalities: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    context: Option<AaContext>,
    #[serde(default)]
    #[serde(rename = "contextTokens")]
    context_tokens: Option<u32>,
    #[serde(default)]
    #[serde(rename = "model_creator")]
    model_creator: Option<AaProvider>,
    #[serde(default)]
    scores: Option<AaScores>,
    #[serde(default)]
    metrics: Option<AaScores>,
    #[serde(default)]
    evaluations: Option<AaScores>,
    #[serde(default)]
    pricing: Option<AaPricing>,
    #[serde(default)]
    #[serde(rename = "lastUpdatedAt")]
    last_updated_at: Option<String>,
    #[serde(default)]
    #[serde(rename = "releaseDate")]
    release_date: Option<String>,
}

impl From<AaPayload> for AaModel {
    fn from(payload: AaPayload) -> Self {
        let AaPayload {
            slug,
            model_slug,
            openrouter_slug,
            name,
            provider,
            modalities,
            tags,
            context,
            context_tokens,
            model_creator,
            scores,
            metrics,
            evaluations,
            pricing,
            last_updated_at,
            release_date,
        } = payload;

        let context_length = context
            .and_then(|ctx| ctx.max_tokens.or(ctx.context_tokens))
            .or(context_tokens);

        let score = scores
            .and_then(|scores| scores.best())
            .or_else(|| metrics.and_then(|scores| scores.best()))
            .or_else(|| evaluations.and_then(|scores| scores.best()));

        let pricing = pricing.unwrap_or_default();
        let price_in = pricing.prompt_price();
        let price_out = pricing.completion_price();

        let provider_slug = provider
            .as_ref()
            .and_then(AaProvider::best_slug)
            .or_else(|| model_creator.as_ref().and_then(AaProvider::best_slug));

        let last_updated =
            parse_timestamp(last_updated_at).or_else(|| parse_timestamp(release_date));

        Self {
            raw_slug: slug.or(model_slug),
            openrouter_slug,
            name: name.unwrap_or_else(|| "unknown".to_string()),
            provider_slug,
            modalities: merge_modalities(modalities, tags),
            context_length,
            aaii: score,
            price_in_per_million: price_in,
            price_out_per_million: price_out,
            last_updated,
        }
    }
}

fn parse_timestamp(raw: Option<String>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| {
        value
            .parse::<DateTime<Utc>>()
            .ok()
            .or_else(|| {
                DateTime::parse_from_rfc3339(&value)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            })
            .or_else(|| {
                NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .ok()
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
            })
    })
}

fn merge_modalities(primary: Vec<String>, tags: Vec<String>) -> Vec<String> {
    let mut seen = BTreeMap::new();
    for modality in primary.into_iter().chain(tags.into_iter()) {
        let normalized = modality.to_ascii_lowercase();
        seen.entry(normalized).or_insert(modality);
    }
    seen.into_iter().map(|(_, value)| value).collect()
}

#[derive(Debug, Deserialize)]
struct AaProvider {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

impl AaProvider {
    fn best_slug(&self) -> Option<String> {
        self.slug.clone().or_else(|| self.id.clone())
    }
}

#[derive(Debug, Deserialize)]
struct AaContext {
    #[serde(default)]
    #[serde(rename = "max")]
    max_tokens: Option<u32>,
    #[serde(default)]
    #[serde(rename = "maxTokens")]
    context_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AaScores {
    #[serde(default)]
    #[serde(rename = "AAII")]
    aaii_upper: Option<f32>,
    #[serde(default)]
    aaii: Option<f32>,
    #[serde(default)]
    #[serde(rename = "artificial_analysis_intelligence_index")]
    aaii_v2: Option<f32>,
    #[serde(default)]
    #[serde(rename = "intelligence_score")]
    intelligence_score: Option<f32>,
    #[serde(default)]
    #[serde(rename = "quality_index")]
    quality_index: Option<f32>,
}

impl AaScores {
    fn best(self) -> Option<f32> {
        self.aaii
            .or(self.aaii_upper)
            .or(self.aaii_v2)
            .or(self.intelligence_score)
            .or(self.quality_index)
    }
}

#[derive(Debug, Deserialize, Default)]
struct AaPricing {
    #[serde(default)]
    input: Option<PriceValue>,
    #[serde(default)]
    #[serde(rename = "prompt")]
    prompt: Option<PriceValue>,
    #[serde(default)]
    output: Option<PriceValue>,
    #[serde(default)]
    #[serde(rename = "completion")]
    completion: Option<PriceValue>,
    #[serde(default)]
    #[serde(rename = "price_1m_input_tokens")]
    price_1m_input_tokens: Option<f64>,
    #[serde(default)]
    #[serde(rename = "price_1m_output_tokens")]
    price_1m_output_tokens: Option<f64>,
    #[serde(default)]
    #[serde(rename = "price_1m_blended_3_to_1")]
    price_1m_blended: Option<f64>,
}

impl AaPricing {
    fn prompt_price(&self) -> Option<f64> {
        self.prompt
            .as_ref()
            .or(self.input.as_ref())
            .and_then(PriceValue::as_f64)
            .or(self.price_1m_input_tokens)
            .or(self.price_1m_blended)
    }

    fn completion_price(&self) -> Option<f64> {
        self.completion
            .as_ref()
            .or(self.output.as_ref())
            .and_then(PriceValue::as_f64)
            .or(self.price_1m_output_tokens)
            .or(self.price_1m_blended)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PriceValue {
    Float(f64),
    String(String),
    Object(PriceObject),
}

impl PriceValue {
    fn as_f64(&self) -> Option<f64> {
        match self {
            PriceValue::Float(value) => Some(*value),
            PriceValue::String(value) => value.trim().parse::<f64>().ok(),
            PriceValue::Object(object) => object.as_f64(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct PriceObject {
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    #[serde(rename = "usdPer1M")]
    usd_per_1m: Option<f64>,
    #[serde(default)]
    #[serde(rename = "usd_per_1m_tokens")]
    usd_per_1m_tokens: Option<f64>,
    #[serde(default)]
    #[serde(rename = "usd_per_1m")]
    usd_per_1m_snake: Option<f64>,
}

impl PriceObject {
    fn as_f64(&self) -> Option<f64> {
        self.value
            .or(self.price)
            .or(self.usd_per_1m)
            .or(self.usd_per_1m_tokens)
            .or(self.usd_per_1m_snake)
    }
}
