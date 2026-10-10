/**
 * Transcript entry types and the pure reducers that build them from live
 * server frames (tokens, tool calls/results, ACP agent frames, turn stats).
 * Split out of App.tsx; everything here is (prev, frame) => next.
 */
import { type AskUserDecision } from '../components/AskUserCard';
import { type CompactionEntry } from '../components/CompactionCard';
import { parseSentAttachments } from '../components/composer/attachments';
import type { DelegateStep } from '../components/DelegateCard';
import { type ToolStatus } from '../components/ToolCard';
import {
  agentCallToToolCall,
  agentToolResult,
  agentToolStatus,
  boundAgentOutput,
} from '../lib/agentTools';
import { isDelegateTaskName } from '../lib/delegateTools';
import { prettyModel } from '../lib/models';
import { appendNativeText, appendNativeToolOutput, boundedOutput } from '../lib/nativeStream';
import { type Turn } from '../lib/turnActivity';
import type {
  AcpPlanItem,
  AcpToolCall,
  AskUserProposal,
  DiffPreview,
  EngineRef,
  Message,
  PlanProposal,
  PlanStep,
  ServerMsg,
  SessionEngine,
  TaskItem,
  ToolCall,
  ToolResult,
  TurnMeta,
  UsageTotals,
} from '../types';

/** Live per-child state for the subagent panel — mirrors the shape of
 *  the parent's own transcript so the panel body can reuse EntryView-style
 *  grouping without special-casing. `done` flips when either an explicit
 *  `subagent_done` frame lands or the parent's ToolEnd for this call
 *  arrives (whichever is first). */
export type SubagentStreamState = {
  parentCallId: string;
  agentId?: string;
  model?: string;
  prompt?: string;
  /** Human-readable name from the agent type frontmatter (e.g. "leo"). */
  agentName?: string | null;
  /** Category from the agent type frontmatter (e.g. "review", "recon"). */
  agentCategory?: string | null;
  entries: Entry[];
  done: boolean;
  /** When set, the child produced a summary that requires human review
   *  before it's returned to the parent. The SubagentPanel renders an
   *  inline card with Approve / Deny buttons; approving fires the
   *  server-bound `PromptResponse` and clears this field. Set from the
   *  `subagent_review_request` frame. */
  pendingReview: null | { promptId: string; summary: string };
};

export let toolActivitySequence = 0;

export type ToolEntry = {
  activityAt?: number;
  kind: 'tool';
  runtimeDelivery?: Extract<ServerMsg, { type: 'runtime_request_updated' }>['request']['delivery'];
  call: ToolCall;
  preview: DiffPreview | null;
  /** For a compound shell command awaiting approval: the parts that
   *  need it, as the server's policy judged them. */
  needs?: string[];
  status: ToolStatus;
  result: ToolResult | null;
  /** Only set for the `plan` tool. `proposal` arrives on `plan_request`;
   *  `decision` fills in when the user approves/cancels. Rendered inline
   *  as a full-fledged plan card instead of the generic tool row. */
  plan?: {
    proposal: PlanProposal;
    decision: null | { approved: boolean; steps?: PlanStep[]; note?: string };
  };
  /** Only set for the `ask_user` tool. `proposal` arrives on
   *  `ask_user_request`; `decision` fills in when the user submits or
   *  skips. Rendered inline as the AskUserCard (multi-choice questions
   *  + "Tell mira what to do differently"). */
  askUser?: {
    proposal: AskUserProposal;
    decision: AskUserDecision | null;
  };
  /** Live output lines streamed via `tool_progress` frames. Populated for
   *  `run_background` and any other long-running tool that emits progress.
   *  Lines accumulate even after the tool result has landed. */
  progressLines?: string[];
  /** Epoch ms the call started, when known (live turns only — not carried
   *  through history reload). Powers the delegation card's elapsed clock. */
  startedAt?: number | null;
  /** Visible steps streamed from a `delegate_task` child (via
   *  `delegate_progress` frames), newest last. The delegation card renders
   *  them as a live activity log; empty for every other tool. */
  delegateSteps?: DelegateStep[];
  /** Set when an external agent made this call: its own description of it,
   *  kept so later updates merge onto it. `call` is its Mira translation,
   *  which is what renders. */
  agentCall?: AcpToolCall;
};

