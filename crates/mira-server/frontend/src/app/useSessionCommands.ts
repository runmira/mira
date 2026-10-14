import { forkSession, putCwd } from '../api';
import { type InfoNotice } from '../components/InfoNoticeHost';
import { callForAttention } from '../lib/attention';
import { isDesktop,pickFolder } from '../lib/desktop';
import { type ShortcutMatchContext } from '../lib/keybindings';
import { type SessionActivityUpdate } from '../lib/sessionActivity';
import { useAppRuntime } from '../state/RuntimeProvider';
import type { SessionEngine } from '../types';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useSessionCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'historyCursor'
    | 'historyLoading'
    | 'getSessionId'
    | 'historyRequestRef'
    | 'recordHistoryLoading'
    | 'recordHistoryError'
    | 'wsRef'
    | 'getEngine'
    | 'recordEngine'
    | 'recordAgentModes'
    | 'recordAgentConfig'
    | 'recordAgentCommands'
    | 'recordAgentUsage'
    | 'recordAgentLimits'
    | 'setInfoNotices'
    | 'reportTurnRunning'
    | 'turnCommands'
    | 'reportThinking'
    | 'setRestoreNote'
    | 'setPickerOpen'
    | 'updateTranscript'
    | 'reviewOpen'
    | 'thinkingIdleTimerRef'
    | 'openSettings'
    | 'setSettingsSection'
    | 'visibleApprovalRef'
    | 'mainView'
    | 'chatTitleRef'
    | 'activityTitlesRef'
    | 'setMainView'
    | 'attachFromRef'
    | 'setSidebarRefresh'
  >,
) {
  const appRuntime = useAppRuntime();
  const {
    reviewOpen,
    historyCursor,
    historyLoading,
    getSessionId,
    historyRequestRef,
    recordHistoryLoading,
    recordHistoryError,
    wsRef,
    getEngine,
    recordEngine,
    recordAgentModes,
    recordAgentConfig,
    recordAgentCommands,
    recordAgentUsage,
    recordAgentLimits,
    setInfoNotices,
    reportTurnRunning,
    turnCommands,
    reportThinking,
    setRestoreNote,
    setPickerOpen,
    updateTranscript,
    thinkingIdleTimerRef,
    openSettings,
    setSettingsSection,
    visibleApprovalRef,
    mainView,
    chatTitleRef,
    activityTitlesRef,
    setMainView,
    attachFromRef,
    setSidebarRefresh,
  } = context;

  function loadOlderHistory() {
    if (!historyCursor || historyLoading || !getSessionId()) return;
    const request_id = crypto.randomUUID();
    historyRequestRef.current = request_id;
    appRuntime.sessionServices.ref('historyGeneration').current = appRuntime.sessionState.get().attachment;
    recordHistoryLoading(true);
    recordHistoryError(null);
    wsRef.current?.send({
      type: 'history',
      session_id: getSessionId(),
      cursor: historyCursor,
      request_id,
    });
    appRuntime.service.schedule(() => {
      if (historyRequestRef.current === request_id) {
        historyRequestRef.current = null;
        recordHistoryLoading(false);
        recordHistoryError('Older messages could not be loaded. Try again.');
      }
    }, 15000);
  }

  /** Adopt a new engine. Switching to a different agent swaps the agent
   *  state for that agent's last-seen capabilities at once, so the model
   *  list and the mode control are right before the agent has reported
   *  anything; its live frames replace them as they arrive. */
  function applyEngine(next: SessionEngine | null) {
    const previousEngine = getEngine();
    const previousDriver = previousEngine?.kind === 'agent' ? previousEngine.driver ?? null : null;
    recordEngine(next);
    const driver = next?.kind === 'agent' ? (next.driver ?? null) : null;
    if (driver === previousDriver) return;
    const caps = driver ? appRuntime.readCapabilities(driver) ?? {} : {};
    recordAgentModes(caps.modes ?? null);
    recordAgentConfig(caps.config ?? []);
    recordAgentCommands([]);
    recordAgentUsage(null);
    recordAgentLimits([]);
  }

  function pushInfoNotice(notice: InfoNotice) {
    setInfoNotices((prev) =>
      [
        ...prev.filter((n) => !(notice.kind === 'persistent' && n.title === notice.title)),
        notice,
      ].slice(-5),
    );
    if (notice.kind === 'short') {
      appRuntime.service.schedule(() => {
        setInfoNotices((prev) => prev.filter((n) => n.id !== notice.id));
      }, 3200);
    }
  }

  function dismissInfoNotice(id: string) {
    setInfoNotices((prev) => prev.filter((n) => n.id !== id));
  }

  /** Switch this window to another chat. The composer stops showing the
   *  last chat's turn at once; the new chat's `ready` turns Stop back on if
   *  a turn is running there. */
  function attachSession(id: string) {
    appRuntime.streams.flush();
    clearThinkingIdle();
    historyRequestRef.current = null;
    recordHistoryLoading(false);
    // Remember which chat we came from: `suspend` below flips the selected
    // session at once, so by the time this chat's `ready` arrives
    // `getSessionId()` already reports the *new* chat and the scroll/follow
    // reset is skipped. The ready handler consumes this.
    attachFromRef.current = getSessionId();
    turnCommands.suspend(id);
    wsRef.current?.attach(id);
  }

  /** Point this chat at `path`. The server answers with the id of the
   *  fresh slot it materialised for that folder, so the socket has to
   *  follow it — otherwise it keeps forwarding the previous slot's
   *  frames and the UI silently stays where it was. */
  async function switchCwd(path: string) {
    try {
      const { session_id } = await putCwd(path, appRuntime.sessionState.get().active ?? undefined);
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

  /** A separate chat continuing a native fork of this agent's whole
   *  conversation. This chat and its agent carry on unchanged. */
  async function forkAcpAgent() {
    const sessionId = getSessionId();
    if (!sessionId) return;
    try {
      const id = await forkSession(sessionId, { text: '', occurrence: 0, whole: true });
      setSidebarRefresh((n) => n + 1);
      attachSession(id);
    } catch (e) {
      setRestoreNote({ text: `Couldn't fork: ${(e as Error).message}`, error: true });
    }
  }

  function compactAcpAgent(focus?: string) {
    if (!wsRef.current?.sendImmediate({ type: 'acp_compact', focus: focus?.trim() || null })) {
      setRestoreNote({ text: 'Reconnect before compacting this chat.', error: true }); return;
    }
    // OpenCode's normal prompt admission owns busy state and the placeholder.
    // A rejected request must not invent a turn or leave a running card behind.
    if (getEngine()?.driver === 'opencode') return;
    // A compaction is a turn like any other: busy until its end arrives, or
    // the composer would take input for a session that is summarizing.
    reportTurnRunning(true);
    reportThinking(true);
    // Agents compact inside their own process and only end the turn, so the
    // card runs from here until that turn ends.
    updateTranscript((prev) => [
      ...prev,
      {
        kind: 'compact',
        state: 'running',
        trigger: 'agent',
        startedAt: Date.now(),
        summarized: null,
        summary: null,
      },
    ]);
  }

  function clearThinkingIdle() {
    if (thinkingIdleTimerRef.current != null) {
      window.clearTimeout(thinkingIdleTimerRef.current);
      thinkingIdleTimerRef.current = null;
    }
  }

  function scheduleThinkingIdle() {
    clearThinkingIdle();
    const scheduledGeneration = appRuntime.sessionState.get().generation;
    thinkingIdleTimerRef.current = appRuntime.service.schedule(() => {
      thinkingIdleTimerRef.current = null;
      if (appRuntime.sessionState.get().generation === scheduledGeneration) reportThinking(true);
    }, 350);
  }

  /** Jump straight to agent configuration (install hints, advanced setup). */
  function openAgentSettings() {
    openSettings();
    setSettingsSection('agents');
  }

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

  function updateSessionActivity(update: SessionActivityUpdate) {
    const completed = appRuntime.commands.applySessionActivity(update);
    for (const id of completed) {
      const title =
        id === getSessionId() ? chatTitleRef.current : activityTitlesRef.current.get(id);
      callForAttention('done', title ?? `Chat ${id.slice(0, 8)}`, {
        sessionId: id,
        whileVisible: id !== getSessionId(),
        onOpen: () => {
          setMainView('chat');
          attachSession(id);
        },
      });
    }
  }
  return {
    loadOlderHistory,
    applyEngine,
    pushInfoNotice,
    dismissInfoNotice,
    attachSession,
    switchCwd,
    openProjectPicker,
    forkAcpAgent,
    compactAcpAgent,
    clearThinkingIdle,
    scheduleThinkingIdle,
    openAgentSettings,
    shortcutContext,
    updateSessionActivity,
  };
}
