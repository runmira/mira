//! `mira slack`: a Slack bot over Socket Mode.
//!
//! Mention the bot in a channel, or message it directly, and Mira works
//! in the folder `mira slack` was started in. Each Slack thread is one
//! Mira session: reply in the thread to continue, say `stop` to cancel
//! the running turn. The answer streams into a single message that is
//! edited as it grows. When a tool needs approval, Mira posts Allow /
//! Deny buttons in the thread.
//!
//! Socket Mode needs no public URL: the bot opens a websocket to Slack.
//! Tokens: `SLACK_BOT_TOKEN` (`xoxb-…`) and `SLACK_APP_TOKEN` (`xapp-…`),
//! from the environment or `keys:` in `~/.mira/mira.yaml`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use mira_core::ToolCall;
use mira_harness::{Approver, HarnessEvent, Session};
use mira_policy::Decision;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

use crate::acp::SessionFactory;

/// Slack rejects messages much past 40k characters; keep well under.
const MAX_MESSAGE: usize = 3500;
/// How often a streaming answer is edited.
const EDIT_EVERY: Duration = Duration::from_millis(1500);
/// How long an approval waits before it counts as denied.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(clap::Args, Debug, Clone)]
pub struct SlackArgs {
    /// Folder Mira works in. Defaults to the current directory.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Slack user IDs (`U…`) allowed to use the bot, comma-separated.
    /// Without it, anyone who can message the bot can.
    #[arg(long, value_delimiter = ',')]
    allow_users: Vec<String>,
}

