//! Amazon Bedrock through the Converse API (`/model/{id}/converse-stream`).
//!
//! One adapter for every Bedrock model (Claude, Llama, Mistral, Nova, …),
//! streaming, with tool use and images.
//!
//! Auth, first match wins:
//! 1. an API key passed in (a Bedrock API key), or `AWS_BEARER_TOKEN_BEDROCK`:
//!    sent as `Authorization: Bearer`;
//! 2. AWS credentials: `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
//!    `AWS_SESSION_TOKEN`, else the `AWS_PROFILE` (or `default`) profile in
//!    `~/.aws/credentials`: requests are signed with SigV4.
//!
//! Region: from the base URL (`https://bedrock-runtime.<region>.amazonaws.com`),
//! else `AWS_REGION` / `AWS_DEFAULT_REGION`, else the profile's region in
//! `~/.aws/config`, else `us-east-1`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use mira_core::Role;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::event::{ChatEvent, FinishReason, TokenUsage, ToolCallBuffer};
use crate::provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError};

#[derive(Clone, Debug)]
pub struct BedrockConfig {
    /// `https://bedrock-runtime.<region>.amazonaws.com`; a URL without a
    /// region gets one from the environment.
    pub base_url: String,
    /// A Bedrock API key; empty to use AWS credentials.
    pub api_key: String,
}

#[derive(Clone, Debug)]
enum Auth {
    Bearer(String),
    SigV4(Credentials),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
}

pub struct Bedrock {
    client: reqwest::Client,
    region: String,
    /// `https://bedrock-runtime.<region>.amazonaws.com`, no trailing slash.
    runtime_url: String,
    auth: Auth,
}

impl Bedrock {
    pub fn new(cfg: BedrockConfig) -> Result<Self, ProviderError> {
        let base = cfg.base_url.trim().trim_end_matches('/').to_owned();
        let region = region_from_url(&base)
            .or_else(region_from_env)
            .unwrap_or_else(|| "us-east-1".to_owned());
        let runtime_url = if base.is_empty() || region_from_url(&base).is_none() {
            format!("https://bedrock-runtime.{region}.amazonaws.com")
        } else {
            base
        };
        let key = Some(cfg.api_key.trim().to_owned())
            .filter(|k| !k.is_empty())
            .or_else(|| env("AWS_BEARER_TOKEN_BEDROCK"));
        let auth = match key {
            Some(k) => Auth::Bearer(k),
            None => Auth::SigV4(
                credentials_from_env()
                    .or_else(credentials_from_profile)
                    .ok_or_else(|| {
                        ProviderError::Config(
                            "no AWS credentials for Bedrock: set a Bedrock API key \
                         (AWS_BEARER_TOKEN_BEDROCK), AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY, \
                         or a profile in ~/.aws/credentials"
                                .into(),
                        )
                    })?,
            ),
        };
        Ok(Self {
            client: reqwest::Client::new(),
            region,
            runtime_url,
            auth,
        })
    }

    fn signed(
        &self,
        method: &str,
        url: &str,
        service: &str,
        body: &[u8],
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let parsed = reqwest::Url::parse(url).map_err(|e| ProviderError::Config(e.to_string()))?;
        let builder = self
            .client
            .request(
                method
                    .parse()
                    .map_err(|_| ProviderError::Config("bad method".into()))?,
                url,
            )
            .header("content-type", "application/json")
            .header("accept", "application/json");
        Ok(match &self.auth {
            Auth::Bearer(k) => builder.header("authorization", format!("Bearer {k}")),
            Auth::SigV4(creds) => {
                let headers = sign(&SignInput {
                    method,
                    host: parsed.host_str().unwrap_or_default(),
                    path: parsed.path(),
                    query: parsed.query().unwrap_or(""),
                    extra_headers: &[("content-type", "application/json")],
                    body,
                    service,
                    region: &self.region,
                    creds,
                    now: SystemTime::now(),
                });
                headers
                    .into_iter()
                    .fold(builder, |b, (k, v)| b.header(k, v))
            }
        })
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// `bedrock-runtime.<region>.amazonaws.com` → `<region>`.
fn region_from_url(url: &str) -> Option<String> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_owned();
    let parts: Vec<&str> = host.split('.').collect();
    let i = parts.iter().position(|p| p.starts_with("bedrock"))?;
    let region = parts.get(i + 1)?;
    (region.contains('-') && *region != "amazonaws").then(|| (*region).to_owned())
}

fn region_from_env() -> Option<String> {
    env("AWS_REGION")
        .or_else(|| env("AWS_DEFAULT_REGION"))
        .or_else(|| {
            ini_value(
                &aws_file("config", "AWS_CONFIG_FILE"),
                &config_section(),
                "region",
            )
        })
}

fn profile() -> String {
    env("AWS_PROFILE").unwrap_or_else(|| "default".into())
}

fn config_section() -> String {
    let p = profile();
    if p == "default" {
        p
    } else {
        format!("profile {p}")
    }
}

fn aws_file(name: &str, env_override: &str) -> PathBuf {
    env(env_override).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".aws")
            .join(name)
    })
}

