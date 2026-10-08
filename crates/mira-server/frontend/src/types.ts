export type ImageSource = { name: string; window?: string; captured_at?: string; width?: number; height?: number; accessible_text?: string };
/** `data` is base64. Large images in loaded history arrive with `data` empty
 *  and a `url` to fetch them from instead — render with `imageSrc`. */
export type ImageAttachment = { media_type: string; data: string; url?: string; source?: ImageSource };
// Wire types — mirror crates/mira-server/src/protocol.rs.

export type Mode = 'plan' | 'manual' | 'auto' | 'edit' | 'yolo';

export type ToolCall = {
  id: string;
  type: 'function';
  function: { name: string; arguments: string };
};

export type ToolResult = {
  call_id: string;
  content: string;
  is_error?: boolean;
  // Optional structured data the tool returned alongside the human-readable
  // `content`. task_* tools use it to publish current state to the UI without
  // asking the client to re-parse `content`.
  data?: unknown;
  // Screenshots from the `computer` / `browser` tools (base64, no prefix).
  images?: ImageAttachment[];
};

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'deleted';

export type TaskItem = {
  id: number;
  subject: string;
  description?: string;
  active_form?: string | null;
  status: TaskStatus;
  created_at?: number;
  updated_at?: number;
};

export type Role = 'system' | 'user' | 'assistant' | 'tool';

/** One block of model reasoning on an assistant message. `signature` /
 *  `redacted` are provider replay data — only `text` is for display. */
export type ReasoningBlock = {
  text?: string;
  signature?: string;
  redacted?: string;
};

export type Message = {
  created_at?: number;
  input_id?: string; input_intent?: 'steer';
  role: Role;
  content?: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string | null;
  name?: string | null;
  reasoning?: ReasoningBlock[];
  /** Images on a user message (pasted screenshots). */
  images?: ImageAttachment[];
};

export type DiffKind = 'edit' | 'overwrite' | 'create';

export type DiffLine =
  | { tag: 'ctx'; text: string }
  | { tag: 'add'; text: string }
  | { tag: 'del'; text: string }
  | { tag: 'hunkgap' };

export type DiffPreview = {
  path: string;
  kind: DiffKind;
  lines: DiffLine[];
};

export type ReviewSeverity = 'critical' | 'high' | 'medium' | 'low';

export type ReviewFinding = {
  severity: ReviewSeverity;
  file: string;
  line?: number | null;
  title: string;
  explanation: string;
  suggested_fix?: string | null;
  verify_note?: string | null;
};

export type ReviewProgressEvent =
  | { kind: 'stage1_started'; diff_lines: number }
  | { kind: 'stage1_completed'; total_findings: number }
  | { kind: 'stage2_started'; total: number }
  | { kind: 'stage2_item'; index: number; total: number; title: string; kept: boolean | null }
  | { kind: 'completed'; kept: number; dropped: number };

export type TurnMeta = {
  started_at: number; // ms epoch
  ended_at?: number | null;
  /** Tokens the turn used, summed over its rounds. Absent for turns
   *  recorded before per-turn usage existed. */
  usage?: UsageTotals | null;
  /** The model the turn ran on — it can differ from the session's now. */
  model?: string | null;
};

/* -------- /goal -------- */

/** Terminal-or-active state of the standing goal. Matches the Rust
 *  enum in `mira-harness/src/goal.rs` (serde `snake_case`). */
export type GoalStatus =
  | 'active'
  | 'met'
  | 'impossible'
  | 'needs_user'
  | 'cleared'
  | 'exhausted';

/** Standing autonomous-run objective. `condition` is the contract the
 *  user wrote; `iterations` / `max_iterations` show the harness's
 *  bounded loop budget; `last_reason` is the evaluator's most recent
 *  note (the "why we're still going" line). */
export type Goal = {
  condition: string;
  status: GoalStatus;
  iterations: number;
  max_iterations: number;
  created_at: number;
  last_reason?: string | null;
  evaluator_model?: string | null;
};

/* -------- interactive tools (plan / ask_user / …) -------- */

export type PlanStep = {
  description: string;
  why?: string | null;
};

export type PlanProposal = {
  title: string;
  steps: PlanStep[];
};

