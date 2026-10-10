import { putCwd } from '../api';
import { type InfoNotice } from '../components/InfoNoticeHost';
import { loadAgentCaps } from '../lib/acpAgents';
import { callForAttention } from '../lib/attention';
import { isDesktop, pickFolder } from '../lib/desktop';
import { type ShortcutMatchContext } from '../lib/keybindings';
import { applySessionActivity, type SessionActivityUpdate } from '../lib/sessionActivity';
import type { SessionEngine } from '../types';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useSessionCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'historyCursor'
    | 'historyLoading'
    | 'sessionIdRef'
    | 'historyRequestRef'
    | 'setHistoryLoading'
    | 'setHistoryError'
    | 'wsRef'
    | 'engineRef'
    | 'setEngine'
    | 'capsDriverRef'
    | 'setAcpModes'
    | 'setAcpConfig'
    | 'setAcpCommands'
    | 'setAcpUsage'
    | 'setAcpLimits'
    | 'setInfoNotices'
    | 'setBusy'
    | 'busyRef'
    | 'setThinking'
    | 'setRestoreNote'
    | 'setPickerOpen'
    | 'setEntries'
    | 'reviewOpen'
    | 'thinkingIdleTimerRef'
    | 'openSettings'
    | 'setSettingsSection'
    | 'visibleApprovalRef'
    | 'mainView'
    | 'sessionActivityRef'
    | 'setSessionActivity'
    | 'setCompletedSessions'
    | 'chatTitleRef'
    | 'activityTitlesRef'
    | 'setMainView'
  >,
) {
  const {
    reviewOpen,
    historyCursor,
    historyLoading,
    sessionIdRef,
    historyRequestRef,
    setHistoryLoading,
    setHistoryError,
    wsRef,
    engineRef,
    setEngine,
    capsDriverRef,
    setAcpModes,
    setAcpConfig,
    setAcpCommands,
    setAcpUsage,
    setAcpLimits,
    setInfoNotices,
    setBusy,
    busyRef,
    setThinking,
    setRestoreNote,
    setPickerOpen,
    setEntries,
    thinkingIdleTimerRef,
    openSettings,
    setSettingsSection,
    visibleApprovalRef,
    mainView,
    sessionActivityRef,
    setSessionActivity,
    setCompletedSessions,
    chatTitleRef,
    activityTitlesRef,
    setMainView,
  } = context;

  function loadOlderHistory() {
    if (!historyCursor || historyLoading || !sessionIdRef.current) return;
    const request_id = crypto.randomUUID();
    historyRequestRef.current = request_id;
    setHistoryLoading(true);
    setHistoryError(null);
    wsRef.current?.send({
      type: 'history',
      session_id: sessionIdRef.current,
      cursor: historyCursor,
      request_id,
    });
    window.setTimeout(() => {
      if (historyRequestRef.current === request_id) {
        historyRequestRef.current = null;
        setHistoryLoading(false);
        setHistoryError('Older messages could not be loaded. Try again.');
      }
    }, 15000);
  }

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

  function pushInfoNotice(notice: InfoNotice) {
    setInfoNotices((prev) =>
      [
        ...prev.filter((n) => !(notice.kind === 'persistent' && n.title === notice.title)),
        notice,
      ].slice(-5),
    );
    if (notice.kind === 'short') {
      window.setTimeout(() => {
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
    setEntries((prev) => [
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
    wsRef.current?.send({ type: 'acp_compact', focus: focus?.trim() || null });
  }

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
    const previous = sessionActivityRef.current;
    const next = applySessionActivity(previous, update);
    if (next === previous) return;
    sessionActivityRef.current = next;
    setSessionActivity(next);
    if (!previous || !next || previous.epoch !== next.epoch) return;
    const completed = [...previous.running].filter((id) => !next.running.has(id));
    if (!completed.length) return;
    setCompletedSessions((previous) => {
      const records = new Map(previous);
      for (const id of completed) {
        records.delete(id);
        records.set(id, next.revision);
      }
      while (records.size > 500) records.delete(records.keys().next().value!);
      return records;
    });
    for (const id of completed) {
      const title =
        id === sessionIdRef.current ? chatTitleRef.current : activityTitlesRef.current.get(id);
      callForAttention('done', title ?? `Chat ${id.slice(0, 8)}`, {
        sessionId: id,
        whileVisible: id !== sessionIdRef.current,
        onOpen: () => {
          setMainView('chat');
          wsRef.current?.attach(id);
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