fn credentials_from_env() -> Option<Credentials> {
    Some(Credentials {
        access_key: env("AWS_ACCESS_KEY_ID")?,
        secret_key: env("AWS_SECRET_ACCESS_KEY")?,
        session_token: env("AWS_SESSION_TOKEN"),
    })
}

fn credentials_from_profile() -> Option<Credentials> {
    let file = aws_file("credentials", "AWS_SHARED_CREDENTIALS_FILE");
    let section = profile();
    Some(Credentials {
        access_key: ini_value(&file, &section, "aws_access_key_id")?,
        secret_key: ini_value(&file, &section, "aws_secret_access_key")?,
        session_token: ini_value(&file, &section, "aws_session_token"),
    })
}

/// `key` in `[section]` of an INI file.
fn ini_value(path: &PathBuf, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_ini(&text, section, key)
}

fn parse_ini(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.trim().to_owned();
            continue;
        }
        if current == section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_owned()).filter(|v| !v.is_empty());
                }
            }
        }
    }
    None
}

/* ---------- SigV4 ---------- */

pub(crate) struct SignInput<'a> {
    pub method: &'a str,
    pub host: &'a str,
    /// Already percent-encoded, as sent.
    pub path: &'a str,
    pub query: &'a str,
    pub extra_headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
    pub service: &'a str,
    pub region: &'a str,
    pub creds: &'a Credentials,
    pub now: SystemTime,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new()
        .chain_update(&ipad)
        .chain_update(data)
        .finalize();
    Sha256::new()
        .chain_update(&opad)
        .chain_update(inner)
        .finalize()
        .to_vec()
}

/// `(YYYYMMDD, YYYYMMDDTHHMMSSZ)` in UTC.
fn amz_dates(now: SystemTime) -> (String, String) {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let date = format!("{year:04}{month:02}{day:02}");
    let stamp = format!(
        "{date}T{:02}{:02}{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    );
    (date, stamp)
}

/// Percent-encode per SigV4 (unreserved characters stay).
fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The headers to add to sign a request with SigV4.
pub(crate) fn sign(i: &SignInput<'_>) -> Vec<(String, String)> {
    let (date, stamp) = amz_dates(i.now);
    let payload_hash = hex(&Sha256::digest(i.body));
    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), i.host.to_owned()),
        ("x-amz-date".into(), stamp.clone()),
    ];
    for (k, v) in i.extra_headers {
        headers.push((k.to_ascii_lowercase(), v.trim().to_owned()));
    }
    if let Some(t) = &i.creds.session_token {
        headers.push(("x-amz-security-token".into(), t.clone()));
    }
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    // Non-S3 services encode the (already encoded) path once more.
    let canonical_uri = if i.path.is_empty() {
        "/".to_owned()
    } else {
        uri_encode(i.path, true)
    };
    let mut query: Vec<(String, String)> = i
        .query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (uri_encode(k, false), uri_encode(v, false))
        })
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let canonical = format!(
        "{}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        i.method
    );
    let scope = format!("{date}/{}/{}/aws4_request", i.region, i.service);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let k_date = hmac(
        format!("AWS4{}", i.creds.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac(&k_date, i.region.as_bytes());
    let k_service = hmac(&k_region, i.service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex(&hmac(&k_signing, to_sign.as_bytes()));
    let mut out = vec![
        ("x-amz-date".to_owned(), stamp),
        (
            "authorization".to_owned(),
            format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
                i.creds.access_key
            ),
        ),
    ];
    if let Some(t) = &i.creds.session_token {
        out.push(("x-amz-security-token".to_owned(), t.clone()));
    }
    out
}

/* ---------- request body ---------- */

fn image_block(media_type: &str, data: &str) -> Option<Value> {
    let format = match media_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpeg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => return None,
    };
    Some(json!({"image": {"format": format, "source": {"bytes": data}}}))
}