export type WarningEntry = { kind: 'warning'; text: string };

/** Where compaction summarized the conversation: a divider, with the
 *  summary behind a toggle when it's known (after a reload). */
export type CompactEntry = CompactionEntry;

export type ErrorEntry = { kind: 'error'; text: string; responseFailure?: boolean };

export type MsgEntry = {
  steerRequestId?: string;
  kind: 'msg';
  msg: Message;
  nativeMessageId?: string;
  nativeCompleted?: boolean;
  nativePhase?: string;
};

/** The model's reasoning before a reply / tool call. `live` while
 *  `reasoning` frames are still arriving; sealed by the next token,
 *  tool call or turn end. Times are epoch ms, null when restored from
 *  history (duration unknown). */
export type ThoughtEntry = {
  kind: 'thought';
  native?: boolean;
  text: string;
  live: boolean;
  startedAt: number | null;
  endedAt: number | null;
};

/** Structured goal event in the transcript. Rendered as its own
 *  purple-tinted card by [`EntryView`] so goal turns visually anchor
 *  the timeline instead of masquerading as generic warnings.
 *  `variant: 'set'` marks the initial `/goal` moment,
 *  `'cleared'` the user drop, `'progress'` each non-terminal
 *  evaluator verdict, and `'done'` the terminal transition. */
export type GoalEntry = {
  kind: 'goal';
  variant: 'set' | 'cleared' | 'progress' | 'done';
  iteration: number | null;
  maxIterations: number | null;
  status: import('../types').GoalStatus | null;
  reason: string | null;
  /** Only set on `variant: 'set'`. */
  condition?: string | null;
};

/** The agent's plan. */
export type AcpPlanEntry = { kind: 'acp_plan'; entries: AcpPlanItem[] };

/** Where the chat moved to another engine (provider ⇄ agent). A quiet
 *  divider, so a reader can tell which engine wrote what. */
/** A page an agent published with `html_render`. */
export type HtmlRenderEntry = { kind: 'html_render'; id: string; title: string; html: string };

export type EngineSwitchEntry = {
  kind: 'engine_switch';
  engine: SessionEngine;
  /** The engine handing over, when known: the divider then reads as a
   *  handoff (from → to) rather than just "switched to". */
  from?: SessionEngine | null;
};

/** An external agent's turn: how long it took and what it spent. Not
 *  rendered — the turn's reply shows it in its hover row. Kept in the
 *  entries (rather than a map by turn number) so it stays with its turn
 *  however provider and agent turns interleave. */
export type TurnStatsEntry = {
  kind: 'turn_stats';
  startedAt: number | null;
  endedAt: number | null;
  usage: UsageTotals | null;
  model: string | null;
  costUsd: number | null;
};

export type Entry = (
  | MsgEntry
  | ToolEntry
  | WarningEntry
  | { kind: 'activity'; activityKind: string; title: string; detail: string }
  | ErrorEntry
  | GoalEntry
  | CompactEntry
  | ThoughtEntry
  | AcpPlanEntry
  | EngineSwitchEntry
  | HtmlRenderEntry
  | TurnStatsEntry
) & { transcriptTurnIndex?: number; providerTurnIndex?: number };

export type TurnTiming = {
  startedAt: number;
  endedAt: number | null;
};

/**
 * Rebuild the transcript from a `Session::history()` snapshot (arrives in
 * the `Ready` frame on connect / reconnect / new-session / session-load).
 *
 * The wire history is a flat `[system, user, assistant, tool, assistant …]`
 * list where:
 *  - an assistant turn may carry text AND/OR `tool_calls`
 *  - a tool-role message answers exactly one `tool_call_id`
 *
 * Naively rendering every message as a bubble drops all the tool work
 * (the previous rendering had `if (role === 'tool') return null` and
 * skipped empty assistant messages too). This walk re-pairs assistant
 * calls with their tool results and emits `ToolEntry`s marked `complete`
 * so the compact tool rows show up again on reconnect.
 *
 * Caveats: `is_error` and diff previews aren't persisted, so we default
 * both to a "clean complete" render. Denials are still visible because
 * the harness writes a "denied by policy: …" content string into the tool
 * message.
 */
