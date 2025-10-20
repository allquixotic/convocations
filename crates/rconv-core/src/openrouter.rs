use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::fmt;

/// Preferred providers for free models
pub const PREFERRED_FREE_PROVIDERS: &[&str] =
    &["x-ai", "google", "openai", "anthropic", "moonshot"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub pricing: ModelPricing,
    pub context_length: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub prompt: String,
    pub completion: String,
}

impl ModelInfo {
    pub fn is_free(&self) -> bool {
        self.pricing.prompt == "0" && self.pricing.completion == "0"
    }

    pub fn provider(&self) -> &str {
        self.id.split('/').next().unwrap_or(&self.id)
    }
}

#[derive(Debug)]
pub struct OpenRouterError {
    message: String,
}

impl fmt::Display for OpenRouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OpenRouter error: {}", self.message)
    }
}

impl StdError for OpenRouterError {}

impl From<String> for OpenRouterError {
    fn from(msg: String) -> Self {
        OpenRouterError { message: msg }
    }
}

impl From<&str> for OpenRouterError {
    fn from(msg: &str) -> Self {
        OpenRouterError {
            message: msg.to_string(),
        }
    }
}

impl From<reqwest::Error> for OpenRouterError {
    fn from(err: reqwest::Error) -> Self {
        OpenRouterError {
            message: format!("HTTP error: {}", err),
        }
    }
}

/// Generate OAuth2 PKCE code verifier and challenge
pub fn generate_pkce_pair() -> (String, String) {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use rand::RngCore;
    use sha2::{Digest, Sha256};

    // Generate code verifier (43-128 characters, base64url encoded)
    let mut verifier_bytes = [0u8; 32];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(&verifier_bytes);

    // Generate code challenge (SHA256 hash of verifier, base64url encoded)
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let challenge_bytes = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(&challenge_bytes);

    (code_verifier, code_challenge)
}

/// Build OAuth2 authorization URL for OpenRouter
pub fn build_oauth_url(
    code_challenge: &str,
    callback_url: &str,
    state: Option<&str>,
    referrer: Option<&str>,
) -> String {
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", "convocations"),
        ("callback_url", callback_url),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
    ];

    if let Some(state_value) = state {
        params.push(("state", state_value));
    }

    if let Some(referrer_value) = referrer {
        params.push(("referrer", referrer_value));
    }

    let query = params
        .into_iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&");

    format!("https://openrouter.ai/auth?{query}")
}

/// Exchange an authorization code for an OpenRouter API key.
pub async fn exchange_code_for_api_key(
    code: &str,
    code_verifier: &str,
) -> Result<String, OpenRouterError> {
    #[derive(Serialize)]
    struct ExchangeRequest<'a> {
        code: &'a str,
        code_verifier: &'a str,
        code_challenge_method: &'a str,
    }

    #[derive(Deserialize)]
    struct ExchangeResponse {
        key: String,
    }

    let client = reqwest::Client::new();
    let response = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&ExchangeRequest {
            code,
            code_verifier,
            code_challenge_method: "S256",
        })
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(OpenRouterError::from(format!(
            "Failed to exchange code: {}",
            response.status()
        )));
    }

    let body: ExchangeResponse = response.json().await?;
    Ok(body.key)
}

/// Fetch the list of available models from OpenRouter API
pub async fn fetch_models() -> Result<Vec<ModelInfo>, OpenRouterError> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(OpenRouterError::from(format!(
            "Failed to fetch models: {}",
            response.status()
        )));
    }

    #[derive(Deserialize)]
    struct ModelsResponse {
        data: Vec<ApiModel>,
    }

    #[derive(Deserialize)]
    struct ApiModel {
        id: String,
        name: Option<String>,
        pricing: ApiPricing,
        context_length: Option<u32>,
    }

    #[derive(Deserialize)]
    struct ApiPricing {
        prompt: String,
        completion: String,
    }

    let body: ModelsResponse = response.json().await?;
    let models = body
        .data
        .into_iter()
        .map(|m| ModelInfo {
            id: m.id.clone(),
            name: m.name.unwrap_or(m.id),
            pricing: ModelPricing {
                prompt: m.pricing.prompt,
                completion: m.pricing.completion,
            },
            context_length: m.context_length,
        })
        .collect();

    Ok(models)
}