/// Converse messages: system prompts hoisted, tool results as user
/// `toolResult` blocks, consecutive same-role turns merged (Converse
/// requires alternating roles, starting with the user).
pub(crate) fn build_body(req: &ChatRequest) -> Value {
    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<(String, Vec<Value>)> = Vec::new();
    let mut push = |role: &str, blocks: Vec<Value>| {
        if blocks.is_empty() {
            return;
        }
        match messages.last_mut() {
            Some((r, b)) if r == role => b.extend(blocks),
            _ => messages.push((role.to_owned(), blocks)),
        }
    };
    for m in &req.messages {
        let text = m.content.as_deref().unwrap_or("");
        match m.role {
            Role::System => {
                if !text.trim().is_empty() {
                    system.push(json!({"text": text}));
                }
            }
            Role::User => {
                let mut blocks = Vec::new();
                if !text.trim().is_empty() {
                    blocks.push(json!({"text": text}));
                }
                blocks.extend(
                    m.images
                        .iter()
                        .filter_map(|i| image_block(&i.media_type, &i.data)),
                );
                push("user", blocks);
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !text.trim().is_empty() {
                    blocks.push(json!({"text": text}));
                }
                for c in &m.tool_calls {
                    let input: Value = serde_json::from_str(&c.function.arguments)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| json!({}));
                    blocks.push(json!({"toolUse": {
                        "toolUseId": c.id.to_string(),
                        "name": c.function.name,
                        "input": input,
                    }}));
                }
                push("assistant", blocks);
            }
            Role::Tool => {
                let mut content =
                    vec![json!({"text": if text.is_empty() { "(no output)" } else { text }})];
                content.extend(
                    m.images
                        .iter()
                        .filter_map(|i| image_block(&i.media_type, &i.data)),
                );
                let id = m
                    .tool_call_id
                    .as_ref()
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                push(
                    "user",
                    vec![json!({"toolResult": {"toolUseId": id, "content": content}})],
                );
            }
        }
    }
    if messages.first().is_some_and(|(r, _)| r == "assistant") {
        messages.insert(0, ("user".into(), vec![json!({"text": "(continue)"})]));
    }
    let mut body = Map::new();
    body.insert(
        "messages".into(),
        Value::Array(
            messages
                .into_iter()
                .map(|(role, content)| json!({"role": role, "content": content}))
                .collect(),
        ),
    );
    if !system.is_empty() {
        body.insert("system".into(), Value::Array(system));
    }
    let mut inference = Map::new();
    if let Some(t) = req.max_tokens {
        inference.insert("maxTokens".into(), json!(t));
    }
    if let Some(t) = req.temperature {
        inference.insert("temperature".into(), json!(t));
    }
    if !inference.is_empty() {
        body.insert("inferenceConfig".into(), Value::Object(inference));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({"toolSpec": {
                    "name": t.name,
                    "description": if t.description.is_empty() { t.name.as_str() } else { t.description.as_str() },
                    "inputSchema": {"json": t.parameters},
                }})
            })
            .collect();
        body.insert("toolConfig".into(), json!({"tools": tools}));
    }
    Value::Object(body)
}

/* ---------- event stream ---------- */

/// One decoded `application/vnd.amazon.eventstream` message.
#[derive(Debug)]
pub(crate) struct Frame {
    pub headers: Vec<(String, String)>,
    pub payload: Vec<u8>,
}

