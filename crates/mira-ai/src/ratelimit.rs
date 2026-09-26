//! Rate limits the provider reports in response headers.
//!
//! - Anthropic: `anthropic-ratelimit-{requests,tokens,input-tokens,
//!   output-tokens}-{limit,remaining,reset}`, reset as an RFC 3339 time.
//! - OpenAI, Groq and most OpenAI-compatible APIs:
//!   `x-ratelimit-{limit,remaining,reset}-{requests,tokens}`, reset as a
//!   duration (`1s`, `6m0s`, `20ms`).
//! - Some gateways (OpenRouter's free tier, …) send the unsuffixed
//!   `x-ratelimit-{limit,remaining,reset}`, reset as a Unix time in
//!   seconds or milliseconds; that's read as requests.

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

/// One limit: how much is left out of how much, and when it refills.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u64>,
    /// Seconds until it's full again, as of when the response arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_secs: Option<u64>,
}

impl Bucket {
    /// Share left, 0.0–1.0, when both numbers are known.
    pub fn fraction_left(&self) -> Option<f64> {
        match (self.remaining, self.limit) {
            (Some(r), Some(l)) if l > 0 => Some((r as f64 / l as f64).clamp(0.0, 1.0)),
            _ => None,
        }
    }

    fn is_empty(&self) -> bool {
        self.limit.is_none() && self.remaining.is_none()
    }
}

/// The provider's rate limits after a request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<Bucket>,
}

impl RateLimit {
    /// Read the headers; `None` when the provider sent none.
    pub fn from_headers(h: &HeaderMap) -> Option<Self> {
        let get = |name: &str| h.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
        let num = |name: &str| {
            get(name)
                .and_then(|v| v.parse::<f64>().ok())
                .map(|n| n.max(0.0) as u64)
        };
        let now = chrono::Utc::now();

        let anthropic = |kind: &str| {
            let p = format!("anthropic-ratelimit-{kind}");
            Bucket {
                limit: num(&format!("{p}-limit")),
                remaining: num(&format!("{p}-remaining")),
                reset_secs: get(&format!("{p}-reset"))
                    .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                    .map(|t| (t.with_timezone(&chrono::Utc) - now).num_seconds().max(0) as u64),
            }
        };
        let openai = |kind: &str| Bucket {
            limit: num(&format!("x-ratelimit-limit-{kind}")),
            remaining: num(&format!("x-ratelimit-remaining-{kind}")),
            reset_secs: get(&format!("x-ratelimit-reset-{kind}")).and_then(parse_duration_secs),
        };
        let plain = Bucket {
            limit: num("x-ratelimit-limit"),
            remaining: num("x-ratelimit-remaining"),
            reset_secs: get("x-ratelimit-reset")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|n| unix_or_secs_until(n, now.timestamp_millis())),
        };

        let keep = |b: Bucket| (!b.is_empty()).then_some(b);
        let rl = RateLimit {
            requests: keep(anthropic("requests"))
                .or_else(|| keep(openai("requests")))
                .or_else(|| keep(plain)),
            tokens: keep(anthropic("tokens")).or_else(|| keep(openai("tokens"))),
            input_tokens: keep(anthropic("input-tokens")),
            output_tokens: keep(anthropic("output-tokens")),
        };
        (rl != RateLimit::default()).then_some(rl)
    }

    /// The limit closest to running out: its name and bucket.
    pub fn tightest(&self) -> Option<(&'static str, Bucket)> {
        [
            ("requests", self.requests),
            ("tokens", self.tokens),
            ("input tokens", self.input_tokens),
            ("output tokens", self.output_tokens),
        ]
        .into_iter()
        .filter_map(|(n, b)| b.and_then(|b| b.fraction_left().map(|f| (n, b, f))))
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(n, b, _)| (n, b))
    }

    /// A short line for status bars, e.g. `12% of tokens left · resets in
    /// 42s`. `None` when nothing's known.
    pub fn summary(&self) -> Option<String> {
        let (name, b) = self.tightest()?;
        let pct = (b.fraction_left()? * 100.0).round() as u64;
        let reset = b
            .reset_secs
            .filter(|s| *s > 0)
            .map(|s| format!(" · resets in {}", short_duration(s)))
            .unwrap_or_default();
        Some(format!("{pct}% of {name} left{reset}"))
    }
}

