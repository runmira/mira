//! ACP `session/update` → Mira's normalized turn events.
//!
//! Two ideas are load-bearing here, both borrowed from what works in a
//! production multi-agent host:
//!
//! 1. **Normalize at the adapter boundary.** Nothing above this module knows
//!    ACP exists. The generic runtime records intent and state; this is the
//!    only place that speaks a vendor's dialect.
//! 2. **Tag every event with its wire origin.** Each normalized event keeps
//!    the exact ACP variant it came from. Agent integrations fail in ways
//!    that are impossible to reproduce from the normalized shape alone, and
//!    knowing "this was `tool_call_update` from grok" versus "a raw
//!    `cursor/*` notification we didn't model" is the difference between a
//!    five-minute fix and an afternoon.
//!
//! Unknown variants and unknown vendor notifications are **absorbed, not
//! errors**. Real traffic carries vendor extensions (`cursor/*`,
//! `_x.ai/*`) and the spec's own docs under-document `SessionUpdate`, so a
//! closed match would break the pane on an agent upgrade.

use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, SessionConfigOption,
    RequestPermissionRequest, SessionConfigKind, SessionConfigSelectOptions,
    SessionMode, SessionNotification, SessionUpdate, StopReason,
    ToolCall, ToolCallContent,
    SessionConfigSelectOption, ToolCallLocation, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
// Three-state container: absent, explicitly null, or a value. Re-exported
// because it appears in the public `SessionInfo` surface. It lives at the
// schema root rather than under `v1`, unlike most of the protocol types.
pub use agent_client_protocol::schema::MaybeUndefined as AcpMaybeUndefined;
// These appear in our own public types (`PermissionRequest.kind`), so
// callers get them from here rather than reaching into the SDK.
pub use agent_client_protocol::schema::v1::PermissionOptionKind as AcpPermissionOptionKind;
pub use agent_client_protocol::schema::v1::ToolKind as AcpToolKind;
// Appears in `ToolCallState.status`, so callers get it from here.
pub use agent_client_protocol::schema::v1::ToolCallStatus as AcpToolCallStatus;
use agent_client_protocol::schema::MaybeUndefined;
use serde::{Deserialize, Serialize};

/// Which wire message produced an event. Retained deliberately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", content = "detail", rename_all = "snake_case")]
pub enum EventSource {
    /// A modelled ACP `session/update` variant. Held as an owned `String`
    /// rather than `&'static str` so the type can derive `Deserialize`.
    Acp { variant: String },
    /// A JSON-RPC notification we don't model. Carried through as a trace
    /// entry so nothing is silently dropped.
    Unmodelled { method: String },
}

impl EventSource {
    fn variant(name: &str) -> Self {
        EventSource::Acp {
            variant: name.to_string(),
        }
    }
}

/// Normalized tool-call state.
///
/// ACP splits creation (`tool_call`) from patching (`tool_call_update`), and
/// the patch semantics have a sharp edge: for `name`, omission *and* `null`
/// both mean "leave unchanged", because v1 cannot clear a name once set. A
/// naive `Option<Option<String>>` merge gets that wrong and will blank a
/// tool's name on the first update that omits it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCallState {
    pub id: String,
    pub title: String,
    pub name: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: ToolCallStatus,
    pub content: Vec<ToolContent>,
    pub locations: Vec<Location>,
    pub raw_input: Option<serde_json::Value>,
    pub raw_output: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    /// Plain output.
    Content { text: String },
    /// A file edit, carried with both sides so the transcript can render a
    /// diff without shelling out to git.
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: Option<String>,
    },
    /// A terminal the agent asked us to run; identified by the terminal id
    /// we handed back.
    Terminal { terminal_id: String },
}