/** Client → server reply for the plan tool. */
export type PlanResponse = {
  approved: boolean;
  /** Present when the user edited before approving. */
  steps?: PlanStep[];
  /** Optional free-text note on cancel. */
  note?: string;
};

/** User's answer to a review-required subagent prompt. `approved: false`
 *  converts the child's summary into a tool-error for the parent; the
 *  optional `note` is prepended to the summary on approval or used as
 *  the error body on denial. */
export type SubagentReviewResponse = {
  approved: boolean;
  note?: string;
};

/* ------------------------------------------------------------------ *
 * ACP — external agent events (Grok, Cursor, Antigravity).
 *
 * These mirror the `Acp*` variants of the Rust `ServerMsg` enum. The
 * normalization from ACP's own dialect happens in `mira-acp`; nothing
 * above that crate has to know what an ACP session update looks like.
 *
 * `crates/mira-server/src/protocol.rs` has a test that parses this
 * file and asserts every arm name here matches, so the hand-maintained
 * boundary between the two cannot drift silently.
 * ------------------------------------------------------------------ */

/** A file a tool call touched. `line` is null when the call is file-level. */
export type AcpLocation = {
  path: string;
  line?: number | null;
};

/** Output attached to a tool call. `type` is the discriminator. */
export type AcpToolContent =
  | { type: 'content'; text: string }
  | { type: 'diff'; path: string; old_text?: string | null; new_text?: string | null }
  | { type: 'terminal'; terminal_id: string };

/** The agent's view of a tool call, forwarded whole.
 *
 *  Kept as the normalized shape rather than folded into Mira's own
 *  `ToolCall`: ACP carries diff and terminal content that `ToolCall` has
 *  no field for, and dropping it would lose the agent's actual output. */
export type AcpToolCall = {
  id: string;
  title: string;
  name?: string | null;
  kind?: string | null;
  status: string;
  content: AcpToolContent[];
  locations: AcpLocation[];
  raw_input?: unknown | null;
  raw_output?: unknown | null;
};

export type AcpPlanItem = {
  content: string;
  priority: string;
  status: string;
};

export type AcpSessionMode = {
  id: string;
  name: string;
  description?: string | null;
};

/**
 * The server's posture mapping for one agent mode. Computed server-side
 * from the fixed five-posture vocabulary, so the client renders options
 * without pattern-matching agent mode ids itself.
 */
export type AgentPostureMapping = {
  key: 'plan' | 'ask' | 'edits' | 'auto' | 'yolo';
  mode_id: string;
  mode_name: string;
  mode_description?: string | null;
  current: boolean;
};

/**
 * Why an agent is not usable right now.
 *
 * Health only — deliberately has no `disabled` variant. Whether the user has
 * switched an agent on is a local preference, and having the server report
 * it meant every row read "Disabled" whatever was installed.
 */
export type AcpAgentState =
  | { state: 'not_found'; looked_for: string }
  | { state: 'failed'; reason: string }
  | { state: 'ready' };

/** One row in the agent list. */
export type AcpAgentStatus = {
  models?: { value: string; label: string }[];
  kind: string;
  display_name: string;
  state: AcpAgentState;
  /** Self-reported version. Most adapters omit it, so often absent even
   *  when the agent works. */
  version?: string | null;
  /** Human-readable auth summary, e.g. "Claude Pro Subscription". */
  auth?: string | null;
  auth_method_ids: string[];
  /** The resolved command, with credentials redacted. */
  launch: string;
  /** Whether the agent's own CLI is present, when it is a separate program
   *  from the ACP adapter we spawn. Undefined for agents that speak ACP
   *  natively. */
  cli_installed?: boolean;
  /** The agent CLI's own version, e.g. "2.1.283" for Claude Code. This is
   *  what the user has installed, and it is distinct from `version`, which
   *  comes from the adapter's `agentInfo`. */
  cli_version?: string | null;
  /** How to install the missing ACP adapter. */
  install_hint?: string;
  /** How Mira reached this agent: its own CLI, or an ACP adapter. */
  transport?: 'auto' | 'native' | 'acp';
};

