import { type RecoveryAction } from './components/UsageRecoveryCard';
import {useBackgroundProcesses} from './lib/backgroundProcesses';
import { QueueMutations } from './lib/queueMutations';
import { applySessionActivity, type SessionActivityState, type SessionActivityUpdate } from './lib/sessionActivity';
import { groupByTurn } from './lib/turnActivity';
import { ChatRelationships } from './components/ChatRelationships';
import { WorkspaceSetupCard } from './components/WorkspaceSetupCard';
import { SourceCitationNavigator } from './components/SourceCitationNavigator';
import { getAgentSettings, putAgentSettings } from './api';
import { AssistantSelectionToolbar } from './components/AssistantSelectionToolbar';
import { composeQuote, setAsidePassage } from './lib/attachBridge';
import { VirtualTranscript, hasTranscriptPosition } from './components/VirtualTranscript';
import { applyNativeMetadata } from './lib/nativeStream';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence } from 'framer-motion';
import {
  ArrowDown,
  Pencil,
  Palette,
  Sun,
  Moon,
  PanelLeft,
  History,
  X,
  Download,
} from 'lucide-react';
import { cn } from './lib/utils';
import { acpOptionsToDescriptors } from './lib/acpOptions';
import { loadAgentCaps, migrateLegacyAgentConfigs, saveAgentCaps } from './lib/acpAgents';
import { SecretPrompt, type SecretRequest } from './components/SecretPrompt';
import { loadModelOptions } from './lib/models';
import type { UsageRingData } from './components/UsageRing';
import { isAgentRequest } from './lib/agentRequest';
import {
  getCustomKeybindingRules,
  resolveShortcutCommand,
  shortcutLabelForCommand,
  useKeybindings,
  type ShortcutMatchContext,
} from './lib/keybindings';
import { applyReduceMotion, getBoolPref, PREF_KEYS } from './lib/prefs';
import { connect, type WsClient, type WsStatus } from './ws';
import { costUsd, ensureServerPricing, shortNum } from './lib/usage';
import { forkSession, getContextBreakdown, previewCheckpoint, restoreCheckpoint, undoRestore, type MessageRef, type RestoreChange, type Restored } from './api';
import { appendMemory, applyUndo, getBranchPr, getGitStatus, getSessionDiff, getSessionHistory, getSettings, gitCommit, gitPush, listCommands, listEngines, listSessions, listSkills, newSession, putCwd, setSessionBackgroundMode, startReview, type BranchPrView, type EngineSnapshot, type GitStatusView, type SessionDiffView, type SkillView, type CommandInfo } from './api';
import {
  ContextPanel,
  CONTEXT_PANEL_RESERVE,
  contextPanelHasContent,
  useContextPanelFits,
} from './components/ContextPanel';
import { extractAgentId } from './components/AgentCard';
import { preloadOnIntent, useLatch } from './lib/lazy';
import { LazyBoundary } from './components/LazyBoundary';
import {
  ActivityPane,
  AsidePane,
  CommandPalette,
  ContextInspector,
  DevicesPane,
  FolderPicker,
  GetStarted,
  GoalPanel,
  ImportChats,
  PluginsPanel,
  ProcessesPane,
  PullRequestPanel,
  ReviewChanges,
  SettingsSurface,
  SubagentPanel,
  TerminalPanel,
  TestsPane,
} from './lazyViews';
import { Sidebar, type MainView } from './components/Sidebar';
import { hasHiddenTitleBar, isDesktop, pickFolder } from './lib/desktop';
import { useIsPhone, useModalFocus } from './lib/mobile';
import { RightPanelButton } from './components/RightPanelButton';
import { attachFilesToComposer, dataUrlToFile } from './lib/attachBridge';
import type { ActivityTurn } from './components/panes/ActivityPane';
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
  type PendingApproval,
  type QueuedComposerMessage,
} from './components/Composer';
import { EditorPicker } from './components/EditorPicker';
import { TimelineMinimap, type MinimapItem } from './components/TimelineMinimap';
import { ImageLightbox } from './components/ImageLightbox';
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
  MessageCircleQuestion,
  FlaskConical,
  Activity,
  Smartphone,
} from 'lucide-react';
import type { PaletteAction } from './components/CommandPalette';
import { splitFileRef } from './lib/refs';
import { playTurnSound } from './lib/sound';
import { resolveTheme, setThemePref } from './lib/theme';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { callForAttention } from './lib/attention';

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
import { Thinking } from './components/Thinking';
import {
  applyReviewEvent,
  emptyReviewState,
  ReviewPanel,
  type ReviewState,
} from './components/ReviewPanel';
import { type AskUserDecision } from './components/AskUserCard';
import { ApprovalDialog } from './components/ApprovalDialog';
import {
  mapPosturesToModes,
  MIRA_MODE_TO_POSTURE,
  POSTURES,
} from './lib/agentPostures';
import type { SubagentTab, FilePanelTab } from './components/SubagentPanel';
import { TaskListPanel } from './components/TaskListPanel';
import { categoryFor } from './components/ToolGroup';
import { EntryBoundary } from './components/EntryBoundary';
import { SecondOpinion } from './components/SecondOpinion';
import type {
  EnvironmentInfo,
  EnvironmentStatus,
  ApprovalScope,
  AskUserProposal,
  DiffPreview,
  Goal,
  Mode,
  PlanProposal,
  PlanStep,
  ServerMsg,
  SettingsView,
  TaskItem,
  UsageTotals,
  RateLimitReading,
  AcpAgentStatus,
  AcpConfigOption,
  AcpSessionMode,
  AgentPostureMapping,
  SessionEngine,
} from './types';
import { agentUpdateNotice, type InfoNotice, InfoNoticeHost, noticeId } from './components/InfoNoticeHost';
import { ComingSoon, EmptyState } from './components/transcript/EmptyState';
import { type MessageActions, MessageActionsContext } from './components/transcript/EntryView';
import { TurnView } from './components/transcript/TurnView';
import { type Entry, type TurnTiming, type SubagentStreamState, titleFromEntries, appendToken, type NativeFrame, applyNativeFrame, upsertRuntimeQuestion, rebuildTurnTimings, rebuildTurnUsage, rebuildTurnModels, appendReasoning, sealThought, upsertToolStart, attachPlanProposal, attachAskUserProposal, attachToolResult, applyTaskResult, settleTools, stampLastTurn, appendProgressLine, finishCompaction, type GoalEntry, updateSubagent, appendDelegateStep, recordAskUserDecision, recordPlanDecision, appendAcpText, appendAcpThought, upsertAcpTool, sealAcpThought, type AcpPlanEntry, isSuccessfulAcpStop, describeAcpStop, withTurnStats, addTurnUsage, updateTool, countUserMessages, type MsgEntry, type ToolEntry, goalActivity } from './transcript/entries';
import { entriesForTranscriptPage, historyToEntries, interleaveReplay } from './transcript/replay';

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

/** Header titles for the pages besides chat, on a phone. */
const PAGE_TITLES: Record<MainView, string> = {
  chat: 'Chat',
  plugins: 'Plugins',
  'pull-request': 'Pull request',
  scheduled: 'Scheduled',
  settings: 'Settings',
};

