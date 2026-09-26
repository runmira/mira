//! TypeSafe **System One** client (Jev).
//!
//! System One models are "fast focused judgments": you send them app *state*
//! plus a set of *typed questions* (Choice / Score / Noul) and they return
//! structured answers — a selected option, a numeric score, or a yes-probability
//! — each carrying `probabilities` and (for Choice/Score) `confidence`. No
//! free text, no prompt-parsing. The harness consumes the answers directly.
//!
//! This client talks HTTP only (`POST /v1/systemone`), following the same
//! reqwest pattern as [`crate::openai`]. It is deliberately **fail-soft by
//! design**: callers check for a missing key (`TypeSafeError::NoApiKey`) or a
//! transport error and fall back to their existing heuristic/LLM path, so a
//! Jev integration is always an enhancement, never a hard dependency.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo() -> Result<(), mira_ai::typesafe::TypeSafeError> {
//! use mira_ai::typesafe::{SystemOneClient, Choice};
//!
//! let client = SystemOneClient::from_env()?; // reads TYPESAFE_API_KEY
//! let answers = client
//!     .ask("Hi, I've been trying to connect my Stripe account for 3 days...")
//!     .with_choice(
//!         "department",
//!         Choice {
//!             instructions: "Which team should handle this".into(),
//!             criteria: [
//!                 ("billing", "Payment or subscription issues"),
//!                 ("technical", "Bugs or integration problems"),
//!             ]
//!             .into_iter()
//!             .map(|(k, v)| (k.to_string(), v.to_string()))
//!             .collect(),
//!         },
//!     )
//!     .send()
//!     .await?;
//!
//! match answers.get("department") {
//!     Some(mira_ai::typesafe::Answer::Choice { choice, confidence, .. }) => {
//!         println!("{choice} (conf {confidence:.2})");
//!     }
//!     _ => unreachable!(),
//! }
//! # Ok(()) }
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default API endpoint.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai/v1";

/// Default model name.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// Wall-clock cap on a single System One call. Jev is a fast model (tens to a
/// couple of hundred ms); a 15s ceiling is generous and keeps a wedged call
/// from stalling an interactive loop.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

// ───────────────────────────── errors ─────────────────────────────

/// Errors a System One call can return.
///
/// Distinguished from "got a bad answer" so callers can fail soft:
/// [`NoApiKey`](TypeSafeError::NoApiKey), [`Http`](TypeSafeError::Http),
/// [`Status`](TypeSafeError::Status), and [`Decode`](TypeSafeError::Decode)
/// all mean "Jev was not usable — fall back". [`MissingAnswer`](TypeSafeError::MissingAnswer)
/// means the call succeeded but did not return a specific question ID (rare).
#[derive(Debug, Error)]
pub enum TypeSafeError {
    /// No API key available (env var unset and none provided).
    #[error("no TypeSafe API key (set TYPESAFE_API_KEY)")]
    NoApiKey,

    /// A transport-level HTTP error (DNS, timeout, connection reset).
    #[error("TypeSafe request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The API returned a non-2xx status.
    #[error("TypeSafe API error (HTTP {code}): {body}")]
    Status { code: u16, body: String },

    /// A 2xx response that could not be decoded.
    #[error("failed to decode TypeSafe response: {0}")]
    Decode(String),

    /// A specific question ID was not present in the response.
    #[error("TypeSafe response missing answer for question `{0}`")]
    MissingAnswer(String),
}

// ───────────────────────────── state ─────────────────────────────

/// Application state sent to the model: the context the judgment is about.
///
/// Strings are the common case. Use JSON when the judgment needs structured
/// state (arrays of tickets, nested objects) — the model references it with
/// backticked paths like `ticket.messages[0].text`.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum State {
    /// Plain text state.
    Text(String),
    /// Structured state (any JSON value).
    Json(serde_json::Value),
}

impl From<&str> for State {
    fn from(s: &str) -> Self {
        State::Text(s.to_string())
    }
}
impl From<String> for State {
    fn from(s: String) -> Self {
        State::Text(s)
    }
}
impl From<serde_json::Value> for State {
    fn from(v: serde_json::Value) -> Self {
        State::Json(v)
    }
}

// ───────────────────────────── questions ─────────────────────────────

