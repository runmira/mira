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
  images?: { media_type: string; data: string }[];
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

export type Message = {
  role: Role;
  content?: string | null;
  tool_calls?: ToolCall[];
  tool_call_id?: string | null;
  name?: string | null;
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
  | ({ kind: 'ask_user' } & AskUserResponse);

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

export type ServerMsg =
  | { type: 'ready'; session_id: string; model: string; mode: Mode; cwd: string; history: Message[]; turns?: TurnMeta[]; usage?: UsageTotals; tasks?: TaskItem[]; goal?: Goal | null; previews?: Record<string, DiffPreview> }
  | { type: 'token'; text: string }
  | { type: 'tool_start'; call: ToolCall }
  | { type: 'tool_end'; result: ToolResult }
  | { type: 'turn_complete' }
  | { type: 'done' }
  | { type: 'approval_request'; call: ToolCall; preview?: DiffPreview | null }
  | { type: 'warning'; text: string }
  | { type: 'environment_status'; status: EnvironmentStatus; environments: EnvironmentInfo[] }
  | { type: 'environment_progress'; text: string }
  | { type: 'environment_switched'; from: string; to: string; lines: string[]; conflicts: string[]; error?: string | null; status: EnvironmentStatus }
  | { type: 'tool_progress'; call_id: string; line: string }
  | { type: 'tool_preview'; call_id: string; preview: DiffPreview }
  | { type: 'skills_reloaded' }
  | { type: 'extensions_changed' }
  | { type: 'model_changed'; model: string }
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
  | { type: 'usage'; round: TokenUsage; totals: UsageTotals }
  | { type: 'memory_learned'; count: number }
  | { type: 'compacted'; messages_removed: number }
  | { type: 'goal_set'; goal: Goal }
  | { type: 'goal_cleared' }
  | { type: 'goal_progress'; iteration: number; max_iterations: number; status: GoalStatus; reason?: string | null }
  | { type: 'goal_done'; status: GoalStatus; reason?: string | null }
  | { type: 'plan_request'; prompt_id: string; plan: PlanProposal }
  | { type: 'ask_user_request'; prompt_id: string; proposal: AskUserProposal }
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
  | { type: 'send'; text: string }
  | { type: 'approve'; call_id: string; allow: boolean; scope?: ApprovalScope }
  | ({ type: 'prompt_response'; prompt_id: string } & PromptResponse)
  | { type: 'set_model'; model: string }
  /** No target = ask for `environment_status`; a name (or `local`) switches. */
  | { type: 'environment'; target?: string | null }
  | { type: 'set_mode'; mode: Mode }
  | { type: 'set_effort'; effort: string | null }
  | { type: 'interrupt' }
  | { type: 'set_goal'; condition: string; max_iterations?: number | null; evaluator_model?: string | null }
  | { type: 'clear_goal' }
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
  default_mode?: string | null;
  max_tokens?: number | null;
  temperature?: number | null;
  providers: ProviderView[];
  keys: KeyView[];
  configured: boolean;
  config_path: string;
  memory: MemoryView;
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
  default_mode?: string | null;
  max_tokens?: number | null;
  temperature?: number | null;
  providers?: ProviderUpdate[];
  keys?: KeyUpdate[];
  memory?: MemoryUpdate;
};

export type WorktreeMergeStatus = 'merged' | 'unmerged';

export type SessionSummary = {
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
  /** Running token totals across the session's turns. Omitted (or all-zero)
   *  for empty sessions that haven't hit the provider yet. */
  usage?: UsageTotals;
};