/// One plan-limit window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    /// The agent's own name for it (`five_hour`, `seven_day`, …).
    pub name: String,
    /// Share used, 0.0–1.0.
    pub utilization: f64,
    /// When it resets, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MiraEvent {
    /// Streamed assistant prose. Chunks sharing a `message_id` belong to one
    /// message; a change of id starts a new one.
    AssistantText { message_id: Option<String>, text: String },
    /// Streamed user-side prose (the agent echoing input).
    UserText { message_id: Option<String>, text: String },
    /// The agent's reasoning, kept separate so the UI can style it apart
    /// from the answer.
    AgentThought { message_id: Option<String>, text: String },
    ToolCall(ToolCallState),
    ToolCallUpdate(ToolCallState),
    Plan { entries: Vec<PlanEntry> },
    /// Slash commands the agent advertises.
    Commands { names: Vec<String> },
    Modes { current: String, available: Vec<SessionModeView> },
    /// The agent's own view of model / reasoning / permission config. This
    /// is where ACP gets its model selector — there is no set-model method,
    /// it's a `configOptions` entry with `category: "model"`.
    ConfigOptions { options: Vec<SessionConfigView> },
    SessionInfo { title: Option<String>, updated_at: Option<String> },
    Usage { used: u64, size: u64, cost: Option<(f64, String)> },
    /// The account's plan limits (Claude's 5-hour and weekly windows):
    /// how much of each is used, and when it resets.
    Limits { windows: Vec<LimitWindow> },
    /// What the agent session has spent so far, per model. Running totals,
    /// not per-turn amounts — agents report it that way, and only the
    /// recorder knows what it has already counted. `session` is the agent's
    /// own session id, so a resumed session isn't counted twice.
    Spend { session: Option<String>, models: Vec<ModelSpend> },
    /// Content we could not model (an image, a resource link). Kept as a
    /// trace rather than dropped.
    Unmodelled { source: EventSource, reason: String },
}

/// One model's running spend within an agent session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelSpend {
    /// The model id as the agent names it; empty when the agent doesn't say.
    pub model: String,
    /// Fresh input tokens (not cache reads).
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input served from (or written to) the prompt cache.
    pub cached_input_tokens: u64,
    /// The agent's own estimate, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

/// One normalized event plus its provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    pub source: EventSource,
    pub event: MiraEvent,
}

