//! WebSocket wire protocol.
//!
//! Every frame is a single JSON object with a `"type"` discriminator. Server
//! frames carry harness events + approval requests; client frames carry user
//! input, approvals, and control (mode/model swap, interrupt, clear).

use mira_ai::TokenUsage;
use mira_core::{Message, ToolCall, ToolResult};
use mira_harness::{Goal, GoalStatus, HarnessEvent, TurnMeta, UsageTotals};
use mira_policy::Mode;
use mira_review::{Finding, Progress as ReviewProgress};
use mira_tools::DiffPreview;
use serde::{Deserialize, Serialize};

use crate::interactive::{AskUserProposal, PlanProposal, PromptResponse};
use crate::slot::BackgroundMode;

/// Scope on an `Approve` reply — how long the user's decision applies
/// for. `Once` (the default and legacy shape) affects only the current
/// call. `Session` promotes the target to `Allow` on the session's
/// in-memory policy so identical follow-up calls skip the modal.
/// `Always` also appends the rule to `~/.mira/mira.yaml` so it persists
/// across sessions.
///
/// Only meaningful when the reply's `allow` is `true` — a scoped deny
/// isn't a concept the current UI exposes.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    #[default]
    Once,
    Session,
    Always,
}

/// Client → server.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    SyncActivity,
    QueueInput {
        session_id: String,
        id: String,
        text: String,
        #[serde(default)]
        images: Vec<mira_core::ImageData>,
    },
    UpdateLimitRecovery {
        session_id: String,
        request_id: String,
        id: String,
        action: String,
    },
    RemoveQueuedInput {
        session_id: String,
        id: String,
    },
    EditQueuedInput {
        session_id: String,
        request_id: String,
        id: String,
        fingerprint: String,
        text: String,
        #[serde(default)]
        images: Vec<mira_core::ImageData>,
    },
    ReorderQueuedInput {
        session_id: String,
        request_id: String,
        id: String,
        before_id: Option<String>,
    },
    Steer {
        request_id: String,
        text: String,
        #[serde(default)]
        images: Vec<mira_core::ImageData>,
    },
    History {
        session_id: String,
        cursor: String,
        request_id: String,
    },
    /// Start a new user turn.
    Send {
        text: String,
        /// Pasted / dropped images for the model to see.
        #[serde(default)]
        images: Vec<mira_core::ImageData>,
    },
    /// Edit & resend / retry: rewind to just before the `occurrence`-th
    /// most recent user message whose text is `original` (0 = latest),
    /// then start a new turn with `text`.
    Resend {
        original: String,
        #[serde(default)]
        occurrence: usize,
        text: String,
    },
    /// Answer a pending approval prompt. `scope` says whether the
    /// decision only covers this specific call, or should also add a
    /// rule to the session policy (and optionally persist it) so
    /// identical future calls skip the modal.
    Approve {
        call_id: String,
        allow: bool,
        #[serde(default)]
        scope: ApprovalScope,
    },
    /// Answer an interactive tool prompt (plan review, question, etc.).
    /// `prompt_id` matches the `prompt_id` on the server frame that opened
    /// the modal.
    PromptResponse {
        prompt_id: String,
        #[serde(flatten)]
        response: PromptResponse,
    },
    /// Hot-swap the model for the next turn, optionally on a
    /// different engine instance. `instance` is a key from
    /// `GET /api/engines` (`anthropic`, `codex`, `claude-code`, …):
    /// switching it routes future turns through that backend, which
    /// for a native instance also swaps the underlying provider.
    /// Both fields optional: `{model}` keeps the instance, `{instance}`
    /// alone adopts that instance's default model, and `{instance,
    /// model}` sets both. Omitting both is a no-op.
    SetModel {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        instance: Option<String>,
        /// Model options (`reasoning_effort`, `service_tier`) applied with
        /// the switch, so they arrive with the model rather than after it.
        #[serde(default)]
        options: std::collections::BTreeMap<String, String>,
    },
    /// Set one of the model options the UI advertises for the current
    /// model (see `mira_ai::provider::OptionDescriptor`).
    ///
    /// `id` is a descriptor id — `reasoning_effort`, `service_tier`. The
    /// client only ever sends ids the server advertised for that model, but
    /// the server still matches on a known set rather than trusting the
    /// wire: an unknown id is ignored rather than written to some field by
    /// name. This is the message the composer's model controls speak; the
    /// previous `set_effort` the UI sent had no handler at all and was
    /// silently dropped, which is why the effort selector did nothing.
    SetModelOption {
        id: String,
        value: String,
    },
    /// Bring an external ACP agent up for this session, replacing any agent
    /// already running.
    AcpStart {
        /// Start a configured engine instance (a key from
        /// `GET /api/engines`) instead of naming a driver by hand —
        /// its display name, binary, env and launch args come from
        /// `mira.yaml`'s `engines:` block. When set, the per-field
        /// overrides below layer on top of the instance config.
        #[serde(default)]
        instance: Option<String>,
        /// Driver slug, e.g. `"claude-code"`, `"codex"`, `"grok"`.
        /// Optional: omitted (or matching what this session already
        /// recorded) resolves to the session's retained launch
        /// settings — how a fresh chat starts the same agent the last
        /// one ran, with the same config, by sending no fields at all.
        #[serde(default)]
        driver: Option<String>,
        /// Overrides the driver's default binary path.
        #[serde(default)]
        binary_path: Option<String>,
        /// Overrides the driver's display name for this instance.
        #[serde(default)]
        display_name: Option<String>,
        /// Extra CLI arguments, appended verbatim.
        #[serde(default)]
        launch_args: Vec<String>,
        /// Per-instance environment.
        #[serde(default)]
        env: std::collections::BTreeMap<String, String>,
        /// API key, for agents that take one.
        #[serde(default)]
        api_key: Option<String>,
        /// Per-instance home / config directory.
        #[serde(default)]
        home_path: Option<String>,
        /// Resume this agent session id instead of starting blank (import).
        #[serde(default)]
        resume: Option<String>,
        /// Effort level, for agents that take one (`claude --effort`).
        #[serde(default)]
        effort: Option<String>,
        /// Setting sources, for agents that take them.
        #[serde(default)]
        setting_sources: Option<String>,
        /// The agent model to run, picked alongside the agent in the model
        /// picker. Applied at launch (native transports) or as the model
        /// config option once the agent is up (ACP), and remembered for
        /// every later restart.
        #[serde(default)]
        model: Option<String>,
    },
    /// Switch the running agent's session mode.
    ///
    /// Not the same as `set_mode`: that changes Mira's own harness mode,
    /// while this asks the external agent (Codex's `read-only` / `agent` /
    /// `agent-full-access`, Claude Code's `code` / `ask` / `architect`).
    AcpSetMode {
        mode_id: String,
        /// Set once the user has been shown what a privileged mode does and
        /// agreed. A privileged mode is refused without it, so a client
        /// cannot widen an agent's authority by setting the flag itself.
        #[serde(default)]
        acknowledge_privileged: bool,
    },
    /// Set a config option on the running agent.
    ///
    /// ACP has no set-model method — the model selector is a config option
    /// with `category: "model"` — so this is how a model is chosen.
    AcpSetConfigOption {
        option_id: String,
        value: String,
    },
    /// Send a turn to the running agent.
    AcpPrompt {
        text: String,
        /// Images staged into the agent's files dir and named in the text.
        #[serde(default)]
        images: Vec<mira_core::ImageData>,
    },
    /// Stop the running agent and release its terminals.
    AcpStop,
    /// Fork the agent session: continue its history under a new session id.
    /// Claude Code only (`--fork-session`); anything else gets an honest
    /// error, not a silent restart.
    AcpFork,
    /// Ask the agent to compact its context. Native transports only; ACP
    /// has no such method. `focus` is what the summary should keep in view
    /// (Claude Code's `/compact <instructions>`; Codex has no equivalent).
    AcpCompact {
        #[serde(default)]
        focus: Option<String>,
    },
    /// Report what each known agent's health is.
    AcpStatus,
    /// Remote environments for the attached session. `target: None`
    /// asks for the current status (answered with `environment_status`);
    /// a name (or `local`) switches, reporting `environment_progress`
    /// lines and then `environment_switched`.
    Environment {
        #[serde(default)]
        target: Option<String>,
    },
    /// Change the permission mode.
    SetMode {
        mode: Mode,
    },
    /// Set reasoning effort for the current session. `None` (or the string
    /// `"off"`) clears the field entirely so non-reasoning models aren't
    /// hit with an unexpected parameter.
    SetEffort {
        #[serde(default)]
        effort: Option<String>,
    },
    /// Best-effort cancel the current turn.
    Interrupt,
    /// Set (or replace) the session's standing `/goal`. `max_iterations`
    /// is optional — the server uses [`mira_harness::DEFAULT_MAX_ITERATIONS`]
    /// when absent so the client doesn't have to hardcode the cap. When
    /// `evaluator_model` is `None` the harness reuses the session's
    /// active model (usually you want a cheaper tier here).
    SetGoal {
        condition: String,
        #[serde(default)]
        max_iterations: Option<usize>,
        #[serde(default)]
        evaluator_model: Option<String>,
        /// Optional external verifier — a shell command whose exit
        /// code (and optional stdout regex) gates the "Met" verdict.
        /// See [`mira_harness::VerifyCommand`] for the contract.
        #[serde(default)]
        verify: Option<mira_harness::VerifyCommand>,
        /// Optional hard token budget (input + output, summed across
        /// all rounds). Breach → `Exhausted`.
        #[serde(default)]
        budget_tokens: Option<u64>,
        /// Optional hard USD budget. Requires the active model to be
        /// in `mira-ai`'s pricing table; unpriced models skip the USD
        /// check silently.
        #[serde(default)]
        budget_usd: Option<f64>,
    },
    /// Drop the session's standing goal. Idempotent — clearing a
    /// session without a goal is a no-op.
    ClearGoal,
    /// Summarize the conversation now (`/compact`), keeping `focus` in
    /// view. Answered with `Compacted`, or `Error` when it can't.
    Compact {
        #[serde(default)]
        focus: Option<String>,
    },
    /// Ask the server to re-emit its current state (used on reconnect).
    Sync,
    /// Watch a specific session on this WS connection. Multi-session
    /// clients switch between sessions by sending Attach rather than
    /// hitting `POST /api/sessions/:id/load` — attaching doesn't kill
    /// any in-flight turn on the previous session, it just re-points
    /// this socket's event forwarder.
    ///
    /// The server responds by:
    /// 1. Decrementing the previously-attached slot's `attached` count.
    /// 2. Swapping the forwarder's subscription to the target slot's
    ///    events channel.
    /// 3. Incrementing the new slot's `attached` count.
    /// 4. Emitting a fresh `Ready` frame for the new session.
    /// 5. Updating the server's `active` pointer so subsequent
    ///    HTTP calls target this session.
    Attach {
        session_id: String,
    },
    /// Detach from any session on this WS connection — used when a client
    /// wants to explicitly stop watching without closing the socket. Rare
    /// on the current UI (tab close does the same job); provided for
    /// completeness so a future "let this session finish in the background"
    /// affordance has a clean signal to send.
    Detach,
    /// Update the *attached* session's background mode. Applies to how
    /// the approver answers `Ask` decisions when no client is attached.
    SetBackgroundMode {
        mode: BackgroundMode,
    },
}