/** The summary text if `content` is a compaction summary message. */
export function compactionSummary(content: string | null | undefined): string | null {
  const prefix = '<conversation-summary>';
  if (!content?.startsWith(prefix)) return null;
  const end = content.indexOf('</conversation-summary>');
  const body = content.slice(prefix.length, end < 0 ? undefined : end);
  return body.replace(/^\s*This session continues[^\n]*\n+/, '').trim();
}

/** Drop the blocks appended to a prompt for the model: prompt hooks'
 *  `<hook-context>` and the turn's `<memory-context>`. Only blocks at the
 *  very end, in exactly the appended form (`\n\n<tag>\n…\n</tag>`), count:
 *  the same tag typed anywhere in the user's own text is theirs. Mirrors
 *  `strip_hook_context` in mira-harness. */
export function stripHookContext(content: string | null | undefined): string | null | undefined {
  if (!content) return content;
  let user = content;
  let peeled = false;
  for (;;) {
    const trimmed = user.trimEnd();
    const tag = ['hook-context', 'memory-context'].find((t) => {
      const close = `\n</${t}>`;
      return (
        trimmed.endsWith(close) && trimmed.slice(0, -close.length).lastIndexOf(`\n\n<${t}>\n`) >= 0
      );
    });
    if (!tag) break;
    user = trimmed.slice(0, trimmed.slice(0, -`\n</${tag}>`.length).lastIndexOf(`\n\n<${tag}>\n`));
    peeled = true;
  }
  return peeled ? user.trimEnd() : content;
}

/** Close the newest running compaction card (or add a finished one, when
 *  the start was missed: a reload mid-compaction, an older server). */
export function finishCompaction(prev: Entry[], patch: Partial<CompactionEntry>): Entry[] {
  const i = prev.map((e) => e.kind === 'compact' && e.state === 'running').lastIndexOf(true);
  const done = { endedAt: Date.now(), ...patch };
  if (i < 0) {
    return [...prev, { kind: 'compact', summarized: null, summary: null, ...done } as Entry];
  }
  const next = prev.slice();
  next[i] = { ...(prev[i] as CompactionEntry), ...done };
  return next;
}

export function engineFromRef(ref: EngineRef): SessionEngine {
  return {
    kind: ref.kind,
    driver: ref.driver ?? null,
    instance: ref.instance ?? null,
    display_name: ref.display_name,
    model: ref.model ?? null,
    status: 'ready',
  };
}

/** An engine's name for the handoff label: the agent's name, or the
 *  provider and model. */
export function engineLabel(engine: SessionEngine): string {
  if (engine.kind === 'agent') return agentDisplayName(engine.driver ?? '', engine.display_name);
  const provider =
    engine.display_name && !['Mira', 'provider'].includes(engine.display_name)
      ? engine.display_name
      : null;
  return (
    [provider, engine.model ? prettyModel(engine.model) : null].filter(Boolean).join(' · ') ||
    'Mira'
  );
}

/** A driver slug as its product name, for places with no health data. */
export function agentDisplayName(driver: string, fallback: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code',
    codex: 'Codex',
    cursor: 'Cursor',
    grok: 'Grok',
    opencode: 'OpenCode',
    antigravity: 'Antigravity',
  };
  return known[driver] ?? (fallback && fallback !== driver ? fallback : driver);
}

/** Index of the current turn's stats entry (after the last user message),
 *  or -1. */
export function turnStatsIndex(entries: Entry[]): number {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.kind === 'turn_stats') return i;
    if (e.kind === 'msg' && e.msg.role === 'user') return -1;
  }
  return -1;
}