impl Frame {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Take one complete frame off the front of `buf`, if there is one.
pub(crate) fn next_frame(buf: &mut Vec<u8>) -> Result<Option<Frame>, ProviderError> {
    if buf.len() < 12 {
        return Ok(None);
    }
    let u32_at =
        |i: usize| u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
    let total = u32_at(0);
    let headers_len = u32_at(4);
    if total < 16 || headers_len + 16 > total {
        return Err(ProviderError::Decode(format!(
            "bad event-stream frame ({total} bytes)"
        )));
    }
    if buf.len() < total {
        return Ok(None);
    }
    let frame: Vec<u8> = buf.drain(..total).collect();
    let mut headers = Vec::new();
    let raw = &frame[12..12 + headers_len];
    let mut i = 0;
    while i < raw.len() {
        let name_len = raw[i] as usize;
        i += 1;
        let name = String::from_utf8_lossy(&raw[i..i + name_len]).into_owned();
        i += name_len;
        let kind = raw[i];
        i += 1;
        let value = match kind {
            0 => "true".to_owned(),
            1 => "false".to_owned(),
            2 => {
                i += 1;
                String::new()
            }
            3 => {
                i += 2;
                String::new()
            }
            4 => {
                i += 4;
                String::new()
            }
            5 | 8 => {
                i += 8;
                String::new()
            }
            6 | 7 => {
                let len = u16::from_be_bytes([raw[i], raw[i + 1]]) as usize;
                i += 2;
                let v = String::from_utf8_lossy(&raw[i..i + len]).into_owned();
                i += len;
                v
            }
            9 => {
                i += 16;
                String::new()
            }
            other => {
                return Err(ProviderError::Decode(format!(
                    "event-stream header type {other}"
                )))
            }
        };
        headers.push((name, value));
    }
    let payload = frame[12 + headers_len..total - 4].to_vec();
    Ok(Some(Frame { headers, payload }))
}

/// Turns Converse stream events into [`ChatEvent`]s.
#[derive(Default)]
pub(crate) struct ConverseState {
    calls: ToolCallBuffer,
    /// Converse content-block index → tool-call slot.
    tool_slots: Vec<(u64, usize)>,
    finish: Option<FinishReason>,
}

impl ConverseState {
    pub fn on_frame(&mut self, frame: &Frame) -> Result<Vec<ChatEvent>, ProviderError> {
        let payload: Value = serde_json::from_slice(&frame.payload).unwrap_or(Value::Null);
        if frame.header(":message-type") == Some("exception") {
            let kind = frame.header(":exception-type").unwrap_or("error");
            let msg = payload
                .get("message")
                .or_else(|| payload.get("Message"))
                .and_then(Value::as_str)
                .unwrap_or("stream error");
            return Err(ProviderError::Status {
                status: 400,
                body: format!("{kind}: {msg}"),
            });
        }
        let mut out = Vec::new();
        match frame.header(":event-type").unwrap_or("") {
            "contentBlockStart" => {
                if let Some(tu) = payload.pointer("/start/toolUse") {
                    let index = payload
                        .get("contentBlockIndex")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let slot = self.tool_slots.len();
                    self.tool_slots.push((index, slot));
                    self.calls.push_delta(
                        slot,
                        tu.get("toolUseId").and_then(Value::as_str),
                        tu.get("name").and_then(Value::as_str),
                        None,
                    );
                }
            }
            "contentBlockDelta" => {
                if let Some(t) = payload.pointer("/delta/text").and_then(Value::as_str) {
                    if !t.is_empty() {
                        out.push(ChatEvent::TextDelta(t.to_owned()));
                    }
                }
                if let Some(frag) = payload
                    .pointer("/delta/toolUse/input")
                    .and_then(Value::as_str)
                {
                    let index = payload
                        .get("contentBlockIndex")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if let Some((_, slot)) = self.tool_slots.iter().find(|(i, _)| *i == index) {
                        self.calls.push_delta(*slot, None, None, Some(frag));
                    }
                }
            }
            "messageStop" => {
                let calls = self.calls.take();
                let has_calls = !calls.is_empty();
                if has_calls {
                    out.push(ChatEvent::ToolCalls(calls));
                }
                self.finish = Some(match payload.get("stopReason").and_then(Value::as_str) {
                    Some("tool_use") => FinishReason::ToolCalls,
                    _ if has_calls => FinishReason::ToolCalls,
                    Some("max_tokens") => FinishReason::Length,
                    Some("guardrail_intervened" | "content_filtered") => {
                        FinishReason::ContentFilter
                    }
                    Some("end_turn" | "stop_sequence") => FinishReason::Stop,
                    _ => FinishReason::Other,
                });
            }
            "metadata" => {
                if let Some(u) = payload.get("usage") {
                    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
                    out.push(ChatEvent::Usage(TokenUsage {
                        prompt_tokens: n("inputTokens")
                            + n("cacheReadInputTokens")
                            + n("cacheWriteInputTokens"),
                        completion_tokens: n("outputTokens"),
                        cached_input_tokens: n("cacheReadInputTokens"),
                    }));
                }
            }
            _ => {}
        }
        Ok(out)
    }