/// Server → client.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// `Ready` is much bigger than the other variants, but it's sent once per
// connection; boxing it would only add churn at every construction site.
#[allow(clippy::large_enum_variant)]
pub enum ServerMsg {
    SessionActivity {
        epoch: String,
        revision: u64,
        session_id: String,
        running: bool,
    },
    SessionActivitySnapshot {
        snapshot: crate::session_activity::ActivitySnapshot,
    },
    QueueUpdated {
        session_id: String,
        items: Vec<crate::message_queue::QueuedInput>,
    },
    QueueDelivery {
        session_id: String,
        item: crate::message_queue::QueuedInput,
    },
    QueueMutationResult {
        session_id: String,
        request_id: String,
        error: Option<String>,
    },
    SteerResult {
        session_id: String,
        request_id: String,
        message: Option<Message>,
        error: Option<String>,
    },
    /// Emitted once when the socket opens, with the session's current state.
    HistoryPage {
        session_id: String,
        request_id: String,
        page: Option<crate::transcript_history::TranscriptPage>,
        error: Option<String>,
    },
    TurnDiffs {
        summaries: Vec<crate::checkpoints::TurnDiffSummary>,
    },
    Ready {
        queued_inputs: Vec<crate::message_queue::QueuedInput>,
        session_activity: crate::session_activity::ActivitySnapshot,
        turn_diffs: Vec<crate::checkpoints::TurnDiffSummary>,
        transcript_page: crate::transcript_history::TranscriptPage,
        session_id: String,
        model: String,
        mode: Mode,
        cwd: String,
        history: Vec<Message>,
        /// Per-turn timing, aligned with user messages in `history`. Empty
        /// for legacy sessions written before turn tracking.
        #[serde(default)]
        turns: Vec<TurnMeta>,
        /// Aggregate token usage carried over from prior turns on this
        /// session. Zeroed for legacy sessions or providers that don't
        /// report usage.
        #[serde(default, skip_serializing_if = "UsageTotals::is_zero")]
        usage: UsageTotals,
        /// Session-scoped task list. Empty for sessions where the
        /// model hasn't called `task_create`. Sent so the UI can
        /// rehydrate the task panel on reload without asking the
        /// harness to replay tool history.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tasks: Vec<mira_tools::TaskItem>,
        /// Standing `/goal` — `None` when the user hasn't set one on
        /// this session (or cleared it). Terminal statuses are also
        /// sent so the UI can render a "last goal" chip until cleared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<Goal>,
        /// Persisted diff previews for edit/write tool calls, keyed by
        /// tool call id. The frontend attaches them to the matching
        /// tool entry during `historyToEntries` so a reloaded transcript
        /// renders the real diff instead of falling back to an
        /// arg-only reconstruction. Empty for legacy sessions written
        /// before this landed.
        #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
        previews: std::collections::HashMap<String, DiffPreview>,
        /// The agent transcript sidecar, oldest first. The client replays
        /// these through its live frame handling, so reloaded agent turns
        /// render exactly like live ones. Empty for harness-only sessions
        /// and for legacy sessions written before this landed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agent_transcript: Vec<serde_json::Value>,
        /// Which agent that transcript belongs to, for badges and resume.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_driver: Option<String>,
        /// Driver driving right now, if any. Attach restores the banner for
        /// live agents and clears it otherwise — client agent state is
        /// per-session, and a stale global "driving" flag routes new chats
        /// to dead agents.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_kind: Option<String>,
        /// The engine instance serving this session (a key from
        /// `GET /api/engines`), when the session has one. Lets a
        /// reloading client restore the picker without guessing which
        /// provider was active.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        /// Driver kind this session is *configured* to run — inherited
        /// from the previous chat or retained after a stop-with-config —
        /// while no agent is live. The UI shows the agent as the
        /// session's setup; the first prompt (or one click) starts it
        /// with exactly those settings.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_configured: Option<String>,
        /// The session's title (AI-written or the agent's), when it has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// What drives this session — provider or agent — and its state.
        /// The single source of truth the composer renders from; the
        /// `agent_*` fields above remain for older clients.
        engine: crate::session_engine::SessionEngine,
        /// A turn is in flight right now (opened mid-turn): the composer
        /// shows Stop and the working indicator until it ends.
        #[serde(default)]
        running: bool,
        runtime_requests: Vec<crate::runtime_requests::RequestRecord>,
        runtime_work: Vec<mira_acp::runtime::RuntimeWork>,
    },
    /// The session's engine changed: a provider or agent was picked, an
    /// agent started, failed, or exited. Sent on every transition so the
    /// composer never has to wait for a turn to learn what it is talking to.
    SessionEngine {
        engine: crate::session_engine::SessionEngine,
    },
    /// Fragment of assistant text.
    Token {
        text: String,
    },
    /// Fragment of the model's reasoning ("thinking"), streamed before
    /// the text / tool calls it leads to. Clients render it in a
    /// collapsible "Thinking" section; the finished blocks also ride on
    /// the assistant message's `reasoning` field in session snapshots.
    Reasoning {
        text: String,
    },
    /// Tool call dispatched (already policy-approved).
    ToolStart {
        call: ToolCall,
    },
    /// Tool call finished with a result.
    ToolEnd {
        result: ToolResult,
    },
    /// Model turn complete — may be followed by another turn if tools ran.
    TurnComplete,
    /// User turn fully complete — waiting for next user input.
    Done,
    /// Approval needed for a tool call the policy flagged `Ask`. Client should
    /// answer with `ClientMsg::Approve { call_id, allow }`.
    ApprovalRequest {
        call: ToolCall,
        /// Structured diff for edit_file / write_file. `None` for tools that
        /// don't have a natural preview (e.g. `bash`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<DiffPreview>,
        /// For a compound shell command, the operations that need approval
        /// — the rest would run unasked. Empty for a single command.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        needs: Vec<String>,
    },
    /// Non-fatal warning surfaced to the UI.
    Warning {
        text: String,
    },
    /// Where the session's tools run, and what it could switch to.
    EnvironmentStatus {
        status: mira_compute::EnvironmentStatus,
        environments: Vec<mira_compute::env::EnvironmentInfo>,
    },
    /// One line of progress from an environment switch (upload, setup
    /// script output, merge).
    EnvironmentProgress {
        text: String,
    },
    /// An environment switch finished. On failure `error` is set and the
    /// session stayed where it was.
    EnvironmentSwitched {
        from: String,
        to: String,
        /// Human-readable summary (merged files, patch location, …).
        lines: Vec<String>,
        conflicts: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        status: mira_compute::EnvironmentStatus,
    },
    /// One line of live stdout+stderr from a still-running tool call.
    /// The renderer routes lines by `call_id` under the matching
    /// pending tool card so the user sees progress before the final
    /// `ToolEnd` frame lands.
    ToolProgress {
        call_id: String,
        line: String,
    },
    /// Diff preview computed for an `edit_file` / `write_file` call.
    /// Fires whether or not the call was approval-gated so live UIs
    /// always have the real diff, not just the reconstruction. Also
    /// persisted; see `Ready.previews` for the reload path.
    ToolPreview {
        call_id: String,
        preview: DiffPreview,
    },
    /// The skill registry was reloaded (a file change was detected in
    /// one of the four skill directories, or a manual reload was
    /// triggered). The frontend refetches `/api/skills` when it sees
    /// this so the composer palette and Settings panel pick up new /
    /// edited skills automatically.
    SkillsReloaded,
    /// MCP servers or plugins changed (a server connected, a plugin was
    /// installed, …). The frontend refetches `/api/mcp`, `/api/plugins`
    /// and `/api/commands`.
    ExtensionsChanged,
    /// Model changed (echoes SetModel). `instance` names the engine
    /// instance now serving the session, when the selection carried
    /// one — absent for legacy clients that only picked a model.
    ModelChanged {
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
    },
    /// Mode changed (echoes SetMode).
    ModeChanged {
        mode: Mode,
    },
    /// A protocol-level error (bad input, unknown call_id, etc.).
    Error {
        text: String,
    },