/** Update the current turn's stats, creating them if this is the first. */
export function withTurnStats(
  entries: Entry[],
  update: (s: TurnStatsEntry) => TurnStatsEntry,
): Entry[] {
  const i = turnStatsIndex(entries);
  const blank: TurnStatsEntry = {
    kind: 'turn_stats',
    startedAt: null,
    endedAt: null,
    usage: null,
    model: null,
    costUsd: null,
  };
  const cur = i >= 0 ? (entries[i] as TurnStatsEntry) : blank;
  const next = update(cur);
  if (i >= 0) return [...entries.slice(0, i), next, ...entries.slice(i + 1)];
  // New: right after the turn's user message, never at the tail — a stats
  // report mid-stream (Codex sends them) would otherwise split the message
  // being streamed in two.
  let at = entries.length;
  for (let j = entries.length - 1; j >= 0; j--) {
    const e = entries[j];
    if (e.kind === 'msg' && e.msg.role === 'user') {
      at = j + 1;
      break;
    }
  }
  return [...entries.slice(0, at), next, ...entries.slice(at)];
}

/** Add one usage report (a delta) to the current turn. */
export function addTurnUsage(
  entries: Entry[],
  f: Extract<ServerMsg, { type: 'acp_turn_usage' }>,
): Entry[] {
  return withTurnStats(entries, (s) => {
    const u = s.usage ?? {
      prompt_tokens: 0,
      completion_tokens: 0,
      cached_input_tokens: 0,
      rounds: 0,
    };
    return {
      ...s,
      // Prompt tokens include cached input, as everywhere else.
      usage: {
        prompt_tokens: u.prompt_tokens + f.input_tokens,
        completion_tokens: u.completion_tokens + f.output_tokens,
        cached_input_tokens: u.cached_input_tokens + f.cached_input_tokens,
        rounds: u.rounds,
      },
      model: f.model || s.model,
      costUsd: f.cost_usd != null ? (s.costUsd ?? 0) + f.cost_usd : s.costUsd,
    };
  });
}

/* ---------- helpers ---------- */

/** Immutable Map update helper — replaces the entry for `parentCallId`
 *  (or seeds a fresh one) with the result of `f`. Returns a new Map so
 *  React sees the state change. */
export function updateSubagent(
  prev: Map<string, SubagentStreamState>,
  parentCallId: string,
  f: (s: SubagentStreamState) => SubagentStreamState,
): Map<string, SubagentStreamState> {
  const cur: SubagentStreamState = prev.get(parentCallId) ?? {
    parentCallId,
    entries: [],
    done: false,
    pendingReview: null,
  };
  const next = new Map(prev);
  next.set(parentCallId, f(cur));
  return next;
}

export function appendReasoning(prev: Entry[], text: string): Entry[] {
  const last = prev[prev.length - 1];
  if (last && last.kind === 'thought' && last.live) {
    return [...prev.slice(0, -1), { ...last, text: last.text + text }];
  }
  return [...prev, { kind: 'thought', text, live: true, startedAt: Date.now(), endedAt: null }];
}

/** A turn is over: nothing in it can still be running or awaiting an
 *  answer. A crashed agent or an aborted turn otherwise leaves an approval
 *  card nobody can answer (the server no longer knows the call) and
 *  spinners that never stop. Subagent calls are left alone — their state is
 *  tracked separately and can outlive the turn that started them. */
export function settleTools(prev: Entry[]): Entry[] {
  let changed = false;
  const next = prev.map((e) => {
    if (e.kind !== 'tool' || e.call.function.name === 'agent') return e;
    if (e.status === 'pending') {
      changed = true;
      return {
        ...e,
        status: 'denied' as const,
        result: e.result ?? {
          call_id: e.call.id,
          content: 'Not run — the turn ended before it was answered.',
          is_error: true,
        },
      };
    }
    if (e.status === 'running') {
      changed = true;
      return {
        ...e,
        status: 'complete' as const,
        result: e.result ?? {
          call_id: e.call.id,
          content: 'Interrupted — the turn ended before this finished.',
          is_error: true,
        },
      };
    }
    return e;
  });
  return changed ? next : prev;
}

