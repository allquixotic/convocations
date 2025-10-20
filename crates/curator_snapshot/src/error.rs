use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CuratorError {
    #[error("{0}")]
    Message(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("OpenRouter error: {0}")]
    OpenRouter(#[from] openrouter_rs::error::OpenRouterError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl CuratorError {
    pub fn message<T: Into<String>>(message: T) -> Self {
        CuratorError::Message(message.into())
    }
}