    /// A `mira review` run started. Every subsequent `ReviewProgress` / `ReviewResult`
    /// / `ReviewError` frame with this `run_id` belongs to it — the frontend
    /// filters on it so overlapping runs don't scramble each other's UI.
    ReviewStarted {
        run_id: String,
    },
    /// Streaming progress event from an in-flight review.
    ReviewProgress {
        run_id: String,
        event: ReviewProgress,
    },
    /// Review finished — final confirmed findings list.
    ReviewResult {
        run_id: String,
        findings: Vec<Finding>,
    },
    /// Review aborted with an error (bad diff, provider failure, etc.).
    ReviewError {
        run_id: String,
        text: String,
    },

    /// A session's AI-generated nickname landed on disk. Frontend uses this
    /// to refresh the sidebar so the newly-titled row replaces the
    /// first-user-message fallback without waiting for the next `done`.
    SessionTitleUpdated {
        session_id: String,
        title: String,
    },

    /// A session's background-mode was changed. Confirms `SetBackgroundMode`.
    BackgroundModeChanged {
        session_id: String,
        mode: BackgroundMode,
    },

    /// A background-only session transitioned running → idle. Emitted when
    /// a turn finishes on a slot with zero attached clients, so the sidebar
    /// can drop the "running" indicator even though nobody's tab is
    /// receiving the per-turn `Done` frame.
    SessionBackgroundIdle {
        session_id: String,
    },