/// `1s`, `6m0s`, `20ms`, `1h2m3.5s` → whole seconds, rounded up.
fn parse_duration_secs(s: &str) -> Option<u64> {
    let mut total = 0.0f64;
    let mut num = String::new();
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let value: f64 = num.parse().ok()?;
        num.clear();
        let unit = match c {
            'h' => 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                0.001
            }
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        total += value * unit;
        any = true;
    }
    if !num.is_empty() {
        // A bare number: seconds.
        total += num.parse::<f64>().ok()?;
        any = true;
    }
    any.then(|| total.ceil() as u64)
}

/// A reset value that may be a Unix time (seconds or milliseconds) or
/// already a number of seconds.
fn unix_or_secs_until(n: f64, now_ms: i64) -> u64 {
    let ms = if n > 1e12 {
        n
    } else if n > 1e9 {
        n * 1000.0
    } else {
        return n.max(0.0).ceil() as u64;
    };
    ((ms - now_ms as f64) / 1000.0).max(0.0).ceil() as u64
}

fn short_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs.div_ceil(60)),
        _ => format!("{}h", secs.div_ceil(3600)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, String)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn anthropic_headers() {
        let reset = (chrono::Utc::now() + chrono::Duration::seconds(42)).to_rfc3339();
        let rl = RateLimit::from_headers(&headers(&[
            ("anthropic-ratelimit-requests-limit", "50".into()),
            ("anthropic-ratelimit-requests-remaining", "49".into()),
            ("anthropic-ratelimit-tokens-limit", "40000".into()),
            ("anthropic-ratelimit-tokens-remaining", "4000".into()),
            ("anthropic-ratelimit-tokens-reset", reset),
            ("anthropic-ratelimit-input-tokens-limit", "30000".into()),
            ("anthropic-ratelimit-input-tokens-remaining", "29000".into()),
        ]))
        .unwrap();
        assert_eq!(rl.requests.unwrap().remaining, Some(49));
        let (name, b) = rl.tightest().unwrap();
        assert_eq!(name, "tokens");
        let secs = b.reset_secs.unwrap();
        assert!((40..=42).contains(&secs), "{secs}");
        assert!(rl.output_tokens.is_none());
        let s = rl.summary().unwrap();
        assert!(s.starts_with("10% of tokens left · resets in 4"), "{s}");
    }

    #[test]
    fn openai_headers() {
        let rl = RateLimit::from_headers(&headers(&[
            ("x-ratelimit-limit-requests", "500".into()),
            ("x-ratelimit-remaining-requests", "5".into()),
            ("x-ratelimit-reset-requests", "6m0s".into()),
            ("x-ratelimit-limit-tokens", "30000".into()),
            ("x-ratelimit-remaining-tokens", "29000".into()),
            ("x-ratelimit-reset-tokens", "20ms".into()),
        ]))
        .unwrap();
        assert_eq!(rl.requests.unwrap().reset_secs, Some(360));
        assert_eq!(rl.tokens.unwrap().reset_secs, Some(1));
        assert_eq!(rl.summary().unwrap(), "1% of requests left · resets in 6m");
    }

    #[test]
    fn unsuffixed_headers_and_none() {
        let in_a_minute = chrono::Utc::now().timestamp_millis() + 60_000;
        let rl = RateLimit::from_headers(&headers(&[
            ("x-ratelimit-limit", "20".into()),
            ("x-ratelimit-remaining", "19".into()),
            ("x-ratelimit-reset", in_a_minute.to_string()),
        ]))
        .unwrap();
        let r = rl.requests.unwrap();
        assert_eq!((r.limit, r.remaining), (Some(20), Some(19)));
        assert!((59..=60).contains(&r.reset_secs.unwrap()));

        assert_eq!(RateLimit::from_headers(&HeaderMap::new()), None);
        assert_eq!(
            RateLimit::from_headers(&headers(&[("content-type", "text/event-stream".into())])),
            None
        );
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration_secs("1s"), Some(1));
        assert_eq!(parse_duration_secs("6m0s"), Some(360));
        assert_eq!(parse_duration_secs("1h2m3.5s"), Some(3724));
        assert_eq!(parse_duration_secs("20ms"), Some(1));
        assert_eq!(parse_duration_secs("12"), Some(12));
        assert_eq!(parse_duration_secs("soon"), None);
        assert_eq!(short_duration(42), "42s");
        assert_eq!(short_duration(61), "2m");
        assert_eq!(short_duration(7200), "2h");
    }
}
