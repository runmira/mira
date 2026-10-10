//! Notices when the provider's prompt cache stops being reused, and says
//! why.
//!
//! Each request is fingerprinted (model, system prompt, tools, and every
//! message). When a large prompt suddenly gets little of it from the cache,
//! comparing that fingerprint with the previous request's names the cause:
//! the model or tools changed, an earlier message was rewritten, or the
//! cache simply expired. A miss is otherwise invisible: the same answer,
//! billed at full price.

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use mira_ai::{ChatRequest, TokenUsage};
use mira_core::{Message, Role};

/// Prompts smaller than this aren't judged: providers don't cache them.
const MIN_JUDGED_TOKENS: u32 = 4_096;

/// How long providers keep a prompt cached without use (Anthropic's
/// default; OpenAI's is similar or longer).
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// What a request looked like, for comparing with the next one.
#[derive(Clone, Debug)]
pub struct RequestPrint {
    model: String,
    system: u64,
    tools: u64,
    messages: Vec<(Role, u64)>,
}

impl RequestPrint {
    pub fn of(req: &ChatRequest) -> Self {
        let leading_system = req
            .messages
            .iter()
            .take_while(|m| m.role == Role::System)
            .count();
        Self {
            model: req.model.clone(),
            system: hash_all(req.messages[..leading_system].iter().map(message_hash)),
            tools: hash_all(req.tools.iter().map(|t| {
                hash_one(&(
                    t.name.as_str(),
                    t.description.as_str(),
                    t.parameters.to_string(),
                ))
            })),
            messages: req.messages[leading_system..]
                .iter()
                .map(|m| (m.role, message_hash(m)))
                .collect(),
        }
    }
}

#[derive(Default)]
pub struct CacheWatch {
    previous: Option<(RequestPrint, Instant, u32)>,
    /// Whether this provider has ever served from cache in this session.
    /// Until it has, a lack of cached tokens says nothing.
    caches: bool,
}

impl CacheWatch {
    /// Record a request and the usage it was billed; the reason the cache
    /// wasn't reused, when it should have been.
    pub fn observe(&mut self, print: RequestPrint, usage: &TokenUsage) -> Option<String> {
        self.observe_at(print, usage, Instant::now())
    }

    fn observe_at(
        &mut self,
        print: RequestPrint,
        usage: &TokenUsage,
        at: Instant,
    ) -> Option<String> {
        let previous = self
            .previous
            .replace((print.clone(), at, usage.prompt_tokens));
        if usage.cached_input_tokens > 0 {
            self.caches = true;
        }
        let (before, then, before_tokens) = previous?;
        let judged = self.caches
            && before_tokens >= MIN_JUDGED_TOKENS
            && usage.prompt_tokens >= MIN_JUDGED_TOKENS;
        if !judged || usage.cached_input_tokens >= before_tokens / 2 {
            return None;
        }
        Some(explain(&before, &print, at.duration_since(then)))
    }
}

fn explain(before: &RequestPrint, now: &RequestPrint, idle: Duration) -> String {
    if before.model != now.model {
        return format!("the model changed ({} → {})", before.model, now.model);
    }
    if before.tools != now.tools {
        return "the tool list changed".into();
    }
    if before.system != now.system {
        return "the system prompt changed".into();
    }
    if let Some(i) = before
        .messages
        .iter()
        .zip(&now.messages)
        .position(|(a, b)| a != b)
    {
        let role = match before.messages[i].0 {
            Role::Tool => "a tool result",
            Role::User => "a user message",
            Role::Assistant => "a reply",
            Role::System => "a note",
        };
        return format!(
            "{role} already sent was changed (message {} of {})",
            i + 1,
            now.messages.len()
        );
    }
    if now.messages.len() < before.messages.len() {
        return "earlier messages were removed".into();
    }
    if idle >= CACHE_TTL {
        return format!(
            "the cache expired ({} minutes since the last request)",
            idle.as_secs() / 60
        );
    }
    "the provider didn't reuse its cache".into()
}

fn message_hash(m: &Message) -> u64 {
    hash_one(&(
        &m.content,
        m.images
            .iter()
            .map(|i| (i.media_type.as_str(), hash_one(&i.data)))
            .collect::<Vec<_>>(),
        m.tool_call_id.as_ref().map(|id| id.as_str().to_owned()),
        m.tool_calls
            .iter()
            .map(|c| (c.function.name.as_str(), c.function.arguments.as_str()))
            .collect::<Vec<_>>(),
    ))
}

fn hash_one(value: &impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

fn hash_all(values: impl Iterator<Item = u64>) -> u64 {
    hash_one(&values.collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(messages: Vec<Message>) -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages,
            tools: vec![],
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            service_tier: None,
            response_format: None,
        }
    }

    fn usage(prompt: u32, cached: u32) -> TokenUsage {
        TokenUsage {
            prompt_tokens: prompt,
            cached_input_tokens: cached,
            ..Default::default()
        }
    }

    #[test]
    fn a_rewritten_message_is_named() {
        let mut w = CacheWatch::default();
        let t = Instant::now();
        let first = req(vec![
            Message::system("s"),
            Message::user("a"),
            Message::assistant("b"),
        ]);
        assert_eq!(
            w.observe_at(RequestPrint::of(&first), &usage(10_000, 9_000), t),
            None
        );
        let second = req(vec![
            Message::system("s"),
            Message::user("A!"),
            Message::assistant("b"),
        ]);
        let why = w
            .observe_at(RequestPrint::of(&second), &usage(10_000, 0), t)
            .unwrap();
        assert!(
            why.contains("a user message already sent was changed (message 1 of 2)"),
            "{why}"
        );
    }

    #[test]
    fn reuse_is_not_a_miss_and_expiry_is_named() {
        let mut w = CacheWatch::default();
        let t = Instant::now();
        let r = req(vec![Message::system("s"), Message::user("a")]);
        assert_eq!(
            w.observe_at(RequestPrint::of(&r), &usage(10_000, 0), t),
            None
        );
        assert_eq!(
            w.observe_at(RequestPrint::of(&r), &usage(10_000, 9_500), t),
            None
        );
        let why = w
            .observe_at(
                RequestPrint::of(&r),
                &usage(10_000, 0),
                t + Duration::from_secs(400),
            )
            .unwrap();
        assert!(why.contains("expired"), "{why}");
    }

    #[test]
    fn providers_that_never_cache_are_not_reported() {
        let mut w = CacheWatch::default();
        let t = Instant::now();
        let r = req(vec![Message::user("a")]);
        for _ in 0..3 {
            assert_eq!(
                w.observe_at(RequestPrint::of(&r), &usage(50_000, 0), t),
                None
            );
        }
    }
}
