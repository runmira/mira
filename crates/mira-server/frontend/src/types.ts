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

/** Discriminated union of all `PromptResponse` shapes. Matches the Rust
 *  `PromptResponse` enum (serde `tag = "kind"`, snake_case). */
export type PromptResponse = ({ kind: 'plan' } & PlanResponse);

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

export type ServerMsg =
  | { type: 'ready'; session_id: string; model: string; mode: Mode; cwd: string; history: Message[]; turns?: TurnMeta[]; usage?: UsageTotals }
  | { type: 'token'; text: string }
  | { type: 'tool_start'; call: ToolCall }
  | { type: 'tool_end'; result: ToolResult }
  | { type: 'turn_complete' }
  | { type: 'done' }
  | { type: 'approval_request'; call: ToolCall; preview?: DiffPreview | null }
  | { type: 'warning'; text: string }
  | { type: 'model_changed'; model: string }
  | { type: 'mode_changed'; mode: Mode }
  | { type: 'error'; text: string }
  | { type: 'review_started'; run_id: string }
  | { type: 'review_progress'; run_id: string; event: ReviewProgressEvent }
  | { type: 'review_result'; run_id: string; findings: ReviewFinding[] }
  | { type: 'review_error'; run_id: string; text: string }
  | { type: 'session_title_updated'; session_id: string; title: string }
  | { type: 'usage'; round: TokenUsage; totals: UsageTotals }
  | { type: 'plan_request'; prompt_id: string; plan: PlanProposal }
  // Subagent live-stream frames. Every subagent-related event carries the
  // parent's tool_call id so the frontend routes it to the right panel tab.
  | { type: 'subagent_started'; parent_call_id: string; agent_id: string; model: string; prompt: string }
  | { type: 'subagent_token'; parent_call_id: string; text: string }
  | { type: 'subagent_tool_start'; parent_call_id: string; call: ToolCall }
  | { type: 'subagent_tool_end'; parent_call_id: string; result: ToolResult }
  | { type: 'subagent_warning'; parent_call_id: string; text: string }
  | { type: 'subagent_done'; parent_call_id: string };

export type ClientMsg =
  | { type: 'send'; text: string }
  | { type: 'approve'; call_id: string; allow: boolean }
  | ({ type: 'prompt_response'; prompt_id: string } & PromptResponse)
  | { type: 'set_model'; model: string }
  | { type: 'set_mode'; mode: Mode }
  | { type: 'set_effort'; effort: string | null }
  | { type: 'interrupt' }
  | { type: 'sync' };

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
  active: boolean;
  /** Merge status of the session's worktree branch vs main/master in the
   *  primary repo. Absent for regular (non-worktree) sessions. */
  worktree_status?: WorktreeMergeStatus | null;
  /** Branch name of the worktree — shown as tooltip on the merge chip. */
  worktree_branch?: string | null;
};
