import { useSelector } from '@tanstack/react-store';
import { useMemo,useState } from 'react';
import {
type CommandInfo,
type SessionDiffView,
type SkillView
} from '../api';
import { useContextPanelFits } from '../components/ContextPanel';
import { useKeybindings } from '../lib/keybindings';
import { useIsPhone } from '../lib/mobile';
import { useResource } from '../state/resources/hooks';
import { useAppRuntime,useContextPanelState,useSessionField,useTurnProjection,useWorkspaceUiField } from '../state/RuntimeProvider';
import { useAppNavigation } from './useAppNavigation';

const EMPTY_SKILLS: SkillView[] = [];
const EMPTY_COMMANDS: CommandInfo[] = [];
const EMPTY_DIFF: SessionDiffView = { added: 0, removed: 0, files: [] };
export function useWorkspaceState() {
  const appRuntime = useAppRuntime();
  const { streams, sessionServices } = appRuntime;
  const { pingPrimedRef, activityTitlesRef, acpAgentsRef, completedUpdatesRef, thinkingIdleTimerRef, drawerTouchX, drawerRef, paneRef, chatShortcutRequest, chatColObserver, lastScrollInputRef, readingAnchorRef, attachFromRef, followRef, chatTitleRef, visibleApprovalRef, browserShownFor, browserPingSeen, wsRef, queueMutationsRef } = appRuntime.service.refs;

  const transportStatus = useSelector(appRuntime.connection, state => state.status);
  const attachmentReady = useSelector(appRuntime.sessionState, state => state.active !== null);
  const status = transportStatus === 'open' && !attachmentReady ? 'connecting' : transportStatus;
  const setStatus = appRuntime.reportConnection;


  const sessionId = useSelector(appRuntime.sessionState, state => state.selected ?? '');

  const settingsResource = useResource('settings');
  const enginesResource = useResource('engines', !!sessionId);
  const skillsResource = useResource('skills', !!sessionId);
  const commandsResource = useResource('commands', !!sessionId);
  const gitResource = useResource('gitStatus', !!sessionId);
  const diffResource = useResource('sessionDiff', !!sessionId);
  const branchResource = useResource('branchPr', !!sessionId);
  const configured = settingsResource.data?.configured ?? (settingsResource.isError ? false : null);
  const providerName = settingsResource.data?.default_provider ?? null;
  const engines = enginesResource.data?.engines ?? null;
  const skills = skillsResource.data ?? EMPTY_SKILLS;
  const commands = commandsResource.data ?? EMPTY_COMMANDS;
  const gitStatus = gitResource.data ?? null;
  const sessionDiff = diffResource.data ?? EMPTY_DIFF;
  const branchPr = branchResource.data ?? null;
  const getSessionId = () => appRuntime.sessionState.get().selected ?? '';

  const queuedBySession = useSelector(appRuntime.sessionState, state => state.queued);
  const updateQueueProjection = appRuntime.updateQueues;
  const getQueues = () => appRuntime.sessionState.get().queued;

  const pendingSteerRef = { current: sessionServices.pendingSteer };

  /** The session's own title (AI-written, or the agent's), once known; the
   *  header falls back to the first message until then. */
  const [sessionTitle, recordSessionTitle] = useSessionField('sessionTitle');

  const [model, recordModel] = useSessionField('model');

  const [mode, recordMode] = useSessionField('mode');

  const [cwd, recordCwd] = useSessionField('cwd');

  const [outlineEntries, updateTranscript] = useSessionField('entries');

  const [ruleEditorCallId, setRuleEditorCallId] = useSessionField('ruleEditorCallId');

  const [approvalRules, recordApprovalRules] = useSessionField('approvalRules');

  const approvalStarted = sessionServices.ref('approvalStarted');

  const [historyCursor, recordHistoryCursor] = useSessionField('historyCursor');

  const [historyLoading, recordHistoryLoading] = useSessionField('historyLoading');

  const [historyError, recordHistoryError] = useSessionField('historyError');

  const historyRequestRef = sessionServices.ref('historyRequest');

  const historyPreviewsRef = sessionServices.ref('historyPreviews');

  const historyOffsetRef = sessionServices.ref('historyOffset');

  const providerTurnCursorRef = sessionServices.ref('providerTurnCursor');

  const [turnDiffs, recordTurnDiffs] = useSessionField('turnDiffs');

  // State an external ACP agent owns. Captured here rather than folded into
  // the transcript because it is configuration, not conversation — the
  // model/mode pickers read it. Held in refs so receiving it never triggers a
  // re-render of the whole transcript.
  /** The agent's own stop reason for the last finished turn. */
  // Kept for diagnostics only: the stop reason is written into the
  // transcript when a turn ends abnormally, and read nowhere else.

  const [acpModes, recordAgentModes] = useSessionField('acpModes');

  /** Every backend from `GET /api/engines` — native providers and external
   *  agents in one list, with health + catalogs. Refetched when extensions
   *  change (an agent install state can move) and before opening pickers. */

  const [acpConfig, recordAgentConfig] = useSessionField('acpConfig');

  const [acpCommands, recordAgentCommands] = useSessionField('acpCommands');

  const [acpUsage, recordAgentUsage] = useSessionField('acpUsage');

  /** The agent account's plan limits, as last reported. */
  const [acpLimits, recordAgentLimits] = useSessionField('acpLimits');

  /** Context fill for a provider chat: tokens in the last request plus its
   *  reply, against the window the harness plans for. */
  const [reportedContext, recordProviderContext] = useSessionField('providerContext');
  const contextResource = useResource('context', !!sessionId);
  const providerContext = useMemo(() => reportedContext ?? (contextResource.data?.source === 'mira' ? {
    used: contextResource.data.breakdown.total, window: contextResource.data.window, compactAt: contextResource.data.compact_at } : null), [reportedContext, contextResource.data]);

  /** What drives this session — a provider or an agent — as the server
   *  reports it on `ready` and on every `session_engine` transition. The
   *  one source of truth for "which engine": nothing below re-derives it. */
  const [engine, recordEngine] = useSessionField('engine');

  /** Which agent the modes / config / commands state above belongs to. */
  const getCapsDriver = () => { const engine = appRuntime.readSession().engine; return engine?.kind === 'agent' ? engine.driver ?? null : null; };

  /** The engine as last applied, for handlers that must not see a stale
   *  render's value. */
  const getEngine = () => appRuntime.readSession().engine;

  /** Health of every known agent, refreshed on request. */
  const acpAgents = useMemo(() => (engines ?? []).flatMap(engine =>
    engine.flavor === 'external' && engine.agent && engine.instance === engine.driver ? [engine.agent] : []), [engines]);

  const acpStatusPending = enginesResource.isFetching;



  /** Small and persistent in-window notices for app-level state. */
  const [infoNotices, setInfoNotices] = useWorkspaceUiField('infoNotices');

  /** A startup failure, surfaced in the Agents panel. */
  const [acpError, recordAgentError] = useSessionField('acpError');

  /** A mode picked in the picker, awaiting confirmation in the universal
   *  approval dialog. A mode is a standing grant of authority, so the pick
   *  alone is never applied — the dialog states the consequence and the user
   *  confirms. `reason` carries the server's explanation when it refused a
   *  privileged mode and asked for acknowledgement. */
  const [pendingAcpMode, setPendingAcpMode] = useSessionField('pendingAcpMode');

  // Per-turn timing. Turn index = 0-based order of user messages in `outlineEntries`.
  // Only turns started in *this* session have timing (reloaded transcripts
  // have no wall-clock data, so their turns skip the "Worked for" header).
  const [turnTimings, recordTurnTimings] = useSessionField('turnTimings');

  /** When the latest turn was sent — what an agent turn's stats start from. */
  const turnStartRef = sessionServices.ref('turnStart');

  const [expandedTurns, setExpandedTurns] = useSessionField('expandedTurns');

  const { busy, thinking, isTurnRunning, reportTurnRunning, reportThinking, turnCommands } = useTurnProjection();

  // When text tokens go quiet mid-turn — typically because the model is
  // emitting tool-call deltas that don't surface as `token` events — we
  // want the "Thinking…" affordance back so the transcript isn't silent
  // for the second-or-so before `tool_start` fires. `scheduleThinking`
  // (below) sets this timer on every `token`; a follow-up token cancels
  // it, and `tool_start` / `approval_request` / `done` clear it too so
  // the indicator doesn't flash after the turn genuinely ends.

  const [pickerOpen, setPickerOpen] = useWorkspaceUiField('pickerOpen');

  const [paletteOpen, setPaletteOpen] = useWorkspaceUiField('paletteOpen');

  /** The context inspector (issue #70). */
  const [inspectOpen, setInspectOpen] = useWorkspaceUiField('inspectOpen');

  /** "Bring chats from other agents", from ⌘K. */
  const [importOpen, setImportOpen] = useWorkspaceUiField('importOpen');

  /** "Report a problem" (issue #156), from ⌘K or Settings → About. */
  const [reportOpen, setReportOpen] = useWorkspaceUiField('reportOpen');

  // File picker opened from the right panel's "+" menu — distinct from the
  // folder picker, which switches the session's cwd.
  const [panelFilePickerOpen, setPanelFilePickerOpen] = useWorkspaceUiField('panelFilePickerOpen');

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
  const [sidebarOpen, setSidebarOpen] = useWorkspaceUiField('sidebarOpen');

  // At phone width there's room for one column: the sidebar becomes a
  // drawer over the chat, closed by default and after every pick in it.
  const phone = useIsPhone();

  const [drawerOpen, setDrawerOpen] = useWorkspaceUiField('drawerOpen');



  // Right-side panel: `agentTabs` is the ordered list of open agent call_ids;
  // `fileTabs` is the ordered list of open file viewer tabs; `toolTabs` holds
  // the utility panes (browser / whiteboard / devtools, one of each);
  // `activeAgentTab` is the visible panel's id — a callId, a file path, or a
  // `tool:<kind>` id. Panel is open iff any list is non-empty.
  const [agentTabs, setAgentTabs] = useWorkspaceUiField('agentTabs');

  const [fileTabs, setFileTabs] = useWorkspaceUiField('fileTabs');

  const [toolTabs, setToolTabs] = useWorkspaceUiField('toolTabs');

  const [activeAgentTab, setActiveAgentTab] = useWorkspaceUiField('activeAgentTab');

  // Right panel pixel width — user-draggable via the resize handle.
  const [rightPanelWidth, setRightPanelWidth] = useWorkspaceUiField('rightPanelWidth');

  // Live subagent transcripts keyed by parent tool-call id. Same Entry
  // shape as the parent transcript so the panel body can reuse the same
  // grouping and rendering helpers. Populated by the `subagent_*` WS
  // frames the AgentTool broadcasts as its child streams events.
  const [subagentState, updateSubagentProjection] = useSessionField('subagentState');



  const [sidebarRefresh, setSidebarRefresh] = useWorkspaceUiField('sidebarRefresh');

  const [reviewPanelOpen, setReviewPanelOpen] = useWorkspaceUiField('reviewPanelOpen');

  const [reviewState, setReviewState] = useSessionField('reviewState');

  const [usage, recordUsage] = useSessionField('usage');

  // Why the provider's prompt cache last went unused, for the usage popover.
  const [cacheMiss, recordCacheMiss] = useSessionField('cacheMiss');

  const [rateLimit, recordRateLimit] = useSessionField('rateLimit');



  // Context panel: collapsed to a pill by default so it stays out of the
  // way; the choice is remembered per browser.
  const [ctxOpen, setCtxOpen] = useContextPanelState();

  // The session committed through the panel, so any unpushed commits on
  // the branch are its own to push.
  const [sessionCommitted, setSessionCommitted] = useSessionField('sessionCommitted');

  const [reviewOpen, setReviewOpen] = useWorkspaceUiField('reviewOpen');

  /** File the review drawer should open on, when opened from the file list. */
  const [reviewFocus, setReviewFocus] = useWorkspaceUiField('reviewFocus');

  const [lightbox, setLightbox] = useWorkspaceUiField('lightbox');

  // Integrated terminal (bottom panel); open state is remembered.
  // Settings → General can disable the restore (always start closed).
  const [terminalOpen, setTerminalOpen] = useWorkspaceUiField('terminalOpen');

  const keybindings = useKeybindings();


  const getTranscript = () => appRuntime.readSession().entries;

  const [selectedProcess, setSelectedProcess] = useWorkspaceUiField('selectedProcess');

  const ctxFits = useContextPanelFits();

  // The chat column's width, for whether the open context panel sits
  // beside the transcript or over it.
  const [chatColWidth, setChatColWidth] = useWorkspaceUiField('chatColWidth');



  /** Re-read the repo state the panels show: git status, what this chat
   *  changed, and the branch's PR. Responses are applied only if no newer
   *  refresh has started since — after a quick session switch, a slow reply
   *  for the previous chat must not overwrite the current one. */

  // Live task list — hydrated from `ready.tasks` on socket open and
  // upserted whenever a `task_*` tool result lands. Rendered as a
  // persistent "Plan" card near the top of the transcript.
  const [tasks, recordTasks] = useSessionField('tasks');

  // Standing `/goal`, if any. Populated from `ready.goal` on socket
  // open and mutated by `goal_set` / `goal_progress` / `goal_done` /
  // `goal_cleared` server frames. Absent = no autonomous run set.
  const [goal, recordGoal] = useSessionField('goal');

  // Remote environment of the attached session (see EnvironmentChip).
  const [environment, recordEnvironment] = useSessionField('environment');

  const [environments, recordEnvironments] = useSessionField('environments');

  const [envSwitching, recordEnvironmentTransition] = useSessionField('envSwitching');

  // Loaded skill roster — powers `/<skill-name>` slash commands in the
  // composer palette. Fetched lazily after the first WS Ready frame
  // (server needs to be up + AppState wired). Empty on error; the
  // palette degrades gracefully to just the built-in commands.

  // Custom commands + MCP prompts for the composer palette, and a
  // counter the Plugins page watches to refetch on `extensions_changed`.

  const [extensionsVersion, setExtensionsVersion] = useWorkspaceUiField('extensionsVersion');

  // Bumps each time the backend broadcasts `SkillsReloaded` (filesystem
  // watcher detected a change). Passed to the Settings panel so its
  // Skills tab re-fetches when a `SKILL.md` lands / vanishes / edits
  // while it's open.
  const [skillsVersion, setSkillsVersion] = useWorkspaceUiField('skillsVersion');

  // Force a re-render every second while a turn is active so the live
  // "Working…" counter ticks. Cheap; the tree is small and only mounts
  // when the browser tab is visible.


  const recoveryBySession = useSelector(appRuntime.sessionState, state => state.recovery);
  const recordQueueRecovery = appRuntime.updateRecovery;



  // Plan payloads that arrived *before* their tool_start (race between the
  // interactive tool's direct broadcast and the harness→WS forwarder). We
  // stash by call_id and drain on tool_start so no plan ever renders as a
  // plain running tool row.
  const pendingProposalsRef = sessionServices.ref('pendingProposals');

  // Same race-guard pattern as pendingProposalsRef but for the ask_user
  // tool: `ask_user_request` might arrive before the matching
  // `tool_start` (they broadcast on the same channel but the harness
  // doesn't guarantee arrival order). Stash the proposal here so the
  // tool_start case can drain it onto the fresh entry.
  const pendingAskUserRef = sessionServices.ref('pendingQuestions');

  // Follow new output only while the reader is at the bottom; scrolling
  // up to read back stops the follow and offers a jump-to-latest button.


  const [showJump, setShowJump] = useWorkspaceUiField('showJump');


  // Desktop notification when a turn finishes while the tab is hidden
  // (Settings → General → Notifications). Only fires if the user granted
  // permission; requesting happens from the settings row.
  // Read from WS handlers, which close over the first render.

  // Smooth streaming: tokens land in a buffer and are released a few
  // characters per animation frame (faster when the buffer is deep), so
  // text flows instead of jumping in network-sized chunks.


  // Per-turn usage: session totals at turn start, diffed at turn end.
  const getUsage = () => appRuntime.readSession().usage;

  const turnBaseRef = sessionServices.ref('turnBase');

  const [turnUsage, recordTurnUsage] = useSessionField('turnUsage');

  /** The model each reloaded turn ran on, for pricing it. Live turns use
   *  the current model. */
  const [turnModels, recordTurnModels] = useSessionField('turnModels');



  // Tool calls waiting for the user's Y/N decision. The Allow / Deny /
  // Always-allow buttons render inline on the pending tool card in
  // the transcript. Kept oldest-first — the top of the queue is what
  // the Y/N global shortcut targets.

  // Restore files to before a message: preview → confirm → restore, with an
  // Undo afterwards.
  const [restoreAsk, setRestoreAsk] = useWorkspaceUiField('restoreAsk');

  const [restoreNote, setRestoreNote] = useWorkspaceUiField('restoreNote');

  // Second opinion: after an external agent's turn that changed files,
  // offer a review by Mira's own reviewer. One offer per turn; dismissing
  // or accepting it retires it.
  const [secondOpinionSeen, setSecondOpinionSeen] = useWorkspaceUiField('secondOpinionSeen');

  // Show the browser when Mira or an agent starts using it (Settings →
  // General → Browser). Only for a live call — reloading a chat with old
  // browser calls must not pop the pane — and once per turn, so closing the
  // pane sticks until the next message.

  // Bumped by `browser_active`: an agent called a browser tool, whatever
  // name its harness gave it.
  const [browserPing, setBrowserPing] = useWorkspaceUiField('browserPing');

  // Secrets agents asked for, waiting on the user (shown above the composer).
  const [secretRequests, recordSecretRequests] = useSessionField('secretRequests');

  return {
    pingPrimedRef,
    status,
    setStatus,
    activityTitlesRef,
    sessionId,
    getSessionId,
    queuedBySession,
    updateQueueProjection,
    getQueues,
    pendingSteerRef,
    sessionTitle,
    recordSessionTitle,
    model,
    recordModel,
    mode,
    recordMode,
    cwd,
    recordCwd,
    outlineEntries,
    updateTranscript,
    ruleEditorCallId,
    setRuleEditorCallId,
    approvalRules,
    recordApprovalRules,
    approvalStarted,
    historyCursor,
    recordHistoryCursor,
    historyLoading,
    recordHistoryLoading,
    historyError,
    recordHistoryError,
    historyRequestRef,
    historyPreviewsRef,
    historyOffsetRef,
    providerTurnCursorRef,
    turnDiffs,
    recordTurnDiffs,
    acpModes,
    recordAgentModes,
    engines,
    acpConfig,
    recordAgentConfig,
    acpCommands,
    recordAgentCommands,
    acpUsage,
    recordAgentUsage,
    acpLimits,
    recordAgentLimits,
    providerContext,
    recordProviderContext,
    engine,
    recordEngine,
    getCapsDriver,
    getEngine,
    acpAgents,
    acpStatusPending,
    acpAgentsRef,
    completedUpdatesRef,
    infoNotices,
    setInfoNotices,
    acpError,
    recordAgentError,
    pendingAcpMode,
    setPendingAcpMode,
    turnTimings,
    recordTurnTimings,
    turnStartRef,
    expandedTurns,
    setExpandedTurns,
    busy,
    reportTurnRunning,
    isTurnRunning,
    turnCommands,
    thinking,
    reportThinking,
    thinkingIdleTimerRef,
    pickerOpen,
    setPickerOpen,
    paletteOpen,
    setPaletteOpen,
    inspectOpen,
    setInspectOpen,
    importOpen,
    setImportOpen,
    reportOpen,
    setReportOpen,
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
    updateSubagentProjection,
    configured,
    providerName,
    sidebarRefresh,
    setSidebarRefresh,
    reviewPanelOpen,
    setReviewPanelOpen,
    reviewState,
    setReviewState,
    usage,
    recordUsage,
    cacheMiss,
    recordCacheMiss,
    rateLimit,
    recordRateLimit,
    gitStatus,
    sessionDiff,
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
    getTranscript,
    selectedProcess,
    setSelectedProcess,
    ctxFits,
    chatColWidth,
    setChatColWidth,
    chatColObserver,
    branchPr,
    tasks,
    recordTasks,
    goal,
    recordGoal,
    environment,
    recordEnvironment,
    environments,
    recordEnvironments,
    envSwitching,
    recordEnvironmentTransition,
    skills,
    commands,
    extensionsVersion,
    setExtensionsVersion,
    skillsVersion,
    setSkillsVersion,
    wsRef,
    streams,
    appRuntime,
    recoveryBySession,
    recordQueueRecovery,
    queueMutationsRef,
    paneRef,
    pendingProposalsRef,
    pendingAskUserRef,
    followRef,
    readingAnchorRef,
    attachFromRef,
    showJump,
    setShowJump,
    lastScrollInputRef,
    chatTitleRef,
    getUsage,
    turnBaseRef,
    turnUsage,
    recordTurnUsage,
    turnModels,
    recordTurnModels,
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
    recordSecretRequests,
    browserPingSeen,
  };
}