    /// Same idea for the running-transition — emitted on the SLOT's own
    /// channel when a turn kicks off, so if a client attaches mid-turn
    /// it can pick up the "still running" indicator without polling.
    SessionBackgroundRunning {
        session_id: String,
    },

    /// Per-round + running-total token usage from the provider. Emitted at
    /// the end of every provider round the moment a usage trailer arrives;
    /// the UI uses `totals` for its status-bar counter and `round` for
    /// per-turn indicators.
    Usage {
        round: TokenUsage,
        totals: UsageTotals,
        /// The model's context window, for the composer's context ring.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_window: Option<u64>,
        /// Where auto-compaction kicks in, in tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compact_at: Option<u64>,
    },

    /// The provider's rate limits after the latest request, from its
    /// response headers. `summary` is the tightest one in words
    /// (`12% of tokens left · resets in 42s`) for the status bar.
    RateLimit {
        rate_limit: mira_ai::RateLimit,
        summary: Option<String>,
    },

    /// Post-round auto-extractor recorded N durable facts to
    /// `.mira/episodic.jsonl`. Emitted only when `count > 0`; the UI can
    /// render a small "mira remembered N things" chip to make cross-session
    /// memory writes visible.
    MemoryLearned {
        count: usize,
    },

    /// The harness rolled up N older non-system messages into a single
    /// summary before the current round's model call. UI can render a
    /// "compacted N turns" chip for transparency.
    Compacted {
        messages_removed: usize,
        /// Estimated history size before and after, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_before: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_after: Option<usize>,
    },
    /// Compaction started: `manual` (/compact) or `auto` (the history
    /// outgrew the context window mid-turn).
    Compacting {
        trigger: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_before: Option<usize>,
    },
    /// Compaction failed; the conversation continues uncompacted.
    CompactionFailed {
        error: String,
    },