/** Close the live thought block, if any — the model moved on. */
export function sealThought(prev: Entry[]): Entry[] {
  const last = prev[prev.length - 1];
  if (!last || last.kind !== 'thought' || !last.live) return prev;
  return [...prev.slice(0, -1), { ...last, live: false, endedAt: Date.now() }];
}

export function appendToken(prevRaw: Entry[], text: string): Entry[] {
  const prev = sealThought(prevRaw);
  const last = prev[prev.length - 1];
  if (last && last.kind === 'msg' && last.msg.role === 'assistant') {
    const updated: Entry = {
      kind: 'msg',
      msg: { ...last.msg, content: (last.msg.content ?? '') + text },
    };
    return [...prev.slice(0, -1), updated];
  }
  return [
    ...prev,
    { kind: 'msg', msg: { role: 'assistant', content: text, created_at: Date.now() } },
  ];
}

/** An agent's reply text. It is the turn's answer like Mira's own, so it
 *  becomes the same assistant message — same rendering, same copy action,
 *  same place in a collapsed turn. */
export type NativeFrame = Extract<
  ServerMsg,
  {
    type:
      | 'acp_text'
      | 'acp_text_snapshot'
      | 'acp_thought'
      | 'acp_tool_call'
      | 'acp_tool_call_update'
      | 'acp_tool_output_delta';
  }
>;

export function applyNativeFrame(entries: Entry[], frame: NativeFrame): Entry[] {
  switch (frame.type) {
    case 'acp_text':
      return appendAcpText(entries, frame.text, frame.message_id);
    case 'acp_text_snapshot':
      return appendNativeText(sealThought(entries), frame.text, frame.message_id, true);
    case 'acp_thought':
      return appendAcpThought(entries, frame.text);
    case 'acp_tool_call':
    case 'acp_tool_call_update':
      return upsertAcpTool(entries, frame.call);
    case 'acp_tool_output_delta':
      return appendNativeToolOutput(entries, frame.id, frame.text);
  }
}

export function appendAcpText(prev: Entry[], text: string, messageId?: string | null): Entry[] {
  if (!text) return prev;
  if (!messageId) return appendToken(prev, text);
  return appendNativeText(sealThought(prev), text, messageId);
}

/** An agent's thinking, as Mira's own thought block. */
export function appendAcpThought(prev: Entry[], text: string): Entry[] {
  if (!text) return prev;
  const next = appendReasoning(prev, text);
  return next.map((e, i) =>
    i === next.length - 1 && e.kind === 'thought' ? { ...e, native: true } : e,
  );
}

/**
 * Whether a stop reason means the turn actually produced an answer.
 *
 * Deliberately a small allowlist rather than a blocklist: an unfamiliar
 * reason is far more likely to be a failure or a limit than a clean finish,
 * and saying so beats claiming success.
 */
export function isSuccessfulAcpStop(stop: string): boolean {
  return ['end_turn', 'success', 'completed', 'stop_sequence'].includes(stop);
}

/** A human explanation of why a turn stopped, for the transcript. */
export function describeAcpStop(stop: string): string {
  if (stop === 'rate_limited') {
    return 'The agent hit a usage limit, so this turn failed without an answer.';
  }
  if (stop === 'cancelled' || stop === 'canceled') {
    return 'This turn was cancelled.';
  }
  if (stop === 'max_tokens' || stop === 'max_turns') {
    return `The agent stopped early (${stop}).`;
  }
  return `The agent stopped: ${stop}.`;
}

export function sealAcpThought(prev: Entry[]): Entry[] {
  return sealThought(prev);
}

/** Questions and plans render as Mira's own cards (the server wraps them),
 *  so the agent's raw call for them would be a duplicate. */
export const AGENT_PROMPT_TOOLS = new Set(['AskUserQuestion', 'ExitPlanMode']);

/** Insert or update an agent's tool call, as a Mira tool entry.
 *
 *  An update carries only what changed (no name, an empty title), so it
 *  merges onto the call already recorded — empty fields never blank what
 *  is there — and the Mira view is rebuilt from the merged call. */
