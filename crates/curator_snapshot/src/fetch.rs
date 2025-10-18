use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tokio::time::sleep;

use crate::config::Tunables;
use crate::error::CuratorError;

#[derive(Debug, Clone)]
pub struct OpenRouterModel {
    pub slug: String,
    pub name: String,
    pub context_length: Option<u32>,
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
    let openrouter = fetch_openrouter_models(client, tunables).await?;
    let aa = fetch_aa_models(client, tunables).await?;
    Ok(FetchResults { openrouter, aa })
}

async fn fetch_openrouter_models(
    client: &Client,
    tunables: &Tunables,
) -> Result<Vec<OpenRouterModel>, CuratorError> {
    let request = || async {
        let mut builder = client.get(&tunables.openrouter_models_url);
        if let Some(key) = &tunables.openrouter_api_key {
            builder = builder.bearer_auth(key);
        }
        builder.send().await
    };

    let response = fetch_with_retries("openrouter", tunables, request).await?;
    if !response.status().is_success() {
        return Err(CuratorError::Message(format!(
            "OpenRouter responded with {}",
            response.status()
        )));
    }

    let payload: OpenRouterResponse = response.json().await?;
    let models = payload
        .data
        .into_iter()
        .map(OpenRouterModel::from)
        .collect();
    Ok(models)
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
struct OpenRouterResponse {
    data: Vec<OpenRouterPayload>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPayload {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

impl From<OpenRouterPayload> for OpenRouterModel {
    fn from(payload: OpenRouterPayload) -> Self {
        let OpenRouterPayload {
            id,
            name,
            pricing,
            context_length,
        } = payload;

        let pricing = pricing.unwrap_or_default();
        Self {
            slug: id.clone(),
            name: name.unwrap_or_else(|| id.clone()),
            context_length,
            prompt_price_per_million: parse_price_field(pricing.prompt),
            completion_price_per_million: parse_price_field(pricing.completion),
        }
    }
}

fn parse_price_field(value: Option<String>) -> Option<f64> {
    value.and_then(|raw| raw.parse::<f64>().ok())
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AaResponse {
    Wrapped { models: Vec<AaPayload> },
    Direct(Vec<AaPayload>),
}

impl AaResponse {
    fn into_vec(self) -> Vec<AaModel> {
        match self {
            AaResponse::Wrapped { models } => models.into_iter().map(AaModel::from).collect(),
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
    scores: Option<AaScores>,
    #[serde(default)]
    metrics: Option<AaScores>,
    #[serde(default)]
    pricing: Option<AaPricing>,
    #[serde(default)]
    #[serde(rename = "lastUpdatedAt")]
    last_updated_at: Option<String>,
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
            scores,
            metrics,
            pricing,
            last_updated_at,
        } = payload;

        let context_length = context
            .and_then(|ctx| ctx.max_tokens.or(ctx.context_tokens))
            .or(context_tokens);

        let score = scores
            .and_then(|scores| scores.best())
            .or_else(|| metrics.and_then(|scores| scores.best()));

        let pricing = pricing.unwrap_or_default();
        let price_in = pricing.prompt_price();
        let price_out = pricing.completion_price();

        let last_updated = last_updated_at.and_then(|raw| raw.parse::<DateTime<Utc>>().ok());

        Self {
            raw_slug: slug.or(model_slug),
            openrouter_slug,
            name: name.unwrap_or_else(|| "unknown".to_string()),
            provider_slug: provider.and_then(|provider| provider.slug.or(provider.id)),
            modalities: merge_modalities(modalities, tags),
            context_length,
            aaii: score,
            price_in_per_million: price_in,
            price_out_per_million: price_out,
            last_updated,
        }
    }
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
}

impl AaScores {
    fn best(self) -> Option<f32> {
        self.aaii.or(self.aaii_upper)
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
}

impl AaPricing {
    fn prompt_price(&self) -> Option<f64> {
        self.prompt
            .as_ref()
            .or(self.input.as_ref())
            .and_then(PriceValue::as_f64)
    }

    fn completion_price(&self) -> Option<f64> {
        self.completion
            .as_ref()
            .or(self.output.as_ref())
            .and_then(PriceValue::as_f64)
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