/** One persisted agent-transcript line: a user prompt or a server frame. */
export type AgentTranscriptLine = {
  t: number;
  driver: string;
  frame?: unknown;
  user?: { text: string; images: number; attached_images?: ImageAttachment[]; input_id?: string; input_intent?: 'steer' };
  /** Written when the session switched provider → agent: the harness
   *  history up to `harness_len` happened before this point. Lets replay
   *  interleave the two histories in order. */
  switch?: {
    harness_len: number;
    to?: 'agent' | 'provider';
    /** The engines on each side, for the handoff label. Absent on
     *  markers written before handoffs were labelled. */
    from_engine?: EngineRef;
    to_engine?: EngineRef;
  };
};

/** One side of an engine handoff, as a switch marker records it. */
export type EngineRef = {
  kind: 'provider' | 'agent';
  driver?: string | null;
  instance?: string | null;
  display_name: string;
  model?: string | null;
};

/** What drives a session — one of Mira's providers, or an external agent —
 *  and where it is in its lifecycle. Pushed by the server on every change;
 *  the composer renders from this alone. */
export type SessionEngine = {
  capabilities?: { asynchronous_questions: boolean; background_work: boolean; native_goals: boolean;
    steering: 'unavailable' | 'native' | 'safe_boundary'; image_input: boolean;
    live_model_switch: boolean; live_mode_switch: boolean; native_fork: boolean;
    native_rollback: boolean; native_snapshot: boolean; cancellation: boolean;
    stop_behavior: 'current_turn' | 'runtime' };
  kind: 'provider' | 'agent';
  /** Provider: engine instance id (`anthropic`, `openrouter`, …). */
  instance?: string | null;
  /** Agent: driver kind (`claude-code`, `codex`, …). */
  driver?: string | null;
  display_name: string;
  model?: string | null;
  /** `idle`: picked, not running (starts on the next message). */
  status: 'idle' | 'starting' | 'ready' | 'error';
  error?: string | null;
};

/** A session from an agent's own history, resumable in Mira. */
export type ExternalAgentSession = {
  id: string;
  cwd: string;
  model?: string | null;
  messages: number;
  first_text?: string | null;
  updated_at?: string | null;
};

export type AcpConfigValue = {
  value: string;
  name: string;
  description?: string | null;
};

/** One config option. `category === 'model'` is the model selector —
 *  ACP has no set-model method, the model list *is* a config option. */
export type AcpConfigOption = {
  id: string;
  name: string;
  description?: string | null;
  category?: string | null;
  current?: string | null;
  values: AcpConfigValue[];
};

/* ask_user tool — model-supplied clarifying questions. */
export type AskUserOption = {
  label: string;
  description?: string | null;
  /** Model's preferred pick; the UI renders a "Recommended" badge. */
  recommended?: boolean;
};

export type AskUserQuestion = {
  question: string;
  /** Short chip label (max ~12 chars). Rendered above the question. */
  header?: string | null;
  options: AskUserOption[];
  /** When true, the question renders as multi-select checkboxes. */
  multi_select?: boolean;
};

export type AskUserProposal = {
  questions: AskUserQuestion[];
};

/** One answer per question, in the order the tool posed them. Empty
 *  `picked` + null `custom` = skipped; `picked` populated = picked
 *  options; `custom` populated = user used the free-text "Tell mira
 *  what to do differently" affordance. */
export type AskUserAnswer = {
  picked: string[];
  custom?: string | null;
};

export type AskUserResponse = {
  answers: AskUserAnswer[];
  /** True when the user dismissed the whole card without answering. */
  cancelled?: boolean;
};

/** Discriminated union of all `PromptResponse` shapes. Matches the Rust
 *  `PromptResponse` enum (serde `tag = "kind"`, snake_case). */
export type PromptResponse =
  | ({ kind: 'plan' } & PlanResponse)
  | ({ kind: 'subagent_review' } & SubagentReviewResponse)
  | ({ kind: 'ask_user' } & AskUserResponse)
  | { kind: 'secret'; value?: string | null; cancelled: boolean };

export type TokenUsage = {
  prompt_tokens: number;
  completion_tokens: number;
  /** Subset of prompt_tokens served from the provider's prompt cache. */
  cached_input_tokens: number;
};

export type UsageTotals = {
  prompt_tokens: number;
  completion_tokens: number;
  cached_input_tokens: number;
  rounds: number;
};

/** One provider limit: how much is left of how much, and when it refills. */
export type RateLimitBucket = { limit?: number; remaining?: number; reset_secs?: number };