    pub fn finish(&mut self) -> ChatEvent {
        ChatEvent::Done(self.finish.take().unwrap_or(FinishReason::Other))
    }
}

/* ---------- provider ---------- */

#[async_trait]
impl ChatProvider for Bedrock {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let body = serde_json::to_vec(&build_body(&request))
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        let url = format!(
            "{}/model/{}/converse-stream",
            self.runtime_url,
            uri_encode(&request.model, false)
        );
        let resp = self
            .signed("POST", &url, "bedrock", &body)?
            .header("accept", "application/vnd.amazon.eventstream")
            .body(body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .or_else(|| v.get("Message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or(body);
            return Err(ProviderError::Status {
                status: status.as_u16(),
                body: msg,
            });
        }
        let bytes = resp.bytes_stream();
        let stream = futures::stream::unfold(
            (
                bytes,
                Vec::<u8>::new(),
                ConverseState::default(),
                std::collections::VecDeque::<ChatEvent>::new(),
                false,
            ),
            |(mut bytes, mut buf, mut state, mut queue, mut done)| async move {
                loop {
                    if let Some(ev) = queue.pop_front() {
                        return Some((Ok(ev), (bytes, buf, state, queue, done)));
                    }
                    if done {
                        return None;
                    }
                    match next_frame(&mut buf) {
                        Ok(Some(frame)) => match state.on_frame(&frame) {
                            Ok(evs) => {
                                queue.extend(evs);
                                continue;
                            }
                            Err(e) => {
                                done = true;
                                return Some((Err(e), (bytes, buf, state, queue, done)));
                            }
                        },
                        Ok(None) => {}
                        Err(e) => {
                            done = true;
                            return Some((Err(e), (bytes, buf, state, queue, done)));
                        }
                    }
                    match bytes.next().await {
                        Some(Ok(chunk)) => buf.extend_from_slice(&chunk),
                        Some(Err(e)) => {
                            done = true;
                            return Some((Err(e.into()), (bytes, buf, state, queue, done)));
                        }
                        None => {
                            done = true;
                            queue.push_back(state.finish());
                        }
                    }
                }
            },
        );
        Ok(stream.boxed())
    }

    /// Text models this account can call: inference profiles (the IDs
    /// newer Claude models need, e.g. `us.anthropic.…`) and on-demand
    /// foundation models.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let control = format!("https://bedrock.{}.amazonaws.com", self.region);
        let mut out = Vec::new();
        let url = format!("{control}/inference-profiles?maxResults=1000&type=SYSTEM_DEFINED");
        if let Ok(resp) = self.signed("GET", &url, "bedrock", b"")?.send().await {
            if let Ok(v) = resp.json::<Value>().await {
                for p in v
                    .get("inferenceProfileSummaries")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = p.get("inferenceProfileId").and_then(Value::as_str) {
                        out.push(ModelInfo {
                            id: id.to_owned(),
                            display_name: p
                                .get("inferenceProfileName")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            owned_by: None,
                            context_length: None,
                        });
                    }
                }
            }
        }
        let url =
            format!("{control}/foundation-models?byOutputModality=TEXT&byInferenceType=ON_DEMAND");
        let resp = self.signed("GET", &url, "bedrock", b"")?.send().await?;
        if !resp.status().is_success() {
            return Ok(out);
        }
        let v: Value = resp.json().await?;
        for m in v
            .get("modelSummaries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = m.get("modelId").and_then(Value::as_str) {
                out.push(ModelInfo {
                    id: id.to_owned(),
                    display_name: m
                        .get("modelName")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    owned_by: m
                        .get("providerName")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    context_length: None,
                });
            }
        }
        Ok(out)
    }
}

