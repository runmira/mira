//! Retries for transient provider failures.
//!
//! [`Retrying`] wraps any [`ChatProvider`]. A request that fails with a
//! rate limit (429), an overload or server error (5xx, Anthropic's 529),
//! a timeout or a dropped connection is tried again with exponential
//! backoff, honouring `Retry-After`. A stream that fails before it
//! produced anything counts as a failed request too. Once output has
//! reached the caller the stream is passed through as is: retrying then
//! would repeat text the user has already seen.
//!
//! `MIRA_PROVIDER_RETRIES` sets how many retries (default 4; 0 turns
//! retrying off).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use tracing::warn;

use crate::event::ChatEvent;
use crate::provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError};

/// How hard to try.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// First backoff; doubles each retry.
    pub base_delay: Duration,
    /// Cap on any single wait, `Retry-After` included.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 4,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// The default, with `MIRA_PROVIDER_RETRIES` applied.
    pub fn from_env() -> Self {
        let mut p = Self::default();
        if let Some(n) = std::env::var("MIRA_PROVIDER_RETRIES")
            .ok()
            .and_then(|v| v.trim().parse().ok())
        {
            p.max_retries = n;
        }
        p
    }

    fn delay(&self, retry: u32, err: &ProviderError) -> Duration {
        let hinted = match err {
            ProviderError::Status {
                retry_after: Some(s),
                ..
            } => Some(Duration::from_secs(*s)),
            _ => None,
        };
        let backoff = self.base_delay.saturating_mul(1u32 << retry.min(16));
        hinted.unwrap_or(backoff).min(self.max_delay)
    }
}

/// True for failures that may go away on their own.
pub fn is_transient(err: &ProviderError) -> bool {
    match err {
        ProviderError::Http(e) => e.is_timeout() || e.is_connect() || e.is_request() || e.is_body(),
        ProviderError::Status { status, body, .. } => match status {
            408 | 409 | 425 | 429 | 500 | 502 | 503 | 504 | 529 => true,
            // Anthropic reports overload and rate limits mid-stream,
            // after a 200.
            200 => ["overloaded", "rate_limit", "api_error"]
                .iter()
                .any(|k| body.contains(k)),
            _ => false,
        },
        ProviderError::Decode(_) | ProviderError::Config(_) => false,
    }
}

/// A provider that retries transient failures of the one it wraps.
pub struct Retrying {
    inner: Arc<dyn ChatProvider>,
    policy: RetryPolicy,
}

impl Retrying {
    pub fn new(inner: Arc<dyn ChatProvider>, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }
}

type EventStream = BoxStream<'static, Result<ChatEvent, ProviderError>>;

#[async_trait]
impl ChatProvider for Retrying {
    async fn stream(&self, request: ChatRequest) -> Result<EventStream, ProviderError> {
        let mut retry = 0;
        loop {
            let err = match self.inner.stream(request.clone()).await {
                Ok(mut s) => match s.next().await {
                    // Output is flowing: hand it over, first item included.
                    Some(Ok(first)) => return Ok(stream::iter([Ok(first)]).chain(s).boxed()),
                    Some(Err(e)) => e,
                    None => return Ok(stream::empty().boxed()),
                },
                Err(e) => e,
            };
            if retry >= self.policy.max_retries || !is_transient(&err) {
                return Err(err);
            }
            let wait = self.policy.delay(retry, &err);
            warn!(%err, retry = retry + 1, wait_secs = wait.as_secs_f32(), "provider: retrying");
            tokio::time::sleep(wait).await;
            retry += 1;
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut retry = 0;
        loop {
            match self.inner.list_models().await {
                Err(e) if retry < self.policy.max_retries && is_transient(&e) => {
                    tokio::time::sleep(self.policy.delay(retry, &e)).await;
                    retry += 1;
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::event::FinishReason;

    /// Plays back one scripted outcome per call.
    struct Script {
        calls: Mutex<Vec<Outcome>>,
        made: Mutex<u32>,
    }

    enum Outcome {
        Fail(u16),
        /// Starts streaming, then drops with a transient error.
        Drop {
            after_text: bool,
        },
        Ok,
    }

    fn status(code: u16) -> ProviderError {
        ProviderError::Status {
            status: code,
            body: "nope".into(),
            retry_after: None,
        }
    }

    #[async_trait]
    impl ChatProvider for Script {
        async fn stream(&self, _r: ChatRequest) -> Result<EventStream, ProviderError> {
            *self.made.lock().unwrap() += 1;
            let next = self.calls.lock().unwrap().remove(0);
            let items: Vec<Result<ChatEvent, ProviderError>> = match next {
                Outcome::Fail(code) => return Err(status(code)),
                Outcome::Drop { after_text } => {
                    let mut v = Vec::new();
                    if after_text {
                        v.push(Ok(ChatEvent::TextDelta("partial".into())));
                    }
                    v.push(Err(status(503)));
                    v
                }
                Outcome::Ok => vec![
                    Ok(ChatEvent::TextDelta("hi".into())),
                    Ok(ChatEvent::Done(FinishReason::Stop)),
                ],
            };
            Ok(stream::iter(items).boxed())
        }
    }

    fn fast() -> RetryPolicy {
        RetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        }
    }

    fn provider(calls: Vec<Outcome>) -> (Arc<Script>, Retrying) {
        let script = Arc::new(Script {
            calls: Mutex::new(calls),
            made: Mutex::new(0),
        });
        let r = Retrying::new(script.clone(), fast());
        (script, r)
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            response_format: None,
        }
    }

    async fn text(r: &Retrying) -> Result<String, ProviderError> {
        let mut s = r.stream(request()).await?;
        let mut out = String::new();
        while let Some(e) = s.next().await {
            if let ChatEvent::TextDelta(t) = e? {
                out.push_str(&t);
            }
        }
        Ok(out)
    }

    #[tokio::test]
    async fn retries_rate_limits_and_outages() {
        let (script, r) = provider(vec![
            Outcome::Fail(429),
            Outcome::Fail(529),
            Outcome::Drop { after_text: false },
            Outcome::Ok,
        ]);
        assert_eq!(text(&r).await.unwrap(), "hi");
        assert_eq!(*script.made.lock().unwrap(), 4);
    }

    #[tokio::test]
    async fn gives_up_on_bad_requests_and_after_the_limit() {
        let (script, r) = provider(vec![Outcome::Fail(400), Outcome::Ok]);
        assert!(text(&r).await.is_err());
        assert_eq!(*script.made.lock().unwrap(), 1);

        let (script, r) = provider((0..5).map(|_| Outcome::Fail(503)).collect());
        assert!(text(&r).await.is_err());
        assert_eq!(*script.made.lock().unwrap(), 4, "first try + 3 retries");
    }

    #[tokio::test]
    async fn never_replays_output_the_caller_has_seen() {
        let (script, r) = provider(vec![Outcome::Drop { after_text: true }, Outcome::Ok]);
        let err = text(&r).await.unwrap_err();
        assert!(matches!(err, ProviderError::Status { status: 503, .. }));
        assert_eq!(*script.made.lock().unwrap(), 1);
    }

    #[test]
    fn honours_retry_after_up_to_the_cap() {
        let p = RetryPolicy::default();
        let hinted = ProviderError::Status {
            status: 429,
            body: String::new(),
            retry_after: Some(7),
        };
        assert_eq!(p.delay(0, &hinted), Duration::from_secs(7));
        assert_eq!(p.delay(2, &status(503)), Duration::from_secs(4));
        assert_eq!(p.delay(10, &status(503)), Duration::from_secs(60));
    }
}