/// A **Choice** question: select one option from a defined set.
///
/// `criteria` maps option id → description. The response gives the selected
/// option, a probability for each option, and a confidence.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Choice {
    /// What the question is deciding. Be self-contained — question IDs are
    /// not sent to the model.
    pub instructions: String,
    /// Option id → human description of when that option applies.
    #[serde(rename = "criteria")]
    pub criteria: BTreeMap<String, String>,
}

/// A **Score** question: rate state against ordered, descriptive levels.
///
/// `criteria` is an *ordered list* (index 0 = lowest). The response gives a
/// float score, a probability per level, a legend, and a confidence.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Score {
    /// What is being rated.
    pub instructions: String,
    /// Ordered level descriptions, lowest to highest.
    #[serde(rename = "criteria")]
    pub criteria: Vec<String>,
}

/// A **Noul** question: a yes/no judgment. The response is the probability
/// that the answer is yes (0.0–1.0).
///
/// A Noul near 0.5 means the model sees roughly equal probability of yes and
/// no — *not* "medium intensity".
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Noul {
    /// The statement being judged. Phrase it so "yes" means the statement
    /// is true.
    pub instructions: String,
}

/// Any question, keyed by id in the request.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Choice(Choice),
    Score(Score),
    Noul(Noul),
}

impl From<Choice> for Question {
    fn from(q: Choice) -> Self {
        Question::Choice(q)
    }
}
impl From<Score> for Question {
    fn from(q: Score) -> Self {
        Question::Score(q)
    }
}
impl From<Noul> for Question {
    fn from(q: Noul) -> Self {
        Question::Noul(q)
    }
}

// ───────────────────────────── request / response ─────────────────────────────

/// The wire request body.
#[derive(Clone, Debug, Serialize)]
struct Request<'a> {
    state: &'a State,
    model: &'a str,
    questions: &'a BTreeMap<String, Question>,
}

/// A typed System One answer.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// Selected one option from the set.
    Choice {
        /// The winning option id.
        choice: String,
        /// Probability per option id.
        probabilities: BTreeMap<String, f64>,
        /// 0–1 measure of how concentrated the distribution is.
        confidence: f64,
    },
    /// Rated state against ordered levels.
    Score {
        /// Float score over the level range.
        score: f64,
        /// Level index → level description.
        legend: BTreeMap<String, String>,
        /// Probability per level index. (Some responses omit this — e.g. the
        /// documented sample — so it defaults to empty.)
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        /// 0–1 measure of how concentrated the distribution is.
        #[serde(default)]
        confidence: f64,
    },
    /// Yes/no probability.
    Noul {
        /// Probability that the statement is true (0.0–1.0).
        noul: f64,
    },
}

impl Answer {
    /// Convenience: the winning option id for a Choice, else `None`.
    pub fn choice(&self) -> Option<&str> {
        match self {
            Answer::Choice { choice, .. } => Some(choice),
            _ => None,
        }
    }

    /// Convenience: confidence for Choice/Score (Noul has none).
    pub fn confidence(&self) -> Option<f64> {
        match self {
            Answer::Choice { confidence, .. } => Some(*confidence),
            Answer::Score { confidence, .. } => Some(*confidence),
            Answer::Noul { .. } => None,
        }
    }
}

/// The wire response body. `model` and `usage` are ignored (serde skips
/// unknown fields) so added fields can't break decode.
#[derive(Debug, Deserialize)]
struct Response {
    answers: BTreeMap<String, Answer>,
}

// ───────────────────────────── client ─────────────────────────────

/// A System One (Jev) client. Cheap to construct; build one and reuse it.
pub struct SystemOneClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl SystemOneClient {
    /// Build with an explicit key and the default endpoint/model.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Build with an explicit key and a custom base URL (e.g. for a proxy).
    /// `base_url` must not include the trailing `/systemone`.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
                .timeout(DEFAULT_TIMEOUT)
                .build()
                .expect("valid reqwest ClientBuilder"),
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    /// Build from the `TYPESAFE_API_KEY` env var and the default endpoint.
    /// Returns [`TypeSafeError::NoApiKey`] if it's unset/empty.
    pub fn from_env() -> Result<Self, TypeSafeError> {
        let key = std::env::var("TYPESAFE_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or(TypeSafeError::NoApiKey)?;
        Ok(Self::new(key))
    }

    /// Use a specific model name instead of the default (`jev-latest`).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(auth) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
            h.insert(AUTHORIZATION, auth);
        }
        h
    }

    /// Start a call over the given `state` (a `&str`/`String` or JSON value).
    /// Chain `.with_*` to add questions, then `.send()`.
    pub fn ask(&self, state: impl Into<State>) -> Call<'_> {
        Call {
            client: self,
            state: state.into(),
            questions: BTreeMap::new(),
        }
    }
}