/** The provider's rate limits after the latest request (from its headers). */
export type RateLimit = {
  requests?: RateLimitBucket;
  tokens?: RateLimitBucket;
  input_tokens?: RateLimitBucket;
  output_tokens?: RateLimitBucket;
};

/** A reading plus the server's one-line summary, and when it arrived. */
export type RateLimitReading = { rate_limit: RateLimit; summary: string | null; at: number };

/** Per-session policy for how the approver answers `Ask` decisions when
 *  no client is currently attached. See slot.rs BackgroundMode for
 *  authoritative semantics. */
export type BackgroundMode = 'deny' | 'auto_approve' | 'park';

/** Where the session's tools run (`local` = this machine). */
export type EnvironmentStatus = {
  current: string;
  backend: string;
  workspace?: string | null;
  /** Environments left paused/running, quick to switch back to. */
  parked: string[];
};

export type EnvironmentInfo = { name: string; backend: string; description: string };

export type RuntimeWork = { id: string; native_thread_id: string | null; kind: 'goal' | 'task' | 'subagent'; status: 'pending' | 'running' | 'waiting' | 'paused' | 'completed' | 'failed' | 'cancelled'; title: string | null };
export type RuntimeRequestRecord = {
  id: string; instance: string; driver: string;
  request: { native_id: string; native_thread_id: string; native_turn_id: string | null; questions: { id: string; question: string; header: string | null; options: string[]; required: boolean }[]; response_capability: 'message' | 'live_rpc' };
  delivery: 'pending' | 'queued' | 'dispatching' | 'delivered' | 'cancelled';
  response: { answers: AskUserAnswer[]; cancelled: boolean } | null;
  message: string | null;
};

export type TranscriptPage = { items: { message?: Message; line?: AgentTranscriptLine; turn_index: number; provider_turn_index?: number }[]; next_cursor: string | null; total_turns: number; first_turn: number };

export type TurnDiffSummary = { text: string; occurrence: number; files: { path: string; added: number; removed: number; binary: boolean }[] };

