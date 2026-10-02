import { createContext, memo, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence } from 'framer-motion';
import {
  RotateCw,
  ArrowDown,
  ChevronDown,
  Check,
  LoaderCircle,
  Copy,
  Info,
  Lightbulb,
  Pencil,
  Palette,
  Sun,
  Moon,
  ShieldAlert,
  PanelLeft,
  Sparkle,
  Target,
  History,
  X,
  Download,
} from 'lucide-react';
import { cn } from './lib/utils';
import { acpOptionsToDescriptors } from './lib/acpOptions';
import { loadAgentCaps, loadInstanceConfigs, parseArgs, parseEnv, saveAgentCaps } from './lib/acpAgents';
import { EngineMark } from './components/EnginePicker';
import { prettyModel } from './lib/models';
import type { UsageRingData } from './components/UsageRing';
import { isAgentRequest } from './lib/agentRequest';
import { agentCallToToolCall, agentToolResult, agentToolStatus } from './lib/agentTools';
import type { AcpInstanceConfig } from './components/settings/AcpAgentsSection';
import {
  getCustomKeybindingRules,
  resolveShortcutCommand,
  shortcutLabelForCommand,
  useKeybindings,
  type ShortcutMatchContext,
} from './lib/keybindings';
import { applyReduceMotion, getBoolPref, PREF_KEYS } from './lib/prefs';
import { connect, type WsClient, type WsStatus } from './ws';
import { costUsd, formatDollars, shortNum } from './lib/usage';
import { getContextBreakdown, previewCheckpoint, restoreCheckpoint, undoRestore, type MessageRef, type RestoreChange, type Restored } from './api';
import { appendMemory, applyUndo, getBranchPr, getGitStatus, getSessionDiff, getSessionHistory, getSettings, gitCommit, gitPush, listCommands, listEngines, listSessions, listSkills, newSession, setSessionBackgroundMode, startReview, type BranchPrView, type EngineSnapshot, type GitStatusView, type SessionDiffView, type SkillView, type CommandInfo } from './api';
import {
  ContextPanel,
  CONTEXT_PANEL_RESERVE,
  contextPanelHasContent,
  useContextPanelFits,
} from './components/ContextPanel';
import { extractAgentId } from './components/AgentCard';
import { SettingsSurface } from './components/Settings';
import { PluginsPanel } from './components/Plugins';
import { PullRequestPanel } from './components/PullRequestPanel';
import { Sidebar, type MainView } from './components/Sidebar';
import { hasHiddenTitleBar } from './lib/desktop';
import { RightPanelButton } from './components/RightPanelButton';
import { attachFilesToComposer, dataUrlToFile } from './lib/attachBridge';
import { FilePicker } from './components/FilePicker';
import {
  TOOL_PANE_DEFS,
  isToolPaneId,
  toolPaneId,
  toolPaneKindOf,
  type ToolPaneKind,
  type ToolPaneTab,
} from './components/panes/toolPanes';
import {
  Composer,
  parseSentAttachments,
  SentAttachmentChip,
  type PendingApproval,
} from './components/Composer';
import { SkillMentionText } from './components/SkillMention';
import { FolderPicker } from './components/FolderPicker';
import { AssistantContent } from './components/AssistantContent';
import { ThoughtBlock } from './components/ThoughtBlock';
import { ReviewChanges } from './components/ReviewChanges';
import { EditorPicker } from './components/EditorPicker';
import { TimelineMinimap, type MinimapItem } from './components/TimelineMinimap';
import { ImageLightbox } from './components/ImageLightbox';
import { TerminalPanel } from './components/TerminalPanel';
import * as agentTerminal from './lib/agentTerminal';
import {
  Bot,
  Brain,
  ChartColumn,
  Cog,
  FileDiff,
  FolderOpen,
  Globe2,
  Keyboard,
  MessageSquarePlus,
  PanelLeftClose,
  Plug,
  ScanSearch,
  Smile,
  SquareTerminal,
  Cpu,
  Sparkles,
  Zap,
} from 'lucide-react';
import { CommandPalette, type PaletteAction } from './components/CommandPalette';
import { splitFileRef } from './lib/refs';
import { ContextInspector } from './components/ContextInspector';
import { ImportChats } from './components/ImportChats';
import { UpdateButton } from './components/UpdateButton';
import { playTurnSound } from './lib/sound';
import { resolveTheme, setThemePref } from './lib/theme';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { callForAttention } from './lib/attention';
import { GetStarted } from './components/onboarding/GetStarted';

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
import { ToolCard, type ToolStatus } from './components/ToolCard';
import { Thinking } from './components/Thinking';
import {
  applyReviewEvent,
  emptyReviewState,
  ReviewPanel,
  type ReviewState,
} from './components/ReviewPanel';
import { PlanCard } from './components/PlanCard';
import { AskUserCard, type AskUserDecision } from './components/AskUserCard';
import { ApprovalDialog } from './components/ApprovalDialog';
import {
  mapPosturesToModes,
  MIRA_MODE_TO_POSTURE,
  POSTURES,
} from './lib/agentPostures';
import { AgentCard, AgentGroup } from './components/AgentCard';
import miraLogo from './assets/mira-logo.png';
import { SubagentPanel, type SubagentTab, type FilePanelTab } from './components/SubagentPanel';
import { TaskListPanel } from './components/TaskListPanel';
import { GoalPanel } from './components/GoalPanel';
import { categoryFor, countsByCategory, countsPhrase, ToolGroup } from './components/ToolGroup';
import { SecondOpinion } from './components/SecondOpinion';
import type {
  EnvironmentInfo,
  EnvironmentStatus,
  ApprovalScope,
  AskUserProposal,
  DiffPreview,
  Goal,
  Message,
  Mode,
  PlanProposal,
  PlanStep,
  ServerMsg,
  SettingsView,
  TaskItem,
  ToolCall,
  ToolResult,
  UsageTotals,
  SessionSummary,
  RateLimitReading,
  AcpAgentStatus,
  AcpConfigOption,
  AgentTranscriptLine,
  AcpPlanItem,
  AcpSessionMode,
  AcpToolCall,
  AgentPostureMapping,
  SessionEngine,
  TurnMeta,
} from './types';

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

type ToolEntry = {
  kind: 'tool';
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
  /** Set when an external agent made this call: its own description of it,
   *  kept so later updates merge onto it. `call` is its Mira translation,
   *  which is what renders. */
  agentCall?: AcpToolCall;
};
type WarningEntry = { kind: 'warning'; text: string };
/** Where compaction summarized the conversation: a divider, with the
 *  summary behind a toggle when it's known (after a reload). */
type CompactEntry = { kind: 'compact'; summarized: number | null; summary: string | null };
type ErrorEntry = { kind: 'error'; text: string };
type MsgEntry = { kind: 'msg'; msg: Message };
/** The model's reasoning before a reply / tool call. `live` while
 *  `reasoning` frames are still arriving; sealed by the next token,
 *  tool call or turn end. Times are epoch ms, null when restored from
 *  history (duration unknown). */
type ThoughtEntry = {
  kind: 'thought';
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
type GoalEntry = {
  kind: 'goal';
  variant: 'set' | 'cleared' | 'progress' | 'done';
  iteration: number | null;
  maxIterations: number | null;
  status: import('./types').GoalStatus | null;
  reason: string | null;
  /** Only set on `variant: 'set'`. */
  condition?: string | null;
};
/** The agent's plan. */
type AcpPlanEntry = { kind: 'acp_plan'; entries: AcpPlanItem[] };
/** Where the chat moved to another engine (provider ⇄ agent). A quiet
 *  divider, so a reader can tell which engine wrote what. */
type EngineSwitchEntry = { kind: 'engine_switch'; engine: SessionEngine };
/** An external agent's turn: how long it took and what it spent. Not
 *  rendered — the turn's reply shows it in its hover row. Kept in the
 *  entries (rather than a map by turn number) so it stays with its turn
 *  however provider and agent turns interleave. */
type TurnStatsEntry = {
  kind: 'turn_stats';
  startedAt: number | null;
  endedAt: number | null;
  usage: UsageTotals | null;
  model: string | null;
  costUsd: number | null;
};

export type Entry =
  | MsgEntry
  | ToolEntry
  | WarningEntry
  | ErrorEntry
  | GoalEntry
  | CompactEntry
  | ThoughtEntry
  | AcpPlanEntry
  | EngineSwitchEntry
  | TurnStatsEntry;

type TurnTiming = {
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
function compactionSummary(content: string | null | undefined): string | null {
  const prefix = '<conversation-summary>';
  if (!content?.startsWith(prefix)) return null;
  const end = content.indexOf('</conversation-summary>');
  const body = content.slice(prefix.length, end < 0 ? undefined : end);
  return body.replace(/^\s*This session continues[^\n]*\n+/, '').trim();
}

/** Drop the `<hook-context>` block prompt hooks append to a message. */
function stripHookContext(content: string | null | undefined): string | null | undefined {
  const i = content?.indexOf('<hook-context>') ?? -1;
  return i >= 0 ? content!.slice(0, i).trimEnd() : content;
}

/**
 * Rebuild transcript entries from an agent sidecar.
 *
 * Pure fold over the persisted lines using the same entry builders as live
 * traffic (`appendAcpText`, `upsertAcpTool`, …), so replayed turns render
 * exactly like live ones. Deliberately free of side effects: no busy flags,
 * no pings, no git refreshes, no turn-timing stamps — replay must not
 * disturb a session that may have a live agent running right now.
 *
 * State frames (modes, config, commands, usage) are NOT applied here; the
 * caller feeds those through the live handler, which owns the picker state.
 * Only transcript content is returned.
 */
/**
 * Rebuild a reloaded session's transcript with provider and agent turns in
 * the order they happened.
 *
 * The two histories are stored apart (the harness must not read agent
 * words as its own), so the server writes a `switch` marker into the agent
 * sidecar at every engine switch, recording how much harness history
 * preceded it. Between two markers, agent turns come first and provider
 * turns after — a switch to the provider is followed by provider turns,
 * and a switch to the agent by agent turns. Sidecars written before the
 * markers existed keep the old order: harness, then agent.
 */
export function interleaveReplay(
  history: Message[],
  previews: Record<string, DiffPreview> | undefined,
  lines: AgentTranscriptLine[],
): Entry[] {
  if (!lines.some((l) => l.switch)) {
    return [...historyToEntries(history, previews), ...replayAgentTranscript(lines)];
  }
  const out: Entry[] = [];
  let from = 0;
  let segment: AgentTranscriptLine[] = [];
  for (const line of lines) {
    if (!line.switch) {
      segment.push(line);
      continue;
    }
    out.push(...replayAgentTranscript(segment));
    segment = [];
    const to = Math.min(Math.max(line.switch.harness_len, from), history.length);
    out.push(...historyToEntries(history.slice(from, to), previews));
    from = to;
    const toAgent = line.switch.to !== 'provider';
    out.push({
      kind: 'engine_switch',
      engine: toAgent
        ? { kind: 'agent', driver: line.driver, display_name: line.driver, status: 'ready' }
        : { kind: 'provider', display_name: 'provider', status: 'ready' },
    });
  }
  out.push(...replayAgentTranscript(segment));
  out.push(...historyToEntries(history.slice(from), previews));
  return out;
}

/** The divider an engine switch leaves in the transcript. */
function EngineSwitchDivider({ engine }: { engine: SessionEngine }) {
  const provider = engine.display_name && !['Mira', 'provider'].includes(engine.display_name)
    ? engine.display_name
    : null;
  const name =
    engine.kind === 'agent'
      ? agentDisplayName(engine.driver ?? '', engine.display_name)
      : [provider, engine.model ? prettyModel(engine.model) : null].filter(Boolean).join(' · ') ||
        'your provider';
  return (
    <div className="my-1 flex items-center gap-3 text-[11.5px] text-muted-foreground/60" role="separator">
      <span className="h-px flex-1 bg-border/50" />
      <span className="inline-flex items-center gap-1.5">
        <EngineMark engine={engine} model={engine.model} />
        {engine.kind === 'agent' ? `Switched to ${name}` : `Back on ${name}`}
      </span>
      <span className="h-px flex-1 bg-border/50" />
    </div>
  );
}

/** A driver slug as its product name, for places with no health data. */
function agentDisplayName(driver: string, fallback: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code', codex: 'Codex', cursor: 'Cursor', grok: 'Grok',
    opencode: 'OpenCode', antigravity: 'Antigravity',
  };
  return known[driver] ?? (fallback && fallback !== driver ? fallback : driver);
}

/** Index of the current turn's stats entry (after the last user message),
 *  or -1. */
function turnStatsIndex(entries: Entry[]): number {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.kind === 'turn_stats') return i;
    if (e.kind === 'msg' && e.msg.role === 'user') return -1;
  }
  return -1;
}

/** Update the current turn's stats, creating them if this is the first. */
function withTurnStats(entries: Entry[], update: (s: TurnStatsEntry) => TurnStatsEntry): Entry[] {
  const i = turnStatsIndex(entries);
  const blank: TurnStatsEntry = { kind: 'turn_stats', startedAt: null, endedAt: null, usage: null, model: null, costUsd: null };
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
function addTurnUsage(entries: Entry[], f: Extract<ServerMsg, { type: 'acp_turn_usage' }>): Entry[] {
  return withTurnStats(entries, (s) => {
    const u = s.usage ?? { prompt_tokens: 0, completion_tokens: 0, cached_input_tokens: 0, rounds: 0 };
    return {
      ...s,
      // Prompt tokens include cached input, as everywhere else.
      usage: {
        prompt_tokens: u.prompt_tokens + f.input_tokens + f.cached_input_tokens,
        completion_tokens: u.completion_tokens + f.output_tokens,
        cached_input_tokens: u.cached_input_tokens + f.cached_input_tokens,
        rounds: u.rounds,
      },
      model: f.model || s.model,
      costUsd: f.cost_usd != null ? (s.costUsd ?? 0) + f.cost_usd : s.costUsd,
    };
  });
}

export function replayAgentTranscript(lines: AgentTranscriptLine[]): Entry[] {
  let out: Entry[] = [];
  let turnStartedAt: number | null = null;
  for (const line of lines) {
    if (line.user) {
      out = [
        ...out,
        { kind: 'msg', msg: { role: 'user', content: line.user.text } },
      ];
      turnStartedAt = line.t;
      continue;
    }
    const f = line.frame as ServerMsg | null | undefined;
    if (!f || typeof f !== 'object' || !('type' in f)) continue;
    switch ((f as ServerMsg).type) {
      case 'acp_text':
        out = appendAcpText(out, (f as Extract<ServerMsg, { type: 'acp_text' }>).text);
        break;
      case 'acp_thought':
        out = appendAcpThought(out, (f as Extract<ServerMsg, { type: 'acp_thought' }>).text);
        break;
      case 'acp_tool_call':
      case 'acp_tool_call_update':
        out = upsertAcpTool(
          out,
          (f as Extract<ServerMsg, { type: 'acp_tool_call' }>).call,
        );
        break;
      case 'acp_plan': {
        const sealed = sealAcpThought(out);
        const msg = f as Extract<ServerMsg, { type: 'acp_plan' }>;
        const entry: AcpPlanEntry = { kind: 'acp_plan', entries: msg.entries };
        const idx = sealed.findIndex((e) => e.kind === 'acp_plan');
        out =
          idx >= 0
            ? [...sealed.slice(0, idx), entry, ...sealed.slice(idx + 1)]
            : [...sealed, entry];
        break;
      }
      case 'acp_turn_usage':
        out = addTurnUsage(out, f as Extract<ServerMsg, { type: 'acp_turn_usage' }>);
        break;
      case 'acp_turn_end': {
        const msg = f as Extract<ServerMsg, { type: 'acp_turn_end' }>;
        const started = turnStartedAt;
        out = withTurnStats(out, (s) => ({ ...s, startedAt: s.startedAt ?? started, endedAt: line.t }));
        const sealed = sealAcpThought(out);
        if (isSuccessfulAcpStop(msg.stop_reason)) {
          out = sealed;
          break;
        }
        const detail = msg.detail ? ` ${msg.detail}` : '';
        out = [
          ...sealed,
          { kind: 'error', text: `${describeAcpStop(msg.stop_reason)}${detail}` },
        ];
        break;
      }
      case 'acp_mode_changed': {
        const msg = f as Extract<ServerMsg, { type: 'acp_mode_changed' }>;
        out = [
          ...out,
          {
            kind: 'warning',
            text: msg.privileged
              ? `${msg.display_name}: ${msg.mode_name} enabled — ${msg.mode_id} grants more access than Mira would`
              : `${msg.display_name}: mode set to ${msg.mode_name}`,
          },
        ];
        break;
      }
      case 'acp_unmodelled':
        // Diagnostics, not conversation — see the live handler.
        break;
      case 'warning':
        out = [...out, { kind: 'warning', text: (f as Extract<ServerMsg, { type: 'warning' }>).text }];
        break;
      case 'error':
        out = [...out, { kind: 'error', text: (f as Extract<ServerMsg, { type: 'error' }>).text }];
        break;
      default:
        // State frames and session ephemera are handled by the caller, or
        // deliberately skipped — never rendered as transcript content.
        break;
    }
  }
  // Replayed thoughts have no real timing: rebuilding them stamped "now"
  // on both ends, which read as "Thought for <1s". Unknown is shown as
  // unknown, the way Mira's own restored history does.
  out = out.map((e) => (e.kind === 'thought' ? { ...e, live: false, startedAt: null, endedAt: null } : e));
  return out;
}

export function historyToEntries(
  history: Message[],
  /** Persisted diff previews from `SessionRecord.previews` (Ready
   *  frame). Attaches per call id so a reloaded transcript shows the
   *  same diff the user saw live, instead of dropping to the arg-only
   *  reconstruction fallback. */
  previews?: Record<string, DiffPreview>,
): Entry[] {
  // First pass — index tool results by call_id so the assistant walk can
  // attach them in O(1) rather than re-scanning history for each call.
  const resultByCallId = new Map<string, ToolResult>();
  for (const m of history) {
    if (m.role === 'tool' && m.tool_call_id) {
      resultByCallId.set(String(m.tool_call_id), {
        call_id: String(m.tool_call_id),
        content: m.content ?? '',
        is_error: false,
      });
    }
  }

  const entries: Entry[] = [];
  for (const m of history) {
    if (m.role === 'system' || m.role === 'tool') continue;

    if (m.role === 'user') {
      const summary = compactionSummary(m.content);
      if (summary != null) {
        entries.push({ kind: 'compact', summarized: null, summary });
      } else {
        // Prompt hooks append a `<hook-context>` block for the model; show
        // only what the user typed.
        entries.push({ kind: 'msg', msg: { ...m, content: stripHookContext(m.content) } });
      }
      continue;
    }

    // Assistant: thinking first, then any text, then a ToolEntry per
    // tool_call — the live turn order (`reasoning…`, `token…`,
    // `tool_start`) so a resumed transcript reads identically.
    const thought = (m.reasoning ?? [])
      .map((b) => (b.text ?? '').trim())
      .filter(Boolean)
      .join('\n\n');
    if (thought) {
      entries.push({ kind: 'thought', text: thought, live: false, startedAt: null, endedAt: null });
    }
    if ((m.content ?? '').trim()) {
      entries.push({ kind: 'msg', msg: m });
    }
    for (const call of m.tool_calls ?? []) {
      const result = resultByCallId.get(call.id) ?? null;
      const entry: ToolEntry = {
        kind: 'tool',
        call,
        preview: previews?.[call.id] ?? null,
        status: 'complete',
        result,
      };
      // On reload the `ask_user_request` / `plan_request` live frames
      // don't fire, so rebuild the interactive-card state directly from
      // the persisted call args (the proposal) + tool result text (the
      // resolved decision). Without this, completed interactive tools
      // render as raw JSON args.
      if (call.function.name === 'ask_user') {
        const restored = restoreAskUserFromCall(call, result);
        if (restored) entry.askUser = restored;
      } else if (call.function.name === 'plan') {
        const restored = restorePlanFromCall(call, result);
        if (restored) entry.plan = restored;
      }
      entries.push(entry);
    }
  }
  return entries;
}

/** Reconstruct the ask_user proposal + decision from persisted tool
 *  state. `call.function.arguments` is the JSON we sent to the tool
 *  (i.e. the AskUserProposal); `result.content` is the textual summary
 *  the tool wrote back — parseable because we own both sides of that
 *  format (see `AskUserTool::invoke` in `interactive.rs`). */
function restoreAskUserFromCall(
  call: ToolCall,
  result: ToolResult | null,
): { proposal: AskUserProposal; decision: AskUserDecision } | undefined {
  let proposal: AskUserProposal;
  try {
    const args = JSON.parse(call.function.arguments) as { questions?: unknown };
    if (!Array.isArray(args.questions)) return undefined;
    proposal = { questions: args.questions as AskUserProposal['questions'] };
  } catch {
    return undefined;
  }
  const decision = parseAskUserResultText(result?.content ?? '', proposal.questions.length);
  return { proposal, decision };
}

/** Reconstruct the plan proposal + decision from persisted tool state.
 *  Same shape as `restoreAskUserFromCall`: the args carry the proposal,
 *  the result text carries the verdict + edited steps. */
function restorePlanFromCall(
  call: ToolCall,
  result: ToolResult | null,
): { proposal: PlanProposal; decision: null | { approved: boolean; steps?: PlanStep[]; note?: string } } | undefined {
  let proposal: PlanProposal;
  try {
    const args = JSON.parse(call.function.arguments) as { title?: unknown; steps?: unknown };
    if (typeof args.title !== 'string' || !Array.isArray(args.steps)) return undefined;
    proposal = { title: args.title, steps: args.steps as PlanStep[] };
  } catch {
    return undefined;
  }
  const decision = parsePlanResultText(result?.content ?? '');
  return { proposal, decision };
}

/** Parse the plan tool's result body — see `PlanTool::invoke` in
 *  `interactive.rs` for the exact strings emitted. Missing result →
 *  render as no-decision-yet so the card stays actionable. */
function parsePlanResultText(text: string):
  | null
  | { approved: boolean; steps?: PlanStep[]; note?: string } {
  if (!text.trim()) return null;
  if (/^Plan cancelled by user/i.test(text)) {
    const noteMatch = text.match(/Note:\s*(.+?)(?:\n\n|$)/s);
    return { approved: false, note: noteMatch ? noteMatch[1].trim() : undefined };
  }
  if (/^Plan approved/i.test(text)) {
    // Parse `1. description (why)` lines from the "Agreed steps:" block.
    const steps: PlanStep[] = [];
    const stepsBlock = text.split(/Agreed steps:\s*\n/i)[1] ?? '';
    for (const raw of stepsBlock.split('\n')) {
      const m = raw.match(/^\s*\d+\.\s*(.+?)(?:\s*\(([^)]+)\))?\s*$/);
      if (m) steps.push({ description: m[1].trim(), why: m[2]?.trim() ?? null });
    }
    return { approved: true, steps: steps.length > 0 ? steps : undefined };
  }
  return null;
}

/** Parse the tool result text into structured answers. Falls back to
 *  `cancelled: true` when the tool wrote its "dismissed" / "cancelled"
 *  copy. Anything we can't parse becomes an empty answer so the resolved
 *  card still lines up with the proposal by index. */