/// Filter models based on free/paid preference and preferred providers
pub fn filter_models(models: Vec<ModelInfo>, free_only: bool) -> Vec<ModelInfo> {
    let mut filtered: Vec<ModelInfo> = if free_only {
        models.into_iter().filter(|m| m.is_free()).collect()
    } else {
        models
    };

    // Sort by preferred providers for free models, then alphabetically
    if free_only {
        filtered.sort_by(|a, b| {
            let a_preferred = PREFERRED_FREE_PROVIDERS
                .iter()
                .position(|&p| p == a.provider());
            let b_preferred = PREFERRED_FREE_PROVIDERS
                .iter()
                .position(|&p| p == b.provider());

            match (a_preferred, b_preferred) {
                (Some(a_pos), Some(b_pos)) => a_pos.cmp(&b_pos),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            }
        });
    } else {
        filtered.sort_by(|a, b| a.name.cmp(&b.name));
    }

    filtered
}

/// Send a completion request to OpenRouter
pub async fn complete(
    api_key: &str,
    model: &str,
    prompt: &str,
    temperature: f32,
) -> Result<String, OpenRouterError> {
    #[derive(Serialize)]
    struct CompletionRequest {
        model: String,
        messages: Vec<Message>,
        temperature: f32,
    }

    #[derive(Serialize)]
    struct Message {
        role: String,
        content: String,
    }

    #[derive(Deserialize)]
    struct CompletionResponse {
        choices: Vec<Choice>,
    }

    #[derive(Deserialize)]
    struct Choice {
        message: ResponseMessage,
    }

    #[derive(Deserialize)]
    struct ResponseMessage {
        content: String,
    }

    let client = reqwest::Client::new();

    let request_body = CompletionRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        temperature,
    };

    let response = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(OpenRouterError::from(format!(
            "OpenRouter API error: {}",
            response.status()
        )));
    }

    let completion: CompletionResponse = response.json().await?;

    if let Some(choice) = completion.choices.first() {
        return Ok(choice.message.content.clone());
    }

    Err(OpenRouterError::from("No response content from OpenRouter"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generation() {
        let (verifier, challenge) = generate_pkce_pair();
        assert!(!verifier.is_empty());
        assert!(!challenge.is_empty());
        assert_ne!(verifier, challenge);
    }

    #[test]
    fn test_oauth_url_generation() {
        let url = build_oauth_url(
            "test_challenge",
            "http://localhost:3000/callback",
            Some("abc123"),
            None,
        );
        assert!(url.contains("openrouter.ai/auth"));
        assert!(url.contains("code_challenge=test_challenge"));
        assert!(url.contains("callback_url=http%3A%2F%2Flocalhost%3A3000%2Fcallback"));
        assert!(url.contains("state=abc123"));
    }

    #[test]
    fn test_model_is_free() {
        let free_model = ModelInfo {
            id: "test/free".to_string(),
            name: "Free Model".to_string(),
            pricing: ModelPricing {
                prompt: "0".to_string(),
                completion: "0".to_string(),
            },
            context_length: None,
        };
        assert!(free_model.is_free());

        let paid_model = ModelInfo {
            id: "test/paid".to_string(),
            name: "Paid Model".to_string(),
            pricing: ModelPricing {
                prompt: "0.001".to_string(),
                completion: "0.002".to_string(),
            },
            context_length: None,
        };
        assert!(!paid_model.is_free());
    }

    #[test]
    fn test_model_provider() {
        let model = ModelInfo {
            id: "openai/gpt-4".to_string(),
            name: "GPT-4".to_string(),
            pricing: ModelPricing {
                prompt: "0.03".to_string(),
                completion: "0.06".to_string(),
            },
            context_length: Some(8192),
        };
        assert_eq!(model.provider(), "openai");
    }
}