/// Flatten a content block to text when it has any. Non-text blocks return
/// `None` here and are surfaced as attachments by the caller.
pub fn content_to_text(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

fn locations(src: &[ToolCallLocation]) -> Vec<Location> {
    src.iter()
        .map(|l| Location {
            path: l.path.to_string_lossy().into_owned(),
            line: l.line,
        })
        .collect()
}

fn tool_contents(src: &[ToolCallContent]) -> Vec<ToolContent> {
    src.iter()
        .filter_map(|c| match c {
            ToolCallContent::Content(c) => {
                content_to_text(&c.content).map(|text| ToolContent::Content { text })
            }
            ToolCallContent::Diff(d) => Some(ToolContent::Diff {
                path: d.path.to_string_lossy().into_owned(),
                old_text: d.old_text.clone(),
                new_text: Some(d.new_text.clone()),
            }),
            ToolCallContent::Terminal(t) => Some(ToolContent::Terminal {
                terminal_id: t.terminal_id.to_string(),
            }),
            _ => None,
        })
        .collect()
}

/// ACP's `tool_call` — a new call.
pub fn from_tool_call(call: &ToolCall) -> NormalizedEvent {
    NormalizedEvent {
        source: EventSource::variant("tool_call"),
        event: MiraEvent::ToolCall(ToolCallState {
            id: call.tool_call_id.to_string(),
            title: call.title.clone(),
            name: call.name.as_ref().map(|n| n.to_string()),
            kind: Some(call.kind.clone()),
            status: call.status.clone(),
            content: tool_contents(&call.content),
            locations: locations(&call.locations),
            raw_input: call.raw_input.clone(),
            raw_output: call.raw_output.clone(),
        }),
    }
}

/// ACP's `tool_call_update` — a patch.
///
/// The merge rules are the spec's, not ours: `content` and `locations`
/// *append*, everything else replaces only when present. Appending is
/// deliberate — a long-running `execute` streams its output across many
/// updates, and replacing would show only the final chunk.
pub fn from_tool_call_update(
    base: Option<&ToolCallState>,
    update: &ToolCallUpdate,
) -> NormalizedEvent {
    // `ToolCallUpdateFields` is `#[non_exhaustive]`, so it is read field by
    // field rather than destructured — a new upstream field must not become
    // a compile error here, and must not be silently ignored either.
    let f: &ToolCallUpdateFields = &update.fields;
    let (kind, status, title) = (f.kind.as_ref(), f.status.as_ref(), f.title.as_ref());
    let (content, locs) = (f.content.as_ref(), f.locations.as_ref());
    let (raw_input, raw_output) = (f.raw_input.as_ref(), f.raw_output.as_ref());

    let mut state = base.cloned().unwrap_or_default();
    if state.id.is_empty() {
        state.id = update.tool_call_id.to_string();
    }
    if let Some(t) = title {
        state.title = t.clone();
    }
    // `name` is Option<Option<String>> upstream: outer None = "not
    // reported", inner None = explicit null. v1 treats *both* as "leave
    // unchanged", so we only overwrite on Some(Some(_)).
    // `name` is deliberately not merged: the v1 schema has no `name` field
    // on `ToolCallUpdateFields` at all, so a tool's programmatic name is
    // set at creation and can never be corrected afterwards. Anything else
    // here would be inventing a field.
    if let Some(k) = kind {
        state.kind = Some(k.clone());
    }
    if let Some(s) = status {
        state.status = s.clone();
    }
    if let Some(c) = content {
        state.content.extend(tool_contents(c));
    }
    if let Some(l) = locs {
        state.locations.extend(locations(l));
    }
    if let Some(i) = raw_input {
        state.raw_input = Some(i.clone());
    }
    if let Some(o) = raw_output {
        state.raw_output = Some(o.clone());
    }
    NormalizedEvent {
        source: EventSource::variant("tool_call_update"),
        event: MiraEvent::ToolCallUpdate(state),
    }
}

/// A permission decision the agent is blocked on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub session_id: String,
    pub tool_call_id: String,
    pub title: String,
    pub kind: Option<ToolKind>,
    pub options: Vec<PermissionChoice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionChoice {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionOptionKind,
}

impl PermissionChoice {
    /// UI semantics come from `kind`, never from the option id — those are
    /// vendor-defined and observed to be `"allow"`, `"allow-once"`, `"yes"`,
    /// and others.
    pub fn is_allow(&self) -> bool {
        matches!(
            self.kind,
            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
        )
    }
    pub fn is_persistent(&self) -> bool {
        matches!(
            self.kind,
            PermissionOptionKind::AllowAlways | PermissionOptionKind::RejectAlways
        )
    }
}

pub fn permission_options(src: &[PermissionOption]) -> Vec<PermissionChoice> {
    src.iter()
        .map(|o| PermissionChoice {
            option_id: o.option_id.to_string(),
            name: o.name.clone(),
            kind: o.kind.clone(),
        })
        .collect()
}

/// How a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnEnd {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

impl TurnEnd {
    pub fn from_stop_reason(r: &StopReason) -> Self {
        match r {
            StopReason::EndTurn => TurnEnd::EndTurn,
            StopReason::MaxTokens => TurnEnd::MaxTokens,
            StopReason::MaxTurnRequests => TurnEnd::MaxTurnRequests,
            StopReason::Refusal => TurnEnd::Refusal,
            StopReason::Cancelled => TurnEnd::Cancelled,
            _ => TurnEnd::EndTurn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ToolCallUpdateFields` is `#[non_exhaustive]`, so it can only be
    /// built through its setters — struct literals are a compile error.
    fn update(id: &str, f: ToolCallUpdateFields) -> ToolCallUpdate {
        // `ToolCallId` is a newtype over `String`, so the id is owned here
        // rather than borrowed out of `&str`.
        ToolCallUpdate::new(agent_client_protocol::schema::v1::ToolCallId::new(
            id.to_string(),
        ), f)
    }

    fn text_block(s: &str) -> ToolCallContent {
        ToolCallContent::Content(agent_client_protocol::schema::v1::Content::new(
            ContentBlock::Text(agent_client_protocol::schema::v1::TextContent::new(
                s.to_string(),
            )),
        ))
    }

    /// `PermissionOption` is `#[non_exhaustive]`, so it is built through
    /// its constructor rather than a struct literal.
    fn perm(id: &str, kind: PermissionOptionKind) -> PermissionOption {
        PermissionOption::new(id.to_string(), id.to_string(), kind)
    }

    fn state(id: &str) -> ToolCallState {
        ToolCallState {
            id: id.into(),
            title: "Read main.py".into(),
            name: Some("read_file".into()),
            kind: Some(ToolKind::Read),
            status: ToolCallStatus::Pending,
            ..Default::default()
        }
    }

    #[test]
    fn a_tool_name_is_creation_only_and_survives_every_update() {
        // v1's `ToolCallUpdateFields` has no `name` field, so a tool's
        // programmatic name is fixed at creation. Pinned because a later
        // "just add name to the update fields" refactor would otherwise look
        // harmless while breaking agents that rely on the initial value.
        let mut acc = state("call_1");
        for status in [ToolCallStatus::InProgress, ToolCallStatus::Completed] {
            let update = update("call_1", ToolCallUpdateFields::default().status(status.clone()));
            if let MiraEvent::ToolCallUpdate(s) = from_tool_call_update(Some(&acc), &update).event {
                acc = s;
            }
            assert_eq!(acc.name.as_deref(), Some("read_file"));
        }
        assert_eq!(acc.status, ToolCallStatus::Completed);
    }

    #[test]
    fn streamed_content_appends_rather_than_replacing() {
        // A long `execute` sends output across many updates; replacing would
        // leave only the final chunk on screen.
        let first = update(
            "c",
            ToolCallUpdateFields::default().content(vec![text_block("line 1\n")]),
        );
        let second = update(
            "c",
            ToolCallUpdateFields::default().content(vec![text_block("line 2\n")]),
        );
        let mut acc = state("c");
        if let MiraEvent::ToolCallUpdate(s) = from_tool_call_update(Some(&acc), &first).event {
            acc = s;
        }
        if let MiraEvent::ToolCallUpdate(s) = from_tool_call_update(Some(&acc), &second).event {
            acc = s;
        }
        assert_eq!(acc.content.len(), 2, "content must accumulate across updates");
    }

    #[test]
    fn an_update_for_an_unknown_call_still_produces_state() {
        // Agents may patch a call we never saw created (a resumed session,
        // or one whose creation we missed). Refusing to model it would drop
        // the tail of a turn.
        let update = update(
            "orphan",
            ToolCallUpdateFields::default()
                .title("Later call")
                .status(ToolCallStatus::InProgress),
        );
        let ev = from_tool_call_update(None, &update);
        let MiraEvent::ToolCallUpdate(s) = ev.event else {
            panic!()
        };
        assert_eq!(s.id, "orphan");
        assert_eq!(s.title, "Later call");
    }

    #[test]
    fn provenance_is_kept_on_every_event() {
        let ev = from_tool_call_update(Some(&state("x")), &update("x", ToolCallUpdateFields::default()));
        assert_eq!(
            ev.source,
            EventSource::Acp {
                variant: "tool_call_update".to_string()
            }
        );
    }

    #[test]
    fn permission_semantics_come_from_kind_not_the_option_id() {
        let options = permission_options(&[
            perm("yes", PermissionOptionKind::AllowOnce),
            perm("definitely-not", PermissionOptionKind::RejectOnce),
        ]);
        // An id-based check would have read "yes" correctly and
        // "definitely-not" as an approval.
        assert!(options[0].is_allow());
        assert!(!options[1].is_allow());
    }

    #[test]
    fn permission_persistence_is_distinguished_from_a_one_off() {
        let opts = permission_options(&[
            perm("a", PermissionOptionKind::AllowAlways),
            perm("b", PermissionOptionKind::AllowOnce),
        ]);
        assert!(opts[0].is_persistent());
        assert!(!opts[1].is_persistent());
    }

    #[test]
    fn stop_reasons_map_onto_our_turn_end() {
        assert_eq!(TurnEnd::from_stop_reason(&StopReason::Cancelled), TurnEnd::Cancelled);
        assert_eq!(TurnEnd::from_stop_reason(&StopReason::EndTurn), TurnEnd::EndTurn);
    }
}

/// `sessionUpdate` variants this build knows exist but does not model.
///
/// Split from the wildcard so a *recognised* gap can be traced quietly while
/// a genuinely unknown one is surfaced. Without this, an agent that emits
/// `PlanUpdate` (which has no capability gate, so it may send it whether or
/// not we asked) puts a warning line in the transcript every turn — and a
/// user who sees a warning often enough stops reading them.
///
/// The three entries are the SDK's unstable variants, gated behind
/// `unstable_plan_operations` and `unstable_session_notices`, which this
/// crate deliberately does not enable. When they stabilise, model them
/// properly — `PlanUpdate` is a *revision* and `PlanRemoved` a *deletion*,
/// neither of which today's `PlanEntry` can express — and then delete the
/// corresponding lines here.
const RECOGNISED_UNMODELLED: &[&str] = &["plan_update", "plan_removed", "notice"];

/// The `sessionUpdate` tag for a wire value, for provenance.
///
/// Derived by re-serializing rather than matched by hand, so a new ACP
/// variant is labelled correctly the day it's added instead of showing up
/// as a guess or "unknown".
fn update_tag(update: &SessionUpdate) -> String {
    serde_json::to_value(update)
        .ok()
        .and_then(|v| {
            v.get("sessionUpdate")
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Normalize one `session/update` notification.
///
/// Returns `None` only when the payload doesn't parse at all. An update
/// whose *content* we can't model is still returned, as
/// [`MiraEvent::Unmodelled`], because dropping it silently would make an
/// agent upgrade look like a silent hang.
pub fn from_session_notification(params: &serde_json::Value) -> Option<NormalizedEvent> {
    // Read the discriminator *before* deserializing.
    //
    // `SessionUpdate` is an internally-tagged enum with no catch-all
    // variant, so serde rejects any tag it does not know — including the
    // three unstable ones this crate does not enable, any variant a future
    // ACP release adds, and any vendor extension. Deserializing first meant
    // all of those returned `None` here and were dropped by the caller,
    // which is precisely the "silent gap indistinguishable from a hung
    // agent" failure this whole `Unmodelled` path exists to prevent. The
    // wildcard arm below was unreachable for exactly that reason.
    //
    // So: tag first, typed parse second. An unrecognised tag becomes an
    // `Unmodelled` event carrying that tag, which is also all the
    // provenance anyone needs to diagnose it.
    let raw_tag = params
        .get("update")
        .and_then(|u| u.get("sessionUpdate"))
        .and_then(|t| t.as_str())
        .map(str::to_string);

    let notif: SessionNotification = match serde_json::from_value(params.clone()) {
        Ok(n) => n,
        Err(_) => {
            // No typed value, but we still know what arrived.
            let tag = raw_tag.unwrap_or_else(|| "unknown".to_string());
            let source = EventSource::Unmodelled {
                method: tag.clone(),
            };
            // Two fields, one value: `NormalizedEvent` carries provenance
            // alongside the event, and both are the same thing here.
            let provenance = source.clone();
            let reason = if RECOGNISED_UNMODELLED.contains(&tag.as_str()) {
                format!("recognised, not yet modelled: {tag}")
            } else {
                format!("unhandled sessionUpdate: {tag}")
            };
            return Some(NormalizedEvent {
                event: MiraEvent::Unmodelled {
                    source: provenance,
                    reason,
                },
                source,
            });
        }
    };
    let update = notif.update;
    let source = EventSource::variant(&update_tag(&update));

    let event = match &update {
        SessionUpdate::UserMessageChunk(c) => MiraEvent::UserText {
            message_id: c.message_id.as_ref().map(|m| m.to_string()),
            text: text_of(&c.content).unwrap_or_default(),
        },
        SessionUpdate::AgentMessageChunk(c) => MiraEvent::AssistantText {
            message_id: c.message_id.as_ref().map(|m| m.to_string()),
            text: text_of(&c.content).unwrap_or_default(),
        },
        SessionUpdate::AgentThoughtChunk(c) => MiraEvent::AgentThought {
            message_id: c.message_id.as_ref().map(|m| m.to_string()),
            text: text_of(&c.content).unwrap_or_default(),
        },
        SessionUpdate::ToolCall(call) => return Some(from_tool_call(call)),
        SessionUpdate::ToolCallUpdate(u) => {
            // Without the creation-time state this is a partial view; the
            // caller folds it onto the call it continues.
            return Some(from_tool_call_update(None, u));
        }
        SessionUpdate::Plan(p) => MiraEvent::Plan {
            entries: p
                .entries
                .iter()
                .map(|e| PlanEntry {
                    content: e.content.clone(),
                    priority: format!("{:?}", e.priority).to_lowercase(),
                    status: format!("{:?}", e.status).to_lowercase(),
                })
                .collect(),
        },
        SessionUpdate::AvailableCommandsUpdate(c) => MiraEvent::Commands {
            names: c.available_commands.iter().map(|x| x.name.clone()).collect(),
        },
        SessionUpdate::CurrentModeUpdate(m) => MiraEvent::Modes {
            current: m.current_mode_id.to_string(),
            // The available set comes from `initialize`, not from here.
            available: Vec::new(),
        },
        SessionUpdate::ConfigOptionUpdate(c) => MiraEvent::ConfigOptions {
            options: c.config_options.iter().map(SessionConfigView::from).collect(),
        },
        SessionUpdate::SessionInfoUpdate(s) => MiraEvent::SessionInfo {
            title: defined(&s.title),
            updated_at: defined(&s.updated_at),
        },
        SessionUpdate::UsageUpdate(u) => MiraEvent::Usage {
            used: u.used,
            size: u.size,
            cost: u.cost.as_ref().map(|c| {
                (c.amount, c.currency.clone())
            }),
        },
        // `SessionUpdate` is `#[non_exhaustive]`, so this arm is required
        // today and is also what keeps a future ACP release from breaking
        // the build.
        other => {
            let tag = update_tag(other);
            if RECOGNISED_UNMODELLED.contains(&tag.as_str()) {
                // A variant we know about and have chosen not to model
                // *yet* — see `RECOGNISED_UNMODELLED`. Surfacing these as
                // unmodelled would put a warning in the transcript for
                // something we are deliberately not supporting, which trains
                // the user to ignore warnings. Traced, not warned.
                tracing::debug!(
                    variant = %tag,
                    "acp: recognised but unmodelled session update"
                );
                MiraEvent::Unmodelled {
                    source: source.clone(),
                    reason: format!("recognised, not yet modelled: {tag}"),
                }
            } else {
                // Genuinely unknown: a future ACP release, or a vendor
                // extension. Provenance is retained and the event is
                // surfaced, because a silent drop here is indistinguishable
                // from a wedged agent.
                MiraEvent::Unmodelled {
                    source: source.clone(),
                    reason: format!("unhandled sessionUpdate: {tag}"),
                }
            }
        }
    };

    Some(NormalizedEvent { source, event })
}

fn text_of(block: &ContentBlock) -> Option<String> {
    content_to_text(block)
}

/// Unwrap ACP's `MaybeUndefined`, where `Null` is a *value* meaning
/// "cleared", distinct from the field being absent.
fn defined<T: Clone>(v: &MaybeUndefined<T>) -> Option<T> {
    match v {
        MaybeUndefined::Value(t) => Some(t.clone()),
        _ => None,
    }
}

/// Normalize an incoming `session/request_permission` params object.
///
/// Parsed via the SDK type rather than our own struct so camelCase
/// wire names and ACP's flattened `toolCall` are handled by the
/// generated code instead of by hand.
pub fn from_permission_request(src: &RequestPermissionRequest) -> PermissionRequest {
    PermissionRequest {
        session_id: src.session_id.to_string(),
        tool_call_id: src.tool_call.tool_call_id.to_string(),
        title: src.tool_call.fields.title.clone().unwrap_or_default(),
        kind: src.tool_call.fields.kind.clone(),
        options: permission_options(&src.options),
    }
}

/// A session mode, projected to a shape we own.
///
/// Deliberately not the SDK's `SessionMode`. Wire formats outlive the
/// library that produced them, so the type the frontend sees is defined here
/// and can stay stable across an SDK upgrade.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionModeView {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl From<&SessionMode> for SessionModeView {
    fn from(m: &SessionMode) -> Self {
        SessionModeView {
            id: m.id.to_string(),
            name: m.name.clone(),
            description: m.description.clone(),
        }
    }
}

/// A selectable value within a config option.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigValueView {
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A config option, projected to a shape we own.
///
/// `category` is what makes ACP's model selector work: the model list is
/// just the option whose category is `"model"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionConfigView {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The value currently selected, for select-style options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// The selectable values, for select-style options. Empty for a toggle.
    #[serde(default)]
    pub values: Vec<ConfigValueView>,
}

impl From<&SessionConfigOption> for SessionConfigView {
    fn from(o: &SessionConfigOption) -> Self {
        let (current, values) = match &o.kind {
            SessionConfigKind::Select(s) => (
                Some(s.current_value.to_string()),
                match &s.options {
                    SessionConfigSelectOptions::Ungrouped(v) => v.iter().map(value_view).collect(),
                    SessionConfigSelectOptions::Grouped(groups) => groups
                        .iter()
                        .flat_map(|g| g.options.iter().map(value_view))
                        .collect(),
                    // `#[non_exhaustive]`: a future shape yields no values
                    // rather than failing to compile.
                    _ => Vec::new(),
                },
            ),
            SessionConfigKind::Boolean(b) => (Some(b.current_value.to_string()), Vec::new()),
            // `SessionConfigKind` is `#[non_exhaustive]`; a future variant
            // shows as an option with no values rather than vanishing.
            _ => (None, Vec::new()),
        };
        SessionConfigView {
            id: o.id.to_string(),
            name: o.name.clone(),
            description: o.description.clone(),
            category: o.category.as_ref().map(|c| format!("{c:?}").to_lowercase()),
            current,
            values,
        }
    }
}

fn value_view(v: &SessionConfigSelectOption) -> ConfigValueView {
    ConfigValueView {
        value: v.value.to_string(),
        name: v.name.clone(),
        description: v.description.clone(),
    }
}

#[cfg(test)]
mod unmodelled_tests {
    use super::*;
    use serde_json::Value;

    fn update_for(tag: &str) -> Value {
        serde_json::json!({
            "sessionId": "s",
            "update": { "sessionUpdate": tag }
        })
    }

    #[test]
    fn a_recognised_gap_is_labelled_as_such() {
        // These are variants we know about and have chosen not to model. The
        // distinction matters: the reason has to say which, or a reader
        // cannot tell a deliberate gap from an unknown one.
        for tag in ["plan_update", "plan_removed", "notice"] {
            let ev = from_session_notification(&update_for(tag))
                .unwrap_or_else(|| panic!("{tag} did not parse"));
            match ev.event {
                MiraEvent::Unmodelled { reason, .. } => assert!(
                    reason.contains("recognised, not yet modelled"),
                    "{tag} produced {reason:?}"
                ),
                other => panic!("{tag} should be unmodelled, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_genuinely_unknown_variant_says_so() {
        // A future ACP release or a vendor extension. Surfaced loudly, and
        // explicitly not conflated with the deliberate gap above.
        let ev = from_session_notification(&update_for("some_future_thing"))
            .expect("should still parse");
        match ev.event {
            MiraEvent::Unmodelled { reason, .. } => {
                assert!(reason.contains("unhandled"), "got {reason:?}")
            }
            other => panic!("expected unmodelled, got {other:?}"),
        }
    }

    #[test]
    fn every_recognised_entry_is_an_unstable_variant_we_deliberately_skip() {
        // If a name lands in this list that is not one of the three gated
        // variants, it means a real event is being quietly swallowed.
        for name in RECOGNISED_UNMODELLED {
            assert!(
                ["plan_update", "plan_removed", "notice"].contains(name),
                "{name} is not a known unstable variant"
            );
        }
    }

    #[test]
    fn provenance_survives_both_paths() {
        // Whatever the path, the wire variant name is retained, so an agent
        // upgrade is diagnosable from a log rather than a guess.
        //
        // A modelled variant carries `Acp { variant }`; an unparsed one
        // carries `Unmodelled { method }`, because the typed value does not
        // exist to attribute it to a known variant. Both name the tag.
        for tag in ["plan_update", "some_future_thing"] {
            let ev = from_session_notification(&update_for(tag)).expect("parses");
            let named = match &ev.source {
                EventSource::Acp { variant } => variant.clone(),
                EventSource::Unmodelled { method } => method.clone(),
            };
            assert_eq!(named, tag, "provenance lost the tag for {tag}");
        }
    }

    #[test]
    fn a_modelled_variant_still_reports_acp_provenance() {
        // The new tag-first path must not have cost the typed path its
        // provenance.
        let ev = from_session_notification(&serde_json::json!({
            "sessionId": "s",
            "update": { "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "hi" } }
        }))
        .expect("parses");
        assert!(matches!(ev.source, EventSource::Acp { .. }), "{:?}", ev.source);
        assert!(matches!(ev.event, MiraEvent::AssistantText { .. }));
    }
}