export interface QueuedInput {
  recovery?: { created_at: number; reset_at: number | null; scheduled_at: number | null; snoozed: boolean };
  id: string; text: string; images: ImageAttachment[]; engine: string | null;
  dispatching: boolean; error: string | null;
  fingerprint?: string;
}
export type ServerMsg =
  | { type: 'session_activity'; epoch: string; revision: number; session_id: string; running: boolean }
  | { type: 'session_activity_snapshot'; snapshot: import('./lib/sessionActivity').SessionActivitySnapshot }
  | { type: 'queue_updated'; session_id: string; items: QueuedInput[] }
  | { type: 'queue_delivery'; session_id: string; item: QueuedInput }
  | { type: 'queue_mutation_result'; session_id: string; request_id: string; error: string | null }
  | { type: 'steer_result'; session_id: string; request_id: string; message: Message | null; error: string | null }
  | { type: 'turn_diffs'; summaries: TurnDiffSummary[] }
  | { type: 'acp_message_metadata'; message_id: string; phase: string }
  | { type: 'stream_activity'; kind: string; title: string; detail: string }
  | { type: 'history_page'; session_id: string; request_id: string; page: TranscriptPage | null; error: string | null }
  | { type: 'ready'; session_activity?: import('./lib/sessionActivity').SessionActivitySnapshot; queued_inputs?: QueuedInput[]; turn_diffs?: TurnDiffSummary[]; transcript_page?: TranscriptPage; session_id: string; model: string; mode: Mode; cwd: string; history: Message[]; turns?: TurnMeta[]; usage?: UsageTotals; tasks?: TaskItem[]; goal?: Goal | null; previews?: Record<string, DiffPreview>; agent_transcript?: AgentTranscriptLine[]; agent_driver?: string | null; agent_kind?: string | null; instance?: string | null; agent_configured?: string | null; engine?: SessionEngine | null; title?: string | null; running?: boolean; runtime_requests?: RuntimeRequestRecord[]; runtime_work?: RuntimeWork[] }
  /** The session's engine changed: picked, starting, ready, failed, exited. */
  | { type: 'session_engine'; engine: SessionEngine }
  | { type: 'runtime_request_updated'; request: RuntimeRequestRecord }
  | { type: 'runtime_work_updated'; work: RuntimeWork }
  | { type: 'runtime_turn_updated'; turn: { native_thread_id: string; native_turn_id: string; running: boolean } }
  | { type: 'token'; text: string }
  | { type: 'reasoning'; text: string }
  | { type: 'tool_start'; call: ToolCall }
  | { type: 'tool_end'; result: ToolResult }
  | { type: 'turn_complete' }
  | { type: 'done' }
  | { type: 'approval_rules'; call_id: string; rules: string[]; error: string | null }
  | { type: 'approval_resolved'; call_id: string; allow: boolean }
  | { type: 'approval_request'; call: ToolCall; preview?: DiffPreview | null; needs?: string[] }
  | { type: 'warning'; text: string }
  | { type: 'environment_status'; status: EnvironmentStatus; environments: EnvironmentInfo[] }
  | { type: 'environment_progress'; text: string }
  | { type: 'environment_switched'; from: string; to: string; lines: string[]; conflicts: string[]; error?: string | null; status: EnvironmentStatus }
  | { type: 'tool_progress'; call_id: string; line: string }
  | { type: 'tool_preview'; call_id: string; preview: DiffPreview }
  | { type: 'skills_reloaded' }
  | { type: 'extensions_changed' }
  | { type: 'model_changed'; model: string; instance?: string | null }
  | { type: 'mode_changed'; mode: Mode }
  | { type: 'error'; text: string }
  | { type: 'review_started'; run_id: string }
  | { type: 'review_progress'; run_id: string; event: ReviewProgressEvent }
  | { type: 'review_result'; run_id: string; findings: ReviewFinding[] }
  | { type: 'review_error'; run_id: string; text: string }
  | { type: 'session_title_updated'; session_id: string; title: string }
  | { type: 'background_mode_changed'; session_id: string; mode: BackgroundMode }
  | { type: 'session_background_idle'; session_id: string }
  | { type: 'session_background_running'; session_id: string }
  | { type: 'usage'; round: TokenUsage; totals: UsageTotals; context_window?: number | null; compact_at?: number | null }
  | { type: 'rate_limit'; rate_limit: RateLimit; summary: string | null }
  | { type: 'memory_learned'; count: number }
  | { type: 'compacted'; messages_removed: number; tokens_before?: number | null; tokens_after?: number | null }
  /** Compaction started (`manual` or `auto`). */
  | { type: 'compacting'; trigger: string; tokens_before?: number | null }
  | { type: 'compaction_failed'; error: string }
  | { type: 'goal_set'; goal: Goal }
  | { type: 'goal_cleared' }
  | { type: 'goal_progress'; iteration: number; max_iterations: number; status: GoalStatus; reason?: string | null }
  | { type: 'goal_done'; status: GoalStatus; reason?: string | null }
  | { type: 'plan_request'; prompt_id: string; plan: PlanProposal }
  | { type: 'ask_user_request'; prompt_id: string; proposal: AskUserProposal }
  | {
      type: 'prompt_resolved';
      prompt_id: string;
      kind: 'ask_user' | 'plan' | 'subagent_review' | 'secret';
      answers?: AskUserAnswer[];
      cancelled?: boolean;
      approved?: boolean;
      steps?: PlanStep[] | null;
      note?: string | null;
    }
  // Subagent live-stream frames. Every subagent-related event carries the
  // parent's tool_call id so the frontend routes it to the right panel tab.
  | { type: 'subagent_started'; parent_call_id: string; agent_id: string; model: string; prompt: string; agent_name?: string | null; agent_category?: string | null }
  | { type: 'subagent_token'; parent_call_id: string; text: string }
  | { type: 'subagent_tool_start'; parent_call_id: string; call: ToolCall }
  | { type: 'subagent_tool_end'; parent_call_id: string; result: ToolResult }
  | { type: 'subagent_warning'; parent_call_id: string; text: string }
  | { type: 'subagent_progress'; parent_call_id: string; text: string }
  | { type: 'subagent_review_request'; parent_call_id: string; prompt_id: string; summary: string }
  | { type: 'subagent_done'; parent_call_id: string }
  | {
      type: 'subagent_scratchpad_note';
      parent_call_id: string;
      parent_session_id: string;
      entry: ScratchpadEntry;
    }
  /** Live activity from a `delegate_task` child, routed to the delegation
   *  card by `call_id` (or, failing an exact match, the most recent running
   *  delegation). `kind` is a coarse verb the UI maps to an icon; `text` is
   *  the human-readable step. */
  | { type: 'delegate_progress'; call_id: string; kind: string; text: string }
  // -------- ACP (external agent) frames --------
  // Distinct variants rather than a wrapped envelope, so this file
  // pattern-matches by `type` the same way it already does for subagents.
  | { type: 'acp_text'; text: string; message_id?: string | null }
  | { type: 'acp_text_snapshot'; text: string; message_id: string }
  | { type: 'acp_tool_output_delta'; text: string; id: string }
  | { type: 'acp_thought'; text: string }
  | { type: 'acp_tool_call'; call: AcpToolCall }
  | { type: 'acp_tool_call_update'; call: AcpToolCall }
  | { type: 'acp_plan'; entries: AcpPlanItem[] }
  | { type: 'acp_modes'; current: string; available: AcpSessionMode[]; postures?: AgentPostureMapping[]; driver?: string | null }
  | { type: 'acp_config_options'; options: AcpConfigOption[]; driver?: string | null }
  | { type: 'acp_commands'; names: string[] }
  | { type: 'acp_usage'; used: number; size: number; cost: { amount: number; currency: string } | null }
  | { type: 'acp_turn_usage'; model: string; input_tokens: number; output_tokens: number; cached_input_tokens: number; cost_usd?: number | null }
  /** The agent account's plan limits (Claude's 5-hour and weekly windows). */
  | { type: 'acp_limits'; windows: { name: string; utilization: number; resets_at?: number | null }[] }
  | { type: 'acp_session_info'; title: string | null; updated_at: string | null }
  /** The agent's own stop reason, verbatim. `cancelled` and
   *  `max_tokens` must not render as a completed answer. */
  | { type: 'acp_turn_end'; stop_reason: string; detail?: string | null }
    /** Something this build does not model. Surfaced as a warning rather
   *  than dropped: a silent gap is indistinguishable from a hung agent. */
  | { type: 'acp_unmodelled'; method: string; reason: string }
  /** An agent called one of Mira's browser tools: show the browser pane. */
  | { type: 'browser_active' }
  /** The engine registry was rebuilt from mira.yaml: refetch /api/engines. */
  | { type: 'engines_changed' }
  /** An agent asked for a secret: show a private input. */
  | { type: 'secret_request'; prompt_id: string; name: string; reason: string; dotenv?: string | null }
  /** A self-contained page an agent published to the chat. */
  | { type: 'html_render'; id: string; title: string; html: string }
  /** An external agent was brought up, or failed to be. `error` is
   *  user-facing: "grok isn't installed" rather than an opaque code. */
  | { type: 'acp_agent_started'; kind: string; display_name: string; launch: string; error?: string | null }
  /** Per-agent health, for the agent list. */
  | { type: 'acp_agent_status'; agents: AcpAgentStatus[] }
  /** The agent's session mode changed. Recorded for every change, not just
   *  privileged ones: it is a standing change to what the agent may do. */
  | {
      type: 'acp_mode_changed';
      kind: string;
      display_name: string;
      mode_id: string;
      mode_name: string;
      privileged: boolean;
    }
  /** A privileged mode was requested without acknowledgement. The UI must
   *  explain `reason` to the user and retry with the flag set. */
  | {
      type: 'acp_privileged_mode_confirmation';
      kind: string;
      display_name: string;
      mode_id: string;
      mode_name: string;
      reason: string;
    };