    /// A `/goal` was set on this session. Emitted immediately after
    /// `SetGoal` so the UI can flip its state without waiting for the
    /// next turn.
    GoalSet {
        goal: Goal,
    },
    /// The session's standing goal was cleared (by the user or the
    /// UI). Emitted whether or not a turn is running.
    GoalCleared,
    /// The evaluator just ran mid-goal. `iteration` is the count
    /// *after* the bump (1-indexed) and `status` reflects the goal
    /// state post-evaluation. `reason` is the evaluator's free-text
    /// note — shown in the goal panel so the user can see why the
    /// loop is still going.
    GoalProgress {
        iteration: usize,
        max_iterations: usize,
        status: GoalStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// The goal reached a terminal state and no further autonomous
    /// iterations will run. The UI switches from the "working…" pill
    /// to the terminal one keyed on `status`.
    GoalDone {
        status: GoalStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// The model called the `plan` tool. Open a review modal so the user can
    /// approve / edit / cancel. Client answers with
    /// `ClientMsg::PromptResponse` carrying a `Plan { … }` variant.
    PlanRequest {
        prompt_id: String,
        plan: PlanProposal,
    },
    /// The model called the `ask_user` tool with a batch of clarifying
    /// questions. Client renders the question card and answers with
    /// `ClientMsg::PromptResponse` carrying an `AskUser { … }` variant.
    RuntimeRequestUpdated {
        request: crate::runtime_requests::RequestRecord,
    },
    RuntimeWorkUpdated {
        work: mira_acp::runtime::RuntimeWork,
    },
    RuntimeTurnUpdated {
        turn: mira_acp::runtime::RuntimeTurn,
    },
    AskUserRequest {
        prompt_id: String,
        proposal: AskUserProposal,
    },
    /// A plan / question card was answered (by any window). Clients that
    /// still show it open record the answer and close it.
    PromptResolved {
        prompt_id: String,
        #[serde(flatten)]
        response: PromptResponse,
    },

    // -------- subagent (child session) event forwarding --------
    //
    // The `agent` tool now streams each child HarnessEvent to the parent's
    // WS with a `parent_call_id` tag so the frontend can build a live
    // per-child transcript in the right-side SubagentPanel. Kept as
    // distinct variants (rather than a wrapped envelope) so the JS side
    // pattern-matches by `type` the same way it already does.
    /// A subagent has been spawned. Sent immediately before the child
    /// begins consuming its prompt so the panel can open a tab.
    SubagentStarted {
        /// The parent's tool-call id — the AgentTool call that spawned
        /// this child. Every subsequent `Subagent*` frame with the same
        /// value belongs to this child.
        parent_call_id: String,
        /// Child session id — useful for future features (persistence,
        /// deep links) but not required by the current UI.
        agent_id: String,
        /// Model the child is running under. Lets the panel surface a
        /// small caption without extra bookkeeping.
        model: String,
        /// Full prompt the child received. Small ceiling upstream via
        /// `truncate_for_history`, so this can safely include the raw text.
        prompt: String,
        /// Human-readable name from the agent type's frontmatter
        /// (e.g. `"leo"`, `"reviewer"`). `None` when spawned without a
        /// named type.
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_name: Option<String>,
        /// Category from the agent type's frontmatter (e.g. `"recon"`,
        /// `"review"`). `None` when not set or no named type.
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_category: Option<String>,
    },
    /// Fragment of the child's assistant text.
    SubagentToken {
        parent_call_id: String,
        text: String,
    },
    /// The child dispatched a tool call.
    SubagentToolStart {
        parent_call_id: String,
        call: ToolCall,
    },
    /// The child's tool call finished.
    SubagentToolEnd {
        parent_call_id: String,
        result: ToolResult,
    },
    /// A child-level warning (verify failure, hit max_rounds, etc.).
    SubagentWarning {
        parent_call_id: String,
        text: String,
    },
    /// Intermediate progress update from the child, emitted when the
    /// subagent explicitly calls the `progress` tool. Distinct from
    /// tokens (which are freeform assistant text) and from warnings
    /// (which imply something's off) — this is the child announcing
    /// "here's where I am" during a long investigation.
    SubagentProgress {
        parent_call_id: String,
        text: String,
    },
    /// The child produced a final summary and its type has
    /// `review_required: true`. The parent's turn is paused; the UI
    /// must show the proposed summary and the user picks approve or
    /// deny (with optional note) before the tool result flows back to
    /// the parent.
    SubagentReviewRequest {
        parent_call_id: String,
        /// Prompt id the client echoes back in `PromptResponse`.
        /// Convention: `<parent_call_id>-review`.
        prompt_id: String,
        /// Full proposed summary (post-warnings, pre-worktree note).
        summary: String,
    },
    /// The child finished — final assistant text is already on the way as
    /// the AgentTool's `ToolEnd` frame. This exists purely so the panel
    /// can flip its status pill from "working" to "done" without waiting
    /// for the parent to update the same call id.
    SubagentDone {
        parent_call_id: String,
    },
    /// A subagent posted an entry to the parent session's shared
    /// scratchpad via the `scratchpad_note` tool. Peers spawned from the
    /// same session share one pad so parallel researchers can see each
    /// other's mid-flight findings without waiting for a final summary
    /// round-trip.
    SubagentScratchpadNote {
        /// The `agent`-tool call id that spawned the author. Lets the
        /// panel attribute the note to the right child tab in addition
        /// to the shared pad view.
        parent_call_id: String,
        /// The parent session's id — the pad key. Multiple `parent_call_id`
        /// values can share this when a single user turn spawned several
        /// children in parallel.
        parent_session_id: String,
        /// The posted note. `author` is the subagent type (e.g. `explore`).
        entry: crate::interactive::ScratchpadEntry,
    },

    /// Live activity from inside a `delegate_task` child, re-broadcast on the
    /// parent's channel so the delegation card can show what the helper is
    /// actually doing instead of just a spinner. `call_id` is the parent's
    /// `delegate_task` call id. `kind` is a coarse verb (`read` / `search` /
    /// `run` / …) the frontend maps to an icon and tense; `text` is the
    /// human-readable step ("Read src/lib.rs").
    DelegateProgress {
        call_id: String,
        kind: String,
        text: String,
    },

    /// An agent used Mira's browser through the tool server. Lets the
    /// client show the browser pane however the agent names its tools.
    BrowserActive,
    /// `mira.yaml` changed and the engine registry was rebuilt: refetch
    /// `GET /api/engines`.
    EnginesChanged,
    /// An agent asked for a secret (an API key, a token). The client shows a
    /// private input and answers with a `secret` prompt response; the value
    /// goes to a file only the agent's commands read, never the transcript.
    SecretRequest {
        prompt_id: String,
        name: String,
        reason: String,
        /// A project file the value will also be written to, when asked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dotenv: Option<String>,
    },
    /// A self-contained HTML page an agent published to the chat (a chart,
    /// table or mockup). Rendered sandboxed, above the agent's reply.
    HtmlRender {
        id: String,
        title: String,
        html: String,
    },

    // -------- ACP (external agent) event forwarding --------
    //
    // Same shape as the subagent family above: distinct variants rather than
    // a wrapped envelope, so the JS side pattern-matches by `type` exactly
    // the way it already does. These carry `mira-acp`'s *normalized* events
    // — the ACP dialect is confined to that crate, so nothing above this
    // point has to know what an ACP session update looks like.
    /// A fragment of the external agent's reply. Chunks arrive as they are
    /// produced, so the UI appends rather than replaces.
    AcpText {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    /// Authoritative completed prose, replacing the streamed partial message.
    AcpMessageMetadata {
        message_id: String,
        phase: String,
    },
    StreamActivity {
        kind: String,
        title: String,
        detail: String,
    },
    AcpTextSnapshot {
        message_id: String,
        text: String,
    },
    /// Incremental tool stdout/stderr.
    AcpToolOutputDelta {
        id: String,
        text: String,
    },
    AcpThought {
        text: String,
    },
    /// The agent started a tool call. The normalized state is forwarded
    /// whole, because it carries diff and terminal content that a bare
    /// `ToolCall` has no place for.
    AcpToolCall {
        call: mira_acp::events::ToolCallState,
    },
    /// Progress or completion for a call already announced by `AcpToolCall`.
    AcpToolCallUpdate {
        call: mira_acp::events::ToolCallState,
    },
    /// The agent's plan for the current task.
    AcpPlan {
        entries: Vec<mira_acp::events::PlanEntry>,
    },
    /// Session modes, and which one is active. `postures` is the
    /// server's mapping of the canonical posture vocabulary onto the
    /// agent's modes — computed here so clients render options without
    /// pattern-matching agent mode names themselves.
    AcpModes {
        current: String,
        available: Vec<mira_acp::events::SessionModeView>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        postures: Vec<mira_engine::PostureMapping>,
        /// Which agent reported this, so the client attributes cached
        /// capabilities to the right driver. Late frames from a previous
        /// agent must not pollute another's "last seen" list.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        driver: Option<String>,
    },
    /// Config options. ACP has no set-model method — the model selector *is*
    /// a `configOptions` entry with `category: "model"`.
    AcpConfigOptions {
        options: Vec<mira_acp::events::SessionConfigView>,
        /// Which agent reported this. See `AcpModes.driver`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        driver: Option<String>,
    },
    /// Slash commands the agent advertises.
    AcpCommands {
        names: Vec<String>,
    },
    /// What an external agent's turn spent since its last report: tokens
    /// and, when the agent estimates it, cost. Deltas — a turn can report
    /// several times (Codex does), and the client adds them up. Saved with
    /// the transcript, so a reloaded chat's replies keep their stats.
    AcpTurnUsage {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    /// Context-window and cost accounting for the turn so far.
    AcpUsage {
        used: u64,
        size: u64,
        cost: Option<AcpCost>,
    },
    /// The agent account's plan limits (5-hour, weekly), for the usage ring.
    AcpLimits {
        windows: Vec<mira_acp::events::LimitWindow>,
    },
    /// The agent retitled the session.
    AcpSessionInfo {
        title: Option<String>,
        updated_at: Option<String>,
    },
    /// A turn finished. `stop_reason` is the agent's own reason verbatim, so
    /// the UI can distinguish a completed answer from a cancelled or
    /// truncated one rather than assuming success. `detail` carries
    /// human context the agent supplied (e.g. when a limit resets), so the
    /// client can render one message instead of a warning plus an error
    /// saying the same thing twice.
    AcpTurnEnd {
        stop_reason: String,
        detail: Option<String>,
    },
    /// Something arrived that this build does not model.
    ///
    /// Surfaced rather than dropped. ACP's own spec under-documents
    /// `SessionUpdate`, and real agents send vendor extensions, so a silent
    /// gap here is indistinguishable from a hang in the UI.
    AcpUnmodelled {
        method: String,
        reason: String,
    },
    /// The agent's session mode changed.
    ///
    /// Emitted for every change, not just privileged ones: a mode change is
    /// a standing change to what the agent may do, so it belongs in the
    /// transcript as a record rather than living only in a dropdown.
    AcpModeChanged {
        kind: String,
        display_name: String,
        mode_id: String,
        mode_name: String,
        /// True when this mode grants more than Mira's own approval
        /// pipeline would.
        privileged: bool,
    },
    /// A privileged mode was requested without acknowledgement.
    ///
    /// The client should re-prompt the user and retry with
    /// `acknowledge_privileged: true`. Carries the reason so the prompt can
    /// explain the consequence rather than just naming the mode.
    AcpPrivilegedModeConfirmation {
        kind: String,
        display_name: String,
        mode_id: String,
        mode_name: String,
        reason: String,
    },
    /// Per-agent health, for the agent list.
    AcpAgentStatus {
        agents: Vec<mira_acp::status::AgentStatus>,
    },
    /// An external agent was brought up, or failed to be.
    AcpAgentStarted {
        kind: String,
        display_name: String,
        /// The resolved command, credentials redacted.
        launch: String,
        /// Present when startup failed; the reason is user-facing.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Cost reported by an external agent.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AcpCost {
    pub amount: f64,
    pub currency: String,
}

impl ServerMsg {
    /// Map a normalized ACP event from an external agent into a wire frame.
    ///
    /// Returns `None` for spend, which is bookkeeping for the usage ledger
    /// (the event port records it) rather than anything a client shows. Every modelled event maps to a frame, and anything
    /// unmodelled becomes `AcpUnmodelled` rather than being dropped — a
    /// silent gap here is indistinguishable from a hung agent in the UI.
    pub fn from_acp(ev: mira_acp::events::NormalizedEvent) -> Option<Self> {
        use mira_acp::events::{EventSource, MiraEvent};
        let event = ev.event;
        Some(match event {
            MiraEvent::MessageMetadata { message_id, phase } => {
                Self::AcpMessageMetadata { message_id, phase }
            }
            MiraEvent::Activity {
                kind,
                title,
                detail,
            } => Self::StreamActivity {
                kind,
                title,
                detail,
            },
            MiraEvent::AssistantText { text, message_id } => Self::AcpText { text, message_id },
            MiraEvent::AssistantSnapshot { message_id, text } => {
                Self::AcpTextSnapshot { message_id, text }
            }
            MiraEvent::ToolOutputDelta { id, text } => Self::AcpToolOutputDelta { id, text },
            MiraEvent::AgentThought { text, .. } => Self::AcpThought { text },
            MiraEvent::UserText { text, message_id } => Self::AcpText { text, message_id },
            MiraEvent::ToolCall(call) => Self::AcpToolCall { call },
            MiraEvent::ToolCallUpdate(call) => Self::AcpToolCallUpdate { call },
            MiraEvent::Plan { entries } => Self::AcpPlan { entries },
            MiraEvent::Modes { current, available } => {
                // The posture mapping is computed once, here, where the
                // agent's own mode names are in scope — the fixed
                // vocabulary lives in `mira-engine`, and clients never
                // regex-match mode ids.
                let modes: Vec<(String, String, Option<String>)> = available
                    .iter()
                    .map(|m| (m.id.clone(), m.name.clone(), m.description.clone()))
                    .collect();
                let postures = mira_engine::map_postures(&modes, Some(&current));
                Self::AcpModes {
                    current,
                    available,
                    postures,
                    driver: None,
                }
            }
            MiraEvent::ConfigOptions { options } => Self::AcpConfigOptions {
                options,
                driver: None,
            },
            MiraEvent::Commands { names } => Self::AcpCommands { names },
            MiraEvent::Limits { windows } => Self::AcpLimits { windows },
            MiraEvent::Spend { .. } | MiraEvent::RuntimeRequest(_) => return None,
            MiraEvent::RuntimeWork(work) => Self::RuntimeWorkUpdated { work },
            MiraEvent::RuntimeTurn(turn) => Self::RuntimeTurnUpdated { turn },
            MiraEvent::Usage { used, size, cost } => Self::AcpUsage {
                used,
                size,
                cost: cost.map(|(amount, currency)| AcpCost { amount, currency }),
            },
            MiraEvent::SessionInfo { title, updated_at } => {
                Self::AcpSessionInfo { title, updated_at }
            }
            MiraEvent::Unmodelled { source, reason } => match source {
                EventSource::Unmodelled { method } => Self::AcpUnmodelled { method, reason },
                // Modelled variant we chose not to surface as its own frame;
                // still reported so the gap is visible rather than silent.
                EventSource::Acp { variant } => Self::AcpUnmodelled {
                    method: variant,
                    reason,
                },
            },
        })
    }

    /// Report a finished turn. Separate from [`Self::from_acp`] because the
    /// stop reason arrives as a `session/prompt` response, not as an update.
    pub fn acp_turn_end(stop_reason: impl Into<String>) -> Self {
        Self::AcpTurnEnd {
            stop_reason: stop_reason.into(),
            detail: None,
        }
    }

    /// Report a finished turn with human context (e.g. a usage-limit reset
    /// time the agent reported). One frame, not a warning plus an end.
    pub fn acp_turn_end_with(stop_reason: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::AcpTurnEnd {
            stop_reason: stop_reason.into(),
            detail: Some(detail.into()),
        }
    }

    /// Map a raw harness event into a wire frame. Approval frames are emitted
    /// from the approver, not the event stream, so they don't appear here.
    pub fn from_harness(evt: HarnessEvent) -> Self {
        match evt {
            HarnessEvent::Token(text) => Self::Token { text },
            HarnessEvent::Reasoning(text) => Self::Reasoning { text },
            HarnessEvent::ToolStart(call) => Self::ToolStart { call },
            HarnessEvent::ToolEnd(result) => Self::ToolEnd { result },
            HarnessEvent::TurnComplete => Self::TurnComplete,
            HarnessEvent::Done => Self::Done,
            HarnessEvent::Warning(text) if text.starts_with("[retry]") => Self::StreamActivity {
                kind: "retry".into(),
                title: "Retrying verification".into(),
                detail: text.trim_start_matches("[retry]").trim().into(),
            },
            HarnessEvent::Warning(text) => Self::Warning { text },
            HarnessEvent::Usage {
                round,
                totals,
                context_window,
                compact_at,
            } => Self::Usage {
                round,
                totals,
                context_window: Some(context_window),
                compact_at: Some(compact_at),
            },
            HarnessEvent::RateLimit(rate_limit) => Self::RateLimit {
                summary: rate_limit.summary(),
                rate_limit,
            },
            HarnessEvent::MemoryLearned { count } => Self::MemoryLearned { count },
            HarnessEvent::Compacted { messages_removed } => Self::Compacted {
                messages_removed,
                tokens_before: None,
                tokens_after: None,
            },
            HarnessEvent::Compacting { tokens_before } => Self::Compacting {
                trigger: "auto".into(),
                tokens_before: Some(tokens_before),
            },
            HarnessEvent::CompactionFailed { error } => Self::CompactionFailed { error },
            HarnessEvent::GoalSet { goal } => Self::GoalSet { goal },
            HarnessEvent::GoalCleared => Self::GoalCleared,
            HarnessEvent::GoalProgress {
                iteration,
                max_iterations,
                status,
                reason,
            } => Self::GoalProgress {
                iteration,
                max_iterations,
                status,
                reason,
            },
            HarnessEvent::GoalDone { status, reason } => Self::GoalDone { status, reason },
            HarnessEvent::ToolProgress { call_id, line } => Self::ToolProgress { call_id, line },
            HarnessEvent::ToolPreview { call_id, preview } => {
                Self::ToolPreview { call_id, preview }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_message_identity_survives_wire_and_legacy_replay() {
        use mira_acp::events::{EventSource, MiraEvent, NormalizedEvent};
        let frame = ServerMsg::from_acp(NormalizedEvent {
            source: EventSource::Acp {
                variant: "agent_message_chunk".into(),
            },
            event: MiraEvent::AssistantText {
                message_id: Some("native-message".into()),
                text: "hello".into(),
            },
        })
        .unwrap();
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["message_id"], "native-message");
        let legacy = serde_json::to_value(ServerMsg::AcpText {
            text: "old".into(),
            message_id: None,
        })
        .unwrap();
        assert_eq!(legacy, serde_json::json!({"type":"acp_text","text":"old"}));
    }

    #[test]
    fn rate_limits_reach_the_web_ui_as_rate_limit_frames() {
        let rl = mira_ai::RateLimit {
            tokens: Some(mira_ai::Bucket {
                limit: Some(1000),
                remaining: Some(100),
                reset_secs: Some(30),
            }),
            ..Default::default()
        };
        let v = serde_json::to_value(ServerMsg::from_harness(HarnessEvent::RateLimit(rl))).unwrap();
        assert_eq!(v["type"], "rate_limit");
        assert_eq!(v["rate_limit"]["tokens"]["remaining"], 100);
        assert_eq!(v["summary"], "10% of tokens left · resets in 30s");
    }
}
