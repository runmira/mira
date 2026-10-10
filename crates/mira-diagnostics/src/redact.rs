//! Remove secrets from text before it leaves the machine.
//!
//! Three passes, strongest first: the exact values of keys Mira knows
//! about (config `api_key`s, `keys:`, secret-looking env vars), then
//! well-known token shapes (`sk-…`, `ghp_…`, JWTs, private keys), then
//! values sitting next to a secret-sounding name (`"api_key": "…"`,
//! `TOKEN=…`). Paths under `$HOME` are shortened to `~` so the bundle
//! doesn't carry the user's account name.

use std::sync::OnceLock;

use regex::{Captures, Regex};

pub const REDACTED: &str = "«redacted»";

/// Literal values shorter than this aren't treated as secrets: replacing
/// every `true` or `8080` would wreck the logs without protecting anything.
const MIN_LITERAL: usize = 8;

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    literals: Vec<String>,
    home: Option<String>,
}

impl Redactor {
    /// Secrets from `cfg` and the process environment, plus `$HOME`.
    pub fn from_env_and_config(cfg: &mira_config::MiraConfig) -> Self {
        let mut r = Self {
            home: std::env::var("HOME").ok().filter(|h| h.len() > 1),
            ..Self::default()
        };
        for (name, value) in std::env::vars() {
            if is_secret_name(&name) {
                r.add_literal(value);
            }
        }
        for p in cfg.providers.values() {
            if let Some(k) = &p.api_key {
                r.add_literal(k.clone());
            }
        }
        for v in cfg.keys.values() {
            r.add_literal(v.clone());
        }
        r
    }

    pub fn add_literal(&mut self, value: String) {
        let value = value.trim().to_string();
        if value.len() >= MIN_LITERAL && !self.literals.contains(&value) {
            self.literals.push(value);
            // Longest first, so a key that contains another is replaced whole.
            self.literals.sort_by_key(|s| std::cmp::Reverse(s.len()));
        }
    }

    pub fn with_home(mut self, home: Option<String>) -> Self {
        self.home = home.filter(|h| h.len() > 1);
        self
    }

    pub fn text(&self, input: &str) -> String {
        let mut out = input.to_string();
        for lit in &self.literals {
            if out.contains(lit.as_str()) {
                out = out.replace(lit.as_str(), REDACTED);
            }
        }
        for re in token_patterns() {
            out = re.replace_all(&out, REDACTED).into_owned();
        }
        out = bearer()
            .replace_all(&out, |c: &Captures| format!("{}{REDACTED}", &c[1]))
            .into_owned();
        out = key_value()
            .replace_all(&out, |c: &Captures| {
                let value = &c[2];
                if value.contains(REDACTED) {
                    return c[0].to_string();
                }
                let quote = value
                    .chars()
                    .next()
                    .filter(|q| *q == '"' || *q == '\'')
                    .map(String::from)
                    .unwrap_or_default();
                format!("{}{quote}{REDACTED}{quote}", &c[1])
            })
            .into_owned();
        out = url_param()
            .replace_all(&out, |c: &Captures| format!("{}{REDACTED}", &c[1]))
            .into_owned();
        if let Some(home) = &self.home {
            out = out.replace(home.as_str(), "~");
        }
        out
    }

    /// Redact a YAML document by structure, then by text. Structure catches
    /// secrets the text pass can't recognise, like a key under a map whose
    /// name is the secret's (`keys: { BRAVE_SEARCH_API_KEY: … }`).
    pub fn yaml(&self, input: &str) -> String {
        match serde_yaml::from_str::<serde_yaml::Value>(input) {
            Ok(mut v) => {
                redact_yaml_value(&mut v);
                let s = serde_yaml::to_string(&v).unwrap_or_default();
                self.text(&s)
            }
            // Unparseable config is itself useful to see; the text pass
            // still removes the secrets.
            Err(_) => self.text(input),
        }
    }
}

/// True for names like `api_key`, `OPENAI_API_KEY`, `accessToken`,
/// `client-secret`, `Authorization`. Not for `max_tokens` or
/// `api_key_env` (whose value is the name of a variable, not a secret).
pub fn is_secret_name(name: &str) -> bool {
    secret_name().is_match(name)
}

fn redact_yaml_value(v: &mut serde_yaml::Value) {
    match v {
        serde_yaml::Value::Mapping(m) => {
            for (k, val) in m.iter_mut() {
                let secret = k.as_str().is_some_and(is_secret_name);
                if secret && !matches!(val, serde_yaml::Value::Null) && !val.is_mapping() {
                    *val = serde_yaml::Value::String(REDACTED.into());
                } else {
                    redact_yaml_value(val);
                }
            }
        }
        serde_yaml::Value::Sequence(s) => s.iter_mut().for_each(redact_yaml_value),
        serde_yaml::Value::Tagged(t) => redact_yaml_value(&mut t.value),
        _ => {}
    }
}

const SECRET_WORDS: &str = r"api[_-]?key|apikey|access[_-]?key|secret[_-]?key|private[_-]?key|token|secret|password|passwd|authorization|cookie|credentials?";

fn secret_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(r"(?i)^[A-Za-z0-9_.\-]*(?:{SECRET_WORDS})$")).expect("valid regex")
    })
}

fn key_value() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The name must end in a secret word, so `max_tokens: 4000` is left
        // alone. Value: a quoted string (JSON escapes allowed) or a bare word.
        Regex::new(&format!(
            r#"(?i)((?:["']?)\b[A-Za-z0-9_.\-]*(?:{SECRET_WORDS})["']?\s*[:=]\s*)("(?:[^"\\\n]|\\.)*"|'[^'\n]*'|[^\s,;}}&"']+)"#
        ))
        .expect("valid regex")
    })
}