export type ScratchpadEntry = {
  author: string;
  ts: number;
  text: string;
};

/** Scope on an `approve` reply. `once` only affects this specific call.
 *  `session` promotes the exact target to `Allow` on the in-memory
 *  policy so identical follow-up calls skip the modal. `always` also
 *  appends the rule to `~/.mira/mira.yaml` so it survives a restart.
 *  Meaningful only when `allow: true`. */
export type ApprovalScope = 'once' | 'session' | 'always';

export type ClientMsg =
  | { type: 'update_limit_recovery'; session_id: string; request_id: string; id: string; action: 'schedule' | 'retry' | 'cancel' | 'snooze' | 'show' | 'dismiss' }
  | {type:'sync_activity'}
  | { type: 'queue_input'; session_id: string; id: string; text: string; images?: ImageAttachment[] }
  | { type: 'remove_queued_input'; session_id: string; id: string }
  | { type: 'edit_queued_input'; session_id: string; request_id: string; id: string; fingerprint: string; text: string; images?: ImageAttachment[] }
  | { type: 'reorder_queued_input'; session_id: string; request_id: string; id: string; before_id: string | null }
  | { type: 'steer'; request_id: string; text: string; images?: ImageAttachment[] }
  | { type: 'history'; session_id: string; cursor: string; request_id: string }
  | { type: 'send'; text: string; images?: ImageAttachment[] }
  | { type: 'resend'; original: string; occurrence: number; text: string }
  | { type: 'approve'; call_id: string; allow: boolean; scope?: ApprovalScope; rules?: string[] }
  | { type: 'approval_rules'; call_id: string }
  | ({ type: 'prompt_response'; prompt_id: string } & PromptResponse)
  | { type: 'set_model'; model: string; instance?: string | null; options?: Record<string, string> }
  // -------- external ACP agents --------
  /** Bring an external agent up for this session, replacing any running one. */
  | {
      type: 'acp_start';
      /** Engine instance from `GET /api/engines`. Its setup (binary, env,
       *  key) lives in mira.yaml on the server; the client sends none. */
      instance?: string | null;
      /** Omit (or send the session's configured kind) to start with the
       *  launch settings inherited from the previous chat. */
      driver?: string | null;
      resume?: string | null;
      /** Agent model picked alongside the agent; remembered for restarts. */
      model?: string | null;
    }
  /** Fork the agent session under a new id (Claude Code only). */
  | { type: 'acp_fork' }
  /** Ask the agent to compact its context (native transports only). */
  | { type: 'acp_compact'; focus?: string | null }
  /** Send a turn to the running agent. */
  | { type: 'acp_prompt'; text: string; images?: ImageAttachment[] }
  /** Stop the agent and release its terminals. */
  | { type: 'acp_stop' }
  /** Ask what every known agent's health is. */
  | { type: 'acp_status' }
  /** Switch the agent's own session mode. Not Mira's `set_mode`. */
  | { type: 'acp_set_mode'; mode_id: string; acknowledge_privileged?: boolean }
  /** Set one of the agent's config options — including its model, which
   *  ACP models as a `category: "model"` option. */
  | { type: 'acp_set_config_option'; option_id: string; value: string }
  /** No target = ask for `environment_status`; a name (or `local`) switches. */
  | { type: 'environment'; target?: string | null }
  | { type: 'set_mode'; mode: Mode }
  /** Set one advertised model option. `id` is a descriptor id such as
   *  `reasoning_effort` or `service_tier`; the server ignores ids it
   *  doesn't know. Replaces the old `set_effort`, which had no server
   *  handler and was silently dropped. */
  | { type: 'set_model_option'; id: string; value: string }
  | { type: 'interrupt' }
  | { type: 'set_goal'; condition: string; max_iterations?: number | null; evaluator_model?: string | null }
  | { type: 'clear_goal' }
  | { type: 'compact'; focus?: string | null }
  | { type: 'sync' }
  /** Switch which session this WS is watching. Server updates its
   *  per-connection attached_id, swaps the forwarder's subscription,
   *  and emits a fresh Ready for the new session. */
  | { type: 'attach'; session_id: string }
  /** Fall back to the server's `active` pointer without closing the
   *  socket. Rarely needed by the current UI (tab close does the same
   *  work) but harmless to expose. */
  | { type: 'detach' }
  /** Set the attached session's background mode — controls how the
   *  approver answers `Ask` decisions when no client is watching. */
  | { type: 'set_background_mode'; mode: BackgroundMode };

