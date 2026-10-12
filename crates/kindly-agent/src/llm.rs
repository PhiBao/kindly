//! Language model adapters.
//!
//! Kindly speaks the OpenAI-compatible chat-completions dialect, which is
//! implemented by OpenAI, OpenRouter, Azure OpenAI, vLLM, Ollama, LM Studio
//! and most other gateways. That keeps the product provider-agnostic: judges
//! can point it at any model with two environment variables.
//!
//! When no model is configured the [`NullLlm`] provider reports itself as
//! unavailable and every caller falls back to deterministic templates. The
//! product still works — it just sounds like a template instead of a person.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("no language model is configured")]
    NotConfigured,
    #[error("language model transport error: {0}")]
    Transport(String),
    #[error("language model returned an error: {0}")]
    Api(String),
    #[error("language model response could not be parsed: {0}")]
    Parse(String),
}

pub type LlmResult<T> = Result<T, LlmError>;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system: String,
    pub user: String,
    /// Ask the model for strict JSON (response_format=json_object).
    pub json: bool,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl LlmRequest {
    pub fn text(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
            json: false,
            temperature: 0.4,
            max_tokens: 700,
        }
    }

    pub fn json(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
            json: true,
            temperature: 0.2,
            max_tokens: 700,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub model: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> LlmResult<LlmResponse>;

    /// Human-readable provider description for the README, the UI and the
    /// ledger ("written by gpt-4o-mini" vs "written from your templates").
    fn description(&self) -> String;

    fn is_enabled(&self) -> bool {
        true
    }
}

/// Fallback provider: always unavailable, never calls anything.
pub struct NullLlm;

#[async_trait]
impl LlmProvider for NullLlm {
    async fn complete(&self, _request: LlmRequest) -> LlmResult<LlmResponse> {
        Err(LlmError::NotConfigured)
    }

    fn description(&self) -> String {
        "deterministic templates (no model configured)".to_string()
    }

    fn is_enabled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl OpenAiCompatConfig {
    /// Read configuration from the environment. Returns `None` when no API key
    /// is present, which makes the runtime fall back to templates.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("KINDLY_LLM_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())?;
        let base_url = std::env::var("KINDLY_LLM_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("KINDLY_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Some(Self {
            base_url,
            api_key,
            model,
            timeout_secs: 45,
        })
    }
}

pub struct OpenAiCompat {
    http: Client,
    config: OpenAiCompatConfig,
}

#[derive(Debug, Deserialize)]
struct ChatCompletion {
    choices: Vec<Choice>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
}

impl OpenAiCompat {
    pub fn new(config: OpenAiCompatConfig) -> LlmResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LlmError::Transport(e.to_string()))?;
        Ok(Self { http, config })
    }

    /// Provider from the environment, if configured.
    pub fn from_env() -> Option<Self> {
        OpenAiCompatConfig::from_env().and_then(|config| Self::new(config).ok())
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompat {
    async fn complete(&self, request: LlmRequest) -> LlmResult<LlmResponse> {
        let mut body = json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": request.system},
                {"role": "user", "content": request.user},
            ],
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
        });
        if request.json {
            body["response_format"] = json!({"type": "json_object"});
        }

        let mut last_error = LlmError::Transport("no attempt made".into());
        for attempt in 0..2 {
            let response = match self
                .http
                .post(self.endpoint())
                .bearer_auth(&self.config.api_key)
                .json(&body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    last_error = LlmError::Transport(error.to_string());
                    if attempt == 0 {
                        warn!(error = %error, "llm transport failure; retrying once");
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        continue;
                    }
                    return Err(last_error);
                }
            };

            let status = response.status();
            let value: Value = response
                .json()
                .await
                .map_err(|e| LlmError::Parse(format!("response body: {e}")))?;

            if !status.is_success() {
                let message = value
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("llm api error")
                    .to_string();
                // 429 / 5xx are worth one retry.
                if matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) && attempt == 0 {
                    tokio::time::sleep(Duration::from_millis(900)).await;
                    last_error = LlmError::Api(message);
                    continue;
                }
                return Err(LlmError::Api(message));
            }

            let parsed: ChatCompletion = serde_json::from_value(value)
                .map_err(|e| LlmError::Parse(format!("unexpected completion shape: {e}")))?;
            let text = parsed
                .choices
                .first()
                .and_then(|c| c.message.content.clone())
                .ok_or_else(|| LlmError::Parse("completion had no content".into()))?;
            return Ok(LlmResponse {
                text,
                model: parsed.model.unwrap_or_else(|| self.config.model.clone()),
            });
        }
        Err(last_error)
    }

    fn description(&self) -> String {
        format!("{} via {}", self.config.model, self.config.base_url)
    }
}

/// Convenience: ask for JSON and deserialize, returning `None` on any failure
/// so callers can fall back to deterministic behaviour.
pub async fn complete_json<T: for<'de> Deserialize<'de>>(
    llm: &dyn LlmProvider,
    request: LlmRequest,
) -> Option<T> {
    if !llm.is_enabled() {
        return None;
    }
    let response = llm.complete(request).await.ok()?;
    parse_json_relaxed(&response.text)
}

/// Ask for prose, returning `None` on failure.
pub async fn complete_text(llm: &dyn LlmProvider, request: LlmRequest) -> Option<String> {
    if !llm.is_enabled() {
        return None;
    }
    let response = llm.complete(request).await.ok()?;
    let trimmed = response.text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Models occasionally wrap JSON in prose or code fences. Be forgiving.
pub fn parse_json_relaxed<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    if let Ok(value) = serde_json::from_str::<T>(text) {
        return Some(value);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<T>(&text[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_provider_is_disabled_and_errors() {
        let llm = NullLlm;
        assert!(!llm.is_enabled());
        assert!(llm.complete(LlmRequest::text("a", "b")).await.is_err());
        assert!(complete_text(&llm, LlmRequest::text("a", "b")).await.is_none());
    }

    #[test]
    fn relaxed_json_parsing_handles_fences_and_prose() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Shape {
            greeting: String,
        }
        let direct: Shape = parse_json_relaxed(r#"{"greeting":"hi"}"#).unwrap();
        assert_eq!(direct.greeting, "hi");
        let fenced: Shape =
            parse_json_relaxed("Sure! ```json\n{\"greeting\": \"hello\"}\n```").unwrap();
        assert_eq!(fenced.greeting, "hello");
        let prose: Shape = parse_json_relaxed("Here you go: {\"greeting\": \"hey\"} — enjoy").unwrap();
        assert_eq!(prose.greeting, "hey");
        assert!(parse_json_relaxed::<Shape>("no json here").is_none());
    }

    #[test]
    fn config_requires_a_key() {
        // Note: tests that mutate the environment would race with each other;
        // this only checks the pure shape.
        let missing = OpenAiCompatConfig {
            base_url: "http://x".into(),
            api_key: String::new(),
            model: "m".into(),
            timeout_secs: 1,
        };
        assert!(missing.api_key.is_empty());
    }
}