pub async fn run(cli: &crate::Cli, args: SlackArgs) -> Result<()> {
    if let Some(dir) = &args.cwd {
        std::env::set_current_dir(dir)
            .with_context(|| format!("can't work in {}", dir.display()))?;
    }
    let cwd = std::env::current_dir()?;
    let factory = crate::acp::MiraFactory::build(cli).await?;
    // `keys:` in mira.yaml are in the environment now.
    let bot_token = std::env::var("SLACK_BOT_TOKEN")
        .context("SLACK_BOT_TOKEN isn't set (the bot token, xoxb-…); see docs/slack.md")?;
    let app_token = std::env::var("SLACK_APP_TOKEN")
        .context("SLACK_APP_TOKEN isn't set (an app-level token with connections:write, xapp-…)")?;
    let api = Arc::new(WebApi::new(bot_token, app_token)?);
    let bot_user = api.bot_user_id().await?;
    println!(
        "mira slack: connected as <@{bot_user}>, working in {}",
        cwd.display()
    );
    let bot = Arc::new(Bot {
        api: api.clone(),
        factory: Arc::new(factory),
        cwd,
        allow: args.allow_users,
        bot_user,
        threads: Mutex::new(HashMap::new()),
        approvals: Arc::new(StdMutex::new(HashMap::new())),
        next_approval: Arc::new(AtomicU64::new(1)),
    });

    loop {
        if let Err(e) = socket(&api, &bot).await {
            eprintln!("mira slack: connection lost ({e:#}); reconnecting");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// One Socket Mode connection, until Slack closes it.
async fn socket(api: &WebApi, bot: &Arc<Bot>) -> Result<()> {
    let url = api.socket_url().await?;
    let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .context("opening the Slack websocket")?;
    let (mut tx, mut rx) = ws.split();
    while let Some(msg) = rx.next().await {
        let text = match msg? {
            Message::Text(t) => t,
            Message::Ping(p) => {
                tx.send(Message::Pong(p)).await?;
                continue;
            }
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        // Acknowledge first: Slack redelivers anything not acked in 3s.
        if let Some(id) = envelope["envelope_id"].as_str() {
            tx.send(Message::Text(json!({"envelope_id": id}).to_string()))
                .await?;
        }
        if envelope["type"] == "disconnect" {
            break;
        }
        bot.clone().handle(envelope);
    }
    Ok(())
}

/* ---------- Slack Web API ---------- */

/// The calls the bot makes. A trait so tests can stand in for Slack.
#[async_trait]
pub(crate) trait SlackApi: Send + Sync {
    /// Post a message; returns its `ts`.
    async fn post(
        &self,
        channel: &str,
        thread: &str,
        text: &str,
        blocks: Option<Value>,
    ) -> Result<String>;
    async fn update(
        &self,
        channel: &str,
        ts: &str,
        text: &str,
        blocks: Option<Value>,
    ) -> Result<()>;
}

struct WebApi {
    http: reqwest::Client,
    base: String,
    bot_token: String,
    app_token: String,
}

impl WebApi {
    fn new(bot_token: String, app_token: String) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("mira/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(30))
                .build()?,
            base: std::env::var("SLACK_API_URL").unwrap_or_else(|_| "https://slack.com/api".into()),
            bot_token,
            app_token,
        })
    }

    async fn call(&self, method: &str, token: &str, body: Value) -> Result<Value> {
        let v: Value = self
            .http
            .post(format!("{}/{method}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("calling Slack {method}"))?
            .json()
            .await?;
        if v["ok"] != true {
            bail!(
                "Slack {method}: {}",
                v["error"].as_str().unwrap_or("unknown error")
            );
        }
        Ok(v)
    }

    async fn socket_url(&self) -> Result<String> {
        let v = self
            .call("apps.connections.open", &self.app_token, json!({}))
            .await?;
        v["url"]
            .as_str()
            .map(str::to_owned)
            .context("Slack returned no socket URL")
    }

    async fn bot_user_id(&self) -> Result<String> {
        let v = self.call("auth.test", &self.bot_token, json!({})).await?;
        v["user_id"]
            .as_str()
            .map(str::to_owned)
            .context("auth.test returned no user")
    }
}

#[async_trait]
impl SlackApi for WebApi {
    async fn post(
        &self,
        channel: &str,
        thread: &str,
        text: &str,
        blocks: Option<Value>,
    ) -> Result<String> {
        let mut body = json!({"channel": channel, "thread_ts": thread, "text": text});
        if let Some(b) = blocks {
            body["blocks"] = b;
        }
        let v = self.call("chat.postMessage", &self.bot_token, body).await?;
        Ok(v["ts"].as_str().unwrap_or_default().to_owned())
    }

    async fn update(
        &self,
        channel: &str,
        ts: &str,
        text: &str,
        blocks: Option<Value>,
    ) -> Result<()> {
        let body = json!({
            "channel": channel,
            "ts": ts,
            "text": text,
            // An empty list clears approval buttons.
            "blocks": blocks.unwrap_or_else(|| json!([])),
        });
        self.call("chat.update", &self.bot_token, body)
            .await
            .map(drop)
    }
}

/* ---------- the bot ---------- */

type Approvals = Arc<StdMutex<HashMap<String, oneshot::Sender<bool>>>>;

pub(crate) struct Bot {
    api: Arc<dyn SlackApi>,
    factory: Arc<dyn SessionFactory>,
    cwd: PathBuf,
    allow: Vec<String>,
    bot_user: String,
    /// One session per Slack thread, keyed `channel/thread_ts`.
    threads: Mutex<HashMap<String, Arc<Thread>>>,
    approvals: Approvals,
    next_approval: Arc<AtomicU64>,
}

struct Thread {
    session: Session,
    /// Held while a turn runs, so messages in a thread go one at a time.
    turn: Mutex<()>,
}

impl Bot {
    fn allowed(&self, user: &str) -> bool {
        self.allow.is_empty() || self.allow.iter().any(|u| u == user)
    }

    /// Route one Socket Mode envelope. Returns at once; work runs on its
    /// own task.
    pub(crate) fn handle(self: Arc<Self>, envelope: Value) {
        match envelope["type"].as_str() {
            Some("events_api") => {
                let event = envelope["payload"]["event"].clone();
                let plain = event["type"] == "message"
                    && event.get("subtype").is_none()
                    && event.get("bot_id").is_none();
                let direct = plain && event["channel_type"] == "im";
                // A reply in a channel thread Mira is already in. Replies
                // that mention the bot also arrive as `app_mention`.
                let follow_up = plain
                    && !direct
                    && event["thread_ts"].is_string()
                    && !event["text"]
                        .as_str()
                        .unwrap_or("")
                        .contains(&format!("<@{}>", self.bot_user));
                if event["type"] == "app_mention" || direct || follow_up {
                    tokio::spawn(async move {
                        if follow_up && !self.knows_thread(&event).await {
                            return;
                        }
                        if let Err(e) = self.on_message(&event).await {
                            eprintln!("mira slack: {e:#}");
                        }
                    });
                }
            }
            Some("interactive") => self.on_click(&envelope["payload"]),
            _ => {}
        }
    }

    async fn knows_thread(&self, event: &Value) -> bool {
        let key = format!(
            "{}/{}",
            event["channel"].as_str().unwrap_or(""),
            event["thread_ts"].as_str().unwrap_or("")
        );
        self.threads.lock().await.contains_key(&key)
    }

    async fn on_message(&self, event: &Value) -> Result<()> {
        let (Some(channel), Some(user), Some(ts)) = (
            event["channel"].as_str(),
            event["user"].as_str(),
            event["ts"].as_str(),
        ) else {
            return Ok(());
        };
        let thread_ts = event["thread_ts"].as_str().unwrap_or(ts);
        let text = strip_mentions(event["text"].as_str().unwrap_or(""), &self.bot_user);
        if text.is_empty() {
            return Ok(());
        }
        if !self.allowed(user) {
            self.api
                .post(
                    channel,
                    thread_ts,
                    "Sorry, you're not on this bot's allow list.",
                    None,
                )
                .await?;
            return Ok(());
        }

        let key = format!("{channel}/{thread_ts}");
        let existing = self.threads.lock().await.get(&key).cloned();
        if text.eq_ignore_ascii_case("stop") {
            if let Some(t) = existing {
                t.session.cancel().await;
            }
            return Ok(());
        }
        let thread = match existing {
            Some(t) => t,
            None => {
                let approver = Arc::new(SlackApprover {
                    api: self.api.clone(),
                    channel: channel.to_owned(),
                    thread: thread_ts.to_owned(),
                    approvals: self.approvals.clone(),
                    next: self.next_approval.clone(),
                });
                let (session, _policy) = self.factory.create(self.cwd.clone(), approver).await?;
                let t = Arc::new(Thread {
                    session,
                    turn: Mutex::new(()),
                });
                self.threads.lock().await.insert(key, t.clone());
                t
            }
        };

        let _turn = thread.turn.lock().await;
        let reply = self
            .api
            .post(channel, thread_ts, "_Working…_", None)
            .await?;
        let mut answer = Answer::default();
        let mut last_edit = Instant::now();
        let mut stream = thread.session.send(text).await;
        while let Some(event) = stream.next().await {
            match event {
                HarnessEvent::Token(t) => answer.text.push_str(&t),
                HarnessEvent::ToolStart(call) => answer.tools.push(tool_line(&call)),
                HarnessEvent::Warning(w) => answer.text.push_str(&format!("\n> :warning: {w}\n")),
                HarnessEvent::Done => break,
                _ => continue,
            }
            if last_edit.elapsed() >= EDIT_EVERY {
                let _ = self
                    .api
                    .update(channel, &reply, &answer.render(true), None)
                    .await;
                last_edit = Instant::now();
            }
        }

        // The final answer, split when it's too long for one message.
        let mut parts = chunks(&answer.render(false), MAX_MESSAGE).into_iter();
        let first = parts.next().unwrap_or_else(|| "_(no reply)_".into());
        self.api.update(channel, &reply, &first, None).await?;
        for part in parts {
            self.api.post(channel, thread_ts, &part, None).await?;
        }
        Ok(())
    }

    fn on_click(&self, payload: &Value) {
        let Some(action) = payload["actions"].get(0) else {
            return;
        };
        let user = payload["user"]["id"].as_str().unwrap_or("");
        if !self.allowed(user) {
            return;
        }
        let allow = action["action_id"] == "mira_allow";
        let Some(id) = action["value"].as_str() else {
            return;
        };
        let Some(tx) = self.approvals.lock().unwrap().remove(id) else {
            return;
        };
        let _ = tx.send(allow);
        // Swap the buttons for who decided.
        if let (Some(channel), Some(ts)) = (
            payload["channel"]["id"].as_str(),
            payload["message"]["ts"].as_str(),
        ) {
            let original = payload["message"]["text"].as_str().unwrap_or("").to_owned();
            let verdict = if allow { "Allowed" } else { "Denied" };
            let text = format!("{original}\n*{verdict}* by <@{user}>");
            let (api, channel, ts) = (self.api.clone(), channel.to_owned(), ts.to_owned());
            tokio::spawn(async move {
                let _ = api.update(&channel, &ts, &text, None).await;
            });
        }
    }
}

#[derive(Default)]
struct Answer {
    text: String,
    tools: Vec<String>,
}

impl Answer {
    fn render(&self, working: bool) -> String {
        let mut out = String::new();
        if !self.tools.is_empty() {
            // The last few steps are enough to follow along.
            let skip = self.tools.len().saturating_sub(8);
            if skip > 0 {
                out.push_str(&format!("_…{skip} earlier steps_\n"));
            }
            for t in &self.tools[skip..] {
                out.push_str(&format!("{t}\n"));
            }
            out.push('\n');
        }
        out.push_str(&to_mrkdwn(self.text.trim()));
        if working {
            out.push_str("\n\n_Working…_");
        }
        out
    }
}

struct SlackApprover {
    api: Arc<dyn SlackApi>,
    channel: String,
    thread: String,
    approvals: Approvals,
    next: Arc<AtomicU64>,
}

#[async_trait]
impl Approver for SlackApprover {
    async fn approve(&self, call: &ToolCall, decision: Decision) -> bool {
        match decision {
            Decision::Allow => return true,
            Decision::Deny => return false,
            Decision::Ask => {}
        }
        let id = format!("a{}", self.next.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.approvals.lock().unwrap().insert(id.clone(), tx);
        let text = format!("Mira wants to run {}", tool_line(call));
        let blocks = json!([
            {"type": "section", "text": {"type": "mrkdwn", "text": text}},
            {"type": "actions", "elements": [
                {"type": "button", "action_id": "mira_allow", "value": id,
                 "style": "primary", "text": {"type": "plain_text", "text": "Allow"}},
                {"type": "button", "action_id": "mira_deny", "value": id,
                 "style": "danger", "text": {"type": "plain_text", "text": "Deny"}},
            ]},
        ]);
        if self
            .api
            .post(&self.channel, &self.thread, &text, Some(blocks))
            .await
            .is_err()
        {
            self.approvals.lock().unwrap().remove(&id);
            return false;
        }
        let answer = tokio::time::timeout(APPROVAL_TIMEOUT, rx).await;
        self.approvals.lock().unwrap().remove(&id);
        matches!(answer, Ok(Ok(true)))
    }
}

/* ---------- text helpers ---------- */

/// `▸ bash: cargo test` style summary of a tool call.
fn tool_line(call: &ToolCall) -> String {
    let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);
    let detail = ["command", "path", "pattern", "query", "url", "description"]
        .iter()
        .find_map(|k| args.get(k).and_then(Value::as_str))
        .map(|d| {
            let flat = d.split_whitespace().collect::<Vec<_>>().join(" ");
            let short: String = flat.chars().take(80).collect();
            let more = if flat.chars().count() > 80 { "…" } else { "" };
            format!(": `{}{more}`", short.replace('`', "'"))
        })
        .unwrap_or_default();
    format!("▸ *{}*{detail}", call.function.name)
}

/// Drop `<@BOT>` mentions and surrounding space.
fn strip_mentions(text: &str, bot: &str) -> String {
    text.replace(&format!("<@{bot}>"), "").trim().to_owned()
}

/// Markdown → Slack mrkdwn, for the parts that differ: bold, headings,
/// links. Code fences and inline code are the same.
fn to_mrkdwn(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut in_code = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push_str("```\n");
            continue;
        }
        if in_code {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let trimmed = line.trim_start();
        let line = match trimmed.trim_start_matches('#') {
            rest if trimmed.starts_with('#') && rest.starts_with(' ') => {
                format!("*{}*", rest.trim())
            }
            _ => line.to_owned(),
        };
        out.push_str(&links(&line.replace("**", "*")));
        out.push('\n');
    }
    out.trim_end().to_owned()
}

/// `[text](url)` → `<url|text>`.
fn links(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let parsed = after.find("](").and_then(|close| {
            let url_part = &after[close + 2..];
            url_part
                .find(')')
                .map(|end| (&after[..close], &url_part[..end], &url_part[end + 1..]))
        });
        match parsed {
            Some((text, url, tail)) if url.starts_with("http") => {
                out.push_str(&rest[..open]);
                out.push_str(&format!("<{url}|{text}>"));
                rest = tail;
            }
            _ => {
                out.push_str(&rest[..open + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Split into messages of at most `max` characters, at line breaks when
/// possible, keeping code fences balanced across the split.
fn chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    for line in text.lines() {
        if cur.chars().count() + line.chars().count() + 1 > max && !cur.is_empty() {
            if in_code {
                cur.push_str("```");
            }
            out.push(std::mem::take(&mut cur));
            if in_code {
                cur.push_str("```\n");
            }
        }
        // A single line longer than a message is cut hard.
        let mut line = line;
        while line.chars().count() > max {
            let cut: usize = line
                .char_indices()
                .nth(max)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            out.push(line[..cut].to_owned());
            line = &line[cut..];
        }
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim_end().to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::{self, BoxStream};
    use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_harness::SessionConfig;
    use mira_policy::{Mode, Policy, PolicyConfig};
    use mira_tools::{builtin, Registry, ToolContext};

    #[test]
    fn text_helpers() {
        assert_eq!(
            strip_mentions("<@UBOT> fix the build", "UBOT"),
            "fix the build"
        );
        assert_eq!(
            to_mrkdwn("## Done\n**Fixed** it, see [the PR](https://x.y/1).\n```\n**raw**\n```"),
            "*Done*\n*Fixed* it, see <https://x.y/1|the PR>.\n```\n**raw**\n```"
        );
        let long = format!("```\n{}\n```", "line\n".repeat(50));
        let parts = chunks(&long, 60);
        assert!(parts.len() > 1);
        for p in &parts {
            assert!(p.chars().count() <= 64, "{p}");
            assert_eq!(p.matches("```").count() % 2, 0, "fences balanced: {p}");
        }
    }

    /// Records every message the bot sends.
    #[derive(Default)]
    struct FakeSlack {
        sent: StdMutex<Vec<(String, String, Option<Value>)>>,
    }

    #[async_trait]
    impl SlackApi for FakeSlack {
        async fn post(
            &self,
            _c: &str,
            _t: &str,
            text: &str,
            blocks: Option<Value>,
        ) -> Result<String> {
            let mut s = self.sent.lock().unwrap();
            s.push(("post".into(), text.into(), blocks));
            Ok(format!("ts{}", s.len()))
        }
        async fn update(
            &self,
            _c: &str,
            _ts: &str,
            text: &str,
            blocks: Option<Value>,
        ) -> Result<()> {
            self.sent
                .lock()
                .unwrap()
                .push(("update".into(), text.into(), blocks));
            Ok(())
        }
    }

    /// Writes a file (which asks in manual mode), then answers.
    struct Scripted(StdMutex<usize>);

    #[async_trait]
    impl ChatProvider for Scripted {
        async fn stream(
            &self,
            _r: ChatRequest,
        ) -> std::result::Result<
            BoxStream<'static, std::result::Result<ChatEvent, ProviderError>>,
            ProviderError,
        > {
            let n = {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                *n
            };
            let events = if n == 1 {
                vec![
                    ChatEvent::ToolCalls(vec![ToolCall {
                        id: "c1".into(),
                        kind: ToolCallKind::Function,
                        function: ToolCallFunction {
                            name: "write_file".into(),
                            arguments: json!({"path": "note.txt", "content": "hi"}).to_string(),
                        },
                    }]),
                    ChatEvent::Done(FinishReason::ToolCalls),
                ]
            } else {
                vec![
                    ChatEvent::TextDelta("**Done.** Wrote note.txt".into()),
                    ChatEvent::Done(FinishReason::Stop),
                ]
            };
            Ok(stream::iter(events.into_iter().map(Ok)).boxed())
        }
    }

    struct TestFactory;

    #[async_trait]
    impl SessionFactory for TestFactory {
        async fn create(
            &self,
            cwd: PathBuf,
            approver: Arc<dyn Approver>,
        ) -> Result<(Session, Arc<Mutex<Policy>>)> {
            let mut registry = Registry::new();
            builtin::register_core(&mut registry);
            let policy = Arc::new(Mutex::new(Policy::from_config(&PolicyConfig {
                mode: Mode::Manual,
                ..Default::default()
            })?));
            let ctx = ToolContext::new(
                cwd.clone(),
                Arc::new(mira_sandbox::Sandbox::for_workspace(&cwd)),
            );
            let session = Session::new(
                SessionConfig::new("m"),
                "sys",
                Arc::new(Scripted(StdMutex::new(0))),
                Arc::new(registry),
                policy.clone(),
                approver,
                ctx,
            );
            Ok((session, policy))
        }
    }

    #[tokio::test]
    async fn a_mention_runs_a_turn_with_a_button_approval() {
        let dir = tempfile::tempdir().unwrap();
        let slack = Arc::new(FakeSlack::default());
        let bot = Arc::new(Bot {
            api: slack.clone(),
            factory: Arc::new(TestFactory),
            cwd: dir.path().to_path_buf(),
            allow: vec!["UME".into()],
            bot_user: "UBOT".into(),
            threads: Mutex::new(HashMap::new()),
            approvals: Arc::new(StdMutex::new(HashMap::new())),
            next_approval: Arc::new(AtomicU64::new(1)),
        });
        let mention = |user: &str| {
            json!({"type": "events_api", "payload": {"event": {
                "type": "app_mention", "channel": "C1", "user": user,
                "ts": "100.1", "text": "<@UBOT> write a note",
            }}})
        };

        // Someone not on the allow list is turned away.
        bot.clone().handle(mention("USTRANGER"));
        // The allowed user gets a turn; it stops at the approval.
        bot.clone().handle(mention("UME"));
        let approval_id = loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let sent = slack.sent.lock().unwrap();
            if let Some((_, _, Some(blocks))) = sent.iter().find(|(_, _, b)| b.is_some()) {
                break blocks[1]["elements"][0]["value"]
                    .as_str()
                    .unwrap()
                    .to_owned();
            }
        };
        bot.clone()
            .handle(json!({"type": "interactive", "payload": {
                "user": {"id": "UME"},
                "actions": [{"action_id": "mira_allow", "value": approval_id}],
                "channel": {"id": "C1"},
                "message": {"ts": "ts9", "text": "Mira wants to run …"},
            }}));

        let final_text = loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let sent = slack.sent.lock().unwrap();
            if let Some((_, t, _)) = sent
                .iter()
                .rev()
                .find(|(k, t, _)| k == "update" && t.contains("Done"))
            {
                break t.clone();
            }
        };
        assert!(
            final_text.contains("*Done.* Wrote note.txt"),
            "{final_text}"
        );
        assert!(
            final_text.contains("▸ *write_file*: `note.txt`"),
            "{final_text}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
            "hi"
        );
        // A plain reply in that thread continues it; one in a thread Mira
        // isn't in is ignored.
        let reply = |thread: &str| {
            json!({"type": "events_api", "payload": {"event": {
                "type": "message", "channel_type": "channel", "channel": "C1",
                "user": "UME", "ts": "100.5", "thread_ts": thread, "text": "thanks, now tidy up",
            }}})
        };
        let finals = |s: &FakeSlack| {
            s.sent
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, t, _)| k == "update" && t.contains("Done") && !t.contains("Working"))
                .count()
        };
        bot.clone().handle(reply("999.9"));
        bot.clone().handle(reply("100.1"));
        for _ in 0..100 {
            if finals(&slack) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            finals(&slack),
            2,
            "one more turn, for the known thread only"
        );

        let sent = slack.sent.lock().unwrap();
        assert!(sent
            .iter()
            .any(|(_, t, _)| t.contains("not on this bot's allow list")));
        assert!(sent
            .iter()
            .any(|(_, t, _)| t.contains("*Allowed* by <@UME>")));
    }
}