export type ProviderView = {
  name: string;
  base_url?: string | null;
  api_key_masked?: string | null;
  has_api_key: boolean;
  api_key_env?: string | null;
};

export type SettingsView = {
  default_provider?: string | null;
  default_model?: string | null;
  small_model?: string | null;
  default_mode?: string | null;
  max_tokens?: number | null;
  temperature?: number | null;
  providers: ProviderView[];
  keys: KeyView[];
  configured: boolean;
  config_path: string;
  memory: MemoryView;
  sessions?: SessionsSettings;
};

/** Effective (defaults applied) memory-runtime knobs. Every field is the
 *  concrete value the harness would use *now*, not the raw yaml Option. */
export type MemoryView = {
  auto_extract: boolean;
  tools_enabled: boolean;
  inject_context: boolean;
  extractor_model?: string | null;
};

export type MemoryUpdate = {
  /** Present with null resets to default; present with value sets. */
  auto_extract?: boolean | null;
  tools_enabled?: boolean | null;
  inject_context?: boolean | null;
  extractor_model?: string | null;
};

export type KeyView = {
  name: string;
  /** Empty string when no key is stored (env-only or fully unset). */
  masked: string;
  from_env: boolean;
};

export type KeyUpdate = {
  name: string;
  /** Omit to keep existing key. Empty string clears. Any value sets. */
  value?: string;
};