function parseAskUserResultText(text: string, expectedQuestions: number): AskUserDecision {
  if (/dismissed the question card/i.test(text) || /prompt cancelled/i.test(text)) {
    return { cancelled: true };
  }
  const answers: { picked: string[]; custom: string | null }[] = [];
  let curr: { picked: string[]; custom: string | null } | null = null;
  for (const raw of text.split('\n')) {
    const line = raw.trimEnd();
    // Each answered question starts with `[Header] question…`.
    if (/^\[[^\]]+\]/.test(line)) {
      if (curr) answers.push(curr);
      curr = { picked: [], custom: null };
      continue;
    }
    if (!curr) continue;
    const trimmed = line.trim();
    const pickedMatch = trimmed.match(/^→\s*picked:\s*(.+)$/);
    if (pickedMatch) {
      curr.picked = pickedMatch[1].split(',').map((s) => s.trim()).filter(Boolean);
      continue;
    }
    const customMatch = trimmed.match(/^→\s*user said:\s*(.+)$/);
    if (customMatch) {
      curr.custom = customMatch[1];
      continue;
    }
    // "(skipped)" / "(no answer captured)" — leave the empty defaults.
  }
  if (curr) answers.push(curr);
  while (answers.length < expectedQuestions) {
    answers.push({ picked: [], custom: null });
  }
  return { cancelled: false, answers };
}

/** A function whose identity never changes but always runs the latest
 *  `fn` — so memoized children (each transcript turn) don't re-render just
 *  because a parent re-created a handler, and never hold a stale one. */
function useStableCallback<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  const ref = useRef(fn);
  ref.current = fn;
  return useCallback((...args: A) => ref.current(...args), []);
}

/** Narrowest the transcript should get before the open context panel
 *  stops taking its own column and floats over the stream instead. */
const MIN_STREAM_WIDTH = 560;