export function upsertAcpTool(prevRaw: Entry[], call: AcpToolCall): Entry[] {
  call = boundAgentOutput(call);
  const prev = sealThought(prevRaw);
  const idx = prev.findIndex((e) => e.kind === 'tool' && e.agentCall?.id === call.id);
  let merged: AcpToolCall = call;
  if (idx >= 0) {
    const existing = prev[idx];
    if (existing.kind !== 'tool' || !existing.agentCall) return prev;
    merged = { ...existing.agentCall };
    for (const [k, v] of Object.entries(call) as [keyof AcpToolCall, unknown][]) {
      if (v === null || v === undefined || v === '') continue;
      if (Array.isArray(v) && v.length === 0) continue;
      (merged as Record<string, unknown>)[k] = v;
    }
  }
  if (AGENT_PROMPT_TOOLS.has(merged.name ?? ''))
    return idx >= 0 ? prev.filter((_, i) => i !== idx) : prev;
  const prior = idx >= 0 ? (prev[idx] as ToolEntry) : null;
  const entry: Entry = {
    kind: 'tool',
    call: agentCallToToolCall(merged),
    preview: null,
    status: agentToolStatus(merged),
    result: (() => {
      const result = agentToolResult(merged);
      return result
        ? { ...result, content: boundedOutput('', result.content || prior?.result?.content || '') }
        : (prior?.result ?? null);
    })(),
    agentCall: merged,
    activityAt: ++toolActivitySequence,
    // Updates rebuild the entry from the agent's merged call; that call
    // carries no Mira-side state, so accumulated fields would be lost on
    // every progress frame unless carried across here.
    startedAt: prior?.startedAt ?? Date.now(),
    delegateSteps: prior?.delegateSteps,
  };
  if (idx >= 0) return [...prev.slice(0, idx), entry, ...prev.slice(idx + 1)];
  return [...prev, entry];
}

// A tool_start arrives after either (a) a user-approved approval_request, in
// which case an entry already exists — leave it alone, or (b) an auto-approved
// call the policy let through with no prompt — add a fresh entry.
export function upsertToolStart(prevRaw: Entry[], call: ToolCall): Entry[] {
  const prev = sealThought(prevRaw);
  const existing = prev.findIndex((e) => e.kind === 'tool' && e.call.id === call.id);
  if (existing >= 0)
    return updateTool(prev, call.id, (entry) => ({
      ...entry,
      status: 'running',
      startedAt: entry.startedAt ?? Date.now(),
    }));
  return [
    ...prev,
    {
      kind: 'tool',
      call,
      preview: null,
      status: 'running',
      result: null,
      startedAt: Date.now(),
      activityAt: ++toolActivitySequence,
    },
  ];
}

export function attachToolResult(prev: Entry[], result: ToolResult): Entry[] {
  return updateTool(prev, result.call_id, (t) => ({
    ...t,
    result,
    // Preserve "denied" state; otherwise mark done.
    status: t.status === 'denied' ? 'denied' : 'complete',
  }));
}

/**
 * Merge a task_* tool result into the live task list.
 *
 * Contract: task_create / task_update / task_get emit
 * `{ task: FullItem }`; task_list emits `{ tasks: FullItem[] }`. Any
 * other tool (or an error result) → no-op.
 */
export function applyTaskResult(prev: TaskItem[], result: ToolResult): TaskItem[] {
  if (result.is_error || !result.data || typeof result.data !== 'object') return prev;
  const d = result.data as { task?: TaskItem; tasks?: TaskItem[] };
  if (Array.isArray(d.tasks)) return d.tasks;
  if (d.task && typeof d.task.id === 'number') {
    const idx = prev.findIndex((t) => t.id === d.task!.id);
    if (idx === -1) return [...prev, d.task];
    return [...prev.slice(0, idx), d.task, ...prev.slice(idx + 1)];
  }
  return prev;
}

