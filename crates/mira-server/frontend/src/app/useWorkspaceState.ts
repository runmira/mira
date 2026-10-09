import { useRef, useState } from 'react';
import {
  type BranchPrView,
  type CommandInfo,
  type EngineSnapshot,
  type GitStatusView,
  type MessageRef,
  type RestoreChange,
  type SessionDiffView,
  type SkillView,
} from '../api';
import { type QueuedComposerMessage } from '../components/Composer';
import { useContextPanelFits } from '../components/ContextPanel';
import { type InfoNotice } from '../components/InfoNoticeHost';
import { type ToolPaneTab } from '../components/panes/toolPanes';
import { type ReviewState } from '../components/ReviewPanel';
import { type SecretRequest } from '../components/SecretPrompt';
import type { FilePanelTab } from '../components/SubagentPanel';
import { useKeybindings } from '../lib/keybindings';
import { useIsPhone } from '../lib/mobile';
import { getBoolPref, PREF_KEYS } from '../lib/prefs';
import { QueueMutations } from '../lib/queueMutations';
import { type SessionActivityState } from '../lib/sessionActivity';
import {
  type Entry,
  type NativeFrame,
  type SubagentStreamState,
  type TurnTiming,
} from '../transcript/entries';
import type {
  AcpAgentStatus,
  AcpConfigOption,
  AcpSessionMode,
  AgentPostureMapping,
  AskUserProposal,
  DiffPreview,
  EnvironmentInfo,
  EnvironmentStatus,
  Goal,
  Mode,
  PlanProposal,
  RateLimitReading,
  ServerMsg,
  SessionEngine,
  TaskItem,
  UsageTotals,
} from '../types';
import { type WsClient, type WsStatus } from '../ws';
import { useAppNavigation } from './useAppNavigation';