export default function App() {
  const pingPrimedRef = useRef(false);

  // Preload the remote price table so Usage shows dollars from the same
  // authoritative rows the server renders, and prime audio playback on the
  // first user gesture so subsequent play() calls are never blocked.
  useEffect(() => {
    void ensureServerPricing();

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
  const [sessionActivity, setSessionActivity] = useState<SessionActivityState | null>(null);
  const sessionActivityRef=useRef<SessionActivityState|null>(null);
  const activityTitlesRef=useRef(new Map<string,string>());
  const [completedSessions,setCompletedSessions]=useState<ReadonlyMap<string,number>>(new Map());
  const [sessionId, setSessionId] = useState<string>('');
  const sessionIdRef = useRef('');
  const [queuedBySession, setQueuedBySession] = useState<Map<string, QueuedComposerMessage[]>>(new Map());
  const queuedBySessionRef = useRef(queuedBySession);
  queuedBySessionRef.current = queuedBySession;
  const pendingSteerRef = useRef(new Map<string, QueuedComposerMessage>());
  /** The session's own title (AI-written, or the agent's), once known; the
   *  header falls back to the first message until then. */
  const [sessionTitle, setSessionTitle] = useState<string | null>(null);
  const [model, setModel] = useState<string>('');
  const [mode, setMode] = useState<Mode>('manual');
  const [cwd, setCwd] = useState<string>('');
  const [entries, setEntries] = useState<Entry[]>([]);
  const [ruleEditorCallId, setRuleEditorCallId] = useState<string | null>(null);
  const [approvalRules, setApprovalRules] = useState<Record<string, { rules: string[]; error: string | null }>>({});
  const approvalStarted = useRef<Record<string, number>>({});
  useEffect(() => {
    const pending = new Set(entries.flatMap(entry => entry.kind === 'tool' && entry.status === 'pending' ? [entry.call.id] : []));
    for (const id of Object.keys(approvalStarted.current)) {
      if (!pending.has(id)) delete approvalStarted.current[id];
    }
    setApprovalRules(previous => {
      const retained = Object.entries(previous).filter(([id]) => pending.has(id));
      return retained.length === Object.keys(previous).length ? previous : Object.fromEntries(retained);
    });
    setRuleEditorCallId(previous => previous && pending.has(previous) ? previous : null);
  }, [entries]);
  const [historyCursor, setHistoryCursor] = useState<string | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const historyRequestRef = useRef<string | null>(null);
  const historyPreviewsRef = useRef<Record<string, DiffPreview>>({});
  const historyOffsetRef = useRef(0);
  const providerTurnCursorRef = useRef(0);
  const [turnDiffs, setTurnDiffs] = useState<import('./types').TurnDiffSummary[]>([]);
  function loadOlderHistory() {
    if (!historyCursor || historyLoading || !sessionIdRef.current) return;
    const request_id = crypto.randomUUID();
    historyRequestRef.current = request_id;
    setHistoryLoading(true); setHistoryError(null);
    wsRef.current?.send({ type: 'history', session_id: sessionIdRef.current, cursor: historyCursor, request_id });
    window.setTimeout(() => { if (historyRequestRef.current === request_id) { historyRequestRef.current = null; setHistoryLoading(false); setHistoryError('Older messages could not be loaded. Try again.'); } }, 15000);
  }


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
  const acpAgentsRef = useRef<AcpAgentStatus[]>([]);
  // The agent list, from the engines list: one row per agent driver (its
  // default instance), with the full status the server probed. Agent update
  // notices compare against the previous list, as they did for `acp_status`.
  useEffect(() => {
    if (!engines) return;
    const next = engines
      .filter((e) => e.flavor === 'external' && e.agent && e.instance === e.driver)
      .map((e) => e.agent as AcpAgentStatus);
    if (next.length === 0) return;
    const notice = acpAgentsRef.current.length > 0 ? agentUpdateNotice(acpAgentsRef.current, next) : null;
    acpAgentsRef.current = next;
    setAcpAgents(next);
    if (notice && (!notice.updateAgent || !completedUpdatesRef.current.has(notice.updateAgent))) pushInfoNotice(notice);
    completedUpdatesRef.current.clear();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engines]);
  const completedUpdatesRef = useRef(new Set<string>());
  /** Small and persistent in-window notices for app-level state. */
  const [infoNotices, setInfoNotices] = useState<InfoNotice[]>([]);
  function pushInfoNotice(notice: InfoNotice) {
    setInfoNotices((prev) => [
      ...prev.filter((n) => !(notice.kind === 'persistent' && n.title === notice.title)),
      notice,
    ].slice(-5));
    if (notice.kind === 'short') {
      window.setTimeout(() => {
        setInfoNotices((prev) => prev.filter((n) => n.id !== notice.id));
      }, 3200);
    }
  }
  function dismissInfoNotice(id: string) {
    setInfoNotices((prev) => prev.filter((n) => n.id !== id));
  }
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

  /** Read the engines list. Right after the server starts, agent rows are
   *  placeholders until its background probe lands (`fresh: false`), so
   *  look again a few times rather than showing them as unknown. */
  const loadEngines = useCallback(() => {
    let tries = 0;
    const load = () => {
      void listEngines()
        .then((v) => {
          setEngines(v.engines);
          if (!v.fresh && ++tries < 6) window.setTimeout(load, 2500);
        })
        .catch(() => {});
    };
    load();
  }, []);

  // Agent setups older builds kept in localStorage (API key included) move
  // to the server once, then leave the browser (#79).
  useEffect(() => {
    void migrateLegacyAgentConfigs(putAgentSettings, getAgentSettings)
      .then((moved) => {
        if (moved.length > 0) loadEngines();
      })
      .catch(() => {});
  }, [loadEngines]);

  /** Agent health comes from one place: `GET /api/engines` (#85). This
   *  forces the server to probe every agent again, for the explicit
   *  "check agents" actions; normal loads read the cached list. */
  const requestAcpStatus = useCallback(() => {
    setAcpStatusPending(true);
    void listEngines(true)
      .then((v) => setEngines(v.engines))
      .catch(() => {})
      .finally(() => setAcpStatusPending(false));
  }, []);

  /** Start an agent in this chat. Only names it: its setup comes from
   *  the engine's settings on the server (#79). */
  const startAcpAgent = useCallback((kind: string, resume?: string | null, model?: string | null, instance?: string | null) => {
    setAcpError(null);
    wsRef.current?.send({
      type: 'acp_start',
      driver: kind,
      instance: instance ?? null,
      resume: resume || null,
      model: model || null,
    });
  }, []);
  /** Switch this window to another chat. The composer stops showing the
   *  last chat's turn at once; the new chat's `ready` turns Stop back on if
   *  a turn is running there. */
  function attachSession(id: string) {
    setBusy(false);
    busyRef.current = false;
    setThinking(false);
    wsRef.current?.attach(id);
  }

  /** Point this chat at `path`. The server answers with the id of the
   *  fresh slot it materialised for that folder, so the socket has to
   *  follow it — otherwise it keeps forwarding the previous slot's
   *  frames and the UI silently stays where it was. */
  async function switchCwd(path: string) {
    try {
      const { session_id } = await putCwd(path);
      if (session_id) attachSession(session_id);
    } catch (e) {
      setRestoreNote({ text: String((e as Error).message ?? e), error: true });
    }
  }

  /** "Open folder…" — the OS panel where there is one.
   *
   *  A desktop build hands this to the platform, which knows about
   *  recent places, tags and Cmd+Shift+G. The browser build has no such
   *  panel, so it keeps the in-app browser dialog. On the desktop a
   *  `null` answer is the user closing the panel, so nothing else opens
   *  behind it. */
  async function openProjectPicker() {
    if (!isDesktop()) {
      setPickerOpen(true);
      return;
    }
    const picked = await pickFolder();
    if (picked) await switchCwd(picked);
  }
  function forkAcpAgent() {
    wsRef.current?.send({ type: 'acp_fork' });
  }
  function compactAcpAgent(focus?: string) {
    // A compaction is a turn like any other: busy until its end arrives, or
    // the composer would take input for a session that is summarizing.
    setBusy(true);
    busyRef.current = true;
    setThinking(true);
    // Agents compact inside their own process and only end the turn, so the
    // card runs from here until that turn ends.
    setEntries((prev) => [...prev, {
      kind: 'compact', state: 'running', trigger: 'agent', startedAt: Date.now(), summarized: null, summary: null,
    }]);
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
  const nativeWorkRef = useRef<Extract<ServerMsg, { type: 'runtime_work_updated' }>['work'][]>([]);
  const nativeTurnsRef = useRef<Set<string>>(new Set());
  // Read by the frame handler, which is a long-lived closure: a frame that
  // arrives after the turn ended (a cancelled tool's result, a late
  // turn_complete) must not restart the working indicator.
  const busyRef = useRef(false);
  busyRef.current = busy;
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
  // Lazy dialogs: fetched on first open, then kept mounted so their close
  // transitions still run.
  const inspectOpenMounted = useLatch(inspectOpen);
  const paletteOpenMounted = useLatch(paletteOpen);
  const pickerOpenMounted = useLatch(pickerOpen);
  // ⌘K is usually how the palette opens, so there's no hover to preload
  // on; fetch it once the first load has settled.
  useEffect(() => {
    const idle = window.requestIdleCallback ?? ((cb: () => void) => window.setTimeout(cb, 2000));
    idle(() => CommandPalette.preload());
  }, []);
  /** "Bring chats from other agents", from ⌘K. */
  const [importOpen, setImportOpen] = useState(false);
  // File picker opened from the right panel's "+" menu — distinct from the
  // folder picker, which switches the session's cwd.
  const [panelFilePickerOpen, setPanelFilePickerOpen] = useState(false);
  // Which primary view fills the main pane. Sidebar nav items switch this;
  // starting a chat / loading a session snaps back to 'chat' so the user
  // isn't stranded on a management screen when the model streams a reply.
  const [mainView, setMainView] = useState<MainView>('chat');
  useEffect(() => {
    if (mainView === 'chat') return;
    const over = (event: DragEvent) => {
      if (event.dataTransfer?.types.includes('Files')) event.preventDefault();
    };
    const drop = (event: DragEvent) => {
      if (!event.dataTransfer?.files.length) return;
      event.preventDefault();
      attachFilesToComposer(Array.from(event.dataTransfer.files));
      setMainView('chat');
    };
    window.addEventListener('dragover', over);
    window.addEventListener('drop', drop);
    return () => { window.removeEventListener('dragover', over); window.removeEventListener('drop', drop); };
  }, [mainView]);
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
  // At phone width there's room for one column: the sidebar becomes a
  // drawer over the chat, closed by default and after every pick in it.
  const phone = useIsPhone();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const toggleSidebar = () => (phone ? setDrawerOpen((v) => !v) : setSidebarOpen((v) => !v));
  const closeDrawer = useCallback(() => setDrawerOpen(false), []);
  const drawerTouchX = useRef<number | null>(null);
  const drawerRef = useRef<HTMLDivElement | null>(null);
  useModalFocus(phone && drawerOpen, drawerRef, closeDrawer);
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
  // Why the provider's prompt cache last went unused, for the usage popover.
  const [cacheMiss, setCacheMiss] = useState<string | null>(null);
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
  // Lazy chunk: fetched on first open, then kept mounted so draft review
  // comments survive closing the drawer.
  const reviewMounted = useLatch(reviewOpen);
  /** File the review drawer should open on, when opened from the file list. */
  const [reviewFocus, setReviewFocus] = useState<string | null>(null);
  const [lightbox, setLightbox] = useState<string | null>(null);
  // "Open session" from the Usage page (Settings) — back to chat on it.
  useEffect(() => {
    const onOpen = (e: Event) => {
      const id = (e as CustomEvent<string>).detail;
      if (!id) return;
      attachSession(id);
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
  const chatShortcutRequest = useRef(0);
  const shortcutEntriesRef = useRef(entries);
  shortcutEntriesRef.current = entries;

  /** Live `when`-clause context for the keybinding engine. */
  function shortcutContext(): ShortcutMatchContext {
    const ae = document.activeElement as HTMLElement | null;
    const tag = ae?.tagName;
    const editable = !!ae && (tag === 'INPUT' || tag === 'TEXTAREA' || ae.isContentEditable);
    return {
      terminalFocus: !!ae?.closest('.xterm'),
      // `pendingApprovals` is declared below; this only runs on keydown,
      // long after the whole component body has initialized.
      approvalOpen: visibleApprovalRef.current != null,
      reviewOpen,
      settingsOpen: mainView === 'settings',
      isWeb: true,
      isDesktop: false,
      editableFocus: editable,
    };
  }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if(e.repeat || e.isComposing || (e.target as HTMLElement)?.closest('[data-keybinding-capture]')) return;
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
      if(e.repeat || e.isComposing || (e.target as HTMLElement)?.closest('[data-keybinding-capture]')) return;
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
  const backgroundProcesses=useBackgroundProcesses(sessionId,status==='open'&&mainView==='chat');
  const [selectedProcess,setSelectedProcess]=useState<{session:string;id:number;nonce:number}|null>(null);
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
  const [recoveryBySession, setRecoveryBySession] = useState<Record<string, import('./types').QueuedInput[]>>({});
  const queueMutationsRef = useRef(new QueueMutations());
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
    const c = connect(onMessage, (nextStatus) => {
      queueMutationsRef.current.setConnected(nextStatus === 'open');
      if (nextStatus !== 'open') { flushNativeFrames(); flushTokens(); }
      setStatus(nextStatus);
    });
    wsRef.current = c;
    return () => {
      queueMutationsRef.current.setConnected(false);
      c.close();
      if (nativeRafRef.current != null) cancelAnimationFrame(nativeRafRef.current);
      if (tokenRafRef.current != null) cancelAnimationFrame(tokenRafRef.current);
      nativeFramesRef.current = [];
      tokenBufRef.current = '';
    };
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
  const followRef = useRef(getBoolPref(PREF_KEYS.transcriptFollow, true));
  const readingAnchorRef = useRef<number | null>(null);
  const [showJump, setShowJump] = useState(false);
  const lastScrollInputRef = useRef(0);
  const noteScrollInput = () => { lastScrollInputRef.current = Date.now(); };
  const jumpToLatest = useCallback(() => {
    followRef.current = true;
    setShowJump(false);
    const el = paneRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: 'instant' });
  }, []);
  const onPaneScroll = useCallback(() => {
    const el = paneRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    // Settings → General → Transcript can disable auto-follow; the
    // jump-to-latest button still works (it re-arms follow explicitly).
    if (Date.now() - lastScrollInputRef.current < 1000) followRef.current = atBottom && getBoolPref(PREF_KEYS.transcriptFollow, true);
    setShowJump(!atBottom);
  }, []);
  useLayoutEffect(() => {
    const el = paneRef.current;
    if (!el) return;
    if (readingAnchorRef.current != null) {
      el.scrollTop = readingAnchorRef.current;
      readingAnchorRef.current = null;
    } else if (followRef.current) el.scrollTop = el.scrollHeight;
  }, [entries, thinking]);

  useEffect(() => {
    const pane = paneRef.current;
    const column = pane?.querySelector('[data-transcript-column]');
    if (!pane || !column) return;
    const observer = new ResizeObserver(() => {
      if (followRef.current) pane.scrollTop = pane.scrollHeight;
    });
    observer.observe(column);
    return () => observer.disconnect();
  }, [sessionId, entries.length === 0]);

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
  // Read from WS handlers, which close over the first render.
  const chatTitleRef = useRef<string | null>(null);
  chatTitleRef.current = sessionTitle ?? titleFromEntries(entries);
  function updateSessionActivity(update:SessionActivityUpdate) {
    const previous=sessionActivityRef.current;
    const next=applySessionActivity(previous,update);
    if(next===previous)return;
    sessionActivityRef.current=next;setSessionActivity(next);
    if(!previous||!next||previous.epoch!==next.epoch)return;
    const completed=[...previous.running].filter(id=>!next.running.has(id));
    if(!completed.length)return;
    setCompletedSessions(previous=>{
      const records=new Map(previous);
      for(const id of completed){records.delete(id);records.set(id,next.revision);}
      while(records.size>500)records.delete(records.keys().next().value!);
      return records;
    });
    for(const id of completed) {
      const title=id===sessionIdRef.current?chatTitleRef.current:activityTitlesRef.current.get(id);
      callForAttention('done',title ?? `Chat ${id.slice(0,8)}`,{sessionId:id,whileVisible:id!==sessionIdRef.current,onOpen:()=>{setMainView('chat');wsRef.current?.attach(id);}});
    }
  }

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

  const nativeFramesRef = useRef<NativeFrame[]>([]);
  const nativeRafRef = useRef<number | null>(null);
  function flushNativeFrames() {
    if (nativeRafRef.current != null) cancelAnimationFrame(nativeRafRef.current);
    nativeRafRef.current = null;
    const frames = nativeFramesRef.current;
    nativeFramesRef.current = [];
    if (frames.length) setEntries((entries) => frames.reduce(applyNativeFrame, entries));
  }
  function onMessage(msg: ServerMsg) {
    if (['acp_text', 'acp_text_snapshot', 'acp_thought', 'acp_tool_call', 'acp_tool_call_update', 'acp_tool_output_delta'].includes(msg.type)) {
      flushTokens();
      nativeFramesRef.current.push(msg as NativeFrame);
      // Hidden tabs throttle RAF: bound the backlog and keep replay current.
      if (document.hidden || nativeFramesRef.current.length >= 256) flushNativeFrames();
      else if (nativeRafRef.current == null) nativeRafRef.current = requestAnimationFrame(flushNativeFrames);
      return;
    }
    flushNativeFrames();
    handleMessage(msg);
  }
  function handleMessage(msg: ServerMsg) {
    if (!['token', 'session_activity', 'session_activity_snapshot', 'queue_updated'].includes(msg.type)) flushTokens();
    switch (msg.type) {
      case 'queue_mutation_result':
        queueMutationsRef.current.receive(msg);
        break;
      case 'session_activity':
        updateSessionActivity(msg);
        break;
      case 'session_activity_snapshot':
        updateSessionActivity(msg);
        break;
      case 'turn_diffs': setTurnDiffs(msg.summaries); break;
      case 'stream_activity':
        setEntries(previous => [...previous, { kind: 'activity', activityKind: msg.kind, title: msg.title, detail: msg.detail }]);
        break;
      case 'acp_message_metadata':
        setEntries(previous => applyNativeMetadata(previous, msg.message_id, msg.phase));
        break;
      case 'history_page': {
        if (msg.session_id !== sessionIdRef.current || msg.request_id !== historyRequestRef.current) break;
        historyRequestRef.current = null;
        setHistoryLoading(false); setHistoryError(msg.error);
        if (msg.page) {
          const page = msg.page;
          const pane = paneRef.current;
          const beforeHeight = pane?.scrollHeight ?? 0;
          const beforeTop = pane?.scrollTop ?? 0;
          followRef.current = false;
          historyOffsetRef.current = page.first_turn;
          setHistoryCursor(page.next_cursor);
          setEntries(previous => [...entriesForTranscriptPage(page, historyPreviewsRef.current), ...previous]);
          requestAnimationFrame(() => { if (pane) { pane.scrollTop = beforeTop + pane.scrollHeight - beforeHeight; readingAnchorRef.current = pane.scrollTop; } });
        }
        break;
      }
      case 'queue_updated': {
        setRecoveryBySession(prev => ({ ...prev, [msg.session_id]: msg.items.filter(item => item.recovery) }));
        setQueuedForSession(msg.session_id, () => msg.items.filter(item => !item.recovery).map(item => ({
          id: item.id, text: item.text, images: item.images,
          fingerprint: item.fingerprint,
          steering: item.dispatching && !item.error, error: item.error ?? undefined,
        })));
        break;
      }
      case 'queue_delivery': {
        if (msg.session_id === sessionIdRef.current) sendNow(msg.item.text, msg.item.images, false, msg.item.id);
        break;
      }
      case 'ready': {
        if (msg.session_activity) updateSessionActivity({snapshot:msg.session_activity});
        if (msg.title) {activityTitlesRef.current.set(msg.session_id,msg.title);if(activityTitlesRef.current.size>1000)activityTitlesRef.current.delete(activityTitlesRef.current.keys().next().value!);}
        setRecoveryBySession(prev => ({ ...prev, [msg.session_id]: (msg.queued_inputs ?? []).filter(item => item.recovery) }));
        setQueuedForSession(msg.session_id, () => (msg.queued_inputs ?? []).filter(item => !item.recovery).map(item => ({
          id: item.id, text: item.text, images: item.images,
          fingerprint: item.fingerprint,
          steering: item.dispatching && !item.error, error: item.error ?? undefined,
        })));
        pendingSteerRef.current.delete(msg.session_id);
        if (msg.session_id === sessionIdRef.current && !followRef.current) readingAnchorRef.current = paneRef.current?.scrollTop ?? null;
        else if (msg.session_id !== sessionIdRef.current) { readingAnchorRef.current = null; followRef.current = !hasTranscriptPosition(msg.session_id) && getBoolPref(PREF_KEYS.transcriptFollow, true); }
        agentTerminal.reset();
        setGitStatus(null);
        setSessionDiff({ added: 0, removed: 0, files: [] });
        setSessionCommitted(false);
        setBranchPr(null);
        setSessionId(msg.session_id);
        approvalStarted.current = {};
        setApprovalRules({});
        setRuleEditorCallId(null);
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
        historyRequestRef.current = null;
        setHistoryLoading(false); setHistoryError(null);
        historyPreviewsRef.current = msg.previews ?? {};
        setTurnDiffs(msg.turn_diffs ?? []);
        setHistoryCursor(msg.transcript_page?.next_cursor ?? null);
        historyOffsetRef.current = msg.transcript_page?.first_turn ?? 0;
        providerTurnCursorRef.current = msg.turns?.length ?? msg.history.filter(message => message.role === 'user' && message.input_intent !== 'steer').length;
        const readyEntries = msg.transcript_page ? entriesForTranscriptPage(msg.transcript_page, msg.previews) : historyToEntries(msg.history, msg.previews);
        setEntries((msg.runtime_requests ?? []).reduce(
          (entries, request) => upsertRuntimeQuestion(entries, request),
          msg.transcript_page ? readyEntries : interleaveReplay(msg.history, msg.previews, msg.agent_transcript ?? []),
        ));
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
        setCacheMiss(null);
        setRateLimit(null);
        setTasks(msg.tasks ?? []);
        setGoal(msg.goal ?? null);
        // Opened mid-turn: show Stop and the working indicator until the
        // turn's end arrives, instead of an idle composer over a live reply.
        nativeWorkRef.current = msg.runtime_work ?? [];
        nativeTurnsRef.current.clear();
        setBusy(!!msg.running);
        busyRef.current = !!msg.running;
        setThinking(!!msg.running);
        clearThinkingIdle();
        setSidebarRefresh((n) => n + 1);
        // Refresh the skill roster on every Ready — a cwd swap may
        // change the project tier (~/.mira vs. <cwd>/.mira). Silent on
        // failure; the palette just shows built-in commands.
        listSkills().then(setSkills).catch(() => setSkills([]));
        listCommands().then(setCommands).catch(() => setCommands([]));
        loadEngines();
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
      case 'steer_result': {
        const pending = pendingSteerRef.current.get(msg.session_id);
        if (pending?.id === msg.request_id) {
          pendingSteerRef.current.delete(msg.session_id);
          setQueuedForSession(msg.session_id, items => msg.error
            ? items.map(item => item.id === msg.request_id ? { ...item, steering: false, error: msg.error ?? undefined } : item)
            : items.filter(item => item.id !== msg.request_id));
        }
        if (msg.session_id !== sessionIdRef.current) break;
        if (!busyRef.current) window.setTimeout(drainQueuedMessage, 0);
        setEntries(entries => {
          const index = entries.findIndex(entry => entry.kind === 'msg' && entry.steerRequestId === msg.request_id);
          if (!msg.message) return index < 0 ? entries : entries.filter((_, i) => i !== index);
          const entry: Entry = { kind: 'msg', msg: msg.message, steerRequestId: msg.request_id };
          return index < 0 ? [...sealThought(entries), entry] : entries.map((previous, i) => i === index ? entry : previous);
        });
        break;
      }
      case 'approval_rules':
        if (approvalStarted.current[msg.call_id] === undefined) break;
        setApprovalRules(previous => ({ ...previous, [msg.call_id]: { rules: msg.rules, error: msg.error } }));
        break;
      case 'approval_resolved':
        setEntries(previous => previous.flatMap(entry => {
          if (entry.kind !== 'tool' || entry.call.id !== msg.call_id) return [entry];
          if (isAgentRequest(entry.call)) return [];
          if (entry.status !== 'pending') return [entry];
          return [{ ...entry, status: msg.allow ? 'running' as const : 'denied' as const }];
        }));
        break;
      case 'approval_request':
        approvalStarted.current[msg.call.id] = Date.now();
        wsRef.current?.send({ type: 'approval_rules', call_id: msg.call.id });
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
        // back before the next text token arrives — while the turn lasts.
        if (busyRef.current) setThinking(true);
        setEntries((prev) => attachToolResult(prev, msg.result));
        agentTerminal.commandEnd(msg.result.call_id, msg.result.content, !!msg.result.is_error);
        // Piggyback: task_* tools ship the current task or full list in
        // `data`. Upsert so the Plan panel stays live without another
        // round-trip.
        setTasks((prev) => applyTaskResult(prev, msg.result));
        break;
      case 'turn_complete':
        // A turn ended (assistant round complete). More may follow if there
        // were tool calls; if not, `done` will clear us right after. A late
        // one after Stop must not set the spinner going again.
        if (busyRef.current) setThinking(true);
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
        busyRef.current = false;
        setThinking(false);
        clearThinkingIdle();
        setEntries((prev) => settleTools(sealThought(prev)));
        playPing();
        // Refresh git status, session diff, and branch PR after each turn.
        refreshRepo();
        // Close out the most recent turn's timing.
        setTurnTimings((prev) => stampLastTurn(prev, Date.now()));
        setSidebarRefresh((n) => n + 1);
        window.setTimeout(drainQueuedMessage, 0);
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
        loadEngines();
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
        activityTitlesRef.current.delete(msg.session_id);
        activityTitlesRef.current.set(msg.session_id,msg.title);
        if(activityTitlesRef.current.size>1000)activityTitlesRef.current.delete(activityTitlesRef.current.keys().next().value!);
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
        setCacheMiss(msg.cache_miss ?? null);
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
      case 'compacting':
        // One card per compaction: a running one already showing (the agent
        // path adds its own) isn't doubled.
        setEntries((prev) =>
          prev.some((e) => e.kind === 'compact' && e.state === 'running')
            ? prev
            : [...prev, {
                kind: 'compact', state: 'running', trigger: msg.trigger === 'auto' ? 'auto' : 'manual',
                startedAt: Date.now(), summarized: null, summary: null, tokensBefore: msg.tokens_before ?? null,
              }],
        );
        break;
      case 'compacted':
        setEntries((prev) => finishCompaction(prev, {
          state: 'done', summarized: msg.messages_removed,
          tokensBefore: msg.tokens_before ?? undefined, tokensAfter: msg.tokens_after ?? null,
        }));
        break;
      case 'compaction_failed':
        setEntries((prev) => finishCompaction(prev, { state: 'failed', error: msg.error }));
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
      case 'delegate_progress':
        // A step the delegated child just took. Routes to its card by call id
        // (the parent's `delegate_task` id), with a fallback to the most
        // recent running delegation for providers that don't echo the id.
        setEntries((prev) => appendDelegateStep(prev, msg.call_id, msg.kind, msg.text));
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
      case 'prompt_resolved':
        // Answered in another window (or tab, or device): close this copy
        // with the same answer instead of leaving it waiting.
        if (msg.kind === 'secret') {
          setSecretRequests((prev) => prev.filter((r) => r.promptId !== msg.prompt_id));
        } else if (msg.kind === 'ask_user') {
          const decision: AskUserDecision = msg.cancelled
            ? { cancelled: true }
            : { cancelled: false, answers: msg.answers ?? [] };
          setEntries((prev) => recordAskUserDecision(prev, msg.prompt_id, decision));
        } else if (msg.kind === 'plan') {
          setEntries((prev) =>
            recordPlanDecision(prev, msg.prompt_id, {
              approved: !!msg.approved,
              steps: msg.steps ?? undefined,
              note: msg.note ?? undefined,
            }),
          );
        }
        break;
      case 'runtime_request_updated':
        setEntries((prev) => upsertRuntimeQuestion(prev, msg.request));
        break;
      case 'runtime_turn_updated':
        if (msg.turn.running) {
          nativeTurnsRef.current.add(msg.turn.native_turn_id);
          setBusy(true); busyRef.current = true; setThinking(true);
        } else { nativeTurnsRef.current.delete(msg.turn.native_turn_id); }
        break;
      case 'runtime_work_updated': {
        const active = ['pending', 'running', 'waiting'].includes(msg.work.status);
        nativeWorkRef.current = nativeWorkRef.current.filter((w) => w.id !== msg.work.id);
        if (active) nativeWorkRef.current.push(msg.work);
        const running = active || nativeTurnsRef.current.size > 0 || nativeWorkRef.current.length > 0;
        setBusy(running); busyRef.current = running;
        break;
      }
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
        setEntries((prev) => appendAcpText(prev, msg.text, msg.message_id));
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
        // An agent compaction ends with its turn.
        setEntries((prev) => prev.some((e) => e.kind === 'compact' && e.state === 'running' && e.trigger === 'agent')
          ? finishCompaction(prev, isSuccessfulAcpStop(msg.stop_reason) ? { state: 'done' } : { state: 'failed', error: describeAcpStop(msg.stop_reason) })
          : prev);
        // End the turn. This is the only place an external agent's turn can
        // be declared over — the prompt is fire-and-forget, so unlike the
        // harness there is no surrounding await to imply completion. Without
        // clearing `busy` here the composer spins forever, which is what a
        // rate-limited turn looks like: the agent is long finished, the UI
        // just never hears about it.
        // The same "turn finished" sound Mira's own turns make — only for a
        // turn that was running here (a Stop already settled the composer,
        // and its confirmation shouldn't chime).
        if (busyRef.current) playPing();
        const nativeStillWorking = nativeTurnsRef.current.size > 0 || nativeWorkRef.current.length > 0;
        setBusy(nativeStillWorking);
        busyRef.current = nativeStillWorking;
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
        window.setTimeout(drainQueuedMessage, 0);
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
      case 'browser_active':
        // An agent is driving Mira's browser through the tool server.
        setBrowserPing((n) => n + 1);
        break;
      case 'engines_changed':
        // mira.yaml changed and the server rebuilt its engines: pickers
        // show new instances and keys without a restart.
        void listEngines().then((view) => setEngines(view.engines)).catch(() => {});
        break;
      case 'html_render':
        setEntries((prev) =>
          prev.some((e) => e.kind === 'html_render' && e.id === msg.id)
            ? prev
            : [...prev, { kind: 'html_render', id: msg.id, title: msg.title, html: msg.html }],
        );
        break;
      case 'secret_request':
        playPing();
        callForAttention('question', chatTitleRef.current);
        setSecretRequests((prev) =>
          prev.some((r) => r.promptId === msg.prompt_id)
            ? prev
            : [...prev, { promptId: msg.prompt_id, name: msg.name, reason: msg.reason, dotenv: msg.dotenv }],
        );
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
      case 'acp_modes': {
        // State, not a ref: these drive the picker and mode row, so the
        // transcript must re-render when they arrive.
        // Frames carry their driver (server-stamped); late frames from a
        // stopped agent must neither overwrite the live state nor pollute
        // another driver's "last seen" cache (OpenCode showing Claude
        // models). Unstamped frames keep the old behaviour.
        const modesDriver = msg.driver ?? capsDriverRef.current;
        if (modesDriver) {
          saveAgentCaps(modesDriver, { modes: { current: msg.current, available: msg.available } });
        }
        if (!msg.driver || msg.driver === capsDriverRef.current) {
          setAcpModes({ current: msg.current, available: msg.available, postures: msg.postures });
        }
        break;
      }
      case 'acp_config_options': {
        const configDriver = msg.driver ?? capsDriverRef.current;
        if (configDriver) saveAgentCaps(configDriver, { config: msg.options });
        if (!msg.driver || msg.driver === capsDriverRef.current) {
          setAcpConfig(msg.options);
        }
        break;
      }
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
          setEntries((es) => [...es, { kind: 'engine_switch', engine: next, from: prev }]);
        }
        // The sidebar badges each chat by its engine.
        if (moved || prev?.model !== next.model) setSidebarRefresh((n) => n + 1);
        applyEngine(next);
        break;
      }
      case 'acp_agent_status':
        // A probe ran on the server: the engines list has the fresh result.
        setAcpStatusPending(false);
        loadEngines();
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

  /** Answer an agent's secret prompt. `null` declines. The value goes only
   *  to the server, which stores it privately; it is not kept here. */
  function replyToSecret(promptId: string, value: string | null) {
    wsRef.current?.send({
      type: 'prompt_response',
      prompt_id: promptId,
      kind: 'secret',
      value,
      cancelled: value == null,
    });
    setSecretRequests((prev) => prev.filter((r) => r.promptId !== promptId));
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
    if (!callId.startsWith('async-')) setEntries((prev) => recordAskUserDecision(prev, callId, decision));
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

  function decideApproval(callId: string, allow: boolean, scope: ApprovalScope = 'once', rules?: string[]) {
    const pendingCall = entries.find(entry => entry.kind === 'tool' && entry.call.id === callId);
    if (allow && scope === 'always' && !rules && pendingCall?.kind === 'tool' && !isAgentRequest(pendingCall.call)) { setRuleEditorCallId(callId); return; }
    wsRef.current?.send({ type: 'approve', call_id: callId, allow, scope, rules });
    if (rules) return; // Keep the editor visible if server validation rejects the rule.
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
  const visibleApprovalRef = useRef<string | null>(null);
  const pendingApprovals = useMemo<PendingApproval[]>(
    () =>
      entries
        .filter((e): e is Extract<Entry, { kind: 'tool' }> => e.kind === 'tool' && e.status === 'pending')
        .map((e) => ({ callId: e.call.id, call: e.call, preview: e.preview, needs: e.needs, startedAt: approvalStarted.current[e.call.id], rulePreview: approvalRules[e.call.id] })),
    [entries, approvalRules],
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
        // A size of 0 means "not known yet" (e.g. just after a model
        // switch): show the ring as unknown, not as a 0-token window.
        used: acpUsage && acpUsage.size > 0 ? acpUsage.used : null,
        window: acpUsage && acpUsage.size > 0 ? acpUsage.size : null,
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
      cacheMiss,
      limitsTitle: limits.length ? 'Provider rate limits' : null,
      limits,
      onCompact: () => onCompact(''),
      onInspect: () => setInspectOpen(true),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [acpDriver, acpUsage, acpLimits, engine, rateLimit, providerContext, usage, model, cacheMiss]);

  // Plan proposal waiting for the user to approve/cancel. Rendered in
  // the Composer rather than inline so the interactive card doesn't
  // scroll away in a long transcript.
  const pendingPlans = useMemo(() => entries.flatMap(e => e.kind === 'tool' && e.plan?.decision === null ? [{callId:e.call.id, proposal:e.plan.proposal}] : []), [entries]);
  const pendingQuestions = useMemo(() => entries.flatMap(e => e.kind === 'tool' && e.askUser?.decision === null ? [{callId:e.call.id, proposal:e.askUser.proposal}] : []), [entries]);
  // Global approval shortcuts for the first pending approval, resolved
  // through the keybinding engine (Settings → Keybindings). Rebinds when
  // the head-of-queue call changes so back-to-back approvals each
  // pick up their own listener. Skipped while the user is typing so
  // approval keys in the composer/settings don't fire the decision.
  const firstPendingCallId = pendingApprovals[0]?.callId ?? null;
  useEffect(() => {
    if (!firstPendingCallId) return;
    function onKey(e: KeyboardEvent) {
      const visibleCallId = visibleApprovalRef.current;
      if (!visibleCallId) return;
      const t = e.target as HTMLElement | null;
      if (t) {
        const tag = t.tagName;
        const queueCheckbox = t instanceof HTMLInputElement && t.type === 'checkbox' && t.dataset.approvalQueue === 'true';
        if ((tag === 'INPUT' && !queueCheckbox) || tag === 'TEXTAREA') return;
        if (t.isContentEditable) return;
      }
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (command === 'approval.accept') {
        e.preventDefault();
        decideApproval(visibleCallId, true);
        return;
      }
      if (command === 'approval.reject') {
        e.preventDefault();
        decideApproval(visibleCallId, false);
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
            decideApproval(visibleCallId, lower === 'y');
          }
        }
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [firstPendingCallId, keybindings]);

  function queuedFor(id = sessionIdRef.current) {
    return queuedBySession.get(id) ?? [];
  }

  function setQueuedForSession(
    id: string,
    update: (items: QueuedComposerMessage[]) => QueuedComposerMessage[],
  ) {
    setQueuedBySession((prev) => {
      const next = new Map(prev);
      const items = update(next.get(id) ?? []);
      if (items.length === 0) next.delete(id);
      else next.set(id, items);
      return next;
    });
  }

  function queueMessage(text: string, images?: { media_type: string; data: string }[]) {
    const id = sessionIdRef.current;
    if (!id) return;
    const item: QueuedComposerMessage = {
      id: crypto.randomUUID(),
      text,
      images,
    };
    setQueuedForSession(id, (items) => [...items, item]);
    wsRef.current?.send({ type: 'queue_input', session_id: id, id: item.id, text, images });
  }

  function removeQueuedMessage(id: string) {
    const session = sessionIdRef.current;
    if (!session) return;
    wsRef.current?.send({ type: 'remove_queued_input', session_id: session, id });
  }

  function editQueuedMessage(item: QueuedComposerMessage, text: string) {
    return queueMutationsRef.current.request({
      type: 'edit_queued_input', session_id: sessionIdRef.current, id: item.id,
      fingerprint: item.fingerprint ?? '', text, images: item.images,
    }, message => { if (!wsRef.current?.sendImmediate(message)) throw new Error('Reconnect before changing queued messages.'); });
  }

  function changeRecovery(id: string, action: RecoveryAction) {
    return queueMutationsRef.current.request({ type: 'update_limit_recovery', session_id: sessionIdRef.current, id, action }, message => {
      if (!wsRef.current?.sendImmediate(message)) throw new Error('Reconnect before changing recovery.');
    });
  }
  function reorderQueuedMessage(id: string, beforeId: string | null) {
    return queueMutationsRef.current.request({
      type: 'reorder_queued_input', session_id: sessionIdRef.current, id, before_id: beforeId,
    }, message => { if (!wsRef.current?.sendImmediate(message)) throw new Error('Reconnect before changing queued messages.'); });
  }

  function steerQueuedMessage(id: string) {
    const session = sessionIdRef.current;
    if (!session || pendingSteerRef.current.has(session)) return;
    const picked = queuedBySessionRef.current.get(session)?.find(item => item.id === id);
    if (!picked) return;
    if (!busyRef.current) {
      // The server outbox will dispatch when the foreground turn releases.
      drainQueuedMessage();
      return;
    }
    flushNativeFrames(); flushTokens();
    pendingSteerRef.current.set(session, picked);
    setQueuedForSession(session, items => items.map(item => item.id === id ? { ...item, steering: true, error: undefined } : item));
    followRef.current = true;
    setEntries(entries => [...sealThought(entries), { kind: 'msg', steerRequestId: id, msg: { role: 'user', created_at: Date.now(), content: picked.text, images: picked.images, input_intent: 'steer' } }]);
    wsRef.current?.send({ type: 'steer', request_id: id, text: picked.text, images: picked.images });
  }

  function sendNow(text: string, images?: { media_type: string; data: string }[], transmit = true, inputId?: string) {
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
      if (inputId && prev.some(entry => entry.kind === 'msg' && entry.msg.input_id === inputId)) return prev;
      const providerIndex = engineRef.current?.kind === 'provider' ? providerTurnCursorRef.current++ : undefined;
      const next: Entry[] = [...prev, { kind: 'msg', providerTurnIndex: providerIndex, transcriptTurnIndex: countUserMessages(prev) + historyOffsetRef.current, msg: { role: 'user', created_at: Date.now(), content: text, images, input_id:inputId } }];
      const turnIndex = providerIndex ?? countUserMessages(next) - 1 + historyOffsetRef.current;
      startTurnUsage(turnIndex);
      setTurnTimings((tt) => {
        const clone = new Map(tt);
        clone.set(turnIndex, { startedAt: now, endedAt: null });
        return clone;
      });
      return next;
    });
    setBusy(true);
    busyRef.current = true;
    setThinking(true);
    // The server routes by the session's engine: a session on an agent
    // sends this to the agent (starting it if needed, with the
    // conversation so far if it just took over), otherwise to the
    // provider. One message type, so a prompt can never go to the wrong
    // engine because this client's view lagged the server's.
    if (transmit) wsRef.current?.send({ type: 'send', text, images });
  }

  function onSend(text: string, images?: { media_type: string; data: string }[]) {
    if (busyRef.current) {
      queueMessage(text, images);
      return;
    }
    sendNow(text, images);
  }

  function drainQueuedMessage() {
    // Server-owned outboxes dispatch without a connected browser. Ready
    // restores their state; terminal frames only update local presentation.
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
  /** "Fork from here": copy the chat through this message's turn into a
   *  new chat (nested under this one in the sidebar) and switch to it. */
  async function forkAt(userIdx: number) {
    const ref = messageRefAt(userIdx);
    if (!ref || !sessionId) return;
    try {
      const id = await forkSession(sessionId, ref);
      setSidebarRefresh((n) => n + 1);
      attachSession(id);
    } catch (e) {
      setRestoreNote({ text: `Couldn't fork: ${(e as Error).message}`, error: true });
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
      { kind: 'msg', transcriptTurnIndex: target.transcriptTurnIndex, providerTurnIndex: target.providerTurnIndex, msg: { role: 'user', created_at: Date.now(), content: text, images: target.msg.images } },
    ];
    const turnIndex = target.providerTurnIndex ?? countUserMessages(next) - 1 + historyOffsetRef.current;
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
    busyRef.current = true;
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
  const stableFailureSettings = useStableCallback(() => { openSettings(); setSettingsSection(acpDriver ? 'agents' : 'provider'); });
  const editMessage = useStableCallback((entry: Entry, text: string) => onResend(entries.indexOf(entry), text));
  const restoreMessage = useStableCallback((entry: Entry) => void askRestore(entries.indexOf(entry)));
  const forkMessage = useStableCallback((entry: Entry) => void forkAt(entries.indexOf(entry)));
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
    () => ({
      busy,
      openImage: setLightbox,
      edit: editMessage,
      restore: restoreMessage,
      retry: retryMessage,
      fork: acpDriver ? null : forkMessage,
    }),
    [busy, editMessage, restoreMessage, retryMessage, forkMessage, acpDriver],
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
    // The model's remembered options travel with the switch (#85).
    wsRef.current?.send({ type: 'set_model', model: m, instance: instance ?? null, options: loadModelOptions(m, instance ?? null) });
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
    // The server announces it (`compacting`), which shows the card.
    wsRef.current?.send({ type: 'compact', focus: focus || null });
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
      attachSession(id);
    } catch (e) {
      setEntries((prev) => [...prev, { kind: 'error', text: `new chat: ${(e as Error).message}` }]);
    }
  }

  // Global app shortcuts, resolved through the keybinding engine
  // Modified app actions work from the composer; bare keys yield to editing.
  // Recording fields, dialogs, composition, and repeated keys retain ownership.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.defaultPrevented || e.repeat || e.isComposing) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest('[data-keybinding-capture]') || document.querySelector('[role=dialog]')) return;
      if (t) {
        const tag = t.tagName;
        if ((tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t.isContentEditable) && !e.metaKey && !e.ctrlKey) return;
      }
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (!command) return;
      if (command.startsWith('panel.slot')) {
        const id=[...agentTabs,...fileTabs.map(tab=>tab.id),...toolTabs.map(tab=>tab.id)][Number(command.at(-1))-1];
        if(id){e.preventDefault();setMainView('chat');setActiveAgentTab(id);}return;
      }
      if (command.startsWith('chat.slot')) {
        e.preventDefault();
        const request=++chatShortcutRequest.current;const origin=sessionIdRef.current;
        void listSessions({all:true}).then(chats=>{
          if(request!==chatShortcutRequest.current||origin!==sessionIdRef.current)return;
          const chat=chats.filter(chat=>!chat.parent_id).sort((a,b)=>b.updated_at-a.updated_at)[Number(command.at(-1))-1];
          if(chat){setMainView('chat');attachSession(chat.id);}
        }).catch(error=>setEntries(prev=>[...prev,{kind:'error',text:`Could not switch chats: ${error.message}`}])) ;return;
      }
      switch (command) {
        case 'panel.tests': case 'panel.activity': case 'panel.devices': case 'panel.whiteboard': case 'panel.aside': case 'panel.devtools':
          e.preventDefault(); setMainView('chat'); openToolPane(command.slice(6) as ToolPaneKind); break;
        case 'panel.next': case 'panel.previous': {
          const tabs=[...agentTabs,...fileTabs.map(tab=>tab.id),...toolTabs.map(tab=>tab.id)];
          if (!tabs.length) return;
          e.preventDefault(); setMainView('chat');
          const found=tabs.indexOf(activeAgentTab ?? '');
          const index=found>=0?found:(command==='panel.next'?-1:0);
          setActiveAgentTab(tabs[(index+(command==='panel.next'?1:tabs.length-1))%tabs.length]);
          break;
        }
        case 'panel.closeTab':
          if (!activeAgentTab) return;
          e.preventDefault(); closeAnyTab(activeAgentTab); break;
        case 'chat.bottom': case 'chat.top':
          if (mainView!=='chat') return;
          e.preventDefault(); paneRef.current?.scrollTo({top:command==='chat.top'?0:paneRef.current.scrollHeight,behavior:'auto'}); break;
        case 'composer.attach':
          e.preventDefault(); setMainView('chat');
          requestAnimationFrame(()=>document.querySelector<HTMLInputElement>('[data-composer-attachments]')?.click()); break;
        case 'composer.focus':
          e.preventDefault();
          setMainView('chat');
          requestAnimationFrame(() => document.querySelector<HTMLElement>('.mention-input[contenteditable=true]')?.focus());
          break;
        case 'chat.stop':
          if (mainView !== 'chat' || !busyRef.current) return;
          e.preventDefault();
          wsRef.current?.send({type:'interrupt'});
          setBusy(false); busyRef.current=false; setThinking(false);
          break;
        case 'panel.files': case 'panel.processes': case 'panel.browser':
          e.preventDefault();
          setMainView('chat');
          if(command === 'panel.files') setPanelFilePickerOpen(true);
          else openToolPane(command === 'panel.processes' ? 'processes' : 'browser');
          break;
        case 'panel.close':
          e.preventDefault(); closeSubagentPanel(); break;
        case 'settings.shortcuts':
          e.preventDefault(); openSettings(); setSettingsSection('keybindings'); break;
        case 'chat.copyResponse': case 'chat.copyCode': {
          if (mainView !== 'chat') return;
          const responses=shortcutEntriesRef.current.filter((entry):entry is MsgEntry => entry.kind==='msg'&&entry.msg.role==='assistant'&&!!entry.msg.content).map(entry=>entry.msg.content!);
          const blocks=responses.flatMap(text=>Array.from(text.matchAll(/```[^\n]*\n([\s\S]*?)```/g),match=>match[1]));
          const text=command==='chat.copyCode'?blocks.at(-1):responses.at(-1);
          if (text) { e.preventDefault(); void navigator.clipboard.writeText(text).catch(() => setEntries(prev => [...prev,{kind:'error',text:'Could not copy to the clipboard.'}])); }
          break;
        }
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
          toggleSidebar();
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
  }, [keybindings, mainView, agentTabs, fileTabs, toolTabs, activeAgentTab, phone]);

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
    { id: 'sidebar', group: 'View', label: 'Toggle sidebar', icon: PanelLeftClose, shortcut: keyFor('sidebar.toggle'), run: toggleSidebar },
    { id: 'terminal', group: 'View', label: 'Toggle terminal', icon: SquareTerminal, shortcut: keyFor('terminal.toggle'), run: () => setTerminal(!terminalOpen) },
    { id: 'browser', group: 'View', label: 'Open browser', icon: Globe2, keywords: ['chrome', 'web'], run: () => { setMainView('chat'); openToolPane('browser'); } },
    { id: 'whiteboard', group: 'View', label: 'Open whiteboard', icon: Pencil, keywords: ['sketch', 'draw'], run: () => { setMainView('chat'); openToolPane('whiteboard'); } },
    { id: 'aside', group: 'View', label: 'Ask aside', icon: MessageCircleQuestion, keywords: ['side question', 'btw', 'quick question'], run: () => { setMainView('chat'); openToolPane('aside'); } },
    { id: 'processes', group: 'View', label: 'Open processes', icon: SquareTerminal, keywords: ['dev server', 'ports', 'logs', 'background'], run: () => { setMainView('chat'); openToolPane('processes'); } },
    { id: 'tests', group: 'View', label: 'Open tests', icon: FlaskConical, keywords: ['run tests', 'failures', 'test runner'], run: () => { setMainView('chat'); openToolPane('tests'); } },
    { id: 'activity', group: 'View', label: 'Open activity', icon: Activity, keywords: ['timeline', 'history', 'restore', 'changes'], run: () => { setMainView('chat'); openToolPane('activity'); } },
    { id: 'devices', group: 'View', label: 'Open device preview', icon: Smartphone, keywords: ['responsive', 'mobile', 'iphone', 'ipad'], run: () => { setMainView('chat'); openToolPane('devices'); } },
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
    loadEngines();
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
  const secondOpinionKey = `${sessionId}:${turns[lastTurnIdx]?.user?.transcriptTurnIndex ?? lastTurnIdx + historyOffsetRef.current}`;
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
        id: `turn-${turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current}`,
        userText: text.trim(),
        assistantText: assistant && assistant.kind === 'msg' ? (assistant.msg.content ?? null) : null,
      });
    });
    return out;
  }, [turns]);

  // Jump the transcript pane to a minimap turn.
  // The Activity pane's cards: one per turn opened by a user message, keyed
  // like the minimap so "Jump to message" lands on the same anchor.
  const activityTurns = useMemo<ActivityTurn[]>(() => {
    const out: ActivityTurn[] = [];
    turns.forEach((turn, i) => {
      if (turn.user?.kind !== 'msg' || turn.user.msg.role !== 'user') return;
      out.push({
        id: `turn-${turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current}`,
        userIdx: entries.indexOf(turn.user),
        text: parseSentAttachments(turn.user.msg.content ?? '').text.trim(),
        body: turn.body,
      });
    });
    return out;
  }, [turns, entries]);

  const jumpToMinimapTurn = useCallback((id: string) => {
    const pane = paneRef.current;
    if (!pane) return;
    const node = pane.querySelector(`[data-minimap-id="${id}"]`);
    if (!(node instanceof HTMLElement)) { followRef.current = false; window.dispatchEvent(new CustomEvent('mira:transcript-jump', { detail: id })); return; }
    const delta = node.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    pane.scrollTo({ top: pane.scrollTop + delta - 12, behavior: 'smooth' });
  }, []);
  // Keep the transcript and composer clear of the context card only while
  // it's open; the collapsed pill floats over the corner.
  const ctxHasContent =
    ctxFits &&
    contextPanelHasContent({processes:backgroundProcesses.processes,
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
  // Bumped by `browser_active`: an agent called a browser tool, whatever
  // name its harness gave it.
  const [browserPing, setBrowserPing] = useState(0);
  // Secrets agents asked for, waiting on the user (shown above the composer).
  const [secretRequests, setSecretRequests] = useState<SecretRequest[]>([]);
  const browserPingSeen = useRef(0);
  useEffect(() => {
    let live = browserPing !== browserPingSeen.current;
    browserPingSeen.current = browserPing;
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
  }, [entries, browserPing]);

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

  const sidebar = (
    <Sidebar
      status={status}
      cwd={cwd}
      activeSessionId={sessionId}
      activeBusy={busy}
      runningSessions={sessionActivity?.running ?? null}
      completedSessions={completedSessions}
      refreshKey={sidebarRefresh}
      activeView={mainView}
      onNavigate={(view) => {
        closeDrawer();
        setMainView(view);
      }}
      onNewChat={async () => {
        closeDrawer();
        setMainView('chat');
        await onNewChat();
      }}
      onOpenSettings={() => {
        closeDrawer();
        openSettings();
      }}
      onOpenPicker={() => {
        closeDrawer();
        void openProjectPicker();
      }}
      onSessionLoaded={() => {
        // Ready broadcast refreshes + jumps to chat.
        closeDrawer();
      }}
      onAttachSession={(id) => {
        closeDrawer();
        attachSession(id);
      }}
      onSetBackgroundMode={async (id, mode) => {
        await setSessionBackgroundMode(id, mode);
        setSidebarRefresh((n) => n + 1);
      }}
      activePr={branchPr}
      settingsSection={settingsSection}
      onSettingsSectionChange={(section) => {
        closeDrawer();
        setSettingsSection(section);
      }}
      onExitSettings={() => {
        closeDrawer();
        exitSettings();
      }}
    />
  );

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
        'grid grid-rows-1',
        phone ? 'bg-background' : 'h-screen transition-[grid-template-columns] duration-150',
        !phone && (hiddenTitleBar ? 'bg-transparent' : 'bg-panel'),
      )}
      style={
        phone
          ? // The visible viewport, so the composer sits above the keyboard.
            { gridTemplateColumns: 'minmax(0,1fr)', height: 'var(--app-height, 100dvh)' }
          : { gridTemplateColumns: `${sidebarCol} minmax(0,1fr) ${rightCol}` }
      }
    >
      <InfoNoticeHost notices={infoNotices.filter(notice => notice.kind === 'short')} onDismiss={dismissInfoNotice} onAgentUpdated={kind => {
        completedUpdatesRef.current.add(kind);
        requestAcpStatus();
        void listEngines().then(view => setEngines(view.engines)).catch(() => {});
        pushInfoNotice({ id: noticeId('updated'), kind: 'short', tone: 'success', title: 'Agent update complete' });
      }} />
      {phone ? (
        <>
          <div
            aria-hidden
            onClick={closeDrawer}
            className={cn(
              'fixed inset-0 z-40 bg-black/50 transition-opacity duration-200',
              drawerOpen ? 'opacity-100' : 'pointer-events-none opacity-0',
            )}
          />
          <div
            ref={drawerRef}
            // A dialog only while open: the app's shortcuts stand down
            // whenever a dialog is on screen.
            role={drawerOpen ? 'dialog' : undefined}
            aria-modal={drawerOpen ? 'true' : undefined}
            aria-label="Sidebar"
            tabIndex={-1}
            inert={!drawerOpen}
            // Swipe left to close, like any drawer.
            onTouchStart={(e) => { drawerTouchX.current = e.touches[0].clientX; }}
            onTouchEnd={(e) => {
              const start = drawerTouchX.current;
              drawerTouchX.current = null;
              if (start != null && start - e.changedTouches[0].clientX > 60) closeDrawer();
            }}
            className={cn(
              'pt-safe pb-safe fixed left-0 top-0 z-50 flex w-[min(86vw,320px)] flex-col overflow-hidden bg-panel shadow-2xl transition-transform duration-200 ease-out',
              drawerOpen ? 'translate-x-0' : '-translate-x-full',
            )}
            style={{ height: 'var(--app-height, 100dvh)' }}
          >
            {sidebar}
          </div>
        </>
      ) : (
        // overflow-hidden clips sidebar content when the grid column animates to 0
        <div className="overflow-hidden">{sidebar}</div>
      )}

      {/* Gutters are asymmetric: 2px on the sidebar edge vs 8px elsewhere,
          so the chat column reads as pulled toward the sidebar without
          touching it. The right panel keeps the full 8px on its outer edge. */}
      <main className={cn('flex min-h-0 min-w-0 flex-col', phone ? 'pt-safe px-safe' : 'py-2 pl-0.5 pr-2')}>
        {/* The main chat surface is the app's base surface, not a card: it
            keeps the flat theme background (pure black in dark) so the
            composer and cards inside it are what read as elevated. */}
        <div className={cn('flex min-h-0 flex-1 flex-col overflow-hidden bg-background', !phone && 'rounded-xl border border-border')}>
        {/* The chat view has its own header with the drawer button; the
            other pages need one too, or a phone has no way back. */}
        {phone && mainView !== 'chat' && (
          <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border/60 px-2">
            <button
              type="button"
              onClick={toggleSidebar}
              aria-label="Open sidebar"
              className="shrink-0 rounded p-2.5 text-foreground/70 transition-colors hover:bg-accent hover:text-foreground"
            >
              <PanelLeft className="size-4" />
            </button>
            <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">{PAGE_TITLES[mainView]}</span>
          </div>
        )}
        {mainView === 'chat' && (
          <>
            {/* Drag region: with the native title bar hidden this row is
                the natural place to move the window, and `Overlay` needs at
                least one or the window can't be dragged at all. */}
            <div
              data-tauri-drag-region
              className="flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4 max-md:gap-2 max-md:px-2"
            >
              {/* `data-tauri-drag-region` matches ancestors, so anything
                  interactive inside the header would drag the window on
                  mousedown. Opting these out keeps the clicks landing. */}
              <button
                data-tauri-drag-region="false"
                type="button"
                onClick={toggleSidebar}
                title={phone ? 'Open sidebar' : sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'}
                aria-label={phone ? 'Open sidebar' : sidebarOpen ? 'Collapse sidebar' : 'Expand sidebar'}
                className={cn(
                  'shrink-0 rounded p-1.5 transition-colors touch:p-2.5',
                  sidebarOpen || phone
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
                  {...preloadOnIntent(ReviewChanges.preload)}
                  title="Review this session's changes"
                  // h-8 and rounded-lg to match the icon controls beside it.
                  // It used to be a `rounded-full` py-1 pill, which read as a
                  // different kind of control because it was both shorter
                  // than its neighbours and the only fully-pill shape in the
                  // row.
                  className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border/60 elev-card dark:bg-secondary/40 px-2 text-[12.5px] text-muted-foreground transition-colors hover:border-border dark:hover:bg-secondary hover:text-foreground"
                >
                  <span className="font-mono text-green-400/80">+{sessionDiff.added}</span>
                  <span className="font-mono text-red-400/80">−{sessionDiff.removed}</span>
                  <span className="hidden lg:inline">Review</span>
                </button>
              )}
              <div className="flex items-center gap-0.5" data-tauri-drag-region="false">
                <div className="contents" {...preloadOnIntent(SubagentPanel.preload)}>
                <RightPanelButton
                  open={panelOpen}
                  activeKind={toolPaneKindOf(activeAgentTab ?? '')}
                  onOpen={() => openToolPane('new')}
                  onOpenPane={openToolPane}
                />
                </div>
                {/* Opening a local editor or a terminal means nothing from a
                    phone; the right panel's panes still work there. */}
                {!phone && <>
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
                    'flex h-8 shrink-0 items-center rounded-lg border border-transparent px-1.5 transition-colors',
                    terminalOpen
                      ? 'border-mira-blue/50 bg-mira-blue/10 text-foreground'
                      : 'elev-card dark:border-border/60 dark:bg-secondary/40 text-muted-foreground hover:bg-secondary hover:text-foreground',
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
                </>}
              </div>
            </div>

            {/* Transcript — full width, panel floats above it */}
            <div className="relative flex-1 min-h-0" ref={chatColRef}>
              <div
                className="absolute inset-0 overflow-y-auto overscroll-contain px-5 pb-5 transition-[padding] duration-200 max-md:px-3"
                style={{
                  paddingRight: ctxReserve ? CONTEXT_PANEL_RESERVE + 12 : phone ? 12 : 20,
                  paddingTop: ctxPill ? 52 : 16,
                }}
                ref={paneRef}
                onScroll={onPaneScroll} onWheel={noteScrollInput} onTouchMove={noteScrollInput} onKeyDown={noteScrollInput} onPointerDown={noteScrollInput} onPointerMove={(e) => { if (e.buttons) noteScrollInput(); }}
              >
                {/* Agents bring their own login, so a chat on one needs no provider. */}
                {configured === false && !acpDriver && isEmpty && (
                  <LazyBoundary>
                    <GetStarted
                      agents={acpAgents}
                      onUseAgent={(kind) => startAcpAgent(kind, null)}
                      onAddProvider={() => {
                        openSettings();
                        setSettingsSection('provider');
                      }}
                      onSetUpAgents={openAgentSettings}
                    />
                  </LazyBoundary>
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

                <div className="mx-auto max-w-3xl"><WorkspaceSetupCard /><ChatRelationships key={sessionId} sessionId={sessionId} refreshKey={sidebarRefresh} onOpen={attachSession} /></div>
                {isEmpty ? (
                  configured === false && !acpDriver ? null : <EmptyState
                    cwd={cwd}
                    onPrompt={(text) => onSend(text)}
                    onOpenSession={(id) => attachSession(id)}
                    onSwitchProject={(path) => void switchCwd(path)}
                    onNewProject={() => void openProjectPicker()}
                  />
                ) : (
                  <div data-transcript-column className="mx-auto flex max-w-3xl flex-col gap-2">
                    {goal && (
                      <LazyBoundary>
                        <GoalPanel
                          goal={goal}
                          busy={busy}
                          activity={goalActivity(entries)}
                          onClear={onClearGoal}
                          onRestart={(condition, maxIter) => onSetGoal(condition, maxIter)}
                        />
                      </LazyBoundary>
                    )}
                    {tasks.length > 0 && <TaskListPanel tasks={tasks} />}
                    <SourceCitationNavigator pane={paneRef} sessionId={sessionId} hasOlder={!!historyCursor} loading={historyLoading} loadOlder={loadOlderHistory} historyError={historyError} onOpenSession={attachSession} />
                    <AssistantSelectionToolbar
                      pane={paneRef}
                      sessionId={sessionId}
                      onQuote={(text, turn, href) => composeQuote({ text, turn, href })}
                      onAskAside={(text) => { setAsidePassage(text); setMainView('chat'); openToolPane('aside'); }}
                    />
                    {historyCursor && <button className="mx-auto py-3 text-xs text-muted-foreground" disabled={historyLoading} onClick={loadOlderHistory}>{historyLoading ? 'Loading older messages…' : 'Load older messages'}</button>}
                    {historyError && <p role="alert" className="text-xs text-destructive">{historyError}</p>}
                    <MessageActionsContext.Provider value={messageActions}>
                      <VirtualTranscript pane={paneRef} identity={sessionId ?? "new"} hasOlder={!!historyCursor} onNeedOlder={loadOlderHistory}>
                      {turns.map((turn, i) => (
                      <EntryBoundary key={turn.user ? `${sessionId}:turn-${turn.user.transcriptTurnIndex ?? i + historyOffsetRef.current}` : `${sessionId}:preamble-${i}`}>
                      <TurnView
                        diffSummary={turn.user?.kind === 'msg' ? turnDiffs.find(summary => summary.text === (turn.user?.kind === 'msg' ? turn.user.msg.content : null) && summary.occurrence === turns.slice(i + 1).filter(other => other.user?.kind === 'msg' && other.user.msg.content === summary.text).length) : undefined}
                        minimapId={`turn-${turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current}`}
                        turn={turn}
                        timing={turnTimings.get(turn.user?.providerTurnIndex ?? i + historyOffsetRef.current) ?? null}
                        usage={turnUsage.get(turn.user?.providerTurnIndex ?? i + historyOffsetRef.current) ?? null}
                        model={turnModels.get(turn.user?.providerTurnIndex ?? i + historyOffsetRef.current) ?? model}
                        index={turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current}
                        expanded={expandedTurns.has(turn.user?.transcriptTurnIndex ?? i + historyOffsetRef.current)}
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
                        recovery={i === turns.length - 1 ? recoveryBySession[sessionId]?.at(-1) : undefined}
                        failureRetryEnabled={i === turns.length - 1 && !!turn.user}
                        onFailureSettings={stableFailureSettings}
                        recoveryDisabled={status !== 'open' || busy}
                        onRecoveryAction={changeRecovery}
                        offscreenOk={false}
                      />
                      </EntryBoundary>
                    ))}
                    </VirtualTranscript>
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
              {reviewMounted && (
                <LazyBoundary>
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
                </LazyBoundary>
              )}

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
                  processes={backgroundProcesses.processes}
                  stoppingProcesses={new Set([...backgroundProcesses.stopping].filter(key=>key.startsWith(`${sessionId}:`)).map(key=>Number(key.split(':').at(-1))))}
                  onOpenProcess={(id)=>{setSelectedProcess({session:sessionId,id,nonce:Date.now()});openToolPane('processes');}}
                  onStopProcess={backgroundProcesses.stop}
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
              className="pb-safe shrink-0 transition-[padding-right] duration-200"
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
              onQueueMessage={queueMessage}
              queuedMessages={queuedFor(sessionId)}
              onRemoveQueuedMessage={removeQueuedMessage}
              onEditQueuedMessage={editQueuedMessage}
              onReorderQueuedMessage={reorderQueuedMessage}
              onSteerQueuedMessage={steerQueuedMessage}
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
                wsRef.current?.send({ type: 'set_model', model: m, instance: instance ?? null, options: loadModelOptions(m, instance ?? null) });
              }}
              onPickAgent={(driver, m) => {
                // Already this chat's agent: only the model can change, and
                // that is an option on the running agent, not a restart
                // with a new setup.
                if (acpDriver === driver) {
                  if (m && m !== engine?.model) onSetModel(m);
                  return;
                }
                // Its saved setup (Settings → Agents) is the engine's, on the
                // server. Prefer the engine instance named after the driver (its
                // default), else the only external instance of that driver.
                const external = (engines ?? []).filter((e) => e.flavor === 'external' && e.driver === driver);
                const instance = (external.find((e) => e.instance === driver) ?? (external.length === 1 ? external[0] : undefined))?.instance ?? null;
                startAcpAgent(driver, null, m, instance);
              }}
              onConfigureAgents={openAgentSettings}
              sessionId={sessionId}
              onAgentCompact={compactAcpAgent}
              onAgentFork={forkAcpAgent}
              onAgentReverted={() => attachSession(sessionId)}
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
              onOpenPicker={() => void openProjectPicker()}
              onCwdSwitched={(_path, id) => { if (id) attachSession(id); }}
              environment={environment}
              environments={environments}
              envSwitching={envSwitching}
              onSwitchEnvironment={(target) => {
                setEnvSwitching(`switching to ${target}…`);
                wsRef.current?.send({ type: 'environment', target });
              }}
              onInterrupt={() => {
      wsRef.current?.send({ type: 'interrupt' });
                // Settle the composer now; the server's turn end follows
                // (within a few seconds even for an agent that never says).
                setBusy(false);
                busyRef.current = false;
                setThinking(false);
              }}
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
              onActiveApprovalChange={callId => { visibleApprovalRef.current = callId; }}
              pendingApprovals={pendingApprovals}
              ruleEditorCallId={ruleEditorCallId}
              onRuleEditorCancel={() => setRuleEditorCallId(null)}
              pendingPlans={pendingPlans}
              pendingQuestions={pendingQuestions}
              notices={[
                ...(status !== 'open' ? [{id:'connection', kind:'connection' as const, title:'Reconnecting to Mira', content:<p role="status" className="px-3 pb-2 text-[12px] text-muted-foreground">Your draft is kept here. Sending and decisions are available when the connection returns.</p>}] : []),
                ...secretRequests.map(request => ({ id:`secret:${request.promptId}`, kind:'question' as const, title:`An agent needs ${request.name}`, content:<SecretPrompt request={request} onReply={replyToSecret} /> })),
                ...infoNotices.filter(notice => notice.kind === 'persistent').map(notice => ({ id:notice.id, kind:'update' as const, title:notice.title, detail:notice.body ?? undefined, content:<InfoNoticeHost inline notices={[notice]} onDismiss={dismissInfoNotice} onAgentUpdated={kind => { completedUpdatesRef.current.add(kind); requestAcpStatus(); void listEngines().then(view => setEngines(view.engines)).catch(() => {}); }} /> })),
              ]}
              onDecide={(callId, allow, scope, rules) => decideApproval(callId, allow, scope, rules)}
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
            {terminalOpen && !phone && (
              <LazyBoundary>
                <TerminalPanel onClose={() => setTerminal(false)} />
              </LazyBoundary>
            )}
          </>
        )}

        {mainView === 'plugins' && (
          <div className="flex-1 min-h-0 overflow-y-auto">
            <LazyBoundary>
              <PluginsPanel version={extensionsVersion} />
            </LazyBoundary>
          </div>
        )}

        {mainView === 'pull-request' && (
          <LazyBoundary>
            <PullRequestPanel
              onOpenSettings={() => openSettings()}
              onReviewPr={runPrReview}
            />
          </LazyBoundary>
        )}
        {mainView === 'scheduled' && <ComingSoon label="Scheduled" />}
        {mainView === 'settings' && (
          <LazyBoundary>
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
          </LazyBoundary>
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
                  mode: opt?.posture.key === 'auto' ? 'edit' : opt?.posture.key === 'yolo' ? 'yolo' : undefined,
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
        <div
          // On a phone the panel is a full-screen sheet over the chat; its
          // close button returns there.
          className={phone ? 'pt-safe pb-safe px-safe fixed left-0 right-0 top-0 z-40 flex flex-col bg-background' : 'min-h-0 min-w-0 py-2 pr-2'}
          style={phone ? { height: 'var(--app-height, 100dvh)' } : undefined}
        >
          <div className={phone ? 'min-h-0 flex-1 overflow-hidden' : 'h-full overflow-hidden rounded-xl border border-border bg-background'}>
        <LazyBoundary>
        <SubagentPanel
          tabs={subagentTabs}
          fileTabs={fileTabs}
          toolTabs={toolTabs}
          onWhiteboardSend={(png) => void sendWhiteboardToChat(png)}
          onOpenPane={openToolPane}
          onBrowseFile={() => setPanelFilePickerOpen(true)}
          renderPane={(kind) => {
            // Panes are lazy chunks; suspend just the pane, not the panel.
            const pane = (() => { switch (kind) {
              case 'aside':
                return <AsidePane sessionId={sessionId} entries={entries} agentBusy={busy} />;
              case 'processes':
                return <ProcessesPane key={sessionId} sessionId={sessionId} selectedProcess={selectedProcess?.session===sessionId?selectedProcess:undefined} onPreview={() => openToolPane('devices')} />;
              case 'tests':
                return <TestsPane sessionId={sessionId} cwd={cwd ?? ''} />;
              case 'activity':
                return (
                  <ActivityPane
                    turns={activityTurns}
                    busy={busy}
                    onJump={(id) => {
                      setMainView('chat');
                      jumpToMinimapTurn(id);
                    }}
                    onRestore={(idx) => void askRestore(idx)}
                    onOpenFile={(path, preview) => openFileTab(path, preview)}
                  />
                );
              case 'devices':
                return <DevicesPane />;
              default:
                return null;
            } })();
            return pane && <LazyBoundary>{pane}</LazyBoundary>;
          }}
          activeCallId={activeAgentTab}
          cwd={cwd ?? ''}
          onSelectTab={setActiveAgentTab}
          onCloseTab={closeAnyTab}
          onClose={closeSubagentPanel}
          onReview={replyToSubagentReview}
          onResizeStart={handlePanelResizeStart}
          onOpenFile={openFileTab}
        />
        </LazyBoundary>
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
            <LazyBoundary>
              <ImportChats
                onDone={() => {
                  setImportOpen(false);
                  setSidebarRefresh((n) => n + 1);
                }}
              />
            </LazyBoundary>
          )}
        </DialogContent>
      </Dialog>
      {inspectOpenMounted && (
        <LazyBoundary>
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
        </LazyBoundary>
      )}
      {paletteOpenMounted && (
        <LazyBoundary>
          <CommandPalette
            open={paletteOpen}
            onOpenChange={setPaletteOpen}
            actions={paletteActions}
            onOpenSession={(id) => {
              attachSession(id);
              setMainView('chat');
            }}
          />
        </LazyBoundary>
      )}
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
      {pickerOpenMounted && (
        <LazyBoundary>
          <FolderPicker
            open={pickerOpen}
            onClose={() => setPickerOpen(false)}
            onPicked={(_path, id) => {
              // Server built a fresh slot for the new cwd. Attach the WS so
              // the freshly-published Ready lands in our transcript — without
              // this, the socket keeps forwarding the previous slot's frames
              // and the UI silently stays on the old folder.
              if (id) attachSession(id);
            }}
          />
        </LazyBoundary>
      )}
      <ReviewPanel
        open={reviewPanelOpen}
        state={reviewState}
        onClose={() => setReviewPanelOpen(false)}
      />
    </div>
  );
}
