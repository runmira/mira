import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence } from 'framer-motion';
import {
  ArrowClockwise,
  ArrowDown,
  CaretDown,
  Check,
  Copy,
  Lightbulb,
  PencilSimple,
  ShieldWarning,
  SidebarSimple,
  Sparkle,
  Target,
} from '@phosphor-icons/react';
import { cn } from './lib/utils';
import { connect, type WsClient, type WsStatus } from './ws';
import { appendMemory, applyUndo, getBranchPr, getGitStatus, getSessionDiff, getSessionHistory, getSettings, gitCommit, gitPush, listCommands, listSkills, newSession, setSessionBackgroundMode, startReview, type BranchPrView, type GitStatusView, type SessionDiffView, type SkillView, type CommandInfo } from './api';
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
import { ImageLightbox } from './components/ImageLightbox';
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
import { AgentCard, AgentGroup } from './components/AgentCard';
import miraLogo from './assets/mira-logo.png';
import { SubagentPanel, type SubagentTab, type FilePanelTab } from './components/SubagentPanel';
import { TaskListPanel } from './components/TaskListPanel';
import { GoalPanel } from './components/GoalPanel';
import { countsByCategory, countsPhrase, ToolGroup } from './components/ToolGroup';
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
  RateLimitReading,
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
export type Entry =
  | MsgEntry
  | ToolEntry
  | WarningEntry
  | ErrorEntry
  | GoalEntry
  | CompactEntry
  | ThoughtEntry;

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
        entries.push({ kind: 'msg', msg: m });
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

  const playPing = useCallback(() => {
    try {
      const audio = new Audio('/ping.mp3');
      audio.volume = 0.7;
      void audio.play();
    } catch { /* audio unavailable */ }
  }, []);

  const [status, setStatus] = useState<WsStatus>('connecting');
  const [sessionId, setSessionId] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [mode, setMode] = useState<Mode>('manual');
  const [cwd, setCwd] = useState<string>('');
  const [entries, setEntries] = useState<Entry[]>([]);
  // Per-turn timing. Turn index = 0-based order of user messages in `entries`.
  // Only turns started in *this* session have timing (reloaded transcripts
  // have no wall-clock data, so their turns skip the "Worked for" header).
  const [turnTimings, setTurnTimings] = useState<Map<number, TurnTiming>>(new Map());
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
  // Leave settings — pop back to wherever the user was. Falls back to
  // chat if the remembered view was somehow also settings (shouldn't
  // happen, but a stale value shouldn't strand the user).
  function exitSettings() {
    setMainView(settingsReturnTo === 'settings' ? 'chat' : settingsReturnTo);
  }
  // Left sidebar visibility — collapses the 300px column to 0.
  const [sidebarOpen, setSidebarOpen] = useState(true);
  // Right-side panel: `agentTabs` is the ordered list of open agent call_ids;
  // `fileTabs` is the ordered list of open file viewer tabs; `activeAgentTab`
  // is the visible panel's id (either a callId or a file path). Panel is open
  // iff either list is non-empty.
  const [agentTabs, setAgentTabs] = useState<string[]>([]);
  const [fileTabs, setFileTabs] = useState<FilePanelTab[]>([]);
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
  const [lightbox, setLightbox] = useState<string | null>(null);
  const ctxFits = useContextPanelFits();
  const [branchPr, setBranchPr] = useState<BranchPrView | null>(null);
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
    followRef.current = atBottom;
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

  function onMessage(msg: ServerMsg) {
    switch (msg.type) {
      case 'ready': {
        setGitStatus(null);
        setSessionDiff({ added: 0, removed: 0, files: [] });
        setSessionCommitted(false);
        setBranchPr(null);
        setSessionId(msg.session_id);
        setModel(msg.model);
        setMode(msg.mode);
        setCwd(msg.cwd);
        const readyEntries = historyToEntries(msg.history, msg.previews);
        setEntries(readyEntries);
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
        getGitStatus().then((s) => { setGitStatus(s); getBranchPr().then(setBranchPr).catch(() => setBranchPr(null)); }).catch(() => {});
        getSessionDiff().then(setSessionDiff).catch(() => {});
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
        setEntries((prev) => appendToken(prev, msg.text));
        break;
      case 'approval_request':
        setThinking(false);
        clearThinkingIdle();
        playPing();
        setEntries((prev) => [
          ...sealThought(prev),
          { kind: 'tool', call: msg.call, preview: msg.preview ?? null, status: 'pending', result: null },
        ]);
        break;
      case 'tool_start':
        setThinking(false);
        clearThinkingIdle();
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
      case 'done':
        setBusy(false);
        setThinking(false);
        clearThinkingIdle();
        setEntries(sealThought);
        playPing();
        // Refresh git status, session diff, and branch PR after each turn.
        getGitStatus().then(setGitStatus).catch(() => {});
        getSessionDiff().then(setSessionDiff).catch(() => {});
        getBranchPr().then(setBranchPr).catch(() => setBranchPr(null));
        // Close out the most recent turn's timing.
        setTurnTimings((prev) => stampLastTurn(prev, Date.now()));
        setSidebarRefresh((n) => n + 1);
        break;
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
        setEntries((prev) => [...prev, { kind: 'warning', text: msg.text }]);
        break;
      case 'tool_progress':
        setEntries((prev) => appendProgressLine(prev, msg.call_id, msg.line));
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
        // switches from the first-user-message fallback to the AI title.
        setSidebarRefresh((n) => n + 1);
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
        setEntries((prev) => {
          const hit = prev.some((e) => e.kind === 'tool' && e.call.id === msg.prompt_id);
          if (!hit) {
            pendingAskUserRef.current.set(msg.prompt_id, msg.proposal);
            return prev;
          }
          return attachAskUserProposal(prev, msg.prompt_id, msg.proposal);
        });
        break;
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
        .map((e) => ({ callId: e.call.id, call: e.call, preview: e.preview })),
    [entries],
  );

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

  // Global Y/N shortcut for the first pending approval. Rebinds when
  // the head-of-queue call changes so back-to-back approvals each
  // pick up their own listener. Skipped while the user is typing so
  // "y" and "n" in the composer/settings don't fire the decision.
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
      if (e.key === 'y' || e.key === 'Y') {
        e.preventDefault();
        decideApproval(firstPendingCallId, true);
      } else if (e.key === 'n' || e.key === 'N') {
        e.preventDefault();
        decideApproval(firstPendingCallId, false);
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [firstPendingCallId]);

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
    setEntries((prev) => {
      const next: Entry[] = [...prev, { kind: 'msg', msg: { role: 'user', content: text, images } }];
      const turnIndex = countUserMessages(next) - 1;
      setTurnTimings((tt) => {
        const clone = new Map(tt);
        clone.set(turnIndex, { startedAt: now, endedAt: null });
        return clone;
      });
      return next;
    });
    setBusy(true);
    setThinking(true);
    wsRef.current?.send({ type: 'send', text, images });
  }

  /** Edit & resend (or retry, with the same text) the user message at
   *  `userIdx`: the server rewinds history to just before it and starts a
   *  new turn. Later entries are dropped here to match. */
  function onResend(userIdx: number, text: string) {
    const target = entries[userIdx];
    if (busy || !target || target.kind !== 'msg' || target.msg.role !== 'user') return;
    const original = target.msg.content ?? '';
    const occurrence = entries
      .slice(userIdx + 1)
      .filter((e) => e.kind === 'msg' && e.msg.role === 'user' && e.msg.content === original).length;
    followRef.current = true;
    setShowJump(false);
    const next: Entry[] = [
      ...entries.slice(0, userIdx),
      { kind: 'msg', msg: { role: 'user', content: text, images: target.msg.images } },
    ];
    const turnIndex = countUserMessages(next) - 1;
    setEntries(next);
    setTurnTimings((tt) => {
      const clone = new Map([...tt].filter(([i]) => i < turnIndex));
      clone.set(turnIndex, { startedAt: Date.now(), endedAt: null });
      return clone;
    });
    setBusy(true);
    setThinking(true);
    wsRef.current?.send({ type: 'resend', original, occurrence, text });
  }

  const messageActions = useMemo<MessageActions>(
    () => ({
      busy,
      openImage: setLightbox,
      edit: (entry, text) => onResend(entries.indexOf(entry), text),
      retry: (entry) => {
        const at = entries.indexOf(entry);
        for (let i = at - 1; i >= 0; i--) {
          const e = entries[i];
          if (e.kind === 'msg' && e.msg.role === 'user') {
            onResend(i, e.msg.content ?? '');
            return;
          }
        }
      },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [busy, entries],
  );

  function onSetMode(m: Mode) { wsRef.current?.send({ type: 'set_mode', mode: m }); }
  function onSetModel(m: string) { wsRef.current?.send({ type: 'set_model', model: m }); }
  function onSetEffort(e: string | null) { wsRef.current?.send({ type: 'set_effort', effort: e }); }

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

  const settingsHandler = (v: SettingsView) => {
    setConfigured(v.configured);
    setProviderName(v.default_provider ?? null);
    if (v.default_model) setModel(v.default_model);
    if (v.default_mode) setMode(v.default_mode as Mode);
  };

  const isEmpty = useMemo(
    () => entries.every((e) => e.kind === 'msg' && !(e.msg.content || '').trim() && (e.msg.tool_calls?.length ?? 0) === 0),
    [entries],
  );

  const turns = useMemo(() => groupByTurn(entries), [entries]);
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
  const ctxReserve = ctxOpen && ctxHasContent;
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
        // Fall through to remaining agent tabs, then file tabs, then null.
        const remaining = [
          ...next,
          ...fileTabs.map((t) => t.id),
        ];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  function closeSubagentPanel() {
    setAgentTabs([]);
    setFileTabs([]);
    setActiveAgentTab(null);
  }

  function openFileTab(path: string, diff: DiffPreview | null) {
    setFileTabs((prev) => {
      // If already open, update the diff (re-opening after a new write).
      if (prev.some((t) => t.id === path)) {
        return prev.map((t) => t.id === path ? { ...t, diff } : t);
      }
      return [...prev, { id: path, path, diff }];
    });
    setActiveAgentTab(path);
  }

  function closeFileTab(tabId: string) {
    setFileTabs((prev) => {
      const next = prev.filter((t) => t.id !== tabId);
      if (activeAgentTab === tabId) {
        const remaining = [...agentTabs, ...next.map((t) => t.id)];
        setActiveAgentTab(remaining.length > 0 ? remaining[remaining.length - 1] : null);
      }
      return next;
    });
  }

  function closeAnyTab(id: string) {
    if (agentTabs.includes(id)) closeAgentTab(id);
    else closeFileTab(id);
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

  const panelOpen = subagentTabs.length > 0 || fileTabs.length > 0;

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
      className="grid h-screen grid-rows-1 bg-background transition-[grid-template-columns] duration-150"
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

      <main className="flex min-w-0 min-h-0 flex-col">
        {mainView === 'chat' && (
          <>
            <div className="flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4">
              <button
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
                <SidebarSimple className="size-4" />
              </button>
              <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">
                {titleFromEntries(entries)}
              </span>
              {(sessionDiff.uncommitted ?? 0) > 0 && (
                <button
                  type="button"
                  onClick={() => setReviewOpen(true)}
                  title="Review this session's changes"
                  className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                >
                  <span className="font-mono text-green-400/80">+{sessionDiff.added}</span>
                  <span className="font-mono text-red-400/80">−{sessionDiff.removed}</span>
                  <span>Review</span>
                </button>
              )}
              <div className="inline-flex rounded-full border border-border bg-secondary/60 p-0.5">
                <button
                  type="button"
                  className="rounded-full bg-secondary px-3.5 py-1 text-[12.5px] font-medium text-foreground"
                >
                  Chat
                </button>
                <button
                  type="button"
                  disabled
                  title="Not implemented yet"
                  className="rounded-full px-3.5 py-1 text-[12.5px] text-muted-foreground/40 cursor-not-allowed"
                >
                  Work
                </button>
              </div>
            </div>

            {/* Transcript — full width, panel floats above it */}
            <div className="relative flex-1 min-h-0">
              <div
                className="absolute inset-0 overflow-y-auto px-5 pb-5 transition-[padding] duration-200"
                style={{
                  paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE + 12 : 20,
                  paddingTop: ctxPill ? 52 : 16,
                }}
                ref={paneRef}
                onScroll={onPaneScroll}
              >
                {configured === false && (
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
                  <EmptyState />
                ) : (
                  <div className="mx-auto flex max-w-3xl flex-col gap-2">
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
                        turn={turn}
                        timing={turnTimings.get(i) ?? null}
                        expanded={expandedTurns.has(i)}
                        onToggle={() => toggleTurn(i)}
                        onDecide={decideApproval}
                        onPlanReply={replyToPlan}
                        onAskUserReply={replyToAskUser}
                        onOpenAgent={openAgentTab}
                        onOpenFile={openFileTab}
                        isActive={busy && i === turns.length - 1}
                        skills={skills}
                        mode={mode}
                        onSetMode={onSetMode}
                      />
                    ))}
                    </MessageActionsContext.Provider>
                    {thinking && (
                      <div className="flex justify-start">
                        <Thinking />
                      </div>
                    )}
                  </div>
                )}
              </div>

              <ImageLightbox src={lightbox} onClose={() => setLightbox(null)} />
              <ReviewChanges
                open={reviewOpen}
                onClose={() => setReviewOpen(false)}
                onSendComments={(text) => onSend(text)}
                onChanged={() => {
                  getGitStatus().then(setGitStatus).catch(() => {});
                  getSessionDiff().then(setSessionDiff).catch(() => {});
                }}
              />

              {showJump && (
                <button
                  type="button"
                  onClick={jumpToLatest}
                  className="absolute bottom-3 left-1/2 z-10 inline-flex -translate-x-1/2 animate-fade-in items-center gap-1.5 rounded-full border border-border bg-secondary/95 px-3 py-1.5 text-[12px] text-muted-foreground shadow-lg backdrop-blur transition-colors hover:text-foreground"
                >
                  <ArrowDown weight="bold" className="size-3" />
                  Jump to latest
                </button>
              )}

              {/* Floating context panel — absolutely anchored to top-right */}
              <AnimatePresence>
                <ContextPanel
                  key="ctx"
                  sessionTitle={titleFromEntries(entries)}
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
                  onReview={() => setReviewOpen(true)}
                  onPush={async () => { await gitPush(); getGitStatus().then(setGitStatus).catch(() => {}); getBranchPr().then(setBranchPr).catch(() => {}); }}
                  onCommit={async (message, includeUnstaged, pushAfter) => {
                    await gitCommit({ message, include_unstaged: includeUnstaged, push_after: pushAfter });
                    setSessionCommitted(true);
                    getGitStatus().then(setGitStatus).catch(() => {});
                    getSessionDiff().then(setSessionDiff).catch(() => {});
                    getBranchPr().then(setBranchPr).catch(() => setBranchPr(null));
                  }}
                />
              </AnimatePresence>
            </div>

            <div
              className="shrink-0 transition-[padding-right] duration-200"
              style={{ paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE : 0 }}
            >
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
              onSetEffort={onSetEffort}
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
              pendingApproval={pendingApprovals[0] ?? null}
              pendingPlan={pendingPlan}
              pendingAskUser={pendingAskUser}
              onDecide={(callId, allow, scope) => decideApproval(callId, allow, scope)}
              onPlanReply={replyToPlan}
              onAskUserReply={replyToAskUser}
              commands={commands}
            />
            </div>
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
          />
        )}
      </main>

      {panelOpen && (
        <SubagentPanel
          tabs={subagentTabs}
          fileTabs={fileTabs}
          activeCallId={activeAgentTab}
          cwd={cwd ?? ''}
          onSelectTab={setActiveAgentTab}
          onCloseTab={closeAnyTab}
          onClose={closeSubagentPanel}
          onReview={replyToSubagentReview}
          onResizeStart={handlePanelResizeStart}
          onOpenFile={openFileTab}
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

function TurnView({
  turn, timing, expanded, isActive, onToggle, onDecide, onPlanReply, onAskUserReply, onOpenAgent, onOpenFile, skills, mode, onSetMode,
}: {
  turn: Turn;
  timing: TurnTiming | null;
  expanded: boolean;
  isActive: boolean;
  onToggle: () => void;
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
}) {
  // Split the body into "intermediate work" and the final assistant text.
  // Rule: the LAST assistant text message with non-empty content is the
  // final answer; everything before it is intermediate. Tool cards + earlier
  // assistant text hide behind the "Worked for" chip when collapsed.
  const finalIdx = findFinalAssistantIndex(turn.body);
  const intermediateRaw = finalIdx >= 0 ? turn.body.slice(0, finalIdx) : turn.body;
  const finalEntry = finalIdx >= 0 ? turn.body[finalIdx] : null;
  const trailing = finalIdx >= 0 ? turn.body.slice(finalIdx + 1) : [];

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
  const showWorkedChip = (intermediate.length > 0 || isActive) && durationMs != null;

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
    <>
      {turn.user && <EntryView entry={turn.user} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />}

      {showWorkedChip && (
        <WorkedForChip
          durationMs={durationMs!}
          active={isActive && timing?.endedAt == null}
          waitingForUser={waitingForUser}
          expanded={effectivelyExpanded}
          locked={forceOpen}
          onToggle={onToggle}
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
          <EntryView key={`t-i-${i}`} entry={item.entry} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />
        );
      })}

      {finalEntry && <EntryView entry={finalEntry} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />}
      {trailing.map((e, i) => (
        <EntryView key={`t-t-${i}`} entry={e} onDecide={onDecide} onPlanReply={onPlanReply} onAskUserReply={onAskUserReply} onOpenAgent={onOpenAgent} onOpenFile={onOpenFile} skills={skills} mode={mode} onSetMode={onSetMode} />
      ))}
    </>
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
      <CaretDown
        weight="bold"
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

function EmptyState() {
  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-4 text-muted-foreground">
      <img
        src={miraLogo}
        alt="Mira"
        className="size-20 rounded-full object-contain drop-shadow-[0_0_28px_rgba(88,101,242,0.35)]"
        draggable={false}
      />
      <div className="text-[22px] font-normal tracking-tight text-foreground">
        What should we build today?
      </div>
    </div>
  );
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
                <Lightbulb weight="fill" className="size-3.5 shrink-0 text-mira-blue/70" />
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
                <Sparkle weight="fill" className="size-3.5 shrink-0 text-mira-blue/70" />
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
              <ShieldWarning weight="fill" className="size-3.5 shrink-0 text-amber-400/80" />
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
          />
        </div>
      );
    case 'warning': {
      // Special channels riding the warning stream get their own chip:
      //   `[undo] ...`          → green success chip
      //   `[file-conflict] ...` → amber warning chip
      //   `[verify] … passed`   → green success chip
      //   `[verify] … failed`   → amber warning chip
      //   `[verify] running`    → blue in-flight chip
      //   `[progress] ...`      → blue "in-flight status" chip (subagent
      //                            emit_progress → SubagentPanel stream)
      //   `[memory] ...`        → violet "learned" chip
      //   `[context] ...`       → slate "compacted" chip
      //   `[environment] ...`   → blue "environment" chip (multi-line)
      // Everything else stays the compact monospace `! …` line.
      const undo = entry.text.match(/^\[undo\]\s*(.*)$/);
      const conflict = entry.text.match(/^\[file-conflict\]\s*(.*)$/);
      const verify = entry.text.match(/^\[verify\]\s*(.*)$/);
      const progress = entry.text.match(/^\[progress\]\s*(.*)$/);
      const memory = entry.text.match(/^\[memory\]\s*(.*)$/);
      const context = entry.text.match(/^\[context\]\s*(.*)$/);
      const environment = entry.text.match(/^\[environment\]\s*([\s\S]*)$/);
      if (environment) {
        const [head, ...rest] = environment[1].split('\n');
        return (
          <div className="flex justify-start">
            <div className="inline-flex max-w-full flex-col gap-0.5 rounded-md border border-mira-blue/25 bg-mira-blue/[0.06] px-3 py-1.5 text-[12.5px] text-mira-blue">
              <span className="font-semibold">{head}</span>
              {rest.length > 0 && (
                <pre className="whitespace-pre-wrap break-words font-mono text-[11.5px] text-foreground/70">{rest.join('\n')}</pre>
              )}
            </div>
          </div>
        );
      }
      if (undo) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-emerald-500/25 bg-emerald-500/[0.06] px-3 py-1.5 text-[12.5px] text-emerald-300">
              <span className="font-semibold">↩ undo</span>
              <span className="min-w-0 break-words text-emerald-200/90">{undo[1]}</span>
            </div>
          </div>
        );
      }
      if (conflict) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-[12.5px] text-amber-200">
              <span className="font-semibold">⚠ file conflict</span>
              <span className="min-w-0 break-words text-amber-100/90">{conflict[1]}</span>
            </div>
          </div>
        );
      }
      if (progress) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-mira-blue/25 bg-mira-blue/[0.06] px-3 py-1.5 text-[12.5px] text-mira-blue">
              <span className="font-semibold">… progress</span>
              <span className="min-w-0 break-words opacity-90">{progress[1]}</span>
            </div>
          </div>
        );
      }
      if (verify) {
        const body = verify[1];
        const passed = /passed/i.test(body);
        const running = /running/i.test(body);
        const cls = passed
          ? 'border-emerald-500/25 bg-emerald-500/[0.06] text-emerald-300'
          : running
            ? 'border-mira-blue/25 bg-mira-blue/[0.06] text-mira-blue'
            : 'border-amber-500/30 bg-amber-500/10 text-amber-200';
        return (
          <div className="flex justify-start">
            <div className={cn('inline-flex items-start gap-2 rounded-md border px-3 py-1.5 text-[12.5px]', cls)}>
              <span className="font-semibold">✓ verify</span>
              <span className="min-w-0 break-words opacity-90">{body}</span>
            </div>
          </div>
        );
      }
      if (memory) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-violet-500/25 bg-violet-500/[0.06] px-3 py-1.5 text-[12.5px] text-violet-300">
              <span className="font-semibold">✦ memory</span>
              <span className="min-w-0 break-words text-violet-200/90">{memory[1]}</span>
            </div>
          </div>
        );
      }
      if (context) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-slate-500/25 bg-slate-500/[0.06] px-3 py-1.5 text-[12.5px] text-slate-300">
              <span className="font-semibold">≡ context</span>
              <span className="min-w-0 break-words text-slate-200/90">{context[1]}</span>
            </div>
          </div>
        );
      }
      return (
        <div className="flex justify-start">
          <div className="font-mono text-xs text-mira-tool">! {entry.text}</div>
        </div>
      );
    }
    case 'error':
      return (
        <div className="flex justify-start">
          <div className="font-mono text-xs text-destructive">error: {entry.text}</div>
        </div>
      );
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
            weight="fill"
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
  /** Show an image full-screen. */
  openImage: (src: string) => void;
};

const MessageActionsContext = createContext<MessageActions | null>(null);

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
  if (!text.trim()) return null;
  return (
    <div className="mt-1 flex items-center gap-0.5 opacity-0 transition-opacity group-hover/msg:opacity-100 focus-within:opacity-100">
      <CopyButton text={text} />
      {actions && (
        <ActionButton title="Retry" disabled={actions.busy} onClick={() => actions.retry(entry)}>
          <ArrowClockwise className="size-3.5" />
        </ActionButton>
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
            Later messages are replaced; file edits stay.
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
            <PencilSimple className="size-3.5" />
          </ActionButton>
        )}
      </div>
    </div>
  );
}