export function updateTool(prev: Entry[], callId: string, f: (t: ToolEntry) => ToolEntry): Entry[] {
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && e.call.id === callId) {
      return [
        ...prev.slice(0, i),
        { ...f(e), activityAt: ++toolActivitySequence },
        ...prev.slice(i + 1),
      ];
    }
  }
  return prev;
}

export function appendProgressLine(prev: Entry[], callId: string, line: string): Entry[] {
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && e.call.id === callId) {
      const updated: ToolEntry = {
        ...e,
        progressLines: [...(e.progressLines ?? []), line],
        activityAt: ++toolActivitySequence,
      };
      return [...prev.slice(0, i), updated, ...prev.slice(i + 1)];
    }
  }
  return prev;
}

/** Attach a delegated child's step to its card. Prefers an exact call-id
 *  match; when none is found (a provider that doesn't echo the spawning id)
 *  falls back to the most recent still-running `delegate_task`, so live
 *  activity is never dropped on the floor. */
export function appendDelegateStep(
  prev: Entry[],
  callId: string,
  kind: string,
  text: string,
): Entry[] {
  const isDelegateRunning = (e: Entry) =>
    e.kind === 'tool' &&
    isDelegateTaskName(e.call.function.name) &&
    (e.status === 'running' || e.status === 'pending');
  let idx = -1;
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && isDelegateTaskName(e.call.function.name) && e.call.id === callId) {
      idx = i;
      break;
    }
  }
  if (idx === -1) {
    for (let i = prev.length - 1; i >= 0; i--) {
      if (isDelegateRunning(prev[i])) {
        idx = i;
        break;
      }
    }
  }
  if (idx === -1) return prev;
  const e = prev[idx] as ToolEntry;
  const step: DelegateStep = { kind, text, at: Date.now() };
  const updated: ToolEntry = { ...e, delegateSteps: [...(e.delegateSteps ?? []), step] };
  return [...prev.slice(0, idx), updated, ...prev.slice(idx + 1)];
}

export function attachPlanProposal(prev: Entry[], callId: string, proposal: PlanProposal): Entry[] {
  return updateTool(prev, callId, (t) => ({
    ...t,
    plan: { proposal, decision: null },
  }));
}

export function recordPlanDecision(
  prev: Entry[],
  callId: string,
  decision: { approved: boolean; steps?: PlanStep[]; note?: string },
): Entry[] {
  return updateTool(prev, callId, (t) => {
    if (!t.plan) return t;
    return { ...t, plan: { ...t.plan, decision } };
  });
}

export function attachAskUserProposal(
  prev: Entry[],
  callId: string,
  proposal: AskUserProposal,
): Entry[] {
  return updateTool(prev, callId, (t) => ({
    ...t,
    askUser: { proposal, decision: null },
  }));
}

export function recordAskUserDecision(
  prev: Entry[],
  callId: string,
  decision: AskUserDecision,
): Entry[] {
  return updateTool(prev, callId, (t) => {
    if (!t.askUser) return t;
    return { ...t, askUser: { ...t.askUser, decision } };
  });
}

export function countUserMessages(entries: Entry[]): number {
  let n = 0;
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user' && e.msg.input_intent !== 'steer') n++;
  }
  return n;
}

export function rebuildTurnTimings(
  serverTurns: { started_at: number; ended_at?: number | null }[],
): Map<number, TurnTiming> {
  const out = new Map<number, TurnTiming>();
  serverTurns.forEach((t, i) => {
    out.set(i, {
      startedAt: t.started_at,
      endedAt: t.ended_at ?? null,
    });
  });
  return out;
}

/** Per-turn usage from the server's turn records, skipping turns that
 *  have none (recorded before per-turn usage existed). */
export function rebuildTurnUsage(serverTurns: TurnMeta[]): Map<number, UsageTotals> {
  const out = new Map<number, UsageTotals>();
  serverTurns.forEach((t, i) => {
    const u = t.usage;
    if (u && u.prompt_tokens + u.completion_tokens > 0) out.set(i, u);
  });
  return out;
}