fn bearer() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\bbearer\s+)[A-Za-z0-9._~+/=\-]{8,}").expect("valid regex"))
}

fn url_param() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)([?&](?:key|code|sig|signature|auth)=)[^&\s#'"]+"#).expect("valid regex")
    })
}

fn token_patterns() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        [
            // OpenAI, Anthropic (`sk-ant-`), OpenRouter (`sk-or-v1-`), DeepSeek, …
            r"\bsk-[A-Za-z0-9_\-]{16,}",
            // Stripe-style live/test keys.
            r"\b[rsp]k_(?:live|test)_[A-Za-z0-9]{16,}",
            // GitHub tokens.
            r"\bgh[pousr]_[A-Za-z0-9]{20,}",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}",
            // GitLab, Slack, Google, AWS, Hugging Face, Tavily, E2B, Supabase.
            r"\bglpat-[A-Za-z0-9_\-]{16,}",
            r"\bxox[abposr]-[A-Za-z0-9\-]{10,}",
            r"\bxapp-[A-Za-z0-9\-]{10,}",
            r"\bAIza[0-9A-Za-z_\-]{30,}",
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
            r"\bhf_[A-Za-z0-9]{30,}",
            r"\btvly-[A-Za-z0-9\-]{16,}",
            r"\be2b_[A-Za-z0-9]{20,}",
            r"\bsbp_[A-Za-z0-9]{20,}",
            // JWTs (OAuth access tokens, Supabase keys).
            r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
            // PEM private keys, whole block.
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid regex"))
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Redactor {
        Redactor::default()
    }

    #[test]
    fn removes_known_token_shapes() {
        let input = "key sk-or-v1-0123456789abcdef0123 and ghp_ABCDEFGHIJKLMNOPQRSTUVWX1234 \
                     and AKIAABCDEFGHIJKLMNOP and eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4";
        let out = r().text(input);
        assert!(!out.contains("sk-or-v1-0123"), "{out}");
        assert!(!out.contains("ghp_ABC"), "{out}");
        assert!(!out.contains("AKIAABC"), "{out}");
        assert!(!out.contains("eyJhbGci"), "{out}");
    }

    #[test]
    fn removes_values_next_to_secret_names_but_keeps_counts() {
        let input = r#"{"api_key": "hunter2-not-a-pattern", "max_tokens": 4000, "input_tokens": 12}
OPENAI_API_KEY=abc123xyz
password: 'letmein'
api_key_env: OPENROUTER_API_KEY"#;
        let out = r().text(input);
        assert!(out.contains(r#""api_key": "«redacted»""#), "{out}");
        assert!(out.contains(r#""max_tokens": 4000"#), "{out}");
        assert!(out.contains(r#""input_tokens": 12"#), "{out}");
        assert!(out.contains("OPENAI_API_KEY=«redacted»"), "{out}");
        assert!(out.contains("password: '«redacted»'"), "{out}");
        assert!(out.contains("api_key_env: OPENROUTER_API_KEY"), "{out}");
    }

    #[test]
    fn json_strings_with_escaped_quotes_are_removed_whole() {
        let out = r().text(r#"{"token": "a\"b\"c-secret", "next": 1}"#);
        assert_eq!(out, r#"{"token": "«redacted»", "next": 1}"#);
    }

    #[test]
    fn bearer_headers_and_url_keys() {
        let out = r().text("Authorization: Bearer abcdefgh12345678 GET /x?key=AbC123&q=1");
        assert!(!out.contains("abcdefgh12345678"), "{out}");
        assert!(out.contains("?key=«redacted»&q=1"), "{out}");
    }

    #[test]
    fn known_literals_go_wherever_they_appear() {
        let mut red = r();
        red.add_literal("my-custom-key-format-42".into());
        red.add_literal("short".into());
        let out = red.text("sent my-custom-key-format-42 to the short host");
        assert_eq!(out, "sent «redacted» to the short host");
    }

    #[test]
    fn home_becomes_tilde() {
        let red = r().with_home(Some("/Users/alice".into()));
        assert_eq!(red.text("/Users/alice/code/app"), "~/code/app");
    }

    #[test]
    fn private_key_blocks() {
        let out = r().text(
            "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\nBBBB\n-----END OPENSSH PRIVATE KEY-----\nb",
        );
        assert_eq!(out, "a\n«redacted»\nb");
    }

    #[test]
    fn yaml_secrets_under_named_maps() {
        let yaml = "default_model: gpt\nproviders:\n  openai:\n    base_url: https://api.openai.com/v1\n    api_key: plain\nkeys:\n  BRAVE_SEARCH_API_KEY: brave123\nmcp_servers:\n  gh:\n    env:\n      GITHUB_TOKEN: tok\n    headers:\n      Authorization: xyz\n";
        let out = r().yaml(yaml);
        for secret in ["plain", "brave123", "tok\n", "xyz"] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        assert!(out.contains("default_model: gpt"), "{out}");
        assert!(out.contains("https://api.openai.com/v1"), "{out}");
    }

    #[test]
    fn secret_names() {
        for n in [
            "api_key",
            "OPENAI_API_KEY",
            "accessToken",
            "client-secret",
            "Authorization",
            "GITHUB_TOKEN",
            "password",
        ] {
            assert!(is_secret_name(n), "{n}");
        }
        for n in ["max_tokens", "api_key_env", "model", "token_budget", "keys"] {
            assert!(!is_secret_name(n), "{n}");
        }
    }
}
