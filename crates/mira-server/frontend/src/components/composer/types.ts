import type { CommandInfo } from '../../api';
import { type EngineSnapshot, type OptionDescriptor } from '../../api';
import type {
  AcpAgentStatus,
  AcpConfigOption,
  AcpSessionMode,
  ApprovalScope,
  AskUserProposal,
  DiffPreview,
  EnvironmentInfo,
  EnvironmentStatus,
  Goal,
  Mode,
  PlanProposal,
  PlanStep,
  RateLimitReading,
  SessionEngine,
  ToolCall,
  UsageTotals,
} from '../../types';
import type { AskUserDecision } from '../AskUserCard';
import { type ComposerNotice } from '../ComposerNoticeStack';
import { type UsageRingData } from '../UsageRing';
import { type PaletteSkill } from '../commands';
export type Props = {
  disabled: boolean;
  busy: boolean;
  mode: Mode;
  model: string;
  /** Configured routing provider (openrouter, openai, groq, …). Shown as
   *  the "Provider" row in the model picker so users see who's actually
   *  serving the request, not the model family extracted from the id. */
  providerName?: string | null;
  cwd: string;
  /** Running session totals. Rendered as the composer's footer readout;
   *  cost is priced against the currently-selected model. Null (or all-zero)
   *  hides the readout entirely — no "$0.00" for a fresh session. */
  usage: UsageTotals | null;
  /** The provider's latest rate-limit reading, when it reports one. */
  rateLimit?: RateLimitReading | null;
  onSend: (text: string, images?: ImageData[]) => void;
  onQueueMessage?: (text: string, images?: ImageData[]) => void;
  queuedMessages?: QueuedComposerMessage[];
  onRemoveQueuedMessage?: (id: string) => void;
  onEditQueuedMessage?: (item: QueuedComposerMessage, text: string) => Promise<void>;
  onReorderQueuedMessage?: (id: string, beforeId: string | null) => Promise<void>;
  onSteerQueuedMessage?: (id: string) => void;
  onSetMode: (m: Mode) => void;
  onSetModel: (m: string, instance?: string | null) => void;
  /** What drives this session, from the server. The picker and the mode
   *  control render from this alone. */
  engine?: SessionEngine | null;
  /** Backend list from `GET /api/engines` — native providers + catalogs. */
  engines?: EngineSnapshot[] | null;
  /** Health of every agent, for the picker's Agents rail. */
  agents?: AcpAgentStatus[] | null;
  agentsChecking?: boolean;
  onCheckAgents?: () => void;
  /** The running agent's config options (its model list included). */
  agentConfig?: AcpConfigOption[] | null;
  /** The running agent's other options, projected for rendering. */
  agentDescriptors?: OptionDescriptor[] | null;
  /** Hand the chat to a provider (instance may be null for the default). */
  onPickProvider?: (instance: string | null, model: string) => void;
  /** Hand the chat to an agent, optionally with a model. */
  onPickAgent?: (driver: string, model: string | null) => void;
  /** Open Settings → Agents. */
  onConfigureAgents?: () => void;
  sessionId?: string | null;
  onAgentCompact?: () => void;
  onAgentFork?: () => void;
  onAgentReverted?: () => void;
  /** Pick the agent's mode through Mira's own picker. The pick is confirmed
   *  in the universal approval dialog (a mode is a grant of standing
   *  authority), then applied to the agent. */
  onPickAgentMode?: (m: Mode) => void;
  /** An external agent drives this session. Mira's own mode picker is then
   *  dead UI — turns go to the agent, not the harness — so the bar shows the
   *  agent's posture instead. One mode control, never two side by side. */
  agentDriving?: boolean;
  /** Session modes an external agent offers, or null. */
  onAcpModes?: AcpSessionMode[] | null;
  /** The agent's active mode id. */
  onAcpCurrentMode?: string | null;
  /** Apply one advertised model option (e.g. `reasoning_effort`,
   *  `service_tier`). The server validates the id. */
  onSetModelOption: (id: string, value: string) => void;
  onOpenPicker: () => void;
  /** Called after a successful in-composer cwd switch (Composer's own
   *  quick-switch dropdown, not the FolderPicker dialog). `sessionId` is
   *  the fresh slot the server built for the new folder — App attaches
   *  its WS to it so the new Ready lands in the transcript. */
  onCwdSwitched?: (path: string, sessionId?: string) => void;
  /** Remote environments for this session (null until the server replies). */
  environment?: EnvironmentStatus | null;
  environments?: EnvironmentInfo[];
  /** Latest progress line while a switch runs; null when idle. */
  envSwitching?: string | null;
  onSwitchEnvironment?: (target: string) => void;
  onInterrupt: () => void;
  onNewChat: () => void;
  onOpenSettings: () => void;
  onRunReview: (args: string) => void;
  /** Kick off an autonomous run. `maxIterations` is optional; server
   *  defaults to `mira_harness::DEFAULT_MAX_ITERATIONS` when omitted. */
  onSetGoal: (condition: string, maxIterations?: number) => void;
  /** Drop the standing goal (if any). */
  onClearGoal: () => void;
  /** `/compact [focus]`: summarize the conversation now. */
  onCompact: (focus: string) => void;
  /** Session's standing `/goal`, if any. Renders a purple chip at the
   *  top of the composer while active — mirrors the Plan chip pattern
   *  so users know autonomy is on. */
  goal: Goal | null;
  onRemember: (scope: 'user' | 'project', text: string) => Promise<string>;
  onUndo: (count: number) => Promise<string>;
  /** Loaded skill roster from `/api/skills`. Rendered inline in the
   *  slash palette after the built-in commands; picking `/<name>`
   *  fires a canned "use the `<name>` skill." user message which the
   *  model turns into a `Skill` tool call. Empty array = no skills or
   *  the roster hasn't loaded yet — palette still works. */
  skills: PaletteSkill[];
  /** Active approval waiting for the user's Y/N decision. When set,
   *  the Composer grows upward to show the approval UI instead of the
   *  text input. */
  /** Context, limits and spend for the usage ring. */
  usageRing?: UsageRingData | null;
  pendingApproval?: PendingApproval | null;
  pendingApprovals?: PendingApproval[];
  ruleEditorCallId?: string | null;
  onRuleEditorCancel?: () => void;
  onActiveApprovalChange?: (callId: string | null) => void;
  pendingPlans?: { callId: string; proposal: PlanProposal }[];
  pendingQuestions?: { callId: string; proposal: AskUserProposal }[];
  notices?: ComposerNotice[];
  /** How many approvals are waiting, including the one shown. */
  pendingApprovalCount?: number;
  /** Allow every waiting request once. */
  onAllowAllPending?: () => void;
  /** Active plan proposal waiting for the user to approve/cancel. */
  pendingPlan?: { callId: string; proposal: PlanProposal } | null;
  /** Active ask_user proposal waiting for answers. */
  pendingAskUser?: { callId: string; proposal: AskUserProposal } | null;
  /** Approve/deny a pending tool call. */
  onDecide?: (callId: string, allow: boolean, scope?: ApprovalScope, rules?: string[]) => void;
  /** Reply to a plan proposal. */
  onPlanReply?: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  /** Reply to an ask_user proposal. */
  onAskUserReply?: (callId: string, decision: AskUserDecision) => void;
  /** Custom commands and MCP prompts (`/api/commands`). */
  commands?: CommandInfo[];
};

export type PendingApproval = {
  callId: string;
  call: ToolCall;
  preview: DiffPreview | null;
  /** Parts of a compound command that need approval (Mira's policy). */
  needs?: string[];
  startedAt?: number;
  rulePreview?: { rules: string[]; error: string | null };
};

export type Attachment = { path: string; content: string; bytes: number };

/** An image the model will see: base64 without the `data:` prefix. */
export type ImageData = import('../../types').ImageAttachment;

export type QueuedComposerMessage = {
  fingerprint?: string;
  steering?: boolean;
  error?: string;
  id: string;
  text: string;
  images?: ImageData[];
};