export type ProviderUpdate = {
  name: string;
  base_url?: string | null;
  /** Omit to keep existing key. Empty string clears. Any value sets. */
  api_key?: string;
  api_key_env?: string | null;
};

// The server distinguishes "key missing" (leave alone) from "key present
// and null" (clear). Fields you always send stay as `string | null`;
// fields you sometimes want to leave alone use the optional `?`.
export type SettingsUpdate = {
  default_provider?: string | null;
  default_model?: string | null;
  small_model?: string | null;
  default_mode?: string | null;
  max_tokens?: number | null;
  temperature?: number | null;
  providers?: ProviderUpdate[];
  keys?: KeyUpdate[];
  memory?: MemoryUpdate;
  sessions?: Partial<SessionsSettings>;
};

/** How running chats behave in the background. Saved in Mira's global
 *  config, so they apply with no window open. */
export type SessionsSettings = {
  auto_resume_after_limit: boolean;
  keep_awake_while_running: boolean;
};

export type WorktreeMergeStatus = 'merged' | 'unmerged';

export type SessionSummary = {
  parent_id?: string | null;
  id: string;
  model: string;
  cwd: string;
  created_at: number;
  updated_at: number;
  message_count: number;
  title?: string | null;
  first_user_message?: string | null;
  /** True when this session is the server's `active` pointer — HTTP
   *  handlers without a session_id in the URL target it. */
  active: boolean;
  /** Driver slug when an external agent drove turns here, for the sidebar
   *  badge. Absent for harness-only sessions. */
  agent_driver?: string | null;
  /** True when at least one WS forwarder is currently subscribed to this
   *  slot's event stream. Sidebar renders a "•" indicator. */
  attached?: boolean;
  /** True when a turn task is in flight on this slot right now. Sidebar
   *  shows a spinner so background sessions announce their state even
   *  when no client is watching them. */
  running?: boolean;
  /** Background-mode setting for the slot. Absent for persisted sessions
   *  that haven't been loaded yet (they have no runtime slot). */
  background_mode?: BackgroundMode | null;
  /** Merge status of the session's worktree branch vs main/master in the
   *  primary repo. Absent for regular (non-worktree) sessions. */
  worktree_status?: WorktreeMergeStatus | null;
  /** Branch name of the worktree — shown as tooltip on the merge chip. */
  worktree_branch?: string | null;
  /** Sidebar pin — floats the session to the top of its project group.
   *  Persisted server-side via `PUT /api/sessions/:id/flags`. */
  pinned?: boolean;
  /** Made with "Fork from here": the chat it branched off, and the
   *  message it was taken at. The sidebar nests it under that chat. */
  forked_from?: string | null;
  forked_at?: string | null;
  /** True when the user archived this session. Archived sessions only
   *  appear in the sidebar's "Archived" view (`?archived=true`). */
  archived?: boolean;
  /** Running token totals across the session's turns. Omitted (or all-zero)
   *  for empty sessions that haven't hit the provider yet. */
  usage?: UsageTotals;
  /** Waiting on the user: an approval, a question, a plan or a secret. */
  needs_attention?: boolean;
  /** The chat that launched this one with `thread_launch`; the sidebar
   *  nests it there. */
  launched_by?: string | null;
};