/// A single in-flight System One call being built up with questions.
pub struct Call<'a> {
    client: &'a SystemOneClient,
    state: State,
    questions: BTreeMap<String, Question>,
}

impl<'a> Call<'a> {
    /// Add a Choice question under `id`.
    pub fn with_choice(mut self, id: impl Into<String>, q: Choice) -> Self {
        self.questions.insert(id.into(), q.into());
        self
    }

    /// Add a Score question under `id`.
    pub fn with_score(mut self, id: impl Into<String>, q: Score) -> Self {
        self.questions.insert(id.into(), q.into());
        self
    }

    /// Add a Noul question under `id`.
    pub fn with_noul(mut self, id: impl Into<String>, q: Noul) -> Self {
        self.questions.insert(id.into(), q.into());
        self
    }

    /// Send the request and return all answers keyed by question id.
    pub async fn send(self) -> Result<BTreeMap<String, Answer>, TypeSafeError> {
        if self.questions.is_empty() {
            return Err(TypeSafeError::Decode("no questions provided".into()));
        }
        let body = Request {
            state: &self.state,
            model: &self.client.model,
            questions: &self.questions,
        };
        let url = format!("{}/systemone", self.client.base_url);
        let resp = self
            .client
            .http
            .post(&url)
            .headers(self.client.headers())
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(TypeSafeError::Status { code, body });
        }