export function rebuildTurnModels(serverTurns: TurnMeta[]): Map<number, string> {
  const out = new Map<number, string>();
  serverTurns.forEach((t, i) => {
    if (t.model) out.set(i, t.model);
  });
  return out;
}

export function stampLastTurn(
  prev: Map<number, TurnTiming>,
  endedAt: number,
): Map<number, TurnTiming> {
  // The `done` frame closes out the most recently started turn — find the
  // highest turn index that's still marked "in flight" and stamp it.
  let target = -1;
  for (const [idx, t] of prev) {
    if (t.endedAt == null && idx > target) target = idx;
  }
  if (target < 0) return prev;
  const clone = new Map(prev);
  const t = clone.get(target)!;
  clone.set(target, { ...t, endedAt });
  return clone;
}

/** Pull the most recent "here's what mira is doing" line off the end
 *  of the entry list. Priority: newest running tool call > newest
 *  tool result > latest assistant fragment head. Empty string when
 *  there's nothing recognizable to show — the GoalPanel renders a
 *  generic "Working" in that case. */
export function goalActivity(entries: Entry[]): string {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.kind === 'tool' && e.status === 'running') {
      return `Running ${e.call.function.name}`;
    }
    if (e.kind === 'tool' && e.status === 'complete' && !e.result?.is_error) {
      return `Ran ${e.call.function.name}`;
    }
    if (
      e.kind === 'msg' &&
      e.msg.role === 'assistant' &&
      e.nativePhase !== 'commentary' &&
      (e.msg.content ?? '').trim()
    ) {
      const line = (e.msg.content ?? '').trim().split('\n')[0];
      return line.length > 90 ? line.slice(0, 90) + '…' : line;
    }
  }
  return '';
}

export function titleFromEntries(entries: Entry[]): string {
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user' && e.msg.content?.trim()) {
      // Strip the `## Attached files …` header (same sanitization the
      // sidebar's `sessionLabel` uses). Prefer the actual user prose;
      // fall back to a filename summary when the turn was attachment-only.
      const { attachments, text } = parseSentAttachments(e.msg.content);
      const clean = text.trim();
      if (clean) {
        const first = clean.split('\n')[0];
        return first.length > 60 ? first.slice(0, 60) + '…' : first;
      }
      if (attachments.length > 0) {
        const filename = attachments[0].filename;
        const more = attachments.length - 1;
        return more > 0 ? `${filename} + ${more} more` : filename;
      }
    }
  }
  return 'New chat';
}

/* ---------- turn grouping ---------- */

/* ---------- turn renderer ---------- */

/** Same turn content: the same entries, by identity. Entries are replaced,
 *  never mutated, so an untouched turn compares equal even though
 *  `groupByTurn` builds a fresh `Turn` object on every update. */
export function sameTurn(a: Turn, b: Turn): boolean {
  return (
    a.user === b.user && a.body.length === b.body.length && a.body.every((e, i) => e === b.body[i])
  );
}

export function upsertRuntimeQuestion(
  prev: Entry[],
  record: Extract<ServerMsg, { type: 'runtime_request_updated' }>['request'],
): Entry[] {
  const proposal: AskUserProposal = {
    questions: record.request.questions.map((q) => ({
      question: q.question,
      header: q.header,
      multi_select: false,
      options: q.options.map((label) => ({ label, recommended: false })),
    })),
  };
  const decision: AskUserDecision | null = record.response
    ? record.response.cancelled
      ? { cancelled: true }
      : { cancelled: false, answers: record.response.answers }
    : null;
  const index = prev.findIndex((e) => e.kind === 'tool' && e.call.id === record.id);
  const entry: ToolEntry = {
    kind: 'tool',
    call: {
      id: record.id,
      type: 'function',
      function: { name: 'ask_user', arguments: JSON.stringify(proposal) },
    },
    preview: null,
    status: 'complete',
    result: null,
    runtimeDelivery: record.delivery,
    askUser: { proposal, decision },
  };
  if (index < 0) return [...prev, entry];
  return [...prev.slice(0, index), entry, ...prev.slice(index + 1)];
}