export default function App() {
  const pingPrimedRef = useRef(false);

  // Preload the file into browser cache on mount, and unlock audio playback
  // on the first user gesture so subsequent play() calls are never blocked.
  useEffect(() => {
    const a = new Audio('/ping.mp3');
    a.preload = 'auto';

    function prime() {
      if (pingPrimedRef.current) return;
      pingPrimedRef.current = true;
      a.volume = 0;
      void a.play().then(() => { a.pause(); a.currentTime = 0; }).catch(() => {});
    }

    window.addEventListener('pointerdown', prime, { once: true });
    window.addEventListener('keydown', prime, { once: true });
    return () => {
      window.removeEventListener('pointerdown', prime);
      window.removeEventListener('keydown', prime);
    };
  }, []);

  // Respects Settings → General → "Sound when a reply finishes".
  const playPing = useCallback(() => playTurnSound(), []);

  const [status, setStatus] = useState<WsStatus>('connecting');
  const [sessionId, setSessionId] = useState<string>('');
  const sessionIdRef = useRef('');
  /** The session's own title (AI-written, or the agent's), once known; the
   *  header falls back to the first message until then. */
  const [sessionTitle, setSessionTitle] = useState<string | null>(null);
  const [model, setModel] = useState<string>('');
  const [mode, setMode] = useState<Mode>('manual');
  const [cwd, setCwd] = useState<string>('');
  const [entries, setEntries] = useState<Entry[]>([]);

  // State an external ACP agent owns. Captured here rather than folded into
  // the transcript because it is configuration, not conversation — the
  // model/mode pickers read it. Held in refs so receiving it never triggers a
  // re-render of the whole transcript.
  /** The agent's own stop reason for the last finished turn. */
  // Kept for diagnostics only: the stop reason is written into the
  // transcript when a turn ends abnormally, and read nowhere else.
  const acpStopRef = useRef<string | null>(null);
  const [acpModes, setAcpModes] = useState<{ current: string; available: AcpSessionMode[]; postures?: AgentPostureMapping[] } | null>(null);
  /** Every backend from `GET /api/engines` — native providers and external
   *  agents in one list, with health + catalogs. Refetched when extensions
   *  change (an agent install state can move) and before opening pickers. */
  const [engines, setEngines] = useState<EngineSnapshot[] | null>(null);
  const [acpConfig, setAcpConfig] = useState<AcpConfigOption[]>([]);
  const [acpCommands, setAcpCommands] = useState<string[]>([]);
  const [acpUsage, setAcpUsage] = useState<{ used: number; size: number; cost: { amount: number; currency: string } | null } | null>(null);
  /** The agent account's plan limits, as last reported. */
  const [acpLimits, setAcpLimits] = useState<{ name: string; utilization: number; resets_at?: number | null }[]>([]);
  /** Context fill for a provider chat: tokens in the last request plus its
   *  reply, against the window the harness plans for. */
  const [providerContext, setProviderContext] = useState<{ used: number; window: number; compactAt: number | null } | null>(null);
  /** What drives this session — a provider or an agent — as the server
   *  reports it on `ready` and on every `session_engine` transition. The
   *  one source of truth for "which engine": nothing below re-derives it. */
  const [engine, setEngine] = useState<SessionEngine | null>(null);
  /** The agent this session runs on (picked, starting or running), or
   *  null on a provider. Agent mode starts the moment one is picked. */
  const acpDriver = engine?.kind === 'agent' ? (engine.driver ?? null) : null;
  /** Which agent the modes / config / commands state above belongs to. */
  const capsDriverRef = useRef<string | null>(null);
  /** The engine as last applied, for handlers that must not see a stale
   *  render's value. */
  const engineRef = useRef<SessionEngine | null>(null);
  /** Adopt a new engine. Switching to a different agent swaps the agent
   *  state for that agent's last-seen capabilities at once, so the model
   *  list and the mode control are right before the agent has reported
   *  anything; its live frames replace them as they arrive. */
  function applyEngine(next: SessionEngine | null) {
    engineRef.current = next;
    setEngine(next);
    const driver = next?.kind === 'agent' ? (next.driver ?? null) : null;
    if (driver === capsDriverRef.current) return;
    capsDriverRef.current = driver;
    const caps = driver ? loadAgentCaps(driver) : {};
    setAcpModes(caps.modes ?? null);
    setAcpConfig(caps.config ?? []);
    setAcpCommands([]);
    setAcpUsage(null);
    setAcpLimits([]);
  }

  // An agent's config options projected onto Mira's picker shape, so the
  // Composer needs no knowledge of ACP. Null when no agent is running, which
  // leaves Mira's own capability-derived options in charge.
  //
  // The `model` option is excluded: it is covered by the Model row and its
  // list, and rendering it as a second select produced two "Model" rows for
  // the same setting. An empty array (not null) still means "the agent's
  // list is authoritative", so Mira's own options stay out.
  const acpModelOption = useMemo(
    () => (acpDriver ? (acpConfig.find((o) => o.category === 'model') ?? null) : null),
    [acpDriver, acpConfig],
  );
  const acpDescriptors = useMemo(() => {
    if (!acpDriver) return null;
    const all = acpOptionsToDescriptors(acpConfig) ?? [];
    const modelId = acpModelOption?.id;
    return all.filter((d) => d.id !== modelId);
  }, [acpDriver, acpConfig, acpModelOption]);
  /** Health of every known agent, refreshed on request. */
  const [acpAgents, setAcpAgents] = useState<AcpAgentStatus[]>([]);
  const [acpStatusPending, setAcpStatusPending] = useState(false);
  /** A startup failure, surfaced in the Agents panel. */
  const [acpError, setAcpError] = useState<string | null>(null);
  /** A mode picked in the picker, awaiting confirmation in the universal
   *  approval dialog. A mode is a standing grant of authority, so the pick
   *  alone is never applied — the dialog states the consequence and the user
   *  confirms. `reason` carries the server's explanation when it refused a
   *  privileged mode and asked for acknowledgement. */
  const [pendingAcpMode, setPendingAcpMode] = useState<{ modeId: string; reason?: string } | null>(null);
  /** Display name of the driving agent, resolved from the last health check
   *  so dialogs name the agent instead of its slug. */
  const acpDriverName = useMemo(() => {
    if (!acpDriver) return 'agent';
    return acpAgents.find((a) => a.kind === acpDriver)?.display_name ?? acpDriver;
  }, [acpDriver, acpAgents]);

  /** Probe every agent. Off the render path because it spawns processes. */
  const requestAcpStatus = useCallback(() => {
    setAcpStatusPending(true);
    wsRef.current?.send({ type: 'acp_status' });
  }, []);

  const startAcpAgent = useCallback((kind: string, cfg: AcpInstanceConfig, resume?: string | null, model?: string | null) => {
    setAcpError(null);
    wsRef.current?.send({
      type: 'acp_start',
      driver: kind,
      binary_path: cfg.binaryPath || null,
      display_name: cfg.displayName || null,
      launch_args: parseArgs(cfg.launchArgs ?? ''),
      env: parseEnv(cfg.env ?? ''),
      api_key: cfg.apiKey || null,
      home_path: cfg.homePath || null,
      effort: cfg.effort || null,
      setting_sources: cfg.settingSources || null,
      resume: resume || null,
      model: model || null,
    });
  }, []);
  function forkAcpAgent() {
    wsRef.current?.send({ type: 'acp_fork' });
  }
  function compactAcpAgent(focus?: string) {
    // A compaction is a turn like any other: busy until its end arrives, or
    // the composer would take input for a session that is summarizing.
    setBusy(true);
    setThinking(true);
    wsRef.current?.send({ type: 'acp_compact', focus: focus?.trim() || null });
  }

  // Agent health, usage and the active driver are state rather than refs
  // because the panel and the picker read them. Logged on change so a broken
  // agent is visible without waiting on a settings UI to exist.
  useEffect(() => {
    if (acpAgents.length > 0) {
      console.debug(
        '[acp] agents',
        acpAgents.map((a) => `${a.display_name}: ${a.state.state}`),
      );
    }
  }, [acpAgents]);

  useEffect(() => {
    if (acpUsage) {
      console.debug(
        `[acp] usage ${acpUsage.used}/${acpUsage.size}` +
          (acpUsage.cost ? ` $${acpUsage.cost.amount} ${acpUsage.cost.currency}` : ''),
      );
    }
  }, [acpUsage]);

  // Per-turn timing. Turn index = 0-based order of user messages in `entries`.
  // Only turns started in *this* session have timing (reloaded transcripts
  // have no wall-clock data, so their turns skip the "Worked for" header).
  const [turnTimings, setTurnTimings] = useState<Map<number, TurnTiming>>(new Map());
  /** When the latest turn was sent — what an agent turn's stats start from. */
  const turnStartRef = useRef<number | null>(null);
  const [expandedTurns, setExpandedTurns] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState<boolean>(false);
  const [thinking, setThinking] = useState<boolean>(false);
  // When text tokens go quiet mid-turn — typically because the model is
  // emitting tool-call deltas that don't surface as `token` events — we
  // want the "Thinking…" affordance back so the transcript isn't silent
  // for the second-or-so before `tool_start` fires. `scheduleThinking`
  // (below) sets this timer on every `token`; a follow-up token cancels
  // it, and `tool_start` / `approval_request` / `done` clear it too so
  // the indicator doesn't flash after the turn genuinely ends.
  const thinkingIdleTimerRef = useRef<number | null>(null);
  function clearThinkingIdle() {
    if (thinkingIdleTimerRef.current != null) {
      window.clearTimeout(thinkingIdleTimerRef.current);
      thinkingIdleTimerRef.current = null;
    }
  }
  function scheduleThinkingIdle() {
    clearThinkingIdle();
    thinkingIdleTimerRef.current = window.setTimeout(() => {
      thinkingIdleTimerRef.current = null;
      setThinking(true);
    }, 350);
  }
  const [pickerOpen, setPickerOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  /** The context inspector (issue #70). */
  const [inspectOpen, setInspectOpen] = useState(false);
  /** "Bring chats from other agents", from ⌘K. */
  const [importOpen, setImportOpen] = useState(false);
  // File picker opened from the right panel's "+" menu — distinct from the
  // folder picker, which switches the session's cwd.
  const [panelFilePickerOpen, setPanelFilePickerOpen] = useState(false);
  // Which primary view fills the main pane. Sidebar nav items switch this;
  // starting a chat / loading a session snaps back to 'chat' so the user
  // isn't stranded on a management screen when the model streams a reply.
  const [mainView, setMainView] = useState<MainView>('chat');
  // Settings is a first-class main view (not a dialog) — the sidebar
  // renders the section tabs while the surface fills the main pane.
  // `settingsSection` drives which section is shown; `settingsReturnTo`
  // remembers where the user came from so "Back to app" pops them back
  // to the chat / plugins / PR view they were on.
  const [settingsSection, setSettingsSection] = useState<import('./components/Settings').SettingsSectionId>('provider');
  const [settingsReturnTo, setSettingsReturnTo] = useState<MainView>('chat');
  // Enter settings by remembering the current non-settings view, then
  // swapping the main pane to `settings`. Guarded against being called
  // while already in settings (would clobber the return-to).
  // Back from installing the GitHub App: GitHub (via the github-app edge
  // function) sends the user here with `?github=…`. Show the result in
  // Settings → Integrations and drop the query.
  const [githubReturn] = useState<import('./components/Settings').GithubReturn>(() => {
    const q = new URLSearchParams(window.location.search);
    const ok = q.get('github');
    const bad = q.get('github_error');
    if (!ok && !bad) return null;
    q.delete('github');
    q.delete('github_error');
    const rest = q.toString();
    window.history.replaceState({}, '', window.location.pathname + (rest ? `?${rest}` : '') + window.location.hash);
    if (bad) return { ok: false, message: `GitHub: ${bad}` };
    return ok === 'requested'
      ? { ok: true, message: 'Installation requested: an organization owner has to approve it on GitHub.' }
      : { ok: true, message: 'GitHub connected. Turn Mira on for the repositories you want.' };
  });
  useEffect(() => {
    if (!githubReturn) return;
    setSettingsSection('integrations');
    openSettings();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function openSettings() {
    setMainView((prev) => {
      if (prev !== 'settings') setSettingsReturnTo(prev);
      return 'settings';
    });
  }
  /** Jump straight to agent configuration (install hints, advanced setup). */
  function openAgentSettings() {
    openSettings();
    setSettingsSection('agents');
  }
  // Leave settings — pop back to wherever the user was. Falls back to
  // chat if the remembered view was somehow also settings (shouldn't
  // happen, but a stale value shouldn't strand the user).
  function exitSettings() {
    setMainView(settingsReturnTo === 'settings' ? 'chat' : settingsReturnTo);
  }
  // Left sidebar visibility — collapses the 300px column to 0.
  // Startup default comes from Settings → General (localStorage).
  const [sidebarOpen, setSidebarOpen] = useState(() => {
    try { return localStorage.getItem('mira.sidebar.open') !== '0'; } catch { return true; }
  });
  // Right-side panel: `agentTabs` is the ordered list of open agent call_ids;
  // `fileTabs` is the ordered list of open file viewer tabs; `toolTabs` holds
  // the utility panes (browser / whiteboard / devtools, one of each);
  // `activeAgentTab` is the visible panel's id — a callId, a file path, or a
  // `tool:<kind>` id. Panel is open iff any list is non-empty.
  const [agentTabs, setAgentTabs] = useState<string[]>([]);
  const [fileTabs, setFileTabs] = useState<FilePanelTab[]>([]);
  const [toolTabs, setToolTabs] = useState<ToolPaneTab[]>([]);
  const [activeAgentTab, setActiveAgentTab] = useState<string | null>(null);
  // Right panel pixel width — user-draggable via the resize handle.
  const [rightPanelWidth, setRightPanelWidth] = useState(390);
  // Live subagent transcripts keyed by parent tool-call id. Same Entry
  // shape as the parent transcript so the panel body can reuse the same
  // grouping and rendering helpers. Populated by the `subagent_*` WS
  // frames the AgentTool broadcasts as its child streams events.
  const [subagentState, setSubagentState] = useState<Map<string, SubagentStreamState>>(new Map());
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [providerName, setProviderName] = useState<string | null>(null);
  const [sidebarRefresh, setSidebarRefresh] = useState(0);
  const [reviewPanelOpen, setReviewPanelOpen] = useState(false);
  const [reviewState, setReviewState] = useState<ReviewState | null>(null);
  const [usage, setUsage] = useState<UsageTotals | null>(null);
  const [rateLimit, setRateLimit] = useState<RateLimitReading | null>(null);
  const [gitStatus, setGitStatus] = useState<GitStatusView | null>(null);
  const [sessionDiff, setSessionDiff] = useState<SessionDiffView>({ added: 0, removed: 0, files: [] });
  // Context panel: collapsed to a pill by default so it stays out of the
  // way; the choice is remembered per browser.
  const [ctxOpen, setCtxOpen] = useState<boolean>(() => {
    try { return localStorage.getItem('mira.context.open') === '1'; } catch { return false; }
  });
  const onCtxOpenChange = (v: boolean) => {
    setCtxOpen(v);
    try { localStorage.setItem('mira.context.open', v ? '1' : '0'); } catch { /* private mode */ }
  };
  // The session committed through the panel, so any unpushed commits on
  // the branch are its own to push.
  const [sessionCommitted, setSessionCommitted] = useState(false);
  const [reviewOpen, setReviewOpen] = useState(false);
  /** File the review drawer should open on, when opened from the file list. */
  const [reviewFocus, setReviewFocus] = useState<string | null>(null);
  const [lightbox, setLightbox] = useState<string | null>(null);
  // "Open session" from the Usage page (Settings) — back to chat on it.
  useEffect(() => {
    const onOpen = (e: Event) => {
      const id = (e as CustomEvent<string>).detail;
      if (!id) return;
      wsRef.current?.attach(id);
      setMainView('chat');
    };
    window.addEventListener('mira:open-session', onOpen);
    return () => window.removeEventListener('mira:open-session', onOpen);
  }, []);
  // Chats were added outside a turn (imported from another agent).
  useEffect(() => {
    const refresh = () => setSidebarRefresh((n) => n + 1);
    window.addEventListener('mira:sessions-changed', refresh);
    return () => window.removeEventListener('mira:sessions-changed', refresh);
  }, []);
  // Integrated terminal (bottom panel); open state is remembered.
  // Settings → General can disable the restore (always start closed).
  const [terminalOpen, setTerminalOpen] = useState<boolean>(() => {
    try {
      if (localStorage.getItem('mira.terminal.restore') === '0') return false;
      return localStorage.getItem('mira.terminal.open') === '1';
    } catch { return false; }
  });
  const setTerminal = useCallback((v: boolean) => {
    setTerminalOpen(v);
    try { localStorage.setItem('mira.terminal.open', v ? '1' : '0'); } catch { /* private mode */ }
  }, []);
  const keybindings = useKeybindings();

  /** Live `when`-clause context for the keybinding engine. */
  function shortcutContext(): ShortcutMatchContext {
    const ae = document.activeElement as HTMLElement | null;
    const tag = ae?.tagName;
    const editable = !!ae && (tag === 'INPUT' || tag === 'TEXTAREA' || ae.isContentEditable);
    return {
      terminalFocus: !!ae?.closest('.xterm'),
      // `pendingApprovals` is declared below; this only runs on keydown,
      // long after the whole component body has initialized.
      approvalOpen: pendingApprovals.length > 0,
      reviewOpen,
      settingsOpen: mainView === 'settings',
      isWeb: true,
      isDesktop: false,
      editableFocus: editable,
    };
  }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (command !== 'terminal.toggle') return;
      e.preventDefault();
      setTerminalOpen((v) => {
        try { localStorage.setItem('mira.terminal.open', v ? '0' : '1'); } catch { /* private mode */ }
        return !v;
      });
    };
    // Capture phase: the shortcut must work while xterm has focus.
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [keybindings]);
  // ⌘K opens the command palette from anywhere, the composer included —
  // it's the one shortcut that has to work mid-sentence.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const command = resolveShortcutCommand(e, keybindings, {
        context: { ...shortcutContext(), editableFocus: false },
      });
      if (command !== 'palette.toggle') return;
      e.preventDefault();
      e.stopPropagation();
      setPaletteOpen((v) => !v);
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keybindings]);
  const ctxFits = useContextPanelFits();
  // The chat column's width, for whether the open context panel sits
  // beside the transcript or over it.
  const [chatColWidth, setChatColWidth] = useState(0);
  const chatColObserver = useRef<ResizeObserver | null>(null);
  const chatColRef = useCallback((el: HTMLDivElement | null) => {
    chatColObserver.current?.disconnect();
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setChatColWidth(Math.round(e.contentRect.width)));
    ro.observe(el);
    chatColObserver.current = ro;
  }, []);
  const [branchPr, setBranchPr] = useState<BranchPrView | null>(null);
  /** Re-read the repo state the panels show: git status, what this chat
   *  changed, and the branch's PR. Responses are applied only if no newer
   *  refresh has started since — after a quick session switch, a slow reply
   *  for the previous chat must not overwrite the current one. */
  const repoSeqRef = useRef(0);
  const refreshRepo = useCallback(() => {
    const seq = ++repoSeqRef.current;
    const live = () => seq === repoSeqRef.current;
    getGitStatus().then((s) => live() && setGitStatus(s)).catch(() => {});
    getSessionDiff().then((d) => live() && setSessionDiff(d)).catch(() => {});
    getBranchPr()
      .then((pr) => live() && setBranchPr(pr))
      .catch(() => live() && setBranchPr(null));
  }, []);
  // Something outside a turn changed files (a diff applied from a reply).
  useEffect(() => {
    window.addEventListener('mira:repo-changed', refreshRepo);
    return () => window.removeEventListener('mira:repo-changed', refreshRepo);
  }, [refreshRepo]);
  // Live task list — hydrated from `ready.tasks` on socket open and
  // upserted whenever a `task_*` tool result lands. Rendered as a
  // persistent "Plan" card near the top of the transcript.
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  // Standing `/goal`, if any. Populated from `ready.goal` on socket
  // open and mutated by `goal_set` / `goal_progress` / `goal_done` /
  // `goal_cleared` server frames. Absent = no autonomous run set.
  const [goal, setGoal] = useState<Goal | null>(null);
  // Remote environment of the attached session (see EnvironmentChip).
  const [environment, setEnvironment] = useState<EnvironmentStatus | null>(null);
  const [environments, setEnvironments] = useState<EnvironmentInfo[]>([]);
  const [envSwitching, setEnvSwitching] = useState<string | null>(null);
  // Loaded skill roster — powers `/<skill-name>` slash commands in the
  // composer palette. Fetched lazily after the first WS Ready frame
  // (server needs to be up + AppState wired). Empty on error; the
  // palette degrades gracefully to just the built-in commands.
  const [skills, setSkills] = useState<SkillView[]>([]);
  // Custom commands + MCP prompts for the composer palette, and a
  // counter the Plugins page watches to refetch on `extensions_changed`.
  const [commands, setCommands] = useState<CommandInfo[]>([]);
  const [extensionsVersion, setExtensionsVersion] = useState(0);
  // Bumps each time the backend broadcasts `SkillsReloaded` (filesystem
  // watcher detected a change). Passed to the Settings panel so its
  // Skills tab re-fetches when a `SKILL.md` lands / vanishes / edits
  // while it's open.
  const [skillsVersion, setSkillsVersion] = useState(0);
  // Force a re-render every second while a turn is active so the live
  // "Working…" counter ticks. Cheap; the tree is small and only mounts
  // when the browser tab is visible.
  const [, setNowTick] = useState(0);
  const wsRef = useRef<WsClient | null>(null);
  const paneRef = useRef<HTMLDivElement | null>(null);
  // Plan payloads that arrived *before* their tool_start (race between the
  // interactive tool's direct broadcast and the harness→WS forwarder). We
  // stash by call_id and drain on tool_start so no plan ever renders as a
  // plain running tool row.
  const pendingProposalsRef = useRef<Map<string, PlanProposal>>(new Map());
  // Same race-guard pattern as pendingProposalsRef but for the ask_user
  // tool: `ask_user_request` might arrive before the matching
  // `tool_start` (they broadcast on the same channel but the harness
  // doesn't guarantee arrival order). Stash the proposal here so the
  // tool_start case can drain it onto the fresh entry.
  const pendingAskUserRef = useRef<Map<string, AskUserProposal>>(new Map());

  useEffect(() => {
    const c = connect(onMessage, setStatus);
    wsRef.current = c;
    return () => c.close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    getSettings()
      .then((v) => {
        setConfigured(v.configured);
        setProviderName(v.default_provider ?? null);
        if (!v.configured) openSettings();
      })
      .catch(() => setConfigured(false));
    // openSettings is stable within this component's lifetime; deps
    // deliberately empty so this only runs once on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Follow new output only while the reader is at the bottom; scrolling
  // up to read back stops the follow and offers a jump-to-latest button.
  const followRef = useRef(true);
  const [showJump, setShowJump] = useState(false);
  const jumpToLatest = useCallback(() => {
    followRef.current = true;
    setShowJump(false);
    const el = paneRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
  }, []);
  const onPaneScroll = useCallback(() => {
    const el = paneRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    // Settings → General → Transcript can disable auto-follow; the
    // jump-to-latest button still works (it re-arms follow explicitly).
    followRef.current = atBottom && getBoolPref(PREF_KEYS.transcriptFollow, true);
    setShowJump(!atBottom);
  }, []);
  useEffect(() => {
    const el = paneRef.current;
    if (el && followRef.current) el.scrollTop = el.scrollHeight;
  }, [entries, thinking]);

  // Tick the live counter while a turn is in flight. Stopping the interval
  // as soon as `busy` clears avoids a needless setInterval that runs forever.
  useEffect(() => {
    if (!busy) return;
    const id = window.setInterval(() => setNowTick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, [busy]);

  // Desktop notification when a turn finishes while the tab is hidden
  // (Settings → General → Notifications). Only fires if the user granted
  // permission; requesting happens from the settings row.
  const prevBusyRef = useRef(busy);
  // Read from WS handlers, which close over the first render.
  const chatTitleRef = useRef<string | null>(null);
  chatTitleRef.current = sessionTitle ?? titleFromEntries(entries);
  useEffect(() => {
    const was = prevBusyRef.current;
    prevBusyRef.current = busy;
    if (!was || busy) return;
    callForAttention('done', chatTitleRef.current);
  }, [busy]);

  // Apply the reduce-motion class on boot (Settings → General → Appearance).
  useEffect(() => {
    applyReduceMotion();
  }, []);

  // Smooth streaming: tokens land in a buffer and are released a few
  // characters per animation frame (faster when the buffer is deep), so
  // text flows instead of jumping in network-sized chunks.
  const tokenBufRef = useRef('');
  const tokenRafRef = useRef<number | null>(null);
  function drainTokens() {
    tokenRafRef.current = null;
    const buf = tokenBufRef.current;
    if (!buf) return;
    const n = Math.max(3, Math.ceil(buf.length / 6));
    const chunk = buf.slice(0, n);
    tokenBufRef.current = buf.slice(n);
    setEntries((prev) => appendToken(prev, chunk));
    if (tokenBufRef.current) tokenRafRef.current = requestAnimationFrame(drainTokens);
  }
  /** Release everything buffered now — before any other frame lands, so
   *  ordering between text and tool calls is preserved. */
  function flushTokens() {
    if (tokenRafRef.current != null) cancelAnimationFrame(tokenRafRef.current);
    tokenRafRef.current = null;
    const buf = tokenBufRef.current;
    tokenBufRef.current = '';
    if (buf) setEntries((prev) => appendToken(prev, buf));
  }

  // Per-turn usage: session totals at turn start, diffed at turn end.
  const usageRef = useRef<UsageTotals | null>(null);
  const turnBaseRef = useRef<{ turn: number; base: UsageTotals } | null>(null);
  const [turnUsage, setTurnUsage] = useState<Map<number, UsageTotals>>(new Map());
  /** The model each reloaded turn ran on, for pricing it. Live turns use
   *  the current model. */
  const [turnModels, setTurnModels] = useState<Map<number, string>>(new Map());
  function startTurnUsage(turn: number) {
    turnBaseRef.current = {
      turn,
      base: usageRef.current ?? { prompt_tokens: 0, completion_tokens: 0, cached_input_tokens: 0, rounds: 0 },
    };
  }

  function onMessage(msg: ServerMsg) {
    if (msg.type !== 'token') flushTokens();
    switch (msg.type) {
      case 'ready': {
        agentTerminal.reset();
        setGitStatus(null);
        setSessionDiff({ added: 0, removed: 0, files: [] });
        setSessionCommitted(false);
        setBranchPr(null);
        setSessionId(msg.session_id);
        wsRef.current?.setSession(msg.session_id);
        sessionIdRef.current = msg.session_id;
        setSessionTitle(msg.title ?? null);
        // Context is per chat; the agent's plan limits are per account and
        // stay. The session's own usage frames (replayed below) restore it.
        setProviderContext(null);
        // A provider chat's context is otherwise only known once its next
        // request reports usage, so a reopened chat showed tokens but no
        // window. Seed it from the breakdown; a live report replaces it.
        {
          const opened = msg.session_id;
          const isAgent = msg.engine ? msg.engine.kind === 'agent' : !!(msg.agent_kind || msg.agent_configured);
          if (!isAgent) {
            getContextBreakdown()
              .then((v) => {
                if (sessionIdRef.current !== opened || v.source !== 'mira') return;
                setProviderContext((cur) => cur ?? { used: v.breakdown.total, window: v.window, compactAt: v.compact_at });
              })
              .catch(() => {});
          }
        }
        setAcpUsage(null);
        setRestoreNote(null);
        setRestoreAsk(null);
        setModel(msg.model);
        setMode(msg.mode);
        setCwd(msg.cwd);
        // The engine first: it decides whose capabilities the state frames
        // below belong to. Older servers send no engine — derive one.
        applyEngine(
          msg.engine ??
            (msg.agent_kind || msg.agent_configured
              ? { kind: 'agent', driver: msg.agent_kind ?? msg.agent_configured, display_name: msg.agent_kind ?? msg.agent_configured ?? 'agent', status: msg.agent_kind ? 'ready' : 'idle' }
              : { kind: 'provider', instance: msg.instance ?? null, display_name: msg.instance ?? 'Mira', model: msg.model, status: 'ready' }),
        );
        // State frames ride the live handler so the pickers reflect the
        // agent's last-known modes and models before any new turn runs.
        for (const line of msg.agent_transcript ?? []) {
          const f = line.frame as ServerMsg | undefined;
          if (!f || typeof f !== 'object' || !('type' in f)) continue;
          if (
            f.type === 'acp_modes' ||
            f.type === 'acp_config_options' ||
            f.type === 'acp_commands' ||
            f.type === 'acp_usage' ||
            f.type === 'acp_limits'
          ) {
            onMessage(f);
          }
        }
        // Provider and agent turns interleave in the order they happened.
        const readyEntries = historyToEntries(msg.history, msg.previews);
        setEntries(interleaveReplay(msg.history, msg.previews, msg.agent_transcript ?? []));
        // Seed subagent state from history so the ContextPanel shows agents
        // on session reload (live subagent_started frames don't replay).
        setSubagentState(() => {
          const seeded = new Map<string, SubagentStreamState>();
          for (const e of readyEntries) {
            if (e.kind !== 'tool' || e.call.function.name !== 'agent') continue;
            try {
              const args = JSON.parse(e.call.function.arguments) as { prompt?: string; type?: string };
              seeded.set(e.call.id, {
                parentCallId: e.call.id,
                prompt: args.prompt ?? '',
                agentName: args.type ?? null,
                agentCategory: null,
                entries: [],
                done: true,
                pendingReview: null,
              });
            } catch { /* skip malformed args */ }
          }
          return seeded;
        });
        // Server-persisted turn timing is aligned with user-message order
        // (turn 0 = first user msg). Rebuild the local Map so "Worked for"
        // chips render on reloaded transcripts.
        setTurnTimings(rebuildTurnTimings(msg.turns ?? []));
        // The reply hover row's tokens and cost, for turns from before this
        // page loaded — the server keeps them per turn.
        setTurnUsage(rebuildTurnUsage(msg.turns ?? []));
        setTurnModels(rebuildTurnModels(msg.turns ?? []));
        setExpandedTurns(new Set());
        setUsage(msg.usage ?? null);
        setRateLimit(null);
        setTasks(msg.tasks ?? []);
        setGoal(msg.goal ?? null);
        setBusy(false);
        setThinking(false);
        clearThinkingIdle();
        setSidebarRefresh((n) => n + 1);
        // Refresh the skill roster on every Ready — a cwd swap may
        // change the project tier (~/.mira vs. <cwd>/.mira). Silent on
        // failure; the palette just shows built-in commands.
        listSkills().then(setSkills).catch(() => setSkills([]));
        listCommands().then(setCommands).catch(() => setCommands([]));
        // Each session (and worktree) has its own environment; ask for it.
        setEnvSwitching(null);
        wsRef.current?.send({ type: 'environment' });
        // Fetch git status, session diff, and branch PR for the new cwd.
        refreshRepo();
        // A Ready frame means the harness swapped session context (new /
        // load / resume / reconnect). If the user was parked on Plugins
        // or another management view, jump back to chat so a fresh
        // transcript actually shows.
        setMainView('chat');
        break;
      }
      case 'reasoning':
        // The live thought block is its own "working" signal.
        setThinking(false);
        clearThinkingIdle();
        setEntries((prev) => appendReasoning(prev, msg.text));
        break;
      case 'token':
        setThinking(false);
        // Text is streaming — hide the indicator, but arm a short idle
        // timer so a silent gap (typically the model emitting tool-call
        // deltas after its assistant text ends) brings the indicator
        // back before `tool_start` finally fires.
        scheduleThinkingIdle();
        tokenBufRef.current += msg.text;
        if (tokenRafRef.current == null) tokenRafRef.current = requestAnimationFrame(drainTokens);
        break;
      case 'approval_request':
        setThinking(false);
        clearThinkingIdle();
        playPing();
        callForAttention('approval', chatTitleRef.current);
        setEntries((prev) => [
          ...sealThought(prev),
          { kind: 'tool', call: msg.call, preview: msg.preview ?? null, needs: msg.needs ?? [], status: 'pending', result: null },
        ]);
        break;
      case 'tool_start':
        setThinking(false);
        clearThinkingIdle();
        agentTerminal.commandStart(msg.call.id, msg.call.function.name, msg.call.function.arguments);
        setEntries((prev) => {
          let next = upsertToolStart(prev, msg.call);
          // If a plan_request arrived before this tool_start (race between
          // the tool's direct broadcast and the harness forwarder), drain
          // the queued proposal onto the fresh entry now.
          const planQ = pendingProposalsRef.current.get(msg.call.id);
          if (planQ) {
            pendingProposalsRef.current.delete(msg.call.id);
            next = attachPlanProposal(next, msg.call.id, planQ);
          }
          const askQ = pendingAskUserRef.current.get(msg.call.id);
          if (askQ) {
            pendingAskUserRef.current.delete(msg.call.id);
            next = attachAskUserProposal(next, msg.call.id, askQ);
          }
          return next;
        });
        break;
      case 'tool_end':
        // The model usually starts thinking again after a tool result comes
        // back before the next text token arrives.
        setThinking(true);
        setEntries((prev) => attachToolResult(prev, msg.result));
        agentTerminal.commandEnd(msg.result.call_id, msg.result.content, !!msg.result.is_error);
        // Piggyback: task_* tools ship the current task or full list in
        // `data`. Upsert so the Plan panel stays live without another
        // round-trip.
        setTasks((prev) => applyTaskResult(prev, msg.result));
        break;
      case 'turn_complete':
        // A turn ended (assistant round complete). More may follow if there
        // were tool calls; if not, `done` will clear us right after.
        setThinking(true);
        // Belt-and-suspenders sidebar refresh — the onSend-triggered
        // refetch can race the harness's first checkpoint on a very
        // fresh session; this fires once the first assistant round has
        // definitively landed on disk.
        setSidebarRefresh((n) => n + 1);
        break;
      case 'done': {
        const started = turnBaseRef.current;
        const now = usageRef.current;
        if (started && now) {
          const d: UsageTotals = {
            prompt_tokens: now.prompt_tokens - started.base.prompt_tokens,
            completion_tokens: now.completion_tokens - started.base.completion_tokens,
            cached_input_tokens: now.cached_input_tokens - started.base.cached_input_tokens,
            rounds: now.rounds - started.base.rounds,
          };
          if (d.prompt_tokens + d.completion_tokens > 0) {
            setTurnUsage((prev) => new Map(prev).set(started.turn, d));
          }
        }
        turnBaseRef.current = null;
        setBusy(false);
        setThinking(false);
        clearThinkingIdle();
        setEntries((prev) => settleTools(sealThought(prev)));
        playPing();
        // Refresh git status, session diff, and branch PR after each turn.
        refreshRepo();
        // Close out the most recent turn's timing.
        setTurnTimings((prev) => stampLastTurn(prev, Date.now()));
        setSidebarRefresh((n) => n + 1);
        break;
      }
      case 'environment_status':
        setEnvironment(msg.status);
        setEnvironments(msg.environments);
        break;
      case 'environment_progress':
        setEnvSwitching(msg.text);
        break;
      case 'environment_switched': {
        setEnvSwitching(null);
        setEnvironment(msg.status);
        const notes: string[] = msg.error
          ? [`[environment] couldn't switch to ${msg.to}`, msg.error]
          : msg.from === msg.to
            ? []
            : [`[environment] ${msg.from} → ${msg.to}`, ...msg.lines];
        if (msg.conflicts.length > 0) {
          notes.push(`Merge conflicts to resolve: ${msg.conflicts.join(', ')}`);
        }
        if (notes.length > 0) {
          setEntries((prev) => [...prev, { kind: 'warning', text: notes.join('\n') }]);
        }
        break;
      }
      case 'warning':
        setEntries((prev) => {
          // A verify result replaces its own "running …" line.
          const last = prev[prev.length - 1];
          if (
            /^\[verify\]/.test(msg.text) &&
            last?.kind === 'warning' &&
            /^\[verify\]\s*running/i.test(last.text)
          ) {
            return [...prev.slice(0, -1), { kind: 'warning', text: msg.text }];
          }
          return [...prev, { kind: 'warning', text: msg.text }];
        });
        break;
      case 'tool_progress':
        setEntries((prev) => appendProgressLine(prev, msg.call_id, msg.line));
        agentTerminal.outputLine(msg.call_id, msg.line);
        break;
      case 'tool_preview':
        // The harness computes a diff preview for edit/write calls
        // just before execution and fires this frame regardless of
        // approval mode. Attach it to the matching in-flight tool
        // entry so auto-allowed writes get the same rich diff view
        // that approval-gated ones already do via ApprovalRequest.
        setEntries((prev) =>
          prev.map((e) => {
            if (e.kind !== 'tool' || e.call.id !== msg.call_id) return e;
            return { ...e, preview: msg.preview };
          }),
        );
        break;
      case 'extensions_changed':
        // An MCP server connected/dropped or a plugin changed.
        listCommands().then(setCommands).catch(() => {});
        listSkills().then(setSkills).catch(() => {});
        setExtensionsVersion((n) => n + 1);
        // Agent install/auth state can move under us (an adapter was
        // installed, a CLI signed in) — refresh the engine list.
        listEngines().then((v) => setEngines(v.engines)).catch(() => {});
        break;
      case 'skills_reloaded':
        // A skill file appeared / changed / vanished. Refetch the
        // roster so the composer palette + the Settings panel pick
        // up the new state without a click.
        listSkills().then(setSkills).catch(() => {});
        setSkillsVersion((n) => n + 1);
        break;
      case 'error':
        setEntries((prev) => [...prev, { kind: 'error', text: msg.text }]);
        break;
      case 'model_changed':
        setModel(msg.model);
        break;
      case 'mode_changed':
        setMode(msg.mode);
        break;
      case 'review_started':
        setReviewState(emptyReviewState(msg.run_id));
        setReviewPanelOpen(true);
        break;
      case 'review_progress':
        setReviewState((prev) => {
          // Guard against stale frames from a previous run bleeding in
          // after a new run started — only apply if the run_ids match.
          if (!prev || prev.runId !== msg.run_id) return prev;
          return applyReviewEvent(prev, msg.event);
        });
        break;
      case 'review_result':
        setReviewState((prev) => {
          if (!prev || prev.runId !== msg.run_id) return prev;
          return { ...prev, findings: msg.findings, done: true };
        });
        break;
      case 'review_error':
        setReviewState((prev) => {
          if (!prev || prev.runId !== msg.run_id) return prev;
          return { ...prev, error: msg.text, done: true };
        });
        break;
      case 'session_title_updated':
        // Nickname landed on disk — refresh the sidebar so the row label
        // switches from the first-user-message fallback to the AI title,
        // and retitle the header when it is this chat's.
        setSidebarRefresh((n) => n + 1);
        if (msg.session_id === sessionIdRef.current) setSessionTitle(msg.title);
        break;
      case 'background_mode_changed':
      case 'session_background_idle':
      case 'session_background_running':
        // These frames drive the sidebar's per-session running / attached
        // / mode indicators. The simplest refresh path is to poke the
        // Sidebar's refetch counter — it re-hits /api/sessions which
        // reports the current live-slot metadata.
        setSidebarRefresh((n) => n + 1);
        break;
      case 'usage':
        setUsage(msg.totals);
        usageRef.current = msg.totals;
        if (msg.context_window) {
          setProviderContext({
            used: msg.round.prompt_tokens + msg.round.completion_tokens,
            window: msg.context_window,
            compactAt: msg.compact_at ?? null,
          });
        }
        break;
      case 'rate_limit':
        setRateLimit({ rate_limit: msg.rate_limit, summary: msg.summary, at: Date.now() });
        break;
      case 'memory_learned':
        setEntries((prev) => [
          ...prev,
          {
            kind: 'warning',
            text: `[memory] remembered ${msg.count} thing${msg.count === 1 ? '' : 's'}`,
          },
        ]);
        break;
      case 'compacted':
        setEntries((prev) => [
          // The "summarizing…" note from /compact is done.
          ...prev.filter((e) => !(e.kind === 'warning' && e.text === '[context] summarizing the conversation…')),
          { kind: 'compact', summarized: msg.messages_removed, summary: null },
        ]);
        break;
      case 'goal_set':
        setGoal(msg.goal);
        setEntries((prev) => [
          ...prev,
          {
            kind: 'goal',
            variant: 'set',
            iteration: null,
            maxIterations: msg.goal.max_iterations,
            status: msg.goal.status,
            reason: null,
            condition: msg.goal.condition,
          } as GoalEntry,
        ]);
        break;
      case 'goal_cleared':
        setGoal(null);
        setEntries((prev) => [
          ...prev,
          {
            kind: 'goal',
            variant: 'cleared',
            iteration: null,
            maxIterations: null,
            status: 'cleared',
            reason: null,
          } as GoalEntry,
        ]);
        break;
      case 'goal_progress': {
        const p = msg;
        setGoal((prev) =>
          prev
            ? {
                ...prev,
                iterations: p.iteration,
                max_iterations: p.max_iterations,
                status: p.status,
                last_reason: p.reason ?? prev.last_reason,
              }
            : prev,
        );
        // Progress chip: only drop into the transcript when the loop
        // is going to keep going (status === 'active'). Terminal
        // transitions get announced once by the following `goal_done`
        // so we don't double-post the same "met"/"impossible" line.
        if (p.status === 'active') {
          setEntries((prev) => [
            ...prev,
            {
              kind: 'goal',
              variant: 'progress',
              iteration: p.iteration,
              maxIterations: p.max_iterations,
              status: p.status,
              reason: p.reason ?? null,
            } as GoalEntry,
          ]);
        }
        break;
      }
      case 'goal_done': {
        const d = msg;
        setGoal((prev) =>
          prev ? { ...prev, status: d.status, last_reason: d.reason ?? prev.last_reason } : prev,
        );
        setEntries((prev) => [
          ...prev,
          {
            kind: 'goal',
            variant: 'done',
            iteration: null,
            maxIterations: null,
            status: d.status,
            reason: d.reason ?? null,
          } as GoalEntry,
        ]);
        break;
      }
      case 'subagent_started':
        setSubagentState((prev) => {
          const next = new Map(prev);
          const existing = next.get(msg.parent_call_id);
          next.set(msg.parent_call_id, {
            parentCallId: msg.parent_call_id,
            agentId: msg.agent_id,
            model: msg.model,
            prompt: msg.prompt,
            agentName: msg.agent_name ?? null,
            agentCategory: msg.agent_category ?? null,
            entries: existing?.entries ?? [],
            done: false,
            pendingReview: existing?.pendingReview ?? null,
          });
          return next;
        });
        // Auto-focus the newly-spawned agent in the panel so the user sees
        // its first tokens without having to click the row. Only if the
        // panel isn't already open on a different agent — respect the
        // user's manual selection when one exists.
        setAgentTabs((prev) => (prev.includes(msg.parent_call_id) ? prev : [...prev, msg.parent_call_id]));
        setActiveAgentTab((prev) => prev ?? msg.parent_call_id);
        break;
      case 'subagent_token':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: appendToken(s.entries, msg.text),
        })));
        break;
      case 'subagent_tool_start':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: upsertToolStart(s.entries, msg.call),
        })));
        break;
      case 'subagent_tool_end':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: attachToolResult(s.entries, msg.result),
        })));
        break;
      case 'subagent_warning':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: [...s.entries, { kind: 'warning', text: msg.text }],
        })));
        break;
      case 'subagent_progress':
        // Streaming intermediate summary — surfaces as a `[progress]`
        // chip in the SubagentPanel (styled distinctly from warnings so
        // the reader can tell "here's where I am" from "something's off").
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: [...s.entries, { kind: 'warning', text: `[progress] ${msg.text}` }],
        })));
        break;
      case 'subagent_review_request':
        // Auto-open the tab so the user can't miss the review — the
        // parent's turn is blocked until Approve/Deny lands. If it's
        // already open, we just annotate its state.
        setAgentTabs((prev) => (prev.includes(msg.parent_call_id) ? prev : [...prev, msg.parent_call_id]));
        setActiveAgentTab(msg.parent_call_id);
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          pendingReview: { promptId: msg.prompt_id, summary: msg.summary },
        })));
        break;
      case 'subagent_done':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          done: true,
        })));
        break;
      case 'subagent_scratchpad_note':
        // Cross-subagent shared findings. Surface as a distinct chip in
        // the author's tab so a viewer can see who posted what, and keep
        // the raw text so a future "Shared notes" pane can dedupe by
        // (session, ts) if we surface it more prominently later.
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: [
            ...s.entries,
            { kind: 'warning', text: `[note ${msg.entry.author}] ${msg.entry.text}` },
          ],
        })));
        break;
      case 'plan_request':
        // Server reuses the tool call id as the prompt id. Attach immediately
        // if the tool_start already arrived; otherwise stash the proposal so
        // tool_start can pick it up when it lands (see the tool_start case).
        playPing();
        callForAttention('plan', chatTitleRef.current);
        setEntries((prev) => {
          const hit = prev.some((e) => e.kind === 'tool' && e.call.id === msg.prompt_id);
          if (!hit) {
            pendingProposalsRef.current.set(msg.prompt_id, msg.plan);
            return prev;
          }
          return attachPlanProposal(prev, msg.prompt_id, msg.plan);
        });
        break;
      case 'ask_user_request':
        // Same race-guard pattern as plan_request — attach immediately when
        // the tool_start already landed; stash otherwise.
        playPing();
        callForAttention('question', chatTitleRef.current);
        setEntries((prev) => {
          const hit = prev.some((e) => e.kind === 'tool' && e.call.id === msg.prompt_id);
          if (!hit) {
            pendingAskUserRef.current.set(msg.prompt_id, msg.proposal);
            return prev;
          }
          return attachAskUserProposal(prev, msg.prompt_id, msg.proposal);
        });
        break;

      // -------- ACP (external agent) --------
      case 'acp_text':
        setEntries((prev) => appendAcpText(prev, msg.text));
        break;
      case 'acp_thought':
        setEntries((prev) => appendAcpThought(prev, msg.text));
        break;
      case 'acp_tool_call':
      case 'acp_tool_call_update':
        setEntries((prev) => upsertAcpTool(prev, msg.call));
        break;
      case 'acp_plan':
        setEntries((prev) => {
          const sealed = sealAcpThought(prev);
          const idx = sealed.findIndex((e) => e.kind === 'acp_plan');
          const entry: AcpPlanEntry = { kind: 'acp_plan', entries: msg.entries };
          if (idx >= 0) {
            return [...sealed.slice(0, idx), entry, ...sealed.slice(idx + 1)];
          }
          return [...sealed, entry];
        });
        break;
      case 'acp_turn_end':
        // End the turn. This is the only place an external agent's turn can
        // be declared over — the prompt is fire-and-forget, so unlike the
        // harness there is no surrounding await to imply completion. Without
        // clearing `busy` here the composer spins forever, which is what a
        // rate-limited turn looks like: the agent is long finished, the UI
        // just never hears about it.
        setBusy(false);
        setThinking(false);
        clearThinkingIdle();
        acpStopRef.current = null;
        // The agent edits files in its own process; nothing else tells the
        // panels their counts are stale.
        refreshRepo();
        // Close out the turn's timing, or "Worked for" keeps ticking on a
        // turn that ended minutes ago. The harness path does this on `done`;
        // the agent path never did, which is why a finished turn still showed
        // a live duration.
        setTurnTimings((prev) => stampLastTurn(prev, Date.now()));
        {
          const started = turnStartRef.current;
          const ended = Date.now();
          setEntries((prev) => withTurnStats(prev, (st) => ({ ...st, startedAt: st.startedAt ?? started, endedAt: ended })));
        }

        // A turn that did not complete should say why. A usage limit is an
        // error, not an aside: the agent produced no answer, so presenting it
        // as a warning styled like a reply made a failed turn look like a
        // completed one. The server puts the agent's own context (e.g. when
        // a limit resets) on the frame so this is one message, not two.
        const stopped = msg.stop_reason;
        setEntries((prev) => {
          const sealed = settleTools(sealAcpThought(prev));
          if (isSuccessfulAcpStop(stopped)) return sealed;
          const detail = msg.detail ? ` ${msg.detail}` : '';
          return [
            ...sealed,
            {
              kind: 'error',
              text: `${describeAcpStop(stopped)}${detail}`,
            },
          ];
        });
        break;
      case 'acp_privileged_mode_confirmation':
        // The server refused without an acknowledgement. Show what the mode
        // does and let the user decide — the flag alone is not consent,
        // because the client sets it.
        // Folded into the same confirm state as a picker pick: one state,
        // one dialog, whether the mode came from the user or the server.
        setPendingAcpMode({ modeId: msg.mode_id, reason: msg.reason });
        break;
      case 'acp_mode_changed':
        // A standing change to what the agent may do, recorded in the
        // transcript so it is visible after the fact and not only in a
        // dropdown that may have been closed by then.
        setEntries((prev) => [
          ...prev,
          {
            kind: 'warning',
            text: msg.privileged
              ? `${msg.display_name}: ${msg.mode_name} enabled — ${msg.mode_id} grants more access than Mira would`
              : `${msg.display_name}: mode set to ${msg.mode_name}`,
          },
        ]);
        setPendingAcpMode(null);
        break;
      case 'acp_unmodelled':
        // Logged, not shown. Printing the agent's raw JSON into the chat
        // buried answers under diagnostics ("ACP system: {…}"), and a
        // wedged agent is already visible: its turn never ends and the
        // composer keeps spinning.
        console.debug(`[acp] unmodelled ${msg.method}:`, msg.reason);
        break;
      // State the agent owns rather than transcript content. Captured so it
      // is available to the model/mode pickers, and logged so none of it is
      // invisible while that wiring lands.
      case 'acp_modes':
        // State, not a ref: these drive the picker and mode row, so the
        // transcript must re-render when they arrive.
        setAcpModes({ current: msg.current, available: msg.available, postures: msg.postures });
        if (capsDriverRef.current) {
          saveAgentCaps(capsDriverRef.current, { modes: { current: msg.current, available: msg.available } });
        }
        break;
      case 'acp_config_options':
        setAcpConfig(msg.options);
        if (capsDriverRef.current) saveAgentCaps(capsDriverRef.current, { config: msg.options });
        break;
      case 'acp_commands':
        setAcpCommands(msg.names);
        break;
      case 'acp_usage':
        setAcpUsage(msg);
        break;
      case 'acp_turn_usage':
        setEntries((prev) => addTurnUsage(prev, msg));
        break;
      case 'acp_limits':
        setAcpLimits(msg.windows);
        break;
      case 'acp_session_info':
        // The agent retitled itself; the sidebar reads its own title source,
        // so just nudge a refresh when one arrived.
        if (msg.title) setSidebarRefresh((n) => n + 1);
        break;

      case 'acp_agent_started':
        // Startup outcome. Surfaced as a warning on failure so a user learns
        // "grok isn't installed" instead of watching an empty pane.
        if (msg.error) {
          setAcpError(`${msg.display_name}: ${msg.error}`);
          setEntries((prev) => [
            ...prev,
            { kind: 'warning', text: `${msg.display_name}: ${msg.error}` },
          ]);
        } else {
          setAcpError(null);
        }
        break;
      case 'session_engine': {
        // A switch the user made (or an agent exiting) marks the
        // transcript, so the reader can see where one engine handed off
        // to the other. Status-only transitions (starting → ready) don't.
        const prev = engineRef.current;
        const next = msg.engine;
        const moved =
          prev != null &&
          (prev.kind !== next.kind ||
            (next.kind === 'agent' ? prev.driver !== next.driver : false));
        if (moved) {
          setEntries((es) => [...es, { kind: 'engine_switch', engine: next }]);
        }
        // The sidebar badges each chat by its engine.
        if (moved || prev?.model !== next.model) setSidebarRefresh((n) => n + 1);
        applyEngine(next);
        break;
      }
      case 'acp_agent_status':
        setAcpAgents(msg.agents);
        setAcpStatusPending(false);
        console.debug('[acp] agent status', msg.agents.map((a) => `${a.display_name}:${a.state.state}`));
        break;

      default: {
        // A frame type this build does not know about. Logged rather than
        // ignored: ACP's spec under-documents `SessionUpdate` and vendors
        // send extensions, so an unhandled type must be greppable instead of
        // looking like a hung agent.
        const unknown = msg as { type?: string };
        console.warn('[ws] unhandled server message type:', unknown.type);
        break;
      }
    }
  }

  function replyToPlan(callId: string, approved: boolean, steps?: PlanStep[], note?: string) {
    wsRef.current?.send({
      type: 'prompt_response',
      prompt_id: callId,
      kind: 'plan',
      approved,
      steps,
      note,
    });
    // Record the decision locally so the card switches to its resolved state
    // immediately, without waiting for tool_end to round-trip.
    setEntries((prev) => recordPlanDecision(prev, callId, { approved, steps, note }));
  }

  /** Send answers (or a skip) for an `ask_user` prompt back to the
   *  server and flip the card into its resolved state locally. */
  function replyToAskUser(callId: string, decision: AskUserDecision) {
    if (decision.cancelled) {
      wsRef.current?.send({
        type: 'prompt_response',
        prompt_id: callId,
        kind: 'ask_user',
        answers: [],
        cancelled: true,
      });
    } else {
      wsRef.current?.send({
        type: 'prompt_response',
        prompt_id: callId,
        kind: 'ask_user',
        answers: decision.answers,
        cancelled: false,
      });
    }
    setEntries((prev) => recordAskUserDecision(prev, callId, decision));
  }

  /** Answer a subagent's review-required prompt. Clears `pendingReview`
   *  locally so the SubagentPanel immediately drops the review card; the
   *  backend's tool_end will land shortly after with the final result. */
  function replyToSubagentReview(
    parentCallId: string,
    promptId: string,
    approved: boolean,
    note?: string,
  ) {
    wsRef.current?.send({
      type: 'prompt_response',
      prompt_id: promptId,
      kind: 'subagent_review',
      approved,
      note,
    });
    setSubagentState((prev) => updateSubagent(prev, parentCallId, (s) => ({
      ...s,
      pendingReview: null,
    })));
  }

  async function runReview(args: string) {
    // Kicks off the server run; the `review_started` frame that comes back
    // over WS opens the panel + wipes prior state, so we don't seed the
    // ReviewState here.
    try {
      await startReview({ range: args || undefined });
    } catch (e) {
      const text = (e as Error).message;
      // If the POST itself failed there's no run_id — synthesize a state
      // so the panel opens and shows the error rather than swallowing it.
      setReviewState({
        runId: 'local-error',
        status: 'Review failed',
        progressPct: null,
        findings: null,
        verdicts: [],
        error: text,
        done: true,
      });
      setReviewPanelOpen(true);
    }
  }

  /** Kick off a review of a remote GitHub PR. Diff comes from the REST API
   *  using the stored `GITHUB_TOKEN`; stage-2 verify is skipped because the
   *  files referenced by the diff live on GitHub, not in the session cwd. */
  async function runPrReview(owner: string, repo: string, number: number) {
    try {
      await startReview({ owner, repo, pr: number, no_verify: true });
    } catch (e) {
      const text = (e as Error).message;
      setReviewState({
        runId: 'local-error',
        status: 'Review failed',
        progressPct: null,
        findings: null,
        verdicts: [],
        error: text,
        done: true,
      });
      setReviewPanelOpen(true);
    }
  }

  function decideApproval(callId: string, allow: boolean, scope: ApprovalScope = 'once') {
    wsRef.current?.send({ type: 'approve', call_id: callId, allow, scope });
    // An agent's request is only the question; the agent's own tool card
    // shows the call running. Keeping the request would leave a second
    // card that never finishes.
    const decided = entries.find((e) => e.kind === 'tool' && e.call.id === callId);
    if (decided && decided.kind === 'tool' && isAgentRequest(decided.call)) {
      setEntries((prev) => prev.filter((e) => !(e.kind === 'tool' && e.call.id === callId)));
      return;
    }
    setEntries((prev) => updateTool(prev, callId, (t) => ({
      ...t,
      status: allow ? 'running' : 'denied',
    })));
  }

  // Tool calls waiting for the user's Y/N decision. The Allow / Deny /
  // Always-allow buttons render inline on the pending tool card in
  // the transcript. Kept oldest-first — the top of the queue is what
  // the Y/N global shortcut targets.
  const pendingApprovals = useMemo<PendingApproval[]>(
    () =>
      entries
        .filter((e): e is Extract<Entry, { kind: 'tool' }> => e.kind === 'tool' && e.status === 'pending')
        .map((e) => ({ callId: e.call.id, call: e.call, preview: e.preview, needs: e.needs })),
    [entries],
  );

  /** The canonical postures mapped onto this agent's modes. Empty when the
   *  agent advertised nothing we recognize — then the dialog offers only
   *  Deny / Allow once rather than inventing options. */
  const agentPostures = useMemo(() => {
    if (!acpModes) return [];
    // Server-computed mapping wins: the posture vocabulary lives in the
    // protocol, so the client never pattern-matches agent mode ids. The
    // local mapping remains as a fallback for older servers.
    if (acpModes.postures?.length) {
      return acpModes.postures.flatMap((m) => {
        const posture = POSTURES.find((p) => p.key === m.key);
        if (!posture) return [];
        return [{
          posture,
          modeId: m.mode_id,
          modeName: m.mode_name,
          modeDescription: m.mode_description ?? null,
          current: m.current,
        }];
      });
    }
    return mapPosturesToModes(acpModes.available, acpModes.current);
  }, [acpModes]);
  /** The composer's usage ring, for whichever engine drives the chat. An
   *  agent reports its own context, cost and plan limits; a provider chat
   *  is measured by Mira (context, auto-compact point, tokens, cost) with
   *  the provider's rate limits as its limits. */
  const usageRing = useMemo<UsageRingData>(() => {
    if (acpDriver) {
      const LIMIT_NAMES: Record<string, string> = {
        five_hour: '5-hour limit',
        seven_day: 'Weekly · all models',
        seven_day_opus: 'Weekly · Opus',
        seven_day_sonnet: 'Weekly · Sonnet',
      };
      return {
        used: acpUsage?.used ?? null,
        window: acpUsage?.size ?? null,
        compactAt: null,
        tokens: null,
        costUsd: acpUsage?.cost && acpUsage.cost.currency === 'USD' ? acpUsage.cost.amount : null,
        limitsTitle: acpLimits.length ? `Plan usage limits · ${engine?.display_name ?? 'agent'}` : null,
        limits: acpLimits.map((w) => ({
          label: LIMIT_NAMES[w.name] ?? w.name.replace(/_/g, ' '),
          used: w.utilization,
          resetsAt: w.resets_at ? w.resets_at * 1000 : null,
        })),
        onCompact: () => compactAcpAgent(),
        onInspect: () => setInspectOpen(true),
      };
    }
    const rl = rateLimit?.rate_limit;
    const limits = rl
      ? (['requests', 'tokens', 'input_tokens', 'output_tokens'] as const).flatMap((k) => {
          const b = rl[k];
          if (!b || b.limit == null || b.remaining == null || b.limit === 0) return [];
          return [{
            label: k === 'requests' ? 'Requests' : k === 'tokens' ? 'Tokens' : k === 'input_tokens' ? 'Input tokens' : 'Output tokens',
            used: 1 - b.remaining / b.limit,
            resetsAt: b.reset_secs != null ? rateLimit!.at + b.reset_secs * 1000 : null,
          }];
        })
      : [];
    return {
      used: providerContext?.used ?? null,
      window: providerContext?.window ?? null,
      compactAt: providerContext?.compactAt ?? null,
      tokens: usage && (usage.prompt_tokens || usage.completion_tokens)
        ? { prompt: usage.prompt_tokens, completion: usage.completion_tokens, cached: usage.cached_input_tokens }
        : null,
      costUsd: usage ? costUsd(model, usage) : null,
      limitsTitle: limits.length ? 'Provider rate limits' : null,
      limits,
      onCompact: () => onCompact(''),
      onInspect: () => setInspectOpen(true),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [acpDriver, acpUsage, acpLimits, engine, rateLimit, providerContext, usage, model]);

  // Plan proposal waiting for the user to approve/cancel. Rendered in
  // the Composer rather than inline so the interactive card doesn't
  // scroll away in a long transcript.
  const pendingPlan = useMemo(() => {
    const e = entries.find(
      (e): e is Extract<Entry, { kind: 'tool' }> =>
        e.kind === 'tool' && !!e.plan && e.plan.decision === null,
    );
    return e ? { callId: e.call.id, proposal: e.plan!.proposal } : null;
  }, [entries]);

  // ask_user proposal waiting for answers. Same pattern as pendingPlan.
  const pendingAskUser = useMemo(() => {
    const e = entries.find(
      (e): e is Extract<Entry, { kind: 'tool' }> =>
        e.kind === 'tool' && !!e.askUser && e.askUser.decision === null,
    );
    return e ? { callId: e.call.id, proposal: e.askUser!.proposal } : null;
  }, [entries]);

  // Global approval shortcuts for the first pending approval, resolved
  // through the keybinding engine (Settings → Keybindings). Rebinds when
  // the head-of-queue call changes so back-to-back approvals each
  // pick up their own listener. Skipped while the user is typing so
  // approval keys in the composer/settings don't fire the decision.
  const firstPendingCallId = pendingApprovals[0]?.callId ?? null;
  useEffect(() => {
    if (!firstPendingCallId) return;
    function onKey(e: KeyboardEvent) {
      const t = e.target as HTMLElement | null;
      if (t) {
        const tag = t.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA') return;
        if (t.isContentEditable) return;
      }
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (command === 'approval.accept') {
        e.preventDefault();
        decideApproval(firstPendingCallId, true);
        return;
      }
      if (command === 'approval.reject') {
        e.preventDefault();
        decideApproval(firstPendingCallId, false);
        return;
      }
      // Legacy fallback: plain y/n (any case) still decides, unless the
      // user rebound that command in Settings → Keybindings.
      if (!e.metaKey && !e.ctrlKey && !e.altKey) {
        const lower = e.key.toLowerCase();
        if (lower === 'y' || lower === 'n') {
          const customized = getCustomKeybindingRules().some(
            (r) => r.command === (lower === 'y' ? 'approval.accept' : 'approval.reject'),
          );
          if (!customized) {
            e.preventDefault();
            decideApproval(firstPendingCallId, lower === 'y');
          }
        }
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [firstPendingCallId, keybindings]);

  function onSend(text: string, images?: { media_type: string; data: string }[]) {
    // Belt-and-suspenders — the composer isn't visible on non-chat views,
    // but a keyboard-driven send would still land the message and it should
    // pull the user back to the transcript.
    setMainView('chat');
    // Refresh the sidebar immediately so a brand-new thread shows up in
    // the projects list on the first send, not after the model finishes
    // responding. The harness checkpoints the pushed user message at the
    // top of `run_loop` so this refetch sees the new row.
    setSidebarRefresh((n) => n + 1);
    followRef.current = true;
    setShowJump(false);
    const now = Date.now();
    turnStartRef.current = now;
    setEntries((prev) => {
      const next: Entry[] = [...prev, { kind: 'msg', msg: { role: 'user', content: text, images } }];
      const turnIndex = countUserMessages(next) - 1;
      startTurnUsage(turnIndex);
      setTurnTimings((tt) => {
        const clone = new Map(tt);
        clone.set(turnIndex, { startedAt: now, endedAt: null });
        return clone;
      });
      return next;
    });
    setBusy(true);
    setThinking(true);
    // The server routes by the session's engine: a session on an agent
    // sends this to the agent (starting it if needed, with the
    // conversation so far if it just took over), otherwise to the
    // provider. One message type, so a prompt can never go to the wrong
    // engine because this client's view lagged the server's.
    wsRef.current?.send({ type: 'send', text, images });
  }

  /** Edit & resend (or retry, with the same text) the user message at
   *  `userIdx`: the server rewinds history to just before it and starts a
   *  new turn. Later entries are dropped here to match. */
  /** How the server identifies a user message: its text, and which match
   *  of it counting from the latest. Shared by edit and restore so they can
   *  never disagree about which message is meant. */
  function messageRefAt(userIdx: number): MessageRef | null {
    const target = entries[userIdx];
    if (!target || target.kind !== 'msg' || target.msg.role !== 'user') return null;
    const text = target.msg.content ?? '';
    const occurrence = entries
      .slice(userIdx + 1)
      .filter((e) => e.kind === 'msg' && e.msg.role === 'user' && e.msg.content === text).length;
    return { text, occurrence };
  }

  // Restore files to before a message: preview → confirm → restore, with an
  // Undo afterwards.
  const [restoreAsk, setRestoreAsk] = useState<{ ref: MessageRef; changes: RestoreChange[] } | null>(null);
  const [restoreNote, setRestoreNote] = useState<
    { text: string; undo?: string; error?: boolean } | null
  >(null);
  async function askRestore(userIdx: number) {
    const ref = messageRefAt(userIdx);
    if (!ref || busy) return;
    try {
      const changes = await previewCheckpoint(ref);
      if (changes.length === 0) {
        setRestoreNote({ text: 'Nothing to restore — the files already match.' });
        return;
      }
      setRestoreAsk({ ref, changes });
    } catch (e) {
      setRestoreNote({ text: (e as Error).message, error: true });
    }
  }
  async function doRestore(run: () => Promise<Restored>, verb: string) {
    try {
      const r = await run();
      const n = r.changes.length;
      setRestoreNote({ text: `${verb} ${n} file${n === 1 ? '' : 's'}.`, undo: verb === 'Restored' ? r.undo : undefined });
    } catch (e) {
      setRestoreNote({ text: (e as Error).message, error: true });
    } finally {
      refreshRepo();
    }
  }

  function onResend(userIdx: number, text: string) {
    const ref = messageRefAt(userIdx);
    const target = entries[userIdx];
    if (busy || !ref || !target || target.kind !== 'msg') return;
    const { text: original, occurrence } = ref;
    followRef.current = true;
    setShowJump(false);
    const next: Entry[] = [
      ...entries.slice(0, userIdx),
      { kind: 'msg', msg: { role: 'user', content: text, images: target.msg.images } },
    ];
    const turnIndex = countUserMessages(next) - 1;
    startTurnUsage(turnIndex);
    setTurnUsage((prev) => new Map([...prev].filter(([i]) => i < turnIndex)));
    setEntries(next);
    setTurnTimings((tt) => {
      const clone = new Map([...tt].filter(([i]) => i < turnIndex));
      turnStartRef.current = Date.now();
      clone.set(turnIndex, { startedAt: turnStartRef.current, endedAt: null });
      return clone;
    });
    setBusy(true);
    setThinking(true);
    wsRef.current?.send({ type: 'resend', original, occurrence, text });
  }

  // Stable identities for everything handed to the transcript, so a
  // streamed token re-renders only the turn it lands in.
  const stableToggleTurn = useStableCallback((idx: number) => toggleTurn(idx));
  const stableDecide = useStableCallback(decideApproval);
  const stablePlanReply = useStableCallback(replyToPlan);
  const stableAskUserReply = useStableCallback(replyToAskUser);
  const stableOpenAgent = useStableCallback(openAgentTab);
  const stableOpenFile = useStableCallback(openFileTab);
  const stableSetMode = useStableCallback(onSetMode);
  const editMessage = useStableCallback((entry: Entry, text: string) => onResend(entries.indexOf(entry), text));
  const restoreMessage = useStableCallback((entry: Entry) => void askRestore(entries.indexOf(entry)));
  const retryMessage = useStableCallback((entry: Entry) => {
    const at = entries.indexOf(entry);
    for (let i = at - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.kind === 'msg' && e.msg.role === 'user') {
        onResend(i, e.msg.content ?? '');
        return;
      }
    }
  });
  // Changes only with `busy`: every message's action row reads this, and a
  // value that changed per token would re-render all of them.
  const messageActions = useMemo<MessageActions>(
    () => ({ busy, openImage: setLightbox, edit: editMessage, restore: restoreMessage, retry: retryMessage }),
    [busy, editMessage, restoreMessage, retryMessage],
  );

  function onSetMode(m: Mode) { wsRef.current?.send({ type: 'set_mode', mode: m }); }
  /** Switch an external agent's own session mode. Separate from Mira's
   *  `set_mode`: different system, and the ids are the agent's. */
  function onSetAcpMode(modeId: string, acknowledgePrivileged = false) {
    wsRef.current?.send({
      type: 'acp_set_mode',
      mode_id: modeId,
      acknowledge_privileged: acknowledgePrivileged,
    });
  }
  function onSetModel(m: string, instance?: string | null) {
    // With an agent driving, a pick from the list is the *agent's* model, not
    // Mira's: it goes out as the agent's model config option. Sending
    // `set_model` here changed Mira's model behind the agent's back while the
    // agent kept running whatever it was running — the picker lied.
    if (acpDriver) {
      // Not yet running: the server remembers the pick and launches with it.
      wsRef.current?.send({ type: 'acp_set_config_option', option_id: acpModelOption?.id ?? 'model', value: m });
      return;
    }
    wsRef.current?.send({ type: 'set_model', model: m, instance: instance ?? null });
  }
  function onSetModelOption(id: string, value: string) {
    // With an ACP agent driving the session, the option ids are the *agent's*
    // (`model`, `thought_level`, …) and Mira's server would ignore them.
    if (acpDriver) {
      wsRef.current?.send({ type: 'acp_set_config_option', option_id: id, value });
      return;
    }
    wsRef.current?.send({ type: 'set_model_option', id, value });
  }

  /** Kick off (or replace) an autonomous run against a condition. The
   *  server broadcasts `goal_set` so the panel state syncs there — we
   *  don't set it locally to avoid a brief drift if the server rejects
   *  the request (e.g. empty condition). */
  function onSetGoal(condition: string, maxIterations?: number) {
    const trimmed = condition.trim();
    if (!trimmed) return;
    wsRef.current?.send({
      type: 'set_goal',
      condition: trimmed,
      max_iterations: maxIterations ?? null,
    });
  }

  function onClearGoal() {
    wsRef.current?.send({ type: 'clear_goal' });
  }

  function onCompact(focus: string) {
    wsRef.current?.send({ type: 'compact', focus: focus || null });
    setEntries((prev) => [...prev, { kind: 'warning', text: '[context] summarizing the conversation…' }]);
  }

  async function onNewChat() {
    // The new chat inherits this one's engine on the server — provider and
    // model, or agent and its settings — and its `ready` reports it, so
    // the composer carries on exactly as it was. Nothing to reset here.
    try {
      const { id } = await newSession();
      // The server just built a fresh slot for `id`, marked it active,
      // and published a Ready on ITS channel. Our WS forwarder is still
      // subscribed to the previous slot — attach so we start receiving
      // the new slot's frames (Ready + subsequent tokens).
      wsRef.current?.attach(id);
    } catch (e) {
      setEntries((prev) => [...prev, { kind: 'error', text: `new chat: ${(e as Error).message}` }]);
    }
  }

  // Global app shortcuts, resolved through the keybinding engine
  // (Settings → Keybindings). Never fires while typing — single-key and
  // mod bindings alike yield to inputs, textareas and editable regions.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const t = e.target as HTMLElement | null;
      if (t) {
        const tag = t.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || t.isContentEditable) return;
      }
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (!command) return;
      switch (command) {
        case 'chat.new':
          e.preventDefault();
          void onNewChat();
          break;
        case 'review.toggle':
          e.preventDefault();
          setReviewOpen((v) => !v);
          break;
        case 'sidebar.toggle':
          e.preventDefault();
          setSidebarOpen((v) => !v);
          break;
        case 'settings.toggle':
          e.preventDefault();
          if (mainView === 'settings') exitSettings();
          else openSettings();
          break;
        default:
          break;
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keybindings, mainView]);

  /** Everything ⌘K can do. Each entry runs exactly what its button or
   *  shortcut does, so the palette never drifts from the rest of the app. */
  const keyFor = (c: Parameters<typeof shortcutLabelForCommand>[1]) => shortcutLabelForCommand(keybindings, c);
  const goSettings = (id: import('./components/Settings').SettingsSectionId) => () => {
    openSettings();
    setSettingsSection(id);
  };
  const paletteActions: PaletteAction[] = [
    { id: 'new', group: 'Chat', label: 'New chat', icon: MessageSquarePlus, shortcut: keyFor('chat.new'), run: () => void onNewChat() },
    { id: 'folder', group: 'Chat', label: 'Open folder…', icon: FolderOpen, keywords: ['project', 'cwd', 'directory'], run: () => setPickerOpen(true) },
    {
      id: 'model', group: 'Chat', label: acpDriver ? 'Switch model or agent…' : 'Switch model…', icon: Cpu,
      keywords: ['provider', 'agent', 'claude', 'codex', 'engine'],
      run: () => { setMainView('chat'); window.dispatchEvent(new Event('mira:open-model-picker')); },
    },
    { id: 'changes', group: 'Chat', label: 'Review changes', icon: FileDiff, shortcut: keyFor('review.toggle'), keywords: ['diff', 'git'], run: () => setReviewOpen(true) },
    { id: 'context', group: 'Chat', label: "What's in the context window", icon: Brain, keywords: ['tokens', 'compact', 'inspector'], run: () => setInspectOpen(true) },
    { id: 'review', group: 'Chat', label: 'Ask Iris to review the changes', icon: ScanSearch, keywords: ['code review', 'second opinion', 'reviewer'], run: () => void runReview('') },
    { id: 'sidebar', group: 'View', label: 'Toggle sidebar', icon: PanelLeftClose, shortcut: keyFor('sidebar.toggle'), run: () => setSidebarOpen((v) => !v) },
    { id: 'terminal', group: 'View', label: 'Toggle terminal', icon: SquareTerminal, shortcut: keyFor('terminal.toggle'), run: () => setTerminal(!terminalOpen) },
    { id: 'browser', group: 'View', label: 'Open browser', icon: Globe2, keywords: ['chrome', 'web'], run: () => { setMainView('chat'); openToolPane('browser'); } },
    { id: 'whiteboard', group: 'View', label: 'Open whiteboard', icon: Pencil, keywords: ['sketch', 'draw'], run: () => { setMainView('chat'); openToolPane('whiteboard'); } },
    { id: 's-general', group: 'Settings', label: 'General settings', icon: Cog, shortcut: keyFor('settings.toggle'), run: goSettings('general') },
    { id: 's-appearance', group: 'Settings', label: 'Appearance', icon: Palette, keywords: ['theme', 'light', 'dark', 'motion'], run: goSettings('appearance') },
    {
      id: 'theme-toggle', group: 'View',
      label: resolveTheme() === 'dark' ? 'Switch to light theme' : 'Switch to dark theme',
      icon: resolveTheme() === 'dark' ? Sun : Moon,
      keywords: ['theme', 'appearance', 'light', 'dark', 'mode'],
      run: () => setThemePref(resolveTheme() === 'dark' ? 'light' : 'dark'),
    },
    { id: 's-provider', group: 'Settings', label: 'Providers & API keys', icon: Plug, run: goSettings('provider') },
    { id: 's-agents', group: 'Settings', label: 'External agents', icon: Bot, keywords: ['claude code', 'codex'], run: goSettings('agents') },
    { id: 'import-chats', group: 'Chat', label: 'Import chats from Claude Code or Codex', icon: Download, keywords: ['history', 'migrate', 'bring'], run: () => setImportOpen(true) },
    { id: 's-subagents', group: 'Settings', label: 'Subagents', icon: Smile, keywords: ['scout', 'iris', 'atlas', 'bolt', 'quill', 'sentry', 'faces'], run: goSettings('subagents') },
    { id: 's-usage', group: 'Settings', label: 'Usage & cost', icon: ChartColumn, keywords: ['tokens', 'spend', 'limits'], run: goSettings('usage') },
    { id: 's-memory', group: 'Settings', label: 'Memory', icon: Brain, run: goSettings('memory') },
    { id: 's-skills', group: 'Settings', label: 'Skills', icon: Sparkles, run: goSettings('skills') },
    { id: 's-hooks', group: 'Settings', label: 'Hooks', icon: Zap, run: goSettings('hooks') },
    { id: 's-keys', group: 'Settings', label: 'Keyboard shortcuts', icon: Keyboard, keywords: ['keybindings'], run: goSettings('keybindings') },
  ];

  const settingsHandler = (v: SettingsView) => {
    setConfigured(v.configured);
    setProviderName(v.default_provider ?? null);
    if (v.default_model) setModel(v.default_model);
    // Backend list for the pickers — native providers + external agents
    // with health + catalogs. Failures degrade to an empty list.
    listEngines().then((v) => setEngines(v.engines)).catch(() => {});
    if (v.default_mode) setMode(v.default_mode as Mode);
  };

  const isEmpty = useMemo(
    () => entries.every((e) => e.kind === 'msg' && !(e.msg.content || '').trim() && (e.msg.tool_calls?.length ?? 0) === 0),
    [entries],
  );

  const turns = useMemo(() => groupByTurn(entries), [entries]);

  // Second opinion: after an external agent's turn that changed files,
  // offer a review by Mira's own reviewer. One offer per turn; dismissing
  // or accepting it retires it.
  const [secondOpinionSeen, setSecondOpinionSeen] = useState<Set<string>>(() => new Set());
  const lastTurnIdx = turns.length - 1;
  const secondOpinionKey = `${sessionId}:${lastTurnIdx}`;
  const lastTurnEdited = useMemo(() => {
    const last = turns[turns.length - 1];
    return !!last?.body.some((e) => {
      if (e.kind !== 'tool' || e.status === 'denied') return false;
      const cat = categoryFor(e.call.function.name);
      return cat === 'write' || cat === 'edit';
    });
  }, [turns]);
  const showSecondOpinion =
    !!acpDriver && !busy && lastTurnIdx >= 0 && lastTurnEdited &&
    sessionDiff.files.length > 0 && !secondOpinionSeen.has(secondOpinionKey);
  const retireSecondOpinion = () =>
    setSecondOpinionSeen((prev) => new Set(prev).add(secondOpinionKey));

  // Timeline minimap items: one per turn opened by a user message.
  // Assistant excerpt = first assistant text in the turn body.
  const minimapItems = useMemo<MinimapItem[]>(() => {
    const out: MinimapItem[] = [];
    turns.forEach((turn, i) => {
      if (turn.user?.kind !== 'msg' || turn.user.msg.role !== 'user') return;
      const { text } = parseSentAttachments(turn.user.msg.content ?? '');
      const assistant = turn.body.find(
        (e): e is Extract<Entry, { kind: 'msg' }> =>
          e.kind === 'msg' && e.msg.role === 'assistant' && !!(e.msg.content ?? '').trim(),
      );
      out.push({
        id: `turn-${i}`,
        userText: text.trim(),
        assistantText: assistant && assistant.kind === 'msg' ? (assistant.msg.content ?? null) : null,
      });
    });
    return out;
  }, [turns]);

  // Jump the transcript pane to a minimap turn.
  const jumpToMinimapTurn = useCallback((id: string) => {
    const pane = paneRef.current;
    if (!pane) return;
    const node = pane.querySelector(`[data-minimap-id="${id}"]`);
    if (!(node instanceof HTMLElement)) return;
    const delta = node.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    pane.scrollTo({ top: pane.scrollTop + delta - 12, behavior: 'smooth' });
  }, []);
  // Keep the transcript and composer clear of the context card only while
  // it's open; the collapsed pill floats over the corner.
  const ctxHasContent =
    ctxFits &&
    contextPanelHasContent({
      tasks,
      gitStatus,
      sessionDiff,
      branchPr,
      sessionCommitted,
      subagentState,
      entries,
    });
  // Beside the stream when the chat column has room for both; over it
  // (stacked, nothing reserved) when reserving would crush the transcript.
  const ctxRoomy = chatColWidth === 0 || chatColWidth >= CONTEXT_PANEL_RESERVE + MIN_STREAM_WIDTH;
  const ctxReserve = ctxOpen && ctxHasContent && ctxRoomy;
  // The collapsed pill floats over the top-right corner; drop the first
  // line of the transcript below it rather than under it.
  const ctxPill = !ctxOpen && ctxHasContent;

  /** Look up each open agent tab's tool entry so status/result stay live
   *  as tool_end frames arrive. Tabs whose backing entry has been wiped
   *  (e.g. session load replaced history) silently drop. */
  const subagentTabs: SubagentTab[] = useMemo(() => {
    const byCallId = new Map<string, ToolEntry>();
    for (const e of entries) {
      if (e.kind === 'tool') byCallId.set(e.call.id, e);
    }
    return agentTabs
      .map((callId) => {
        const entry = byCallId.get(callId);
        if (!entry) return null;
        const stream = subagentState.get(callId);
        return {
          callId,
          call: entry.call,
          status: entry.status,
          result: entry.result,
          streamEntries: stream?.entries ?? [],
          streamDone: stream?.done ?? false,
          pendingReview: stream?.pendingReview ?? null,
        };
      })
      .filter((t): t is SubagentTab => t !== null);
  }, [agentTabs, entries, subagentState]);

  function openAgentTab(callId: string) {
    setAgentTabs((prev) => (prev.includes(callId) ? prev : [...prev, callId]));
    setActiveAgentTab(callId);
    // Fire-and-forget: if we don't already have a live stream for this
    // agent (fresh page after a reload, or a resumed session), pull the
    // child's persisted history and reconstruct the transcript.
    void hydrateSubagentIfNeeded(callId);
  }

  /** If subagentState has no entries for `callId`, extract the child
   *  session id from the parent's tool_result marker and fetch the
   *  child's history from the backend. Populates subagentState so the
   *  panel body renders the full timeline. No-op when the marker is
   *  missing (older sessions before the marker landed) or when a live
   *  stream already exists. */
  async function hydrateSubagentIfNeeded(callId: string) {
    // Bail if we already have a stream in flight or on record for this call.
    const existing = subagentState.get(callId);
    if (existing && existing.entries.length > 0) return;
    // Find the parent's tool entry to read the tool_result content.
    const entry = entries.find(
      (e): e is ToolEntry => e.kind === 'tool' && e.call.id === callId,
    );
    if (!entry?.result) return;
    const agentId = extractAgentId(entry.result.content);
    if (!agentId) return;
    try {
      const view = await getSessionHistory(agentId);
      const rebuilt = historyToEntries(view.messages, view.previews);
      setSubagentState((prev) => updateSubagent(prev, callId, (s) => ({
        ...s,
        agentId,
        model: view.model,
        entries: rebuilt,
        done: true,
      })));
    } catch (e) {
      // Non-fatal: panel falls back to the summary block. Log so
      // developers see the failure without breaking the user's flow.
      console.warn('subagent hydrate failed', callId, agentId, e);
    }
  }

  function closeAgentTab(callId: string) {
    setAgentTabs((prev) => {
      const next = prev.filter((id) => id !== callId);
      if (activeAgentTab === callId) {
        // Fall through to remaining agent tabs, then file/tool tabs, then null.
        const remaining = [
          ...next,
          ...fileTabs.map((t) => t.id),
          ...toolTabs.map((t) => t.id),
        ];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  function closeSubagentPanel() {
    setAgentTabs([]);
    setFileTabs([]);
    setToolTabs([]);
    setActiveAgentTab(null);
  }

  function openFileTab(ref: string, diff: DiffPreview | null, atLine?: number | null) {
    // `src/a.rs:42` is a file and a line, not a file named that.
    const { path, line: refLine } = splitFileRef(ref);
    const line = atLine ?? refLine;
    const reveal = Date.now();
    setFileTabs((prev) => {
      // If already open, update the diff (re-opening after a new write).
      if (prev.some((t) => t.id === path)) {
        return prev.map((t) => t.id === path ? { ...t, diff, line, reveal } : t);
      }
      return [...prev, { id: path, path, diff, line, reveal }];
    });
    setActiveAgentTab(path);
  }

  function closeFileTab(tabId: string) {
    setFileTabs((prev) => {
      const next = prev.filter((t) => t.id !== tabId);
      if (activeAgentTab === tabId) {
        const remaining = [
          ...agentTabs,
          ...next.map((t) => t.id),
          ...toolTabs.map((t) => t.id),
        ];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  function closeToolTab(tabId: string) {
    setToolTabs((prev) => {
      const next = prev.filter((t) => t.id !== tabId);
      if (activeAgentTab === tabId) {
        const remaining = [
          ...agentTabs,
          ...fileTabs.map((t) => t.id),
          ...next.map((t) => t.id),
        ];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  /**
   * Open (or focus) a utility pane. Singletons by kind, so asking twice just
   * re-focuses.
   *
   * The `new` launcher is a tab like any other, but it's retired the moment
   * you pick something — otherwise picking a pane leaves a stale "New" tab
   * sitting in the strip forever.
   */
  function openToolPane(kind: ToolPaneKind) {
    const id = toolPaneId(kind);
    setToolTabs((prev) => {
      const withoutLauncher =
        kind === 'new' ? prev : prev.filter((t) => t.kind !== 'new');
      return withoutLauncher.some((t) => t.id === id)
        ? withoutLauncher
        : [...withoutLauncher, { id, kind, title: TOOL_PANE_DEFS[kind].title }];
    });
    setActiveAgentTab(id);
  }

  // Show the browser when Mira or an agent starts using it (Settings →
  // General → Browser). Only for a live call — reloading a chat with old
  // browser calls must not pop the pane — and once per turn, so closing the
  // pane sticks until the next message.
  const browserShownFor = useRef<string | null>(null);
  useEffect(() => {
    let live = false;
    let turnStart = -1;
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.kind === 'msg' && e.msg.role === 'user') {
        turnStart = i;
        break;
      }
      if (e.kind === 'tool' && e.call.function.name === 'browser' && (e.status === 'running' || e.status === 'pending')) {
        live = true;
      }
    }
    if (!live) return;
    const turn = `${sessionId}:${turnStart}`;
    if (browserShownFor.current === turn) return;
    browserShownFor.current = turn;
    if (getBoolPref(PREF_KEYS.browserAutoOpen, true)) openToolPane('browser');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entries]);

  function closeAnyTab(id: string) {
    if (agentTabs.includes(id)) closeAgentTab(id);
    else if (isToolPaneId(id)) closeToolTab(id);
    else closeFileTab(id);
  }

  /** Whiteboard "Send": hand the drawing to the composer as an image
   *  attachment so the model can actually see it. */
  async function sendWhiteboardToChat(pngDataUrl: string) {
    const file = await dataUrlToFile(pngDataUrl, 'whiteboard.png');
    attachFilesToComposer(file);
  }

  function handlePanelResizeStart(e: React.MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = rightPanelWidth;
    function onMove(ev: MouseEvent) {
      // Dragging left increases panel width (panel is on the right side).
      const next = Math.max(280, Math.min(800, startWidth + (startX - ev.clientX)));
      setRightPanelWidth(next);
    }
    function onUp() {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    }
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }

  const panelOpen =
    subagentTabs.length > 0 || fileTabs.length > 0 || toolTabs.length > 0;

  /** True in the desktop app on macOS, where the native title bar is gone
   *  and our own header has to stand in for it. */
  const hiddenTitleBar = hasHiddenTitleBar();

  function toggleTurn(idx: number) {
    setExpandedTurns((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx); else next.add(idx);
      return next;
    });
  }

  const sidebarCol = sidebarOpen ? '300px' : '0px';
  const rightCol = panelOpen ? `${rightPanelWidth}px` : '0px';

  return (
    <div
      // Window wash ( `--color-sidebar`): the sidebar
      // sits full-bleed on it; the main + right columns float as inset
      // rounded cards (`--color-panel`).
      // With the native title bar hidden, the window is transparent and the
      // OS paints vibrancy behind it. The grid's own wash has to be
      // transparent too or it would cover that; the sidebar supplies a
      // translucent tint over the vibrancy, and the main and right columns
      // stay opaque panels so body text never sits on wallpaper.
      className={cn(
        'grid h-screen grid-rows-1 transition-[grid-template-columns] duration-150',
        hiddenTitleBar ? 'bg-transparent' : 'bg-panel',
      )}
      style={{ gridTemplateColumns: `${sidebarCol} minmax(0,1fr) ${rightCol}` }}
    >
      {/* overflow-hidden clips sidebar content when the grid column animates to 0 */}
      <div className="overflow-hidden">
        <Sidebar
          status={status}
          cwd={cwd}
          activeSessionId={sessionId}
          activeBusy={busy}
          refreshKey={sidebarRefresh}
          activeView={mainView}
          onNavigate={setMainView}
          onNewChat={async () => {
            setMainView('chat');
            await onNewChat();
          }}
          onOpenSettings={() => openSettings()}
          onOpenPicker={() => setPickerOpen(true)}
          onSessionLoaded={() => { /* Ready broadcast refreshes + jumps to chat */ }}
          onAttachSession={(id) => wsRef.current?.attach(id)}
          onSetBackgroundMode={async (id, mode) => {
            await setSessionBackgroundMode(id, mode);
            setSidebarRefresh((n) => n + 1);
          }}
          settingsSection={settingsSection}
          onSettingsSectionChange={setSettingsSection}
          onExitSettings={exitSettings}
        />
      </div>

      {/* Gutters are asymmetric: 2px on the sidebar edge vs 8px elsewhere,
          so the chat column reads as pulled toward the sidebar without
          touching it. The right panel keeps the full 8px on its outer edge. */}
      <main className="flex min-h-0 min-w-0 flex-col py-2 pl-0.5 pr-2">
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-background">
        {mainView === 'chat' && (
          <>
            {/* Drag region: with the native title bar hidden this row is
                the natural place to move the window, and `Overlay` needs at
                least one or the window can't be dragged at all. */}
            <div
              data-tauri-drag-region
              className="flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4"
            >
              {/* `data-tauri-drag-region` matches ancestors, so anything
                  interactive inside the header would drag the window on
                  mousedown. Opting these out keeps the clicks landing. */}
              <button
                data-tauri-drag-region="false"
                type="button"
                onClick={() => setSidebarOpen((v) => !v)}
                title={sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'}
                className={cn(
                  'shrink-0 rounded p-1.5 transition-colors',
                  sidebarOpen
                    ? 'text-foreground/70 hover:bg-accent hover:text-foreground'
                    : 'text-muted-foreground/50 hover:bg-accent hover:text-foreground',
                )}
              >
                <PanelLeft className="size-4" />
              </button>
              <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">
                {sessionTitle ?? titleFromEntries(entries)}
              </span>
              {(sessionDiff.uncommitted ?? 0) > 0 && (
                <button
                  type="button"
                  data-tauri-drag-region="false"
                  onClick={() => setReviewOpen(true)}
                  title="Review this session's changes"
                  // h-8 and rounded-lg to match the icon controls beside it.
                  // It used to be a `rounded-full` py-1 pill, which read as a
                  // different kind of control because it was both shorter
                  // than its neighbours and the only fully-pill shape in the
                  // row.
                  className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border/60 bg-secondary/40 px-2 text-[12.5px] text-muted-foreground transition-colors hover:border-border hover:bg-secondary hover:text-foreground"
                >
                  <span className="font-mono text-green-400/80">+{sessionDiff.added}</span>
                  <span className="font-mono text-red-400/80">−{sessionDiff.removed}</span>
                  <span className="hidden lg:inline">Review</span>
                </button>
              )}
              <div className="flex items-center gap-0.5" data-tauri-drag-region="false">
                <UpdateButton />
                <RightPanelButton
                  open={panelOpen}
                  activeKind={toolPaneKindOf(activeAgentTab ?? '')}
                  onOpen={() => openToolPane('new')}
                  onOpenPane={openToolPane}
                />
                <EditorPicker
                  cwd={cwd}
                  onOpenSettings={() => {
                    setSettingsSection('general');
                    openSettings();
                  }}
                />
                <div className="group relative">
                <button
                  type="button"
                  onClick={() => setTerminal(!terminalOpen)}
                  aria-label="Toggle terminal"
                  aria-pressed={terminalOpen}
                  className={cn(
                    'flex h-8 shrink-0 items-center rounded-lg px-1.5 transition-colors',
                    terminalOpen
                      ? 'border-mira-blue/50 bg-mira-blue/10 text-foreground'
                      : 'bg-secondary/40 text-muted-foreground hover:bg-secondary hover:text-foreground',
                  )}
                >
                  <span className="inline-flex size-6 items-center justify-center overflow-hidden rounded-md bg-fg/[0.04] ring-1 ring-fg/10">
                    <SquareTerminal className="size-3.5" strokeWidth={1.75} />
                  </span>
                </button>
                <span className="tooltip pointer-events-none absolute right-0 top-full z-30 mt-1.5 opacity-0 transition-opacity delay-300 group-hover:opacity-100">
                  Toggle terminal <span className="tooltip-tag">{shortcutLabelForCommand(keybindings, 'terminal.toggle') ?? (IS_MAC ? '⌘J' : 'Ctrl+J')}</span>
                </span>
                </div>
              </div>
            </div>

            {/* Transcript — full width, panel floats above it */}
            <div className="relative flex-1 min-h-0" ref={chatColRef}>
              <div
                className="absolute inset-0 overflow-y-auto px-5 pb-5 transition-[padding] duration-200"
                style={{
                  paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE + 12 : 20,
                  paddingTop: ctxPill ? 52 : 16,
                }}
                ref={paneRef}
                onScroll={onPaneScroll}
              >
                {/* Agents bring their own login, so a chat on one needs no provider. */}
                {configured === false && !acpDriver && isEmpty && (
                  <GetStarted
                    agents={acpAgents}
                    onUseAgent={(kind) => {
                      const cfg = loadInstanceConfigs()[kind] ?? { enabled: true };
                      startAcpAgent(kind, { ...cfg, launchArgs: cfg.launchArgs ?? '', env: cfg.env ?? '', enabled: true }, null);
                    }}
                    onAddProvider={() => {
                      openSettings();
                      setSettingsSection('provider');
                    }}
                    onSetUpAgents={openAgentSettings}
                  />
                )}
                {configured === false && !acpDriver && !isEmpty && (
                  <div className="mx-auto mb-4 max-w-3xl rounded-lg border border-amber-500/30 bg-amber-500/[0.08] px-3 py-2 text-[13px] text-amber-200">
                    No provider configured —{' '}
                    <button
                      className="underline underline-offset-2 hover:text-amber-100"
                      onClick={() => openSettings()}
                    >
                      open Settings
                    </button>{' '}
                    to add one.
                  </div>
                )}

                {isEmpty ? (
                  configured === false && !acpDriver ? null : <EmptyState
                    cwd={cwd}
                    onPrompt={(text) => onSend(text)}
                    onOpenSession={(id) => wsRef.current?.attach(id)}
                  />
                ) : (
                  <div data-transcript-column className="mx-auto flex max-w-3xl flex-col gap-2">
                    {goal && (
                      <GoalPanel
                        goal={goal}
                        busy={busy}
                        activity={goalActivity(entries)}
                        onClear={onClearGoal}
                        onRestart={(condition, maxIter) => onSetGoal(condition, maxIter)}
                      />
                    )}
                    {tasks.length > 0 && <TaskListPanel tasks={tasks} />}
                    <MessageActionsContext.Provider value={messageActions}>
                      {turns.map((turn, i) => (
                      <TurnView
                        key={`turn-${i}`}
                        minimapId={`turn-${i}`}
                        turn={turn}
                        timing={turnTimings.get(i) ?? null}
                        usage={turnUsage.get(i) ?? null}
                        model={turnModels.get(i) ?? model}
                        index={i}
                        expanded={expandedTurns.has(i)}
                        onToggle={stableToggleTurn}
                        onDecide={stableDecide}
                        onPlanReply={stablePlanReply}
                        onAskUserReply={stableAskUserReply}
                        onOpenAgent={stableOpenAgent}
                        onOpenFile={stableOpenFile}
                        isActive={busy && i === turns.length - 1}
                        skills={skills}
                        mode={mode}
                        onSetMode={stableSetMode}
                        approvalViaDialog={false}
                        offscreenOk={i < turns.length - 3}
                      />
                    ))}
                    </MessageActionsContext.Provider>
                    {showSecondOpinion && (
                      <SecondOpinion
                        agentName={acpDriverName}
                        files={sessionDiff.files.length}
                        canReview={configured !== false}
                        onReview={() => {
                          retireSecondOpinion();
                          void runReview('');
                        }}
                        onAddProvider={() => {
                          openSettings();
                          setSettingsSection('provider');
                        }}
                        onDismiss={retireSecondOpinion}
                      />
                    )}
                    {thinking && (
                      <div className="flex justify-start">
                        <Thinking />
                      </div>
                    )}
                  </div>
                )}
              </div>

              <TimelineMinimap items={minimapItems} paneRef={paneRef} onSelect={jumpToMinimapTurn} />
              <ImageLightbox src={lightbox} onClose={() => setLightbox(null)} />
              <ReviewChanges
                open={reviewOpen}
                onClose={() => {
                  setReviewOpen(false);
                  setReviewFocus(null);
                }}
                onSendComments={(text) => onSend(text)}
                onChanged={refreshRepo}
                focusPath={reviewFocus}
              />

              {showJump && (
                <button
                  type="button"
                  onClick={jumpToLatest}
                  className="absolute bottom-3 left-1/2 z-10 inline-flex -translate-x-1/2 animate-fade-in items-center gap-1.5 rounded-full border border-border bg-secondary/95 px-3 py-1.5 text-[12px] text-muted-foreground shadow-lg backdrop-blur transition-colors hover:text-foreground"
                >
                  <ArrowDown strokeWidth={2.5} className="size-3" />
                  Jump to latest
                </button>
              )}

              {/* Floating context panel — absolutely anchored to top-right */}
              <AnimatePresence>
                <ContextPanel
                  key="ctx"
                  sessionTitle={sessionTitle ?? titleFromEntries(entries)}
                  tasks={tasks}
                  subagentState={subagentState}
                  entries={entries}
                  gitStatus={gitStatus}
                  sessionDiff={sessionDiff}
                  branchPr={branchPr}
                  sessionCommitted={sessionCommitted}
                  open={ctxOpen}
                  onOpenChange={onCtxOpenChange}
                  onOpenAgent={openAgentTab}
                  onReview={(path) => {
                    setReviewFocus(path ?? null);
                    setReviewOpen(true);
                  }}
                  onPush={async () => {
                    await gitPush();
                    refreshRepo();
                  }}
                  onCommit={async (message, includeUnstaged, pushAfter) => {
                    await gitCommit({ message, include_unstaged: includeUnstaged, push_after: pushAfter });
                    setSessionCommitted(true);
                    refreshRepo();
                  }}
                />
              </AnimatePresence>
            </div>

            <div
              className="shrink-0 transition-[padding-right] duration-200"
              style={{ paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE : 0 }}
            >
            {restoreNote && (
              <div className="mx-auto mb-2 flex w-full max-w-3xl animate-fade-in items-center gap-2 rounded-lg border border-border/60 bg-secondary/70 px-3 py-1.5 text-[12.5px]">
                <History className={cn('size-3.5 shrink-0', restoreNote.error ? 'text-amber-400' : 'text-muted-foreground')} />
                <span className={cn('min-w-0 flex-1', restoreNote.error ? 'text-amber-200' : 'text-foreground/85')}>
                  {restoreNote.text}
                </span>
                {restoreNote.undo && (
                  <button
                    type="button"
                    onClick={() => {
                      const undo = restoreNote.undo!;
                      setRestoreNote(null);
                      void doRestore(() => undoRestore(undo), 'Put back');
                    }}
                    className="shrink-0 rounded px-1.5 py-0.5 text-[12px] font-medium text-foreground/80 hover:bg-fg/[0.06] hover:text-foreground"
                  >
                    Undo
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => setRestoreNote(null)}
                  aria-label="Dismiss"
                  className="shrink-0 rounded p-0.5 text-muted-foreground/60 hover:text-foreground"
                >
                  <X className="size-3.5" />
                </button>
              </div>
            )}
            <Composer
              disabled={status !== 'open'}
              busy={busy}
              mode={mode}
              model={model}
              providerName={providerName}
              cwd={cwd}
              usage={usage}
              rateLimit={rateLimit}
              onSend={onSend}
              onSetMode={onSetMode}
              onSetModel={onSetModel}
              engine={engine}
              engines={engines}
              agents={acpAgents}
              agentsChecking={acpStatusPending}
              onCheckAgents={requestAcpStatus}
              agentConfig={acpDriver ? acpConfig : null}
              agentDescriptors={acpDescriptors}
              onSetModelOption={onSetModelOption}
              onPickProvider={(instance, m) => {
                wsRef.current?.send({ type: 'set_model', model: m, instance: instance ?? null });
              }}
              onPickAgent={(driver, m) => {
                // Already this chat's agent: only the model can change, and
                // that is an option on the running agent, not a restart
                // with a new setup.
                if (acpDriver === driver) {
                  if (m && m !== engine?.model) onSetModel(m);
                  return;
                }
                // The agent's saved setup (Settings → Agents) travels with
                // the pick, so its paths, env and effort apply.
                const cfg = loadInstanceConfigs()[driver] ?? { enabled: true };
                startAcpAgent(driver, { ...cfg, launchArgs: cfg.launchArgs ?? '', env: cfg.env ?? '', enabled: true }, null, m);
              }}
              onConfigureAgents={openAgentSettings}
              sessionId={sessionId}
              onAgentCompact={compactAcpAgent}
              onAgentFork={forkAcpAgent}
              onAgentReverted={() => wsRef.current?.attach(sessionId)}
              onAcpModes={acpModes?.available ?? null}
              onAcpCurrentMode={acpModes?.current ?? null}
              onPickAgentMode={(m) => {
                if (!acpDriver) return;
                const key = MIRA_MODE_TO_POSTURE[m];
                const opt = agentPostures.find((o) => o.posture.key === key);
                // Unmapped or already active: nothing to confirm.
                if (!opt || opt.current) return;
                setPendingAcpMode({ modeId: opt.modeId });
              }}
              agentDriving={acpDriver != null}
              onOpenPicker={() => setPickerOpen(true)}
              onCwdSwitched={(_path, id) => { if (id) wsRef.current?.attach(id); }}
              environment={environment}
              environments={environments}
              envSwitching={envSwitching}
              onSwitchEnvironment={(target) => {
                setEnvSwitching(`switching to ${target}…`);
                wsRef.current?.send({ type: 'environment', target });
              }}
              onInterrupt={() => wsRef.current?.send({ type: 'interrupt' })}
              onNewChat={onNewChat}
              onOpenSettings={() => openSettings()}
              onRunReview={runReview}
              onSetGoal={onSetGoal}
              onClearGoal={onClearGoal}
              onCompact={onCompact}
              goal={goal}
              onRemember={async (scope, text) => {
                const r = await appendMemory(scope, text);
                return `remembered → ${r.path}`;
              }}
              onUndo={async (count) => {
                const r = await applyUndo(count);
                if (r.applied.length === 0) return 'nothing to undo';
                return `reverted ${r.applied.length} write${r.applied.length === 1 ? '' : 's'}`;
              }}
              skills={skills}
              usageRing={usageRing}
              pendingApproval={pendingApprovals[0] ?? null}
              pendingApprovalCount={pendingApprovals.length}
              onAllowAllPending={() => {
                for (const a of pendingApprovals) decideApproval(a.callId, true, 'once');
              }}
              pendingPlan={pendingPlan}
              pendingAskUser={pendingAskUser}
              onDecide={(callId, allow, scope) => decideApproval(callId, allow, scope)}
              onPlanReply={replyToPlan}
              onAskUserReply={replyToAskUser}
              commands={
                // An agent's advertised slash commands take over while it is
                // driving the session — Mira's own command list would offer
                // prompts the agent has never heard of.
                acpDriver && acpCommands.length > 0
                  ? acpCommands.map((name) => ({
                      name,
                      description: '',
                      argument_hint: null,
                      source: 'acp',
                      kind: 'command' as const,
                      origin: {
                        kind: 'user' as const,
                        key: 'acp',
                        label: acpDriver,
                        icon_url: null,
                        homepage: null,
                      },
                    }))
                  : commands
              }
            />
            </div>
            {terminalOpen && <TerminalPanel onClose={() => setTerminal(false)} />}
          </>
        )}

        {mainView === 'plugins' && (
          <div className="flex-1 min-h-0 overflow-y-auto">
            <PluginsPanel version={extensionsVersion} />
          </div>
        )}

        {mainView === 'pull-request' && (
          <PullRequestPanel
            onOpenSettings={() => openSettings()}
            onReviewPr={runPrReview}
          />
        )}
        {mainView === 'scheduled' && <ComingSoon label="Scheduled" />}
        {mainView === 'settings' && (
          <SettingsSurface
            section={settingsSection}
            onSectionChange={setSettingsSection}
            onSaved={settingsHandler}
            onExit={exitSettings}
            skillsVersion={skillsVersion}
            githubReturn={githubReturn}
            acpAgents={acpAgents}
            acpRefreshing={acpStatusPending}
            acpDriver={acpDriver}
            acpError={acpError}
            onAcpRefresh={requestAcpStatus}
            onAcpStart={startAcpAgent}
          />
        )}
        </div>
      </main>

      {/* A privileged agent mode is a standing grant of more access than Mira
          would allow, so it gets an explicit confirmation that says what it
          does. The server refuses without this acknowledgement; the client
          setting the flag is not consent on its own. */}
      {/* Changing the agent's mode from the composer. The mode picker used to
          live inside the model dialog as a silent dropdown; a permission-mode
          change is a grant of standing authority, not a preference, so it is
          confirmed here — one dialog for every decision. */}
      {pendingAcpMode && (() => {
        // One confirm dialog for every mode decision: a pick from the picker
        // and a server-refused privilege converge here. The choice was made
        // in the picker, so this states the consequence and asks for the nod
        // — it does not list every posture again.
        const opt = agentPostures.find((o) => o.modeId === pendingAcpMode.modeId);
        const label = opt?.posture.label ?? pendingAcpMode.modeId;
        return (
          <ApprovalDialog
            tone="consequential"
            request={{
              title: `Switch to ${label}?`,
              source: { label: acpDriverName, detail: 'agent mode' },
              body: (
                <div className="space-y-2">
                  {pendingAcpMode.reason && <p>{pendingAcpMode.reason}</p>}
                  <p>
                    {opt?.modeDescription ?? opt?.posture.blurb ?? 'This changes what the agent may do without asking.'}
                  </p>
                  <p className="text-muted-foreground/70">
                    {acpDriverName} will keep this mode until you change it
                    back. File, terminal and permission requests still route
                    through Mira either way.
                  </p>
                </div>
              ),
              choices: [
                { id: '__cancel', label: 'Cancel' },
                {
                  id: 'confirm',
                  label: `Switch to ${label}`,
                  primary: true,
                  destructive: opt?.posture.key === 'yolo',
                },
              ],
              onDismiss: () => setPendingAcpMode(null),
              onChoose: (id) => {
                const modeId = pendingAcpMode.modeId;
                setPendingAcpMode(null);
                // Confirmed in this dialog, so the acknowledgement rides
                // along: the server must not ask again for what was just
                // agreed.
                if (id !== '__cancel') onSetAcpMode(modeId, true);
              },
            }}
          />
        );
      })()}
      {panelOpen && (
        <div className="min-h-0 min-w-0 py-2 pr-2">
          <div className="h-full overflow-hidden rounded-xl border border-border bg-background">
        <SubagentPanel
          tabs={subagentTabs}
          fileTabs={fileTabs}
          toolTabs={toolTabs}
          onWhiteboardSend={(png) => void sendWhiteboardToChat(png)}
          onOpenPane={openToolPane}
          onBrowseFile={() => setPanelFilePickerOpen(true)}
          activeCallId={activeAgentTab}
          cwd={cwd ?? ''}
          onSelectTab={setActiveAgentTab}
          onCloseTab={closeAnyTab}
          onClose={closeSubagentPanel}
          onReview={replyToSubagentReview}
          onResizeStart={handlePanelResizeStart}
          onOpenFile={openFileTab}
        />
          </div>
        </div>
      )}

      <FilePicker
        open={panelFilePickerOpen}
        startPath={cwd || undefined}
        onClose={() => setPanelFilePickerOpen(false)}
        onPicked={(p) => {
          setPanelFilePickerOpen(false);
          openFileTab(p, null);
        }}
      />

      <Dialog open={importOpen} onOpenChange={setImportOpen}>
        <DialogContent className="max-w-3xl gap-4 p-5">
          <DialogTitle className="text-[16px] font-semibold">Bring chats from other agents</DialogTitle>
          {importOpen && (
            <ImportChats
              onDone={() => {
                setImportOpen(false);
                setSidebarRefresh((n) => n + 1);
              }}
            />
          )}
        </DialogContent>
      </Dialog>
      <ContextInspector
        open={inspectOpen}
        onOpenChange={setInspectOpen}
        busy={busy}
        onCompact={(focus) => (acpDriver ? compactAcpAgent(focus) : onCompact(focus))}
        onDropped={(callId, d) => {
          // The tool card shows what the model now sees, and a status line
          // records the drop where it happened.
          setEntries((prev) => [
            ...prev.map((e) =>
              e.kind === 'tool' && e.call.id === callId && e.result
                ? { ...e, result: { ...e.result, content: `[Removed from context — ~${shortNum(d.tokens)} tokens]`, images: undefined } }
                : e,
            ),
            { kind: 'warning', text: `[context] removed ${d.label || d.tool} from context (~${shortNum(d.tokens)} tokens)` },
          ]);
        }}
      />
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        actions={paletteActions}
        onOpenSession={(id) => {
          wsRef.current?.attach(id);
          setMainView('chat');
        }}
      />
      {restoreAsk && (
        <ApprovalDialog
          tone="consequential"
          request={{
            title: `Restore ${restoreAsk.changes.length} file${restoreAsk.changes.length === 1 ? '' : 's'}?`,
            source: { label: 'Checkpoint', detail: 'before this message' },
            body: (
              <div className="space-y-2">
                <p>
                  Every file that changed since this message goes back to how it was — including
                  changes made after it, by anyone. You can undo this.
                </p>
                <ul className="max-h-48 space-y-0.5 overflow-auto rounded-md bg-background/60 px-2.5 py-2 font-mono text-[11.5px]">
                  {restoreAsk.changes.map((c) => (
                    <li key={c.path} className="flex gap-2">
                      <span
                        className={cn(
                          'w-14 shrink-0',
                          c.action === 'remove' ? 'text-red-400/80' : c.action === 'recreate' ? 'text-green-400/80' : 'text-amber-300/80',
                        )}
                      >
                        {c.action === 'remove' ? 'remove' : c.action === 'recreate' ? 'bring back' : 'revert'}
                      </span>
                      <span className="min-w-0 break-all text-foreground/80">{c.path}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ),
            choices: [
              { id: 'cancel', label: 'Cancel' },
              { id: 'restore', label: 'Restore files', primary: true },
            ],
            onDismiss: () => setRestoreAsk(null),
            onChoose: (id) => {
              const ask = restoreAsk;
              setRestoreAsk(null);
              if (id === 'restore') void doRestore(() => restoreCheckpoint(ask.ref), 'Restored');
            },
          }}
        />
      )}
      <FolderPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onPicked={(_path, id) => {
          // Server built a fresh slot for the new cwd. Attach the WS so
          // the freshly-published Ready lands in our transcript — without
          // this, the socket keeps forwarding the previous slot's frames
          // and the UI silently stays on the old folder.
          if (id) wsRef.current?.attach(id);
        }}
      />
      <ReviewPanel
        open={reviewPanelOpen}
        state={reviewState}
        onClose={() => setReviewPanelOpen(false)}
      />
    </div>
  );
}

/* ---------- helpers ---------- */

/** Immutable Map update helper — replaces the entry for `parentCallId`
 *  (or seeds a fresh one) with the result of `f`. Returns a new Map so
 *  React sees the state change. */
function updateSubagent(
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

function appendReasoning(prev: Entry[], text: string): Entry[] {
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
function settleTools(prev: Entry[]): Entry[] {
  let changed = false;
  const next = prev.map((e) => {
    if (e.kind !== 'tool' || e.call.function.name === 'agent') return e;
    if (e.status === 'pending') {
      changed = true;
      return {
        ...e,
        status: 'denied' as const,
        result: e.result ?? { call_id: e.call.id, content: 'Not run — the turn ended before it was answered.', is_error: true },
      };
    }
    if (e.status === 'running') {
      changed = true;
      return {
        ...e,
        status: 'complete' as const,
        result: e.result ?? { call_id: e.call.id, content: 'Interrupted — the turn ended before this finished.', is_error: true },
      };
    }
    return e;
  });
  return changed ? next : prev;
}

/** Close the live thought block, if any — the model moved on. */
function sealThought(prev: Entry[]): Entry[] {
  const last = prev[prev.length - 1];
  if (!last || last.kind !== 'thought' || !last.live) return prev;
  return [...prev.slice(0, -1), { ...last, live: false, endedAt: Date.now() }];
}

function appendToken(prevRaw: Entry[], text: string): Entry[] {
  const prev = sealThought(prevRaw);
  const last = prev[prev.length - 1];
  if (last && last.kind === 'msg' && last.msg.role === 'assistant') {
    const updated: Entry = {
      kind: 'msg',
      msg: { ...last.msg, content: (last.msg.content ?? '') + text },
    };
    return [...prev.slice(0, -1), updated];
  }
  return [...prev, { kind: 'msg', msg: { role: 'assistant', content: text } }];
}

/** An agent's reply text. It is the turn's answer like Mira's own, so it
 *  becomes the same assistant message — same rendering, same copy action,
 *  same place in a collapsed turn. */
function appendAcpText(prev: Entry[], text: string): Entry[] {
  return appendToken(prev, text);
}

/** An agent's thinking, as Mira's own thought block. */
function appendAcpThought(prev: Entry[], text: string): Entry[] {
  if (!text) return prev;
  return appendReasoning(prev, text);
}

/**
 * Whether a stop reason means the turn actually produced an answer.
 *
 * Deliberately a small allowlist rather than a blocklist: an unfamiliar
 * reason is far more likely to be a failure or a limit than a clean finish,
 * and saying so beats claiming success.
 */
function isSuccessfulAcpStop(stop: string): boolean {
  return ['end_turn', 'success', 'completed', 'stop_sequence'].includes(stop);
}

/** A human explanation of why a turn stopped, for the transcript. */
function describeAcpStop(stop: string): string {
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

function sealAcpThought(prev: Entry[]): Entry[] {
  return sealThought(prev);
}

/** Questions and plans render as Mira's own cards (the server wraps them),
 *  so the agent's raw call for them would be a duplicate. */
const AGENT_PROMPT_TOOLS = new Set(['AskUserQuestion', 'ExitPlanMode']);

/** Insert or update an agent's tool call, as a Mira tool entry.
 *
 *  An update carries only what changed (no name, an empty title), so it
 *  merges onto the call already recorded — empty fields never blank what
 *  is there — and the Mira view is rebuilt from the merged call. */
function upsertAcpTool(prevRaw: Entry[], call: AcpToolCall): Entry[] {
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
  if (AGENT_PROMPT_TOOLS.has(merged.name ?? '')) return idx >= 0 ? prev.filter((_, i) => i !== idx) : prev;
  const entry: Entry = {
    kind: 'tool',
    call: agentCallToToolCall(merged),
    preview: null,
    status: agentToolStatus(merged),
    result: agentToolResult(merged),
    agentCall: merged,
  };
  if (idx >= 0) return [...prev.slice(0, idx), entry, ...prev.slice(idx + 1)];
  return [...prev, entry];
}

// A tool_start arrives after either (a) a user-approved approval_request, in
// which case an entry already exists — leave it alone, or (b) an auto-approved
// call the policy let through with no prompt — add a fresh entry.
function upsertToolStart(prevRaw: Entry[], call: ToolCall): Entry[] {
  const prev = sealThought(prevRaw);
  const existing = prev.findIndex((e) => e.kind === 'tool' && e.call.id === call.id);
  if (existing >= 0) return prev;
  return [...prev, { kind: 'tool', call, preview: null, status: 'running', result: null }];
}

function attachToolResult(prev: Entry[], result: ToolResult): Entry[] {
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
function applyTaskResult(prev: TaskItem[], result: ToolResult): TaskItem[] {
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

function updateTool(prev: Entry[], callId: string, f: (t: ToolEntry) => ToolEntry): Entry[] {
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && e.call.id === callId) {
      return [...prev.slice(0, i), f(e), ...prev.slice(i + 1)];
    }
  }
  return prev;
}

function appendProgressLine(prev: Entry[], callId: string, line: string): Entry[] {
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && e.call.id === callId) {
      const updated: ToolEntry = {
        ...e,
        progressLines: [...(e.progressLines ?? []), line],
      };
      return [...prev.slice(0, i), updated, ...prev.slice(i + 1)];
    }
  }
  return prev;
}

function attachPlanProposal(prev: Entry[], callId: string, proposal: PlanProposal): Entry[] {
  return updateTool(prev, callId, (t) => ({
    ...t,
    plan: { proposal, decision: null },
  }));
}

function recordPlanDecision(
  prev: Entry[],
  callId: string,
  decision: { approved: boolean; steps?: PlanStep[]; note?: string },
): Entry[] {
  return updateTool(prev, callId, (t) => {
    if (!t.plan) return t;
    return { ...t, plan: { ...t.plan, decision } };
  });
}

function attachAskUserProposal(prev: Entry[], callId: string, proposal: AskUserProposal): Entry[] {
  return updateTool(prev, callId, (t) => ({
    ...t,
    askUser: { proposal, decision: null },
  }));
}

function recordAskUserDecision(
  prev: Entry[],
  callId: string,
  decision: AskUserDecision,
): Entry[] {
  return updateTool(prev, callId, (t) => {
    if (!t.askUser) return t;
    return { ...t, askUser: { ...t.askUser, decision } };
  });
}

function countUserMessages(entries: Entry[]): number {
  let n = 0;
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user') n++;
  }
  return n;
}

function rebuildTurnTimings(serverTurns: { started_at: number; ended_at?: number | null }[]): Map<number, TurnTiming> {
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
function rebuildTurnUsage(serverTurns: TurnMeta[]): Map<number, UsageTotals> {
  const out = new Map<number, UsageTotals>();
  serverTurns.forEach((t, i) => {
    const u = t.usage;
    if (u && u.prompt_tokens + u.completion_tokens > 0) out.set(i, u);
  });
  return out;
}

function rebuildTurnModels(serverTurns: TurnMeta[]): Map<number, string> {
  const out = new Map<number, string>();
  serverTurns.forEach((t, i) => {
    if (t.model) out.set(i, t.model);
  });
  return out;
}

function stampLastTurn(prev: Map<number, TurnTiming>, endedAt: number): Map<number, TurnTiming> {
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
function goalActivity(entries: Entry[]): string {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.kind === 'tool' && e.status === 'running') {
      return `Running ${e.call.function.name}`;
    }
    if (e.kind === 'tool' && e.status === 'complete' && !e.result?.is_error) {
      return `Ran ${e.call.function.name}`;
    }
    if (e.kind === 'msg' && e.msg.role === 'assistant' && (e.msg.content ?? '').trim()) {
      const line = (e.msg.content ?? '').trim().split('\n')[0];
      return line.length > 90 ? line.slice(0, 90) + '…' : line;
    }
  }
  return '';
}

function titleFromEntries(entries: Entry[]): string {
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

type Turn = {
  /** The user's message that opened this turn. `null` for any pre-user
   *  entries (e.g. a system-emitted warning before the first send). */
  user: Entry | null;
  /** Everything after `user` up to the next user message. */
  body: Entry[];
};

function groupByTurn(entries: Entry[]): Turn[] {
  const turns: Turn[] = [];
  let current: Turn = { user: null, body: [] };
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user') {
      // Close previous turn if it had anything.
      if (current.user || current.body.length) turns.push(current);
      current = { user: e, body: [] };
    } else {
      current.body.push(e);
    }
  }
  if (current.user || current.body.length) turns.push(current);
  return turns;
}

/* ---------- turn renderer ---------- */

/** Same turn content: the same entries, by identity. Entries are replaced,
 *  never mutated, so an untouched turn compares equal even though
 *  `groupByTurn` builds a fresh `Turn` object on every update. */
function sameTurn(a: Turn, b: Turn): boolean {
  return a.user === b.user && a.body.length === b.body.length && a.body.every((e, i) => e === b.body[i]);
}

/** Re-render a turn only when something it shows changed. Handlers are
 *  stable (see `useStableCallback`), so plain identity works for them. */
const TurnView = memo(TurnViewImpl, (prev, next) => {
  for (const k of Object.keys(next) as (keyof TurnViewProps)[]) {
    if (k === 'turn') {
      if (!sameTurn(prev.turn, next.turn)) return false;
    } else if (prev[k] !== next[k]) {
      return false;
    }
  }
  return true;
});

type TurnViewProps = Parameters<typeof TurnViewImpl>[0];

function TurnViewImpl({
  turn, timing: timingProp, usage: usageProp, model: modelProp, index, expanded, isActive, onToggle, onDecide, onPlanReply, onAskUserReply, onOpenAgent, onOpenFile, skills, mode, onSetMode, minimapId, approvalViaDialog, offscreenOk = false,
}: {
  turn: Turn;
  /** Position in the transcript; what `onToggle` is called with. */
  index: number;
  /** Not one of the latest turns: the browser may skip laying it out while
   *  it's off screen. */
  offscreenOk?: boolean;
  timing: TurnTiming | null;
  /** Tokens this turn used (live turns only; not persisted). */
  usage: UsageTotals | null;
  model: string;
  expanded: boolean;
  isActive: boolean;
  onToggle: (index: number) => void;
  onDecide: (callId: string, allow: boolean, scope?: ApprovalScope) => void;
  onPlanReply: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  /** Answer callback for the `ask_user` clarification tool. */
  onAskUserReply: (callId: string, decision: AskUserDecision) => void;
  /** Opens (or focuses) the right-side SubagentPanel tab for the given
   *  agent call. Wired from App.tsx via `openAgentTab`. */
  onOpenAgent: (callId: string) => void;
  /** Opens (or focuses) a file viewer tab in the right-side panel. */
  onOpenFile: (path: string, diff: DiffPreview | null) => void;
  /** Loaded skill roster — passed through so the user bubble can render
   *  `@skill:<name>` mentions as pretty chips (icon + display label +
   *  hash-derived color) rather than raw tokens. */
  skills: SkillView[];
  /** Session mode + setter — plumbed to pending tool-approval cards so
   *  the "Always allow" button can bump the mode to `edit` (auto
   *  everything unless a rule blocks) for the current session. */
  mode: Mode;
  onSetMode: (m: Mode) => void;
  /** Stable scroll-target id for the timeline minimap (`turn-${index}`). */
  minimapId?: string | null;
  /** Forwarded to entry tool cards: when an agent drives, decisions live in
   *  the unified dialog, not on the cards. */
  approvalViaDialog?: boolean;
}) {
  // An agent turn carries its own stats; they win over the per-index
  // maps, which only line up with Mira's own turns.
  const own = turn.body.find((e): e is TurnStatsEntry => e.kind === 'turn_stats');
  const timing: TurnTiming | null =
    own?.startedAt != null ? { startedAt: own.startedAt, endedAt: own.endedAt } : timingProp;
  const usage = own?.usage ?? usageProp;
  const model = own?.model ?? modelProp;

  // Split the body into "intermediate work" and the final assistant text.
  // Rule: the LAST assistant text message with non-empty content is the
  // final answer; everything before it is intermediate. Tool cards + earlier
  // assistant text hide behind the "Worked for" chip when collapsed.
  // The stats entry is data, not content: it mustn't count as work done.
  const body = useMemo(() => turn.body.filter((e) => e.kind !== 'turn_stats'), [turn.body]);
  const finalIdx = findFinalAssistantIndex(body);
  const intermediateRaw = finalIdx >= 0 ? body.slice(0, finalIdx) : body;
  const finalEntry = finalIdx >= 0 ? body[finalIdx] : null;
  const trailing = finalIdx >= 0 ? body.slice(finalIdx + 1) : [];

  // Fold consecutive `agent` tool entries into a single group so a parallel
  // spawn ("N agents working") reads as one bar instead of N loud cards.
  // Non-agent entries pass through unchanged.
  const intermediate = useMemo(() => groupAgentRuns(intermediateRaw), [intermediateRaw]);

  // While a turn is in flight, force the intermediate section open so
  // in-progress tool calls (esp. pending approval bubbles) stay visible.
  // The user can collapse it after `done` fires. A pending approval
  // requires a click to keep the model moving; hiding it would deadlock.
  const hasPendingApproval = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.status === 'pending',
  );
  const hasPendingAskUser = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.askUser != null && e.askUser.decision === null,
  );
  const hasPendingPlan = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.plan != null && e.plan.decision === null,
  );
  // `waitingForUser` covers every state where mira has handed the turn
  // back to the human: approval prompts, plan review, ask_user cards.
  // The "Working…" timer pauses while this is true so the elapsed
  // display reflects work-done-by-mira, not wall-clock-minus-thinking.
  const waitingForUser = hasPendingApproval || hasPendingAskUser || hasPendingPlan;

  // Freeze the timer during wait periods. `waitStartedRef` marks the
  // wall-clock instant the current wait began; `waitAccumRef` keeps the
  // running total of prior wait segments in this same turn (so a
  // turn with multiple approval rounds still reads correctly). Both are
  // per-turn state — the component instance is stable across renders. */
  const waitStartedRef = useRef<number | null>(null);
  const waitAccumRef = useRef<number>(0);
  useEffect(() => {
    const now = Date.now();
    if (waitingForUser && waitStartedRef.current === null) {
      waitStartedRef.current = now;
    } else if (!waitingForUser && waitStartedRef.current !== null) {
      waitAccumRef.current += now - waitStartedRef.current;
      waitStartedRef.current = null;
    }
  }, [waitingForUser]);

  const activeWaitMs =
    waitStartedRef.current !== null ? Date.now() - waitStartedRef.current : 0;
  const totalWaitMs = waitAccumRef.current + activeWaitMs;
  const rawDurationMs = timing
    ? (timing.endedAt ?? Date.now()) - timing.startedAt
    : null;
  const durationMs =
    rawDurationMs !== null ? Math.max(0, rawDurationMs - totalWaitMs) : null;
  const showWorkedChip =
    getBoolPref(PREF_KEYS.transcriptTurnStats, true) &&
    (intermediate.length > 0 || isActive) && durationMs != null;

  const forceOpen = isActive || hasPendingApproval || hasPendingAskUser || hasPendingPlan;
  const effectivelyExpanded = expanded || forceOpen;

  // Codex-style categorised activity phrase ("3 reads, 4 searches, 1 write")
  // shown next to the "Worked for" duration so a collapsed turn still tells
  // the reader WHAT mira did, not just for how long. Counts every tool
  // entry across the whole turn body — some flows put the assistant text
  // BEFORE the tool run (e.g. "huh?" turns, or turns interrupted after the
  // model started answering), which would land tools in `trailing` rather
  // than `intermediateRaw` and drop them from the count.
  const activitySummary = useMemo(() => {
    const calls = turn.body
      .filter((e): e is Extract<Entry, { kind: 'tool' }> => e.kind === 'tool')
      .map((e) => e.call);
    if (calls.length === 0) return '';
    return countsPhrase(countsByCategory(calls));
  }, [turn.body]);

  return (
    <div
      data-minimap-id={minimapId ?? undefined}
      className="flex min-w-0 flex-col gap-2"
      // Long chats: skip layout and paint for older turns while they're off
      // screen. The browser remembers each one's last size, so scrolling and
      // the minimap's jumps stay accurate.
      style={offscreenOk ? { contentVisibility: 'auto', containIntrinsicSize: 'auto 480px' } : undefined}
    >
      {turn.user && <EntryView entry={turn.user} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />}

      {showWorkedChip && (
        <WorkedForChip
          durationMs={durationMs!}
          active={isActive && timing?.endedAt == null}
          waitingForUser={waitingForUser}
          expanded={effectivelyExpanded}
          locked={forceOpen}
          onToggle={() => onToggle(index)}
          activity={activitySummary}
        />
      )}

      {(effectivelyExpanded || !showWorkedChip) && intermediate.map((item, i) => {
        if (item.kind === 'agent-group') {
          return (
            <div key={`t-a-${i}`} className="flex justify-start">
              <AgentGroup
                entries={item.entries.map((e) => ({
                  call: e.call,
                  status: e.status,
                  result: e.result,
                }))}
                onOpen={onOpenAgent}
              />
            </div>
          );
        }
        if (item.kind === 'tool-group') {
          return (
            <div key={`t-g-${i}`} className="flex justify-start">
              <ToolGroup
                entries={item.entries.map((e) => ({
                  call: e.call,
                  preview: e.preview,
                  status: e.status,
                  result: e.result,
                  progressLines: e.progressLines,
                }))}
                onOpenFile={onOpenFile}
              />
            </div>
          );
        }
        return (
          <EntryView key={`t-i-${i}`} entry={item.entry} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />
        );
      })}

      {finalEntry && (
        <TurnStatsContext.Provider
          value={
            timing?.endedAt != null
              ? { durationMs: durationMs ?? timing.endedAt - timing.startedAt, usage, model, costUsd: own?.costUsd ?? null }
              : null
          }
        >
          <EntryView entry={finalEntry} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />
        </TurnStatsContext.Provider>
      )}
      {trailing.map((e, i) => (
        <EntryView key={`t-t-${i}`} entry={e} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} approvalViaDialog={approvalViaDialog} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />
      ))}
    </div>
  );
}

/** Item produced by `groupAgentRuns`: either a single passthrough entry,
 *  a run of `>=2` consecutive `agent` tool entries folded into a group,
 *  or a run of `>=2` consecutive same-name non-agent tool entries folded
 *  into a group (e.g. three `read_file`s → one "Read × 3" chip).
 *  A run of length 1 stays as a single entry — the individual card is
 *  enough on its own without the group chrome. */
export type GroupItem =
  | { kind: 'entry'; entry: Entry }
  | { kind: 'agent-group'; entries: (Entry & { kind: 'tool' })[] }
  | { kind: 'tool-group'; entries: (Entry & { kind: 'tool' })[] };

/** Types that render as their own cards (agent, plan, ask_user) — never
 *  fold into a generic tool-group. Agent has its own AgentGroup path;
 *  plan and ask_user each swap in for the tool row when their proposal
 *  attaches, so grouping would hide the interactive card. */
const SPECIAL_TOOLS = new Set(['agent', 'plan', 'ask_user']);

export function groupAgentRuns(entries: Entry[]): GroupItem[] {
  const out: GroupItem[] = [];
  let i = 0;
  while (i < entries.length) {
    const e = entries[i];

    // Agent run: consume all consecutive `agent` entries into a single
    // AgentGroup regardless of individual state (pending is auto-approved
    // anyway since AgentTool is Pure).
    if (isAgentEntry(e)) {
      const run: (Entry & { kind: 'tool' })[] = [];
      while (i < entries.length && isAgentEntry(entries[i])) {
        run.push(entries[i] as Entry & { kind: 'tool' });
        i++;
      }
      out.push(
        run.length >= 2
          ? { kind: 'agent-group', entries: run }
          : { kind: 'entry', entry: run[0] },
      );
      continue;
    }

    // Generic tool run: any consecutive tool entries, none pending. Mixing
    // tool names is fine — ToolGroup renders a "Working/Worked" umbrella
    // with per-entry verbs in the preview when the run isn't homogeneous.
    // Pending calls must render individually so the approval UI is visible
    // and unmissable — folding them into a group would hide the y/n prompt.
    if (isGroupableTool(e)) {
      const run: (Entry & { kind: 'tool' })[] = [];
      while (i < entries.length && isGroupableTool(entries[i])) {
        run.push(entries[i] as Entry & { kind: 'tool' });
        i++;
      }
      out.push(
        run.length >= 2
          ? { kind: 'tool-group', entries: run }
          : { kind: 'entry', entry: run[0] },
      );
      continue;
    }

    out.push({ kind: 'entry', entry: e });
    i++;
  }
  return out;
}

function isAgentEntry(e: Entry): boolean {
  return e.kind === 'tool' && e.call.function.name === 'agent';
}

/** A tool entry is groupable when it isn't a special one-off renderer
 *  (agent/plan) and isn't currently awaiting user approval. */
function isGroupableTool(e: Entry): boolean {
  if (e.kind !== 'tool') return false;
  if (SPECIAL_TOOLS.has(e.call.function.name)) return false;
  if (e.status === 'pending') return false;
  return true;
}

function findFinalAssistantIndex(body: Entry[]): number {
  for (let i = body.length - 1; i >= 0; i--) {
    const e = body[i];
    if (e.kind === 'msg' && e.msg.role === 'assistant' && (e.msg.content ?? '').trim()) {
      return i;
    }
  }
  return -1;
}

function WorkedForChip({
  durationMs, active, waitingForUser, expanded, locked, onToggle, activity,
}: {
  durationMs: number;
  active: boolean;
  /** True while mira has handed the turn back to the user (approval,
   *  plan review, ask_user card). The chip flips to a "Waiting for you"
   *  label and drops the duration — the timer visibly pauses. */
  waitingForUser: boolean;
  expanded: boolean;
  /** Force-open due to in-flight work or a pending approval — the chip
   *  goes non-interactive so the user can't collapse away important state. */
  locked: boolean;
  onToggle: () => void;
  /** Categorised activity phrase ("3 reads, 4 searches, 1 write") for the
   *  turn's intermediate work — appended after the duration so a collapsed
   *  turn still surfaces WHAT mira did. Empty when nothing ran (e.g. a
   *  no-tool answer). */
  activity: string;
}) {
  const durationLabel = waitingForUser
    ? 'Waiting for you'
    : active
      ? `Working… ${formatDuration(durationMs)}`
      : `Worked for ${formatDuration(durationMs)}`;
  const dot = waitingForUser
    ? 'bg-amber-400'
    : active
      ? 'bg-mira-blue'
      : null;
  // "Active" here = the turn is still running (blue dot) or waiting
  // for the user (amber dot). In either case the label needs full
  // contrast — it's telling the user *something is happening*.
  // Idle "Worked for" recedes into muted grey so scrolling past
  // finished turns doesn't visually shout; hover brings it back.
  const isActive = active || waitingForUser;
  return (
    <button
      type="button"
      onClick={locked ? undefined : onToggle}
      disabled={locked}
      title={locked ? 'Auto-expanded while in progress' : undefined}
      // `group` so the trailing caret can key off hover state via
      // `group-hover:*` — hidden until the row is hovered or already
      // expanded, so a collapsed transcript stays quiet.
      className={cn(
        'group flex w-fit items-center gap-1.5 rounded-md px-2 py-1 text-[13px] font-semibold transition-colors',
        // Idle: muted grey. Hover/active/expanded: full contrast.
        isActive || expanded ? 'text-foreground/90' : 'text-muted-foreground/70',
        locked ? 'cursor-default opacity-80' : 'hover:bg-accent/40 hover:text-foreground',
      )}
    >
      <span>{durationLabel}</span>
      {activity && (
        // Middle-dot separator + un-bolded activity phrase so the
        // duration stays the primary read and the counts trail as a
        // subtitle. Hidden while waiting on the user — the amber
        // "Waiting for you" label is already carrying the message.
        !waitingForUser && (
          <>
            <span
              aria-hidden
              className={cn(
                'font-normal opacity-70',
                isActive || expanded ? 'text-foreground/60' : 'text-muted-foreground/50',
              )}
            >
              ·
            </span>
            <span
              className={cn(
                'font-normal',
                isActive || expanded ? 'text-foreground/70' : 'text-muted-foreground/70',
              )}
            >
              {activity}
            </span>
          </>
        )
      )}
      {dot && <span className={cn('size-1.5 animate-pulse rounded-full', dot)} />}
      <ChevronDown
        strokeWidth={2.5}
        className={cn(
          'size-3.5 transition-all',
          // Caret adopts the row's text colour so it fades with the
          // label instead of standing out against the muted grey.
          isActive || expanded ? 'text-foreground/70' : 'text-muted-foreground/70',
          !expanded && '-rotate-90',
          // Hide when collapsed AND not hovered; always show when
          // expanded (or hovered) so state is legible without
          // needing a second visual language for open/closed.
          !expanded && 'opacity-0 group-hover:opacity-100',
        )}
      />
    </button>
  );
}

function formatDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m}m` : `${m}m ${rem}s`;
}

const STARTER_PROMPTS = [
  'Give me a tour of this codebase',
  'Review my uncommitted changes',
  'Find likely bugs in the recent commits',
  'Write tests for the least-covered module',
];

function EmptyState({
  cwd,
  onPrompt,
  onOpenSession,
}: {
  cwd: string;
  onPrompt: (text: string) => void;
  onOpenSession: (id: string) => void;
}) {
  const [recent, setRecent] = useState<SessionSummary[]>([]);
  useEffect(() => {
    listSessions()
      .then((all) =>
        setRecent(
          all
            .filter((s) => s.cwd === cwd && s.message_count > 0 && !s.active)
            .sort((a, b) => b.updated_at - a.updated_at)
            .slice(0, 4),
        ),
      )
      .catch(() => setRecent([]));
  }, [cwd]);
  return (
    <div className="flex min-h-full flex-col items-center justify-center px-6 py-10">
      <img
        src={miraLogo}
        alt=""
        className="size-11 rounded-full object-contain opacity-90"
        draggable={false}
      />
      <h2 className="mt-4 text-[19px] font-medium tracking-tight text-foreground">
        What should we build today?
      </h2>
      <p className="mt-1.5 max-w-[46ch] text-center text-[13px] leading-relaxed text-muted-foreground">
        Ask Mira anything, or pick up where you left off in this folder.
      </p>

      {/* Suggestions as inline chips rather than a grid of bordered boxes.
          Four identical outlined rectangles read as a form, not as
          suggestions; chips wrap, scale down to one column on a narrow pane,
          and don't draw a grid of lines across the middle of the screen. */}
      <div className="mt-7 flex max-w-2xl flex-wrap items-center justify-center gap-1.5">
        {STARTER_PROMPTS.map((p) => (
          <button
            key={p}
            type="button"
            onClick={() => onPrompt(p)}
            className="rounded-full border border-border/70 bg-fg/[0.03] px-3 py-1.5 text-[12.5px] text-muted-foreground backdrop-blur-sm transition-colors hover:border-border hover:bg-fg/[0.07] hover:text-foreground"
          >
            {p}
          </button>
        ))}
      </div>

      {recent.length > 0 && (
        <div className="mt-8 w-full max-w-lg">
          <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground/50">
            Recent in this folder
          </div>
          <div className="flex flex-col">
            {recent.map((r) => (
              <button
                key={r.id}
                type="button"
                onClick={() => onOpenSession(r.id)}
                className="group flex items-center gap-3 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors hover:bg-fg/[0.04]"
              >
                <span className="min-w-0 flex-1 truncate text-foreground/75 group-hover:text-foreground">
                  {r.title || r.first_user_message || 'Untitled'}
                </span>
                <span className="shrink-0 text-[11.5px] tabular-nums text-muted-foreground/50">
                  {timeAgo(r.updated_at)}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function timeAgo(ts: number): string {
  // Session timestamps are epoch seconds or ms depending on the store.
  const ms = ts < 1e12 ? ts * 1000 : ts;
  const mins = Math.round((Date.now() - ms) / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.round(hrs / 24)}d ago`;
}

/** Placeholder view rendered for sidebar entries that don't have a real
 *  panel yet (Pull request, Scheduled). Keeps the shell responsive while
 *  we build out the actual features. */
function ComingSoon({ label }: { label: string }) {
  return (
    <div className="flex flex-1 min-h-0 flex-col items-center justify-center gap-2 text-muted-foreground">
      <div className="text-[18px] font-medium text-foreground/80">{label}</div>
      <div className="text-[13px]">Coming soon.</div>
    </div>
  );
}

function EntryView({
  entry,
  onDecide,
  onPlanReply,
  onOpenAgent,
  onOpenFile,
  skills,
  mode,
  onSetMode,
  onAskUserReply,
  approvalViaDialog,
}: {
  entry: Entry;
  onDecide: (callId: string, allow: boolean, scope?: ApprovalScope) => void;
  onPlanReply: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  onOpenAgent: (callId: string) => void;
  /** Opens a file viewer tab in the right-side panel. */
  onOpenFile?: (path: string, diff: DiffPreview | null) => void;
  /** Answer callback for the `ask_user` tool card. Fires when the user
   *  submits picks (or dismisses); flips the card into its resolved
   *  state locally and posts back to the server. */
  onAskUserReply?: (callId: string, decision: AskUserDecision) => void;
  approvalViaDialog?: boolean;
  /** Roster used by the user bubble to pretty-print `@skill:<name>`
   *  mentions. Defaults to empty when the parent doesn't pass one
   *  (e.g. tool/plan/agent entries never touch it). */
  skills?: SkillView[];
  /** Session mode + setter — forwarded to the pending tool-approval
   *  card so its "Always allow" button can bump the session out of a
   *  gating mode. Optional so tool/plan/agent-only callers don't have
   *  to pass them. */
  mode?: Mode;
  onSetMode?: (m: Mode) => void;
}) {
  const actions = useContext(MessageActionsContext);
  switch (entry.kind) {
    case 'msg': {
      const { role, content } = entry.msg;
      if (role === 'tool') return null;
      const body = (content ?? '').trim();
      if (!body && !entry.msg.images?.length) return null;
      if (role === 'user') {
        // Attachments live above the bubble as chips (Codex-style). The
        // model still sees the fenced content in the body — we just hide
        // that from the reader so the transcript stays scannable.
        const { attachments, text } = parseSentAttachments(content ?? '');
        const images = entry.msg.images ?? [];
        return (
          <div className="flex flex-col items-end gap-1.5">
            {attachments.length > 0 && (
              <div className="flex max-w-[78%] flex-wrap justify-end gap-1.5">
                {attachments.map((a, i) => (
                  <SentAttachmentChip key={`att-${i}-${a.filename}`} filename={a.filename} subtype={a.subtype} />
                ))}
              </div>
            )}
            {images.length > 0 && (
              <div className="flex max-w-[78%] flex-wrap justify-end gap-1.5">
                {images.map((img, i) => (
                  <img
                    key={i}
                    src={`data:${img.media_type};base64,${img.data}`}
                    alt="attached image"
                    onClick={() => actions?.openImage(`data:${img.media_type};base64,${img.data}`)}
                    className="max-h-40 max-w-[240px] cursor-zoom-in rounded-xl border border-border object-cover"
                  />
                ))}
              </div>
            )}
            {text.trim() && (
              <UserMessage entry={entry} text={text} raw={content ?? ''}>
                <SkillMentionText text={text} roster={skills ?? []} />
              </UserMessage>
            )}
          </div>
        );
      }
      return (
        <div className="group/msg flex justify-start">
          <div className="max-w-[90%]">
            <AssistantContent text={content ?? ''} onOpenFile={onOpenFile} />
            <AssistantActions entry={entry} text={content ?? ''} />
          </div>
        </div>
      );
    }
    case 'tool':
      // An agent's permission request is answered in the composer card; in
      // the transcript the agent's own tool card already shows the call.
      if (entry.status === 'pending' && isAgentRequest(entry.call)) return null;
      // The `plan` tool gets a dedicated inline card with an editable step
      // list; the `ask_user` tool gets a multi-choice question card; the
      // `agent` tool gets a compact per-agent card so parallel spawns
      // don't dominate the transcript. Everything else falls through to
      // the generic tool row.
      //
      // While a plan / ask_user is still pending (no decision yet), we show
      // a compact chip in the transcript — the interactive version lives in
      // the Composer so it stays anchored at the bottom even in long chats.
      if (entry.plan) {
        if (!entry.plan.decision) {
          return (
            <div className="flex justify-start">
              <div className="inline-flex items-center gap-2 rounded-xl border border-mira-blue/20 bg-mira-blue/[0.05] px-3 py-1.5 text-[12.5px]">
                <Lightbulb fill="currentColor" className="size-3.5 shrink-0 text-mira-blue/70" />
                <span className="font-medium text-mira-blue/80">Plan</span>
                <span className="text-muted-foreground/40">·</span>
                <span className="text-muted-foreground/80 truncate max-w-[40ch]">{entry.plan.proposal.title}</span>
                <span className="text-muted-foreground/40">·</span>
                <span className="text-[11px] text-muted-foreground/60">review below ↓</span>
              </div>
            </div>
          );
        }
        return (
          <div className="flex justify-start">
            <PlanCard
              proposal={entry.plan.proposal}
              decision={entry.plan.decision}
              onApprove={(steps) => onPlanReply(entry.call.id, true, steps)}
              onCancel={(note) => onPlanReply(entry.call.id, false, undefined, note || undefined)}
            />
          </div>
        );
      }
      if (entry.askUser) {
        if (!entry.askUser.decision) {
          return (
            <div className="flex justify-start">
              <div className="inline-flex items-center gap-2 rounded-xl border border-mira-blue/20 bg-mira-blue/[0.05] px-3 py-1.5 text-[12.5px]">
                <Sparkle fill="currentColor" className="size-3.5 shrink-0 text-mira-blue/70" />
                <span className="font-medium text-mira-blue/80">Question</span>
                <span className="text-muted-foreground/40">·</span>
                <span className="text-[11px] text-muted-foreground/60">answer below ↓</span>
              </div>
            </div>
          );
        }
        return (
          <div className="flex justify-start">
            <AskUserCard
              proposal={entry.askUser.proposal}
              decision={entry.askUser.decision}
              onSubmit={(answers) =>
                onAskUserReply?.(entry.call.id, { cancelled: false, answers })
              }
              onCancel={() =>
                onAskUserReply?.(entry.call.id, { cancelled: true })
              }
            />
          </div>
        );
      }
      if (entry.call.function.name === 'agent') {
        return (
          <div className="flex justify-start">
            <AgentCard
              call={entry.call}
              status={entry.status}
              result={entry.result}
              onOpen={onOpenAgent}
            />
          </div>
        );
      }
      // Pending approval: show a compact chip in transcript since the
      // interactive card is now anchored in the Composer.
      if (entry.status === 'pending') {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-center gap-2 rounded-xl border border-amber-500/20 bg-amber-500/[0.05] px-3 py-1.5 text-[12.5px]">
              <ShieldAlert fill="currentColor" className="size-3.5 shrink-0 text-amber-400/80" />
              <span className="font-medium text-amber-400/80">Approval</span>
              <span className="text-muted-foreground/40">·</span>
              <span className="text-muted-foreground/80 truncate max-w-[40ch] font-mono text-[11.5px]">
                {entry.call.function.name}
              </span>
              <span className="text-muted-foreground/40">·</span>
              <span className="text-[11px] text-muted-foreground/60">review below ↓</span>
            </div>
          </div>
        );
      }
      return (
        <div className="flex justify-start">
          <ToolCard
            call={entry.call}
            preview={entry.preview}
            status={entry.status}
            result={entry.result}
            progressLines={entry.progressLines}
            onDecide={(allow, scope) => onDecide(entry.call.id, allow, scope)}
            mode={mode}
            onSetMode={onSetMode}
            onOpenFile={onOpenFile}
            decisionsViaDialog={approvalViaDialog}
          />
        </div>
      );
    case 'acp_plan':
      // The agent's plan, as a quiet checklist — the same visual weight as
      // the tool rows around it, not a separately labelled box.
      return (
        <div className="flex justify-start">
          <div className="w-full max-w-[90%] px-1 py-0.5 text-[13px]">
            <div className="mb-1.5 text-[12px] font-medium text-muted-foreground">Plan</div>
            <ul className="space-y-1">
              {entry.entries.map((item, i) => {
                const done = item.status === 'completed';
                const active = item.status === 'in_progress';
                return (
                  <li key={i} className="flex items-start gap-2">
                    <span
                      className={cn(
                        'mt-[5px] size-2.5 shrink-0 rounded-full border',
                        done
                          ? 'border-emerald-500/70 bg-emerald-500/70'
                          : active
                            ? 'border-mira-blue bg-mira-blue/30'
                            : 'border-muted-foreground/40',
                      )}
                    />
                    <span className={cn(done ? 'text-muted-foreground line-through decoration-muted-foreground/40' : 'text-foreground/90')}>
                      {item.content}
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
        </div>
      );
    case 'engine_switch':
      return <EngineSwitchDivider engine={entry.engine} />;
    case 'turn_stats':
      return null;
    case 'warning':
      return <StatusLine text={entry.text} />;
    case 'error':
      return <StatusLine text={entry.text} tone="error" />;
    case 'goal':
      return <GoalTranscriptChip entry={entry} />;
    case 'compact':
      return <CompactDivider entry={entry} />;
    case 'thought':
      return (
        <div className="flex justify-start">
          <div className="max-w-[90%]">
            <ThoughtBlock
              content={entry.text}
              live={entry.live}
              startedAt={entry.startedAt}
              endedAt={entry.endedAt}
            />
          </div>
        </div>
      );
    default: {
      // A transcript entry kind this build does not know how to render.
      //
      // This arm exists because the switch is *not* exhaustiveness-checked,
      // and that is exactly the trap: adding an `Entry` variant compiles
      // cleanly, `tsc` passes, and the row silently renders nothing — which
      // in a transcript is indistinguishable from a wedged agent. Rendering a
      // visible marker turns that class of mistake into something obvious.
      // `never` here would make the switch exhaustive and turn a *missing*
      // case into a compile error. We deliberately do not assert that: the
      // point of this arm is to survive new variants gracefully, and a
      // cast keeps the compiler from rejecting the very additions it should
      // tolerate.
      const unknown = entry as { kind: string };
      return (
        <StatusLine
          text={`unrendered transcript entry: ${unknown.kind}`}
          tone="error"
        />
      );
    }
  }
}

/** "Conversation compacted" line across the transcript. Everything above
 *  it is still shown, but the model now has the summary instead. */
function CompactDivider({ entry }: { entry: CompactEntry }) {
  const [open, setOpen] = useState(false);
  const detail = entry.summarized != null
    ? ` · ${entry.summarized} earlier message${entry.summarized === 1 ? '' : 's'} summarized`
    : '';
  return (
    <div className="flex flex-col gap-2 py-1">
      <div className="flex items-center gap-3 text-[11.5px] text-muted-foreground">
        <span className="h-px flex-1 bg-border/70" />
        <span className="shrink-0">
          Conversation compacted{detail}
          {entry.summary && (
            <>
              {' · '}
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                className="underline decoration-dotted underline-offset-2 hover:text-foreground"
              >
                {open ? 'hide summary' : 'show summary'}
              </button>
            </>
          )}
        </span>
        <span className="h-px flex-1 bg-border/70" />
      </div>
      {open && entry.summary && (
        <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border/60 bg-muted/30 px-3 py-2 font-sans text-[12.5px] leading-relaxed text-foreground/85">
          {entry.summary}
        </pre>
      )}
    </div>
  );
}

/** Goal lifecycle events in the transcript.
 *
 * Design principles:
 *  - No border, no left accent bar, no pill. The event is anchored by
 *    a colored Target icon + a status-tinted headline; that's enough
 *    to distinguish a goal beat from surrounding assistant text
 *    without a second visual layer of chrome.
 *  - The reason (evaluator note or original condition) breathes below
 *    the headline in muted body text, indented to align under the
 *    headline for a clean two-tier read.
 *  - Terminal statuses use short, positive English ("Goal met") not
 *    protocol-speak ("goal_done · met"). */
function GoalTranscriptChip({ entry }: { entry: GoalEntry }) {
  const tint = goalChipTint(entry.status, entry.variant);
  const headline = goalChipHeadline(entry);
  const body = entry.reason ?? entry.condition ?? null;
  return (
    <div className="flex justify-start">
      <div className="w-full max-w-2xl py-1.5">
        <div className="flex items-baseline gap-2">
          <Target
            fill="currentColor"
            className={cn(
              // Baseline-align the icon with the headline text — the
              // `translate-y-[1px]` nudges it visually onto the x-height
              // instead of floating above the cap-line.
              'size-3.5 shrink-0 translate-y-[1px]',
              tint.icon,
            )}
          />
          <span
            className={cn(
              'text-[13px] font-semibold tracking-tight',
              tint.head,
            )}
          >
            {headline}
          </span>
        </div>
        {body && (
          <div className="mt-1 pl-[22px] whitespace-pre-wrap break-words text-[13px] leading-relaxed text-muted-foreground">
            {body}
          </div>
        )}
      </div>
    </div>
  );
}

function goalChipHeadline(entry: GoalEntry): string {
  const iter = entry.iteration && entry.maxIterations
    ? ` · iteration ${entry.iteration} of ${entry.maxIterations}`
    : '';
  switch (entry.variant) {
    case 'set':
      return 'Goal set';
    case 'cleared':
      return 'Goal cleared';
    case 'progress':
      return `Goal · still working${iter}`;
    case 'done': {
      const status = entry.status;
      if (status === 'met') return 'Goal met';
      if (status === 'impossible') return 'Goal is impossible';
      if (status === 'needs_user') return 'Goal needs your input';
      if (status === 'exhausted') return 'Goal hit its iteration cap';
      if (status === 'cleared') return 'Goal cleared';
      return 'Goal finished';
    }
  }
}

/** Icon + headline color per (variant, status). No bg / bar / border
 *  fields — those live in the render function above and are always
 *  transparent by design. */
function goalChipTint(
  status: GoalEntry['status'],
  variant: GoalEntry['variant'],
) {
  if (variant === 'set' || variant === 'progress') {
    return { icon: 'text-mira-purple', head: 'text-mira-purple' };
  }
  if (variant === 'cleared' || status === 'cleared') {
    return { icon: 'text-muted-foreground', head: 'text-muted-foreground' };
  }
  switch (status) {
    case 'met':
      return { icon: 'text-emerald-400', head: 'text-emerald-300' };
    case 'impossible':
      return { icon: 'text-red-400', head: 'text-red-300' };
    case 'needs_user':
    case 'exhausted':
      return { icon: 'text-amber-400', head: 'text-amber-300' };
    default:
      return { icon: 'text-mira-purple', head: 'text-mira-purple' };
  }
}

/* ------------------------------------------------------------------ */
/* Message actions: copy, edit & resend, retry                          */
/* ------------------------------------------------------------------ */

type MessageActions = {
  busy: boolean;
  /** Rewind to this user message and send `text` in its place. */
  edit: (entry: Entry, text: string) => void;
  /** Re-send the user message that led to this reply. */
  retry: (entry: Entry) => void;
  /** Offer to put the files back the way they were before this message. */
  restore: (entry: Entry) => void;
  /** Show an image full-screen. */
  openImage: (src: string) => void;
};

const MessageActionsContext = createContext<MessageActions | null>(null);

/** Stats for the turn a final reply closes — shown in its hover row. */
type TurnStats = {
  durationMs: number;
  usage: UsageTotals | null;
  model: string;
  /** The agent's own cost estimate, when it reports one. */
  costUsd?: number | null;
};
const TurnStatsContext = createContext<TurnStats | null>(null);

function turnStatsLabel(t: TurnStats): string {
  const parts = [formatDuration(t.durationMs)];
  if (t.usage) {
    parts.push(`${shortNum(t.usage.prompt_tokens)} in · ${shortNum(t.usage.completion_tokens)} out`);
    const cost = t.costUsd ?? costUsd(t.model, t.usage);
    if (cost != null) parts.push(formatDollars(cost));
  }
  return parts.join(' · ');
}

function ActionButton({
  title,
  onClick,
  disabled,
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      className="rounded-md p-1 text-muted-foreground/60 transition-colors hover:bg-accent/50 hover:text-foreground disabled:pointer-events-none disabled:opacity-30"
    >
      {children}
    </button>
  );
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <ActionButton
      title={copied ? 'Copied' : 'Copy'}
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
    </ActionButton>
  );
}

function AssistantActions({ entry, text }: { entry: Entry; text: string }) {
  const actions = useContext(MessageActionsContext);
  const stats = useContext(TurnStatsContext);
  const showStats = getBoolPref(PREF_KEYS.transcriptTurnStats, true);
  if (!text.trim()) return null;
  return (
    <div className="mt-1 flex items-center gap-0.5 opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100">
      <CopyButton text={text} />
      {actions && (
        <ActionButton title="Retry" disabled={actions.busy} onClick={() => actions.retry(entry)}>
          <RotateCw className="size-3.5" />
        </ActionButton>
      )}
      {showStats && stats && (
        <span className="ml-1.5 text-[11.5px] tabular-nums text-muted-foreground/60">{turnStatsLabel(stats)}</span>
      )}
    </div>
  );
}

/** User bubble with hover actions; Edit swaps it for a textarea that
 *  re-sends from this point (Enter to send, Esc to cancel). */
function UserMessage({
  entry,
  text,
  raw,
  children,
}: {
  entry: Entry;
  /** Display text (attachments stripped) — what Copy copies. */
  text: string;
  /** Exact sent text — what Edit starts from. */
  raw: string;
  children: React.ReactNode;
}) {
  const actions = useContext(MessageActionsContext);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(raw);
  const submit = () => {
    const t = draft.trim();
    if (!t || !actions) return;
    setEditing(false);
    actions.edit(entry, t);
  };

  if (editing) {
    return (
      <div className="flex w-full max-w-[78%] flex-col gap-2 rounded-2xl border border-border bg-secondary/60 p-2.5">
        <textarea
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              submit();
            } else if (e.key === 'Escape') {
              setEditing(false);
            }
          }}
          rows={Math.min(10, Math.max(2, draft.split('\n').length))}
          className="w-full resize-none bg-transparent px-1.5 text-[14.5px] outline-none"
        />
        <div className="flex items-center justify-end gap-1.5">
          <span className="mr-auto px-1 text-[11px] text-muted-foreground/60">
            Later messages are replaced; file edits stay (restore them with ↺).
          </span>
          <button
            type="button"
            onClick={() => setEditing(false)}
            className="rounded-md px-2.5 py-1 text-[12px] text-muted-foreground hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={!draft.trim() || actions?.busy}
            className="rounded-md bg-foreground px-2.5 py-1 text-[12px] font-medium text-background disabled:opacity-40"
          >
            Send
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="group/msg flex max-w-[78%] flex-col items-end">
      <div className="whitespace-pre-wrap break-words rounded-2xl rounded-br-md bg-secondary px-4 py-2.5 text-[14.5px]">
        {children}
      </div>
      <div className="mt-1 flex items-center gap-0.5 opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100">
        <CopyButton text={text} />
        {actions && (
          <ActionButton
            title="Edit"
            disabled={actions.busy}
            onClick={() => {
              setDraft(raw);
              setEditing(true);
            }}
          >
            <Pencil className="size-3.5" />
          </ActionButton>
        )}
        {actions && (
          <ActionButton
            title="Restore files to before this message"
            disabled={actions.busy}
            onClick={() => actions.restore(entry)}
          >
            <History className="size-3.5" />
          </ActionButton>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Status lines: harness notes riding the warning stream                */
/* ------------------------------------------------------------------ */

type StatusTone = 'ok' | 'warn' | 'error' | 'busy' | 'info' | 'memory';

/** Classify a `[channel] body` note into an icon tone and display text. */
function parseStatus(raw: string, forced?: StatusTone): { tone: StatusTone; text: string; detail?: string } {
  const m = raw.match(/^\[([a-z-]+)\]\s*([\s\S]*)$/);
  const channel = m?.[1] ?? '';
  let text = m ? m[2] : raw;
  let detail: string | undefined;
  if (channel === 'environment') {
    const [head, ...rest] = text.split('\n');
    text = head;
    detail = rest.join('\n') || undefined;
  }
  if (forced) return { tone: forced, text, detail };
  switch (channel) {
    case 'verify':
      if (/running/i.test(text)) return { tone: 'busy', text: text.replace(/^running\s*/i, 'Running '), detail };
      if (/passed/i.test(text)) return { tone: 'ok', text, detail };
      return { tone: 'warn', text, detail };
    case 'undo':
      return { tone: 'ok', text, detail };
    case 'progress':
      return { tone: 'busy', text, detail };
    case 'memory':
      return { tone: 'memory', text, detail };
    case 'context':
    case 'environment':
      return { tone: 'info', text, detail };
    default:
      return { tone: 'warn', text, detail };
  }
}

/** Render `code` spans in a status message. */
function withCode(text: string): React.ReactNode[] {
  return text.split(/(`[^`]+`)/g).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') && part.length > 2 ? (
      <code key={i} className="rounded bg-fg/[0.06] px-1 font-mono text-[11.5px] text-foreground/75">
        {part.slice(1, -1)}
      </code>
    ) : (
      part
    ),
  );
}

/** One quiet line: a small tone icon and muted text. No box, no border —
 *  the icon carries the state so notes don't compete with the reply. */
function StatusLine({ text, tone }: { text: string; tone?: StatusTone }) {
  const s = parseStatus(text, tone);
  const icon = {
    ok: <Check strokeWidth={2.5} className="size-3 text-emerald-400/80" />,
    warn: <ShieldAlert className="size-3.5 text-amber-400/80" />,
    error: <ShieldAlert className="size-3.5 text-destructive" />,
    busy: <LoaderCircle strokeWidth={2.5} className="size-3 animate-spin text-muted-foreground" />,
    info: <Info className="size-3.5 text-muted-foreground/70" />,
    memory: <Sparkle className="size-3.5 text-violet-300/70" />,
  }[s.tone];
  return (
    <div className="flex items-start gap-2 py-0.5 text-[12.5px] leading-5 text-muted-foreground">
      <span className="flex h-5 shrink-0 items-center">{icon}</span>
      <span className="min-w-0 break-words">
        {withCode(s.text)}
        {s.detail && (
          <pre className="mt-0.5 whitespace-pre-wrap break-words font-mono text-[11.5px] text-muted-foreground/70">
            {s.detail}
          </pre>
        )}
      </span>
    </div>
  );
}