        let raw: Response = resp
            .json()
            .await
            .map_err(|e| TypeSafeError::Decode(e.to_string()))?;
        Ok(raw.answers)
    }

    /// Send and require a specific question id, unwrapping it.
    pub async fn send_one(self, id: &str) -> Result<Answer, TypeSafeError> {
        let answers = self.send().await?;
        answers
            .get(id)
            .cloned()
            .ok_or_else(|| TypeSafeError::MissingAnswer(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn choice() -> Choice {
        let mut criteria = BTreeMap::new();
        criteria.insert("billing".into(), "Payment or subscription issues".into());
        criteria.insert("technical".into(), "Bugs or integration problems".into());
        criteria.insert("sales".into(), "Pricing or account questions".into());
        Choice {
            instructions: "Which team should handle this".into(),
            criteria,
        }
    }

    fn score() -> Score {
        Score {
            instructions: "How frustrated the customer appears".into(),
            criteria: vec![
                "Calm, just stating facts".into(),
                "Frustrated but civil".into(),
                "Very angry, strong language".into(),
            ],
        }
    }

    fn noul() -> Noul {
        Noul {
            instructions: "The message conveys urgency or time-sensitivity".into(),
        }
    }

    #[test]
    fn request_serializes_all_three_types() {
        let state = State::Text("a message".into());
        let mut questions = BTreeMap::new();
        questions.insert("department".into(), Question::Choice(choice()));
        questions.insert("frustration".into(), Question::Score(score()));
        questions.insert("is_urgent".into(), Question::Noul(noul()));
        let body = Request {
            state: &state,
            model: DEFAULT_MODEL,
            questions: &questions,
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["state"], "a message");
        assert_eq!(v["model"], DEFAULT_MODEL);
        // tag + snake_case
        assert_eq!(v["questions"]["department"]["type"], "choice");
        assert_eq!(
            v["questions"]["department"]["instructions"],
            "Which team should handle this"
        );
        assert_eq!(
            v["questions"]["department"]["criteria"]["billing"],
            "Payment or subscription issues"
        );
        assert_eq!(v["questions"]["frustration"]["type"], "score");
        assert!(v["questions"]["frustration"]["criteria"].is_array());
        assert_eq!(
            v["questions"]["frustration"]["criteria"][0],
            "Calm, just stating facts"
        );
        assert_eq!(v["questions"]["is_urgent"]["type"], "noul");
    }

    #[test]
    fn json_state_serializes_as_structured() {
        let state = State::Json(json!({"ticket": {"messages": [{"text": "hi"}]}}));
        let body = Request {
            state: &state,
            model: DEFAULT_MODEL,
            questions: &BTreeMap::new(),
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["state"]["ticket"]["messages"][0]["text"], "hi");
    }

    /// The sample response from the TypeSafe quickstart docs. Decoding this is
    /// the wire contract we depend on.
    const SAMPLE_RESPONSE: &str = r#"{
        "model": "jev-latest",
        "answers": {
            "department": {
                "type": "choice",
                "choice": "technical",
                "probabilities": {"billing": 0.159, "technical": 0.84, "sales": 0.001},
                "confidence": 0.596
            },
            "frustration": {
                "type": "score",
                "score": 1.035,
                "legend": {
                    "0": "Calm, just stating facts",
                    "1": "Frustrated but civil",
                    "2": "Very angry, strong language"
                },
                "confidence": 0.842
            },
            "is_urgent": {
                "type": "noul",
                "noul": 0.999
            }
        },
        "usage": {"input_tokens": 312, "output_tokens": 48}
    }"#;

    #[test]
    fn decodes_sample_response() {
        let r: Response = serde_json::from_str(SAMPLE_RESPONSE).unwrap();
        let dept = match r.answers.get("department").unwrap() {
            Answer::Choice {
                choice,
                probabilities,
                confidence,
            } => (choice.clone(), probabilities.clone(), *confidence),
            _ => panic!("expected choice"),
        };
        assert_eq!(dept.0, "technical");
        assert!((dept.1["technical"] - 0.84).abs() < 1e-9);
        assert!((dept.2 - 0.596).abs() < 1e-9);

        let frust = match r.answers.get("frustration").unwrap() {
            Answer::Score {
                score,
                legend,
                probabilities,
                confidence,
            } => (score, legend, probabilities, confidence),
            _ => panic!("expected score"),
        };
        assert!((frust.0 - 1.035).abs() < 1e-9);
        assert_eq!(frust.1["2"], "Very angry, strong language");
        // The sample omits per-level probabilities; they default to empty.
        assert!(frust.2.is_empty());
        assert!((frust.3 - 0.842).abs() < 1e-9);

        let urgent = match r.answers.get("is_urgent").unwrap() {
            Answer::Noul { noul } => *noul,
            _ => panic!("expected noul"),
        };
        assert!((urgent - 0.999).abs() < 1e-9);
    }

    #[test]
    fn answer_helpers() {
        let c = Answer::Choice {
            choice: "technical".into(),
            probabilities: BTreeMap::new(),
            confidence: 0.596,
        };
        assert_eq!(c.choice(), Some("technical"));
        assert!((c.confidence().unwrap() - 0.596).abs() < 1e-9);
        let n = Answer::Noul { noul: 0.9 };
        assert_eq!(n.choice(), None);
        assert_eq!(n.confidence(), None);
    }

    #[test]
    fn from_env_missing_key_is_no_api_key() {
        // SAFETY: test-isolated env mutation.
        #[allow(unused_unsafe)]
        unsafe {
            std::env::remove_var("TYPESAFE_API_KEY");
        }
        matches!(SystemOneClient::from_env(), Err(TypeSafeError::NoApiKey));
    }

    /// Live end-to-end check. Skipped unless TYPESAFE_API_KEY is set.
    #[tokio::test]
    async fn live_call_skipped_without_key() {
        let key = std::env::var("TYPESAFE_API_KEY").unwrap_or_default();
        if key.trim().is_empty() {
            eprintln!("SKIP live_call: TYPESAFE_API_KEY not set");
            return;
        }
        let client = SystemOneClient::new(key);
        let answers = client
            .ask("Hi, I've been trying to connect my Stripe account for 3 days and it keeps failing. I'm losing sales. Please help ASAP.")
            .with_noul(
                "is_urgent",
                Noul {
                    instructions: "The message conveys urgency or time-sensitivity".into(),
                },
            )
            .send()
            .await
            .expect("live call");
        match answers.get("is_urgent") {
            Some(Answer::Noul { noul }) => {
                assert!((0.0f64..=1.0f64).contains(noul));
            }
            other => panic!("unexpected answer: {other:?}"),
        }
    }
}