export function useWorkspaceState() {
  const pingPrimedRef = useRef(false);

  const [status, setStatus] = useState<WsStatus>('connecting');

  const [sessionActivity, setSessionActivity] = useState<SessionActivityState | null>(null);

  const sessionActivityRef = useRef<SessionActivityState | null>(null);

  const activityTitlesRef = useRef(new Map<string, string>());

  const [completedSessions, setCompletedSessions] = useState<ReadonlyMap<string, number>>(
    new Map(),
  );

  const [sessionId, setSessionId] = useState<string>('');

  const sessionIdRef = useRef('');

  const [queuedBySession, setQueuedBySession] = useState<Map<string, QueuedComposerMessage[]>>(
    new Map(),
  );

  const queuedBySessionRef = useRef(queuedBySession);

  const pendingSteerRef = useRef(new Map<string, QueuedComposerMessage>());

  /** The session's own title (AI-written, or the agent's), once known; the
   *  header falls back to the first message until then. */
  const [sessionTitle, setSessionTitle] = useState<string | null>(null);

  const [model, setModel] = useState<string>('');

  const [mode, setMode] = useState<Mode>('manual');

  const [cwd, setCwd] = useState<string>('');

  const [entries, setEntries] = useState<Entry[]>([]);

  const [ruleEditorCallId, setRuleEditorCallId] = useState<string | null>(null);

  const [approvalRules, setApprovalRules] = useState<
    Record<string, { rules: string[]; error: string | null }>
  >({});

  const approvalStarted = useRef<Record<string, number>>({});

  const [historyCursor, setHistoryCursor] = useState<string | null>(null);

  const [historyLoading, setHistoryLoading] = useState(false);

  const [historyError, setHistoryError] = useState<string | null>(null);

  const historyRequestRef = useRef<string | null>(null);

  const historyPreviewsRef = useRef<Record<string, DiffPreview>>({});

  const historyOffsetRef = useRef(0);

  const providerTurnCursorRef = useRef(0);

  const [turnDiffs, setTurnDiffs] = useState<import('../types').TurnDiffSummary[]>([]);

  // State an external ACP agent owns. Captured here rather than folded into
  // the transcript because it is configuration, not conversation — the
  // model/mode pickers read it. Held in refs so receiving it never triggers a
  // re-render of the whole transcript.
  /** The agent's own stop reason for the last finished turn. */
  // Kept for diagnostics only: the stop reason is written into the
  // transcript when a turn ends abnormally, and read nowhere else.
  const acpStopRef = useRef<string | null>(null);

  const [acpModes, setAcpModes] = useState<{
    current: string;
    available: AcpSessionMode[];
    postures?: AgentPostureMapping[];
  } | null>(null);

  /** Every backend from `GET /api/engines` — native providers and external
   *  agents in one list, with health + catalogs. Refetched when extensions
   *  change (an agent install state can move) and before opening pickers. */
  const [engines, setEngines] = useState<EngineSnapshot[] | null>(null);

  const [acpConfig, setAcpConfig] = useState<AcpConfigOption[]>([]);

  const [acpCommands, setAcpCommands] = useState<string[]>([]);

  const [acpUsage, setAcpUsage] = useState<{
    used: number;
    size: number;
    cost: { amount: number; currency: string } | null;
  } | null>(null);

  /** The agent account's plan limits, as last reported. */
  const [acpLimits, setAcpLimits] = useState<
    { name: string; utilization: number; resets_at?: number | null }[]
  >([]);

  /** Context fill for a provider chat: tokens in the last request plus its
   *  reply, against the window the harness plans for. */
  const [providerContext, setProviderContext] = useState<{
    used: number;
    window: number;
    compactAt: number | null;
  } | null>(null);

  /** What drives this session — a provider or an agent — as the server
   *  reports it on `ready` and on every `session_engine` transition. The
   *  one source of truth for "which engine": nothing below re-derives it. */
  const [engine, setEngine] = useState<SessionEngine | null>(null);

  /** Which agent the modes / config / commands state above belongs to. */
  const capsDriverRef = useRef<string | null>(null);

  /** The engine as last applied, for handlers that must not see a stale
   *  render's value. */
  const engineRef = useRef<SessionEngine | null>(null);

  /** Health of every known agent, refreshed on request. */
  const [acpAgents, setAcpAgents] = useState<AcpAgentStatus[]>([]);

  const [acpStatusPending, setAcpStatusPending] = useState(false);

  const acpAgentsRef = useRef<AcpAgentStatus[]>([]);

  const completedUpdatesRef = useRef(new Set<string>());

  /** Small and persistent in-window notices for app-level state. */
  const [infoNotices, setInfoNotices] = useState<InfoNotice[]>([]);

  /** A startup failure, surfaced in the Agents panel. */
  const [acpError, setAcpError] = useState<string | null>(null);

  /** A mode picked in the picker, awaiting confirmation in the universal
   *  approval dialog. A mode is a standing grant of authority, so the pick
   *  alone is never applied — the dialog states the consequence and the user
   *  confirms. `reason` carries the server's explanation when it refused a
   *  privileged mode and asked for acknowledgement. */
  const [pendingAcpMode, setPendingAcpMode] = useState<{ modeId: string; reason?: string } | null>(
    null,
  );

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

  const [thinking, setThinking] = useState<boolean>(false);

  // When text tokens go quiet mid-turn — typically because the model is
  // emitting tool-call deltas that don't surface as `token` events — we
  // want the "Thinking…" affordance back so the transcript isn't silent
  // for the second-or-so before `tool_start` fires. `scheduleThinking`
  // (below) sets this timer on every `token`; a follow-up token cancels
  // it, and `tool_start` / `approval_request` / `done` clear it too so
  // the indicator doesn't flash after the turn genuinely ends.
  const thinkingIdleTimerRef = useRef<number | null>(null);

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
  const { mainView, setMainView, settingsSection, setSettingsSection, openSettings, exitSettings } =
    useAppNavigation();

  // Back from installing the GitHub App: GitHub (via the github-app edge
  // function) sends the user here with `?github=…`. Show the result in
  // Settings → Integrations and drop the query.
  const [githubReturn] = useState<import('../components/Settings').GithubReturn>(() => {
    const q = new URLSearchParams(window.location.search);
    const ok = q.get('github');
    const bad = q.get('github_error');
    if (!ok && !bad) return null;
    q.delete('github');
    q.delete('github_error');
    const rest = q.toString();
    window.history.replaceState(
      window.history.state,
      '',
      window.location.pathname + (rest ? `?${rest}` : '') + window.location.hash,
    );
    if (bad) return { ok: false, message: `GitHub: ${bad}` };
    return ok === 'requested'
      ? {
          ok: true,
          message: 'Installation requested: an organization owner has to approve it on GitHub.',
        }
      : { ok: true, message: 'GitHub connected. Turn Mira on for the repositories you want.' };
  });

  // Left sidebar visibility — collapses the 300px column to 0.
  // Startup default comes from Settings → General (localStorage).
  const [sidebarOpen, setSidebarOpen] = useState(() => {
    try {
      return localStorage.getItem('mira.sidebar.open') !== '0';
    } catch {
      return true;
    }
  });

  // At phone width there's room for one column: the sidebar becomes a
  // drawer over the chat, closed by default and after every pick in it.
  const phone = useIsPhone();

  const [drawerOpen, setDrawerOpen] = useState(false);

  const drawerTouchX = useRef<number | null>(null);

  const drawerRef = useRef<HTMLDivElement | null>(null);

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

  const [sessionDiff, setSessionDiff] = useState<SessionDiffView>({
    added: 0,
    removed: 0,
    files: [],
  });

  // Context panel: collapsed to a pill by default so it stays out of the
  // way; the choice is remembered per browser.
  const [ctxOpen, setCtxOpen] = useState<boolean>(() => {
    try {
      return localStorage.getItem('mira.context.open') === '1';
    } catch {
      return false;
    }
  });

  // The session committed through the panel, so any unpushed commits on
  // the branch are its own to push.
  const [sessionCommitted, setSessionCommitted] = useState(false);

  const [reviewOpen, setReviewOpen] = useState(false);

  /** File the review drawer should open on, when opened from the file list. */
  const [reviewFocus, setReviewFocus] = useState<string | null>(null);

  const [lightbox, setLightbox] = useState<string | null>(null);

  // Integrated terminal (bottom panel); open state is remembered.
  // Settings → General can disable the restore (always start closed).
  const [terminalOpen, setTerminalOpen] = useState<boolean>(() => {
    try {
      if (localStorage.getItem('mira.terminal.restore') === '0') return false;
      return localStorage.getItem('mira.terminal.open') === '1';
    } catch {
      return false;
    }
  });

  const keybindings = useKeybindings();

  const chatShortcutRequest = useRef(0);

  const shortcutEntriesRef = useRef(entries);

  const [selectedProcess, setSelectedProcess] = useState<{
    session: string;
    id: number;
    nonce: number;
  } | null>(null);

  const ctxFits = useContextPanelFits();

  // The chat column's width, for whether the open context panel sits
  // beside the transcript or over it.
  const [chatColWidth, setChatColWidth] = useState(0);

  const chatColObserver = useRef<ResizeObserver | null>(null);

  const [branchPr, setBranchPr] = useState<BranchPrView | null>(null);

  /** Re-read the repo state the panels show: git status, what this chat
   *  changed, and the branch's PR. Responses are applied only if no newer
   *  refresh has started since — after a quick session switch, a slow reply
   *  for the previous chat must not overwrite the current one. */
  const repoSeqRef = useRef(0);

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

  const [recoveryBySession, setRecoveryBySession] = useState<
    Record<string, import('../types').QueuedInput[]>
  >({});

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

  // Follow new output only while the reader is at the bottom; scrolling
  // up to read back stops the follow and offers a jump-to-latest button.
  const followRef = useRef(getBoolPref(PREF_KEYS.transcriptFollow, true));

  const readingAnchorRef = useRef<number | null>(null);

  const [showJump, setShowJump] = useState(false);

  const lastScrollInputRef = useRef(0);

  // Desktop notification when a turn finishes while the tab is hidden
  // (Settings → General → Notifications). Only fires if the user granted
  // permission; requesting happens from the settings row.
  // Read from WS handlers, which close over the first render.
  const chatTitleRef = useRef<string | null>(null);

  // Smooth streaming: tokens land in a buffer and are released a few
  // characters per animation frame (faster when the buffer is deep), so
  // text flows instead of jumping in network-sized chunks.
  const tokenBufRef = useRef('');

  const tokenRafRef = useRef<number | null>(null);

  // Per-turn usage: session totals at turn start, diffed at turn end.
  const usageRef = useRef<UsageTotals | null>(null);

  const turnBaseRef = useRef<{ turn: number; base: UsageTotals } | null>(null);

  const [turnUsage, setTurnUsage] = useState<Map<number, UsageTotals>>(new Map());

  /** The model each reloaded turn ran on, for pricing it. Live turns use
   *  the current model. */
  const [turnModels, setTurnModels] = useState<Map<number, string>>(new Map());

  const nativeFramesRef = useRef<NativeFrame[]>([]);

  const nativeRafRef = useRef<number | null>(null);

  // Tool calls waiting for the user's Y/N decision. The Allow / Deny /
  // Always-allow buttons render inline on the pending tool card in
  // the transcript. Kept oldest-first — the top of the queue is what
  // the Y/N global shortcut targets.
  const visibleApprovalRef = useRef<string | null>(null);

  // Restore files to before a message: preview → confirm → restore, with an
  // Undo afterwards.
  const [restoreAsk, setRestoreAsk] = useState<{
    ref: MessageRef;
    changes: RestoreChange[];
  } | null>(null);

  const [restoreNote, setRestoreNote] = useState<{
    text: string;
    undo?: string;
    error?: boolean;
  } | null>(null);

  // Second opinion: after an external agent's turn that changed files,
  // offer a review by Mira's own reviewer. One offer per turn; dismissing
  // or accepting it retires it.
  const [secondOpinionSeen, setSecondOpinionSeen] = useState<Set<string>>(() => new Set());

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
  return {
    pingPrimedRef,
    status,
    setStatus,
    sessionActivity,
    setSessionActivity,
    sessionActivityRef,
    activityTitlesRef,
    completedSessions,
    setCompletedSessions,
    sessionId,
    setSessionId,
    sessionIdRef,
    queuedBySession,
    setQueuedBySession,
    queuedBySessionRef,
    pendingSteerRef,
    sessionTitle,
    setSessionTitle,
    model,
    setModel,
    mode,
    setMode,
    cwd,
    setCwd,
    entries,
    setEntries,
    ruleEditorCallId,
    setRuleEditorCallId,
    approvalRules,
    setApprovalRules,
    approvalStarted,
    historyCursor,
    setHistoryCursor,
    historyLoading,
    setHistoryLoading,
    historyError,
    setHistoryError,
    historyRequestRef,
    historyPreviewsRef,
    historyOffsetRef,
    providerTurnCursorRef,
    turnDiffs,
    setTurnDiffs,
    acpStopRef,
    acpModes,
    setAcpModes,
    engines,
    setEngines,
    acpConfig,
    setAcpConfig,
    acpCommands,
    setAcpCommands,
    acpUsage,
    setAcpUsage,
    acpLimits,
    setAcpLimits,
    providerContext,
    setProviderContext,
    engine,
    setEngine,
    capsDriverRef,
    engineRef,
    acpAgents,
    setAcpAgents,
    acpStatusPending,
    setAcpStatusPending,
    acpAgentsRef,
    completedUpdatesRef,
    infoNotices,
    setInfoNotices,
    acpError,
    setAcpError,
    pendingAcpMode,
    setPendingAcpMode,
    turnTimings,
    setTurnTimings,
    turnStartRef,
    expandedTurns,
    setExpandedTurns,
    busy,
    setBusy,
    nativeWorkRef,
    nativeTurnsRef,
    busyRef,
    thinking,
    setThinking,
    thinkingIdleTimerRef,
    pickerOpen,
    setPickerOpen,
    paletteOpen,
    setPaletteOpen,
    inspectOpen,
    setInspectOpen,
    importOpen,
    setImportOpen,
    panelFilePickerOpen,
    setPanelFilePickerOpen,
    mainView,
    setMainView,
    settingsSection,
    setSettingsSection,
    openSettings,
    exitSettings,
    githubReturn,
    sidebarOpen,
    setSidebarOpen,
    phone,
    drawerOpen,
    setDrawerOpen,
    drawerTouchX,
    drawerRef,
    agentTabs,
    setAgentTabs,
    fileTabs,
    setFileTabs,
    toolTabs,
    setToolTabs,
    activeAgentTab,
    setActiveAgentTab,
    rightPanelWidth,
    setRightPanelWidth,
    subagentState,
    setSubagentState,
    configured,
    setConfigured,
    providerName,
    setProviderName,
    sidebarRefresh,
    setSidebarRefresh,
    reviewPanelOpen,
    setReviewPanelOpen,
    reviewState,
    setReviewState,
    usage,
    setUsage,
    cacheMiss,
    setCacheMiss,
    rateLimit,
    setRateLimit,
    gitStatus,
    setGitStatus,
    sessionDiff,
    setSessionDiff,
    ctxOpen,
    setCtxOpen,
    sessionCommitted,
    setSessionCommitted,
    reviewOpen,
    setReviewOpen,
    reviewFocus,
    setReviewFocus,
    lightbox,
    setLightbox,
    terminalOpen,
    setTerminalOpen,
    keybindings,
    chatShortcutRequest,
    shortcutEntriesRef,
    selectedProcess,
    setSelectedProcess,
    ctxFits,
    chatColWidth,
    setChatColWidth,
    chatColObserver,
    branchPr,
    setBranchPr,
    repoSeqRef,
    tasks,
    setTasks,
    goal,
    setGoal,
    environment,
    setEnvironment,
    environments,
    setEnvironments,
    envSwitching,
    setEnvSwitching,
    skills,
    setSkills,
    commands,
    setCommands,
    extensionsVersion,
    setExtensionsVersion,
    skillsVersion,
    setSkillsVersion,
    setNowTick,
    wsRef,
    recoveryBySession,
    setRecoveryBySession,
    queueMutationsRef,
    paneRef,
    pendingProposalsRef,
    pendingAskUserRef,
    followRef,
    readingAnchorRef,
    showJump,
    setShowJump,
    lastScrollInputRef,
    chatTitleRef,
    tokenBufRef,
    tokenRafRef,
    usageRef,
    turnBaseRef,
    turnUsage,
    setTurnUsage,
    turnModels,
    setTurnModels,
    nativeFramesRef,
    nativeRafRef,
    visibleApprovalRef,
    restoreAsk,
    setRestoreAsk,
    restoreNote,
    setRestoreNote,
    secondOpinionSeen,
    setSecondOpinionSeen,
    browserShownFor,
    browserPing,
    setBrowserPing,
    secretRequests,
    setSecretRequests,
    browserPingSeen,
  };
}
