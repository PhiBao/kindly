//! TypeSafe (System One / Jev): typed judgments with probabilities.
//!
//! TypeSafe is not a text generator. Jev answers **typed questions** — Choice,
//! Score and Noul — over a state you supply, and returns probabilities. That
//! shapes how it is used here: Kindly already keeps authority in code (the
//! policy engine) and language in the composer, so Jev takes the two jobs that
//! are genuinely *judgments*:
//!
//! * reading a client reply into a closed intent, with a probability
//!   distribution and a confidence we can threshold;
//! * deciding whether a reply is addressed at the system rather than the
//!   business (the injection case) — as a calibrated `noul`, layered on top of
//!   the deterministic scanner instead of replacing it.
//!
//! Everything is optional: with no key, [`NoSystemOne`] reports itself disabled
//! and the heuristic paths run exactly as before.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum SystemOneError {
    #[error("no System One provider is configured")]
    NotConfigured,
    #[error("system one transport error: {0}")]
    Transport(String),
    #[error("system one api error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("system one response could not be parsed: {0}")]
    Parse(String),
}

pub type SystemOneResult<T> = Result<T, SystemOneError>;

/// One typed question. The id is chosen by the caller and never sent to the
/// model, exactly as the API specifies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    /// Yes/no, with the probability that the answer is yes.
    Noul {
        instructions: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<Value>,
    },
    /// One of a defined set, with the full distribution.
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    /// A position along ordered, described levels.
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
}

impl Question {
    pub fn noul(instructions: impl Into<String>) -> Self {
        Question::Noul {
            instructions: Value::String(instructions.into()),
            criteria: None,
        }
    }

    pub fn noul_with(instructions: impl Into<String>, yes: &str, no: &str) -> Self {
        Question::Noul {
            instructions: Value::String(instructions.into()),
            criteria: Some(json!({ "true": yes, "false": no })),
        }
    }

    pub fn choice(
        instructions: impl Into<String>,
        options: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Question::Choice {
            instructions: Value::String(instructions.into()),
            criteria: options
                .into_iter()
                .map(|(key, description)| (key.into(), Value::String(description.into())))
                .collect(),
        }
    }