impl std::fmt::Debug for Bedrock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bedrock")
            .field("region", &self.region)
            .field("runtime_url", &self.runtime_url)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolSpec;
    use mira_core::message::{ImageData, ToolCallFunction, ToolCallKind};
    use mira_core::Message;
    use mira_core::ToolCall;
    use std::time::Duration;

    #[test]
    fn sigv4_matches_the_aws_test_suite() {
        // AWS SigV4 test suite, "get-vanilla".
        let creds = Credentials {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        // 2015-08-30T12:36:00Z
        let now = UNIX_EPOCH + Duration::from_secs(1_440_938_160);
        let headers = sign(&SignInput {
            method: "GET",
            host: "example.amazonaws.com",
            path: "/",
            query: "",
            extra_headers: &[],
            body: b"",
            service: "service",
            region: "us-east-1",
            creds: &creds,
            now,
        });
        let auth = &headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .unwrap()
            .1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn dates_and_regions() {
        let (d, s) = amz_dates(UNIX_EPOCH + Duration::from_secs(1_709_251_199)); // 2024-02-29T23:59:59Z
        assert_eq!((d.as_str(), s.as_str()), ("20240229", "20240229T235959Z"));
        assert_eq!(
            region_from_url("https://bedrock-runtime.eu-west-1.amazonaws.com").as_deref(),
            Some("eu-west-1")
        );
        assert_eq!(
            region_from_url("https://bedrock-runtime.amazonaws.com"),
            None
        );
        let ini = "[default]\naws_access_key_id = AK\n[profile dev]\nregion = us-west-2\n";
        assert_eq!(
            parse_ini(ini, "default", "aws_access_key_id").as_deref(),
            Some("AK")
        );
        assert_eq!(
            parse_ini(ini, "profile dev", "region").as_deref(),
            Some("us-west-2")
        );
    }

    fn msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: Some(text.into()),
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
            images: vec![],
        }
    }

    #[test]
    fn body_merges_turns_and_maps_tools() {
        let mut assistant = msg(Role::Assistant, "");
        assistant.tool_calls = vec![ToolCall {
            id: "t1".into(),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "read_file".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            },
        }];
        let mut tool = msg(Role::Tool, "fn main() {}");
        tool.tool_call_id = Some("t1".into());
        tool.images = vec![ImageData {
            media_type: "image/png".into(),
            data: "aGk=".into(),
        }];
        let req = ChatRequest {
            model: "anthropic.claude".into(),
            messages: vec![
                msg(Role::System, "be brief"),
                msg(Role::User, "read it"),
                assistant,
                tool,
                msg(Role::User, "thanks"),
            ],
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Read".into(),
                parameters: json!({"type": "object"}),
            }],
            temperature: Some(0.2),
            max_tokens: Some(100),
            reasoning_effort: None,
            response_format: None,
        };
        let b = build_body(&req);
        assert_eq!(b["system"][0]["text"], "be brief");
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3, "{m:#?}");
        assert_eq!(m[1]["content"][0]["toolUse"]["input"]["path"], "a.rs");
        // Tool result and the next user text share one user turn.
        assert_eq!(m[2]["role"], "user");
        assert_eq!(m[2]["content"][0]["toolResult"]["toolUseId"], "t1");
        assert_eq!(
            m[2]["content"][0]["toolResult"]["content"][1]["image"]["format"],
            "png"
        );
        assert_eq!(m[2]["content"][1]["text"], "thanks");
        assert_eq!(
            b["toolConfig"]["tools"][0]["toolSpec"]["inputSchema"]["json"]["type"],
            "object"
        );
        assert_eq!(b["inferenceConfig"]["maxTokens"], 100);
    }

    /// Encode a frame the way Bedrock does (CRCs aren't checked).
    fn frame(event: &str, payload: Value) -> Vec<u8> {
        let mut headers = Vec::new();
        for (k, v) in [(":event-type", event), (":message-type", "event")] {
            headers.push(k.len() as u8);
            headers.extend_from_slice(k.as_bytes());
            headers.push(7);
            headers.extend_from_slice(&(v.len() as u16).to_be_bytes());
            headers.extend_from_slice(v.as_bytes());
        }
        let payload = payload.to_string().into_bytes();
        let total = 12 + headers.len() + payload.len() + 4;
        let mut out = Vec::new();
        out.extend_from_slice(&(total as u32).to_be_bytes());
        out.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&headers);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&[0; 4]);
        out
    }

    #[test]
    fn decodes_a_converse_stream() {
        let mut bytes = Vec::new();
        for (e, p) in [
            ("messageStart", json!({"role": "assistant"})),
            (
                "contentBlockDelta",
                json!({"contentBlockIndex": 0, "delta": {"text": "Let me look."}}),
            ),
            (
                "contentBlockStart",
                json!({"contentBlockIndex": 1, "start": {"toolUse": {"toolUseId": "tu1", "name": "read_file"}}}),
            ),
            (
                "contentBlockDelta",
                json!({"contentBlockIndex": 1, "delta": {"toolUse": {"input": "{\"path\":"}}}),
            ),
            (
                "contentBlockDelta",
                json!({"contentBlockIndex": 1, "delta": {"toolUse": {"input": "\"a.rs\"}"}}}),
            ),
            ("messageStop", json!({"stopReason": "tool_use"})),
            (
                "metadata",
                json!({"usage": {"inputTokens": 10, "outputTokens": 5, "cacheReadInputTokens": 3}}),
            ),
        ] {
            bytes.extend(frame(e, p));
        }
        // Arrives in awkward chunks.
        let mut buf = Vec::new();
        let mut state = ConverseState::default();
        let mut events = Vec::new();
        for chunk in bytes.chunks(7) {
            buf.extend_from_slice(chunk);
            while let Some(f) = next_frame(&mut buf).unwrap() {
                events.extend(state.on_frame(&f).unwrap());
            }
        }
        events.push(state.finish());
        assert!(matches!(&events[0], ChatEvent::TextDelta(t) if t == "Let me look."));
        match &events[1] {
            ChatEvent::ToolCalls(c) => {
                assert_eq!(c[0].id.to_string(), "tu1");
                assert_eq!(c[0].function.arguments, r#"{"path":"a.rs"}"#);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(&events[2], ChatEvent::Usage(u) if u.prompt_tokens == 13 && u.cached_input_tokens == 3)
        );
        assert!(matches!(
            events[3],
            ChatEvent::Done(FinishReason::ToolCalls)
        ));
    }

    #[test]
    fn exceptions_become_errors() {
        let mut bytes = Vec::new();
        let mut headers = Vec::new();
        for (k, v) in [
            (":exception-type", "throttlingException"),
            (":message-type", "exception"),
        ] {
            headers.push(k.len() as u8);
            headers.extend_from_slice(k.as_bytes());
            headers.push(7);
            headers.extend_from_slice(&(v.len() as u16).to_be_bytes());
            headers.extend_from_slice(v.as_bytes());
        }
        let payload = br#"{"message":"slow down"}"#;
        let total = 12 + headers.len() + payload.len() + 4;
        bytes.clear();
        bytes.extend_from_slice(&(total as u32).to_be_bytes());
        bytes.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&headers);
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&[0; 4]);
        let f = next_frame(&mut bytes).unwrap().unwrap();
        let err = ConverseState::default()
            .on_frame(&f)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("throttlingException") && err.contains("slow down"),
            "{err}"
        );
    }
}