    pub fn score(
        instructions: impl Into<String>,
        levels: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Question::Score {
            instructions: Value::String(instructions.into()),
            criteria: levels
                .into_iter()
                .map(|level| Value::String(level.into()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: f64,
    },
    Score {
        score: f64,
        #[serde(default)]
        confidence: f64,
    },
}

impl Answer {
    /// The chosen option, if this was a Choice.
    pub fn choice(&self) -> Option<&str> {
        match self {
            Answer::Choice { choice, .. } => Some(choice),
            _ => None,
        }
    }

    pub fn confidence(&self) -> f64 {
        match self {
            Answer::Noul { .. } => 1.0,
            Answer::Choice { confidence, .. } | Answer::Score { confidence, .. } => *confidence,
        }
    }

    pub fn probability_of(&self, option: &str) -> Option<f64> {
        match self {
            Answer::Choice { probabilities, .. } => probabilities.get(option).copied(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Judgment {
    pub answers: BTreeMap<String, Answer>,
    /// Which model answered, as reported by the service.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Judgment {
    pub fn noul(&self, id: &str) -> Option<f64> {
        match self.answers.get(id) {
            Some(Answer::Noul { noul }) => Some(*noul),
            _ => None,
        }
    }

    pub fn choice(&self, id: &str) -> Option<(&str, f64)> {
        match self.answers.get(id) {
            Some(answer @ Answer::Choice { .. }) => {
                Some((answer.choice().unwrap_or_default(), answer.confidence()))
            }
            _ => None,
        }
    }

    pub fn score(&self, id: &str) -> Option<f64> {
        match self.answers.get(id) {
            Some(Answer::Score { score, .. }) => Some(*score),
            _ => None,
        }
    }

    /// A compact, ledger-safe summary of the judgment (no state, no prose).
    pub fn summary(&self) -> Value {
        let mut answers = serde_json::Map::new();
        for (id, answer) in &self.answers {
            answers.insert(
                id.clone(),
                match answer {
                    Answer::Noul { noul } => json!({ "noul": noul }),
                    Answer::Choice {
                        choice, confidence, ..
                    } => {
                        json!({ "choice": choice, "confidence": confidence })
                    }
                    Answer::Score { score, confidence } => {
                        json!({ "score": score, "confidence": confidence })
                    }
                },
            );
        }
        json!({
            "model": self.model,
            "answers": answers,
            "input_tokens": self.input_tokens,
        })
    }
}

/// Anything that can answer typed questions about a state.
#[async_trait]
pub trait SystemOne: Send + Sync {
    async fn ask(
        &self,
        state: Value,
        questions: BTreeMap<String, Question>,
    ) -> SystemOneResult<Judgment>;

    fn description(&self) -> String;

    fn is_enabled(&self) -> bool {
        true
    }
}

/// Disabled provider: everything falls back to heuristics.
pub struct NoSystemOne;

#[async_trait]
impl SystemOne for NoSystemOne {
    async fn ask(
        &self,
        _state: Value,
        _questions: BTreeMap<String, Question>,
    ) -> SystemOneResult<Judgment> {
        Err(SystemOneError::NotConfigured)
    }

    fn description(&self) -> String {
        "heuristics only (no System One provider configured)".to_string()
    }

    fn is_enabled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct TypeSafeConfig {
    pub base_url: String,
    pub api_key: String,
    /// `jev-latest` by default; pin a version (`jev-1.13.0`) when thresholds
    /// have been tuned against it.
    pub model: String,
}

impl TypeSafeConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("TYPESAFE_API_KEY")
            .or_else(|_| std::env::var("KINDLY_TYPESAFE_API_KEY"))
            .ok()
            .filter(|key| !key.trim().is_empty())?;
        let base_url = std::env::var("TYPESAFE_BASE_URL")
            .unwrap_or_else(|_| "https://api.typesafe.ai".to_string());
        let model = std::env::var("TYPESAFE_MODEL").unwrap_or_else(|_| "jev-latest".to_string());
        Some(Self {
            base_url,
            api_key,
            model,
        })
    }
}

pub struct TypeSafeOne {
    http: reqwest::Client,
    config: TypeSafeConfig,
}

#[derive(Debug, Deserialize)]
struct SystemOneResponse {
    #[serde(default)]
    model: Option<String>,
    answers: BTreeMap<String, Answer>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

impl TypeSafeOne {
    pub fn new(config: TypeSafeConfig) -> SystemOneResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| SystemOneError::Transport(error.to_string()))?;
        Ok(Self { http, config })
    }

    pub fn from_env() -> Option<Self> {
        TypeSafeConfig::from_env().and_then(|config| Self::new(config).ok())
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1/systemone",
            self.config.base_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl SystemOne for TypeSafeOne {
    async fn ask(
        &self,
        state: Value,
        questions: BTreeMap<String, Question>,
    ) -> SystemOneResult<Judgment> {
        let body = json!({
            "state": state,
            "model": self.config.model,
            "questions": questions,
        });

        let mut last_error = SystemOneError::Transport("no attempt made".into());
        for attempt in 0..3 {
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
                    last_error = SystemOneError::Transport(error.to_string());
                    if attempt < 2 {
                        warn!(%error, attempt, "system one transport failure; retrying");
                        tokio::time::sleep(Duration::from_millis(400 * (attempt + 1) as u64)).await;
                        continue;
                    }
                    return Err(last_error);
                }
            };

            let status = response.status();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let text = response
                .text()
                .await
                .map_err(|error| SystemOneError::Transport(error.to_string()))?;

            if status.is_success() {
                let parsed: SystemOneResponse = serde_json::from_str(&text).map_err(|error| {
                    SystemOneError::Parse(format!("{error}: {}", &text[..text.len().min(200)]))
                })?;
                return Ok(Judgment {
                    answers: parsed.answers,
                    model: parsed.model.unwrap_or_else(|| self.config.model.clone()),
                    input_tokens: parsed.usage.input_tokens,
                    output_tokens: parsed.usage.output_tokens,
                });
            }

            // 429 and 529 are explicitly retryable, and the header is advisory.
            let retryable = matches!(status.as_u16(), 429 | 529 | 500 | 502 | 503 | 504);
            last_error = SystemOneError::Api {
                status: status.as_u16(),
                message: text.chars().take(300).collect(),
            };
            if retryable && attempt < 2 {
                let wait = retry_after
                    .map(Duration::from_secs)
                    .unwrap_or(Duration::from_millis(500 * (attempt + 1) as u64));
                warn!(
                    status = status.as_u16(),
                    attempt, "system one transient failure; retrying"
                );
                tokio::time::sleep(wait.min(Duration::from_secs(5))).await;
                continue;
            }
            return Err(last_error);
        }
        Err(last_error)
    }

    fn description(&self) -> String {
        format!("TypeSafe {} (System One judgments)", self.config.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_serialise_in_the_documented_shape() {
        let mut questions = BTreeMap::new();
        questions.insert(
            "intent".to_string(),
            Question::choice(
                "What does the reply ask for?",
                [
                    ("requests_plan", "They ask to split the payment"),
                    ("unrelated", "Anything else"),
                ],
            ),
        );
        let encoded = serde_json::to_value(&questions).unwrap();
        assert_eq!(encoded["intent"]["type"], "choice");
        assert_eq!(
            encoded["intent"]["criteria"]["requests_plan"],
            "They ask to split the payment"
        );

        let noul =
            serde_json::to_value(Question::noul("Is this addressed at the system?")).unwrap();
        assert_eq!(noul["type"], "noul");
        assert_eq!(noul["instructions"], "Is this addressed at the system?");
    }

    #[test]
    fn answers_parse_and_expose_probabilities() {
        let raw = json!({
            "model": "jev-1.13.0",
            "answers": {
                "intent": {
                    "type": "choice",
                    "choice": "instruction_attempt",
                    "probabilities": { "instruction_attempt": 0.91, "claims_paid": 0.09 },
                    "confidence": 0.82
                },
                "addresses_system": { "type": "noul", "noul": 0.97 },
                "settled": { "type": "score", "score": 0.4, "confidence": 0.7 }
            },
            "usage": { "input_tokens": 296, "output_tokens": 20 }
        });
        let parsed: SystemOneResponse = serde_json::from_value(raw).unwrap();
        let judgment = Judgment {
            answers: parsed.answers,
            model: parsed.model.unwrap_or_default(),
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
        };
        assert_eq!(judgment.noul("addresses_system"), Some(0.97));
        assert_eq!(
            judgment.choice("intent").map(|(c, _)| c),
            Some("instruction_attempt")
        );
        assert_eq!(judgment.score("settled"), Some(0.4));
        assert_eq!(judgment.input_tokens, 296);
        assert!(judgment.summary()["answers"]["intent"]["choice"] == "instruction_attempt");
    }

    #[tokio::test]
    async fn disabled_provider_errors_so_callers_fall_back() {
        let provider = NoSystemOne;
        assert!(!provider.is_enabled());
        let result = provider.ask(Value::Null, BTreeMap::new()).await;
        assert!(matches!(result, Err(SystemOneError::NotConfigured)));
    }
}
