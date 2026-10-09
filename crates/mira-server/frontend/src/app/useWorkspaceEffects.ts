import { useEffect, useLayoutEffect } from 'react';
import { getAgentSettings, getSettings, listSessions, putAgentSettings } from '../api';
import { agentUpdateNotice } from '../components/InfoNoticeHost';
import { type ToolPaneKind } from '../components/panes/toolPanes';
import { CommandPalette } from '../lazyViews';
import { migrateLegacyAgentConfigs } from '../lib/acpAgents';
import { attachFilesToComposer } from '../lib/attachBridge';
import { getCustomKeybindingRules, resolveShortcutCommand } from '../lib/keybindings';
import { useModalFocus } from '../lib/mobile';
import { applyReduceMotion, getBoolPref, PREF_KEYS } from '../lib/prefs';
import { ensureServerPricing } from '../lib/usage';
import { titleFromEntries, type MsgEntry } from '../transcript/entries';
import type { AcpAgentStatus } from '../types';
import { connect } from '../ws';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useWorkspaceEffects(
  context: Pick<
    WorkspaceRuntime,
    | 'pingPrimedRef'
    | 'queuedBySessionRef'
    | 'queuedBySession'
    | 'entries'
    | 'approvalStarted'
    | 'setApprovalRules'
    | 'setRuleEditorCallId'
    | 'engines'
    | 'acpAgentsRef'
    | 'setAcpAgents'
    | 'completedUpdatesRef'
    | 'pushInfoNotice'
    | 'loadEngines'
    | 'acpAgents'
    | 'acpUsage'
    | 'busyRef'
    | 'busy'
    | 'mainView'
    | 'setMainView'
    | 'githubReturn'
    | 'setSettingsSection'
    | 'openSettings'
    | 'phone'
    | 'drawerOpen'
    | 'drawerRef'
    | 'closeDrawer'
    | 'attachSession'
    | 'setSidebarRefresh'
    | 'shortcutEntriesRef'
    | 'keybindings'
    | 'shortcutContext'
    | 'setTerminalOpen'
    | 'setPaletteOpen'
    | 'refreshRepo'
    | 'onMessage'
    | 'queueMutationsRef'
    | 'flushNativeFrames'
    | 'flushTokens'
    | 'setStatus'
    | 'wsRef'
    | 'nativeRafRef'
    | 'tokenRafRef'
    | 'nativeFramesRef'
    | 'tokenBufRef'
    | 'setConfigured'
    | 'setProviderName'
    | 'paneRef'
    | 'readingAnchorRef'
    | 'followRef'
    | 'thinking'
    | 'sessionId'
    | 'setNowTick'
    | 'chatTitleRef'
    | 'sessionTitle'
    | 'firstPendingCallId'
    | 'visibleApprovalRef'
    | 'decideApproval'
    | 'agentTabs'
    | 'fileTabs'
    | 'toolTabs'
    | 'setActiveAgentTab'
    | 'chatShortcutRequest'
    | 'sessionIdRef'
    | 'setEntries'
    | 'openToolPane'
    | 'activeAgentTab'
    | 'closeAnyTab'
    | 'setBusy'
    | 'setThinking'
    | 'setPanelFilePickerOpen'
    | 'closeSubagentPanel'
    | 'onNewChat'
    | 'setReviewOpen'
    | 'toggleSidebar'
    | 'exitSettings'
    | 'browserPing'
    | 'browserPingSeen'
    | 'browserShownFor'
  >,
) {
  const {
    pingPrimedRef,
    queuedBySessionRef,
    queuedBySession,
    entries,
    approvalStarted,
    setApprovalRules,
    setRuleEditorCallId,
    engines,
    acpAgentsRef,
    setAcpAgents,
    completedUpdatesRef,
    pushInfoNotice,
    loadEngines,
    acpAgents,
    acpUsage,
    busyRef,
    busy,
    mainView,
    setMainView,
    githubReturn,
    setSettingsSection,
    openSettings,
    phone,
    drawerOpen,
    drawerRef,
    closeDrawer,
    attachSession,
    setSidebarRefresh,
    shortcutEntriesRef,
    keybindings,
    shortcutContext,
    setTerminalOpen,
    setPaletteOpen,
    refreshRepo,
    onMessage,
    queueMutationsRef,
    flushNativeFrames,
    flushTokens,
    setStatus,
    wsRef,
    nativeRafRef,
    tokenRafRef,
    nativeFramesRef,
    tokenBufRef,
    setConfigured,
    setProviderName,
    paneRef,
    readingAnchorRef,
    followRef,
    thinking,
    sessionId,
    setNowTick,
    chatTitleRef,
    sessionTitle,
    firstPendingCallId,
    visibleApprovalRef,
    decideApproval,
    agentTabs,
    fileTabs,
    toolTabs,
    setActiveAgentTab,
    chatShortcutRequest,
    sessionIdRef,
    setEntries,
    openToolPane,
    activeAgentTab,
    closeAnyTab,
    setBusy,
    setThinking,
    setPanelFilePickerOpen,
    closeSubagentPanel,
    onNewChat,
    setReviewOpen,
    toggleSidebar,
    exitSettings,
    browserPing,
    browserPingSeen,
    browserShownFor,
  } = context;

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
      void a
        .play()
        .then(() => {
          a.pause();
          a.currentTime = 0;
        })
        .catch(() => {});
    }

    window.addEventListener('pointerdown', prime, { once: true });
    window.addEventListener('keydown', prime, { once: true });
    return () => {
      window.removeEventListener('pointerdown', prime);
      window.removeEventListener('keydown', prime);
    };
  }, []);

  queuedBySessionRef.current = queuedBySession;

  useEffect(() => {
    const pending = new Set(
      entries.flatMap((entry) =>
        entry.kind === 'tool' && entry.status === 'pending' ? [entry.call.id] : [],
      ),
    );
    for (const id of Object.keys(approvalStarted.current)) {
      if (!pending.has(id)) delete approvalStarted.current[id];
    }
    setApprovalRules((previous) => {
      const retained = Object.entries(previous).filter(([id]) => pending.has(id));
      return retained.length === Object.keys(previous).length
        ? previous
        : Object.fromEntries(retained);
    });
    setRuleEditorCallId((previous) => (previous && pending.has(previous) ? previous : null));
  }, [entries]);

  // The agent list, from the engines list: one row per agent driver (its
  // default instance), with the full status the server probed. Agent update
  // notices compare against the previous list, as they did for `acp_status`.
  useEffect(() => {
    if (!engines) return;
    const next = engines
      .filter((e) => e.flavor === 'external' && e.agent && e.instance === e.driver)
      .map((e) => e.agent as AcpAgentStatus);
    if (next.length === 0) return;
    const notice =
      acpAgentsRef.current.length > 0 ? agentUpdateNotice(acpAgentsRef.current, next) : null;
    acpAgentsRef.current = next;
    setAcpAgents(next);
    if (notice && (!notice.updateAgent || !completedUpdatesRef.current.has(notice.updateAgent)))
      pushInfoNotice(notice);
    completedUpdatesRef.current.clear();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engines]);

  // Agent setups older builds kept in localStorage (API key included) move
  // to the server once, then leave the browser (#79).
  useEffect(() => {
    void migrateLegacyAgentConfigs(putAgentSettings, getAgentSettings)
      .then((moved) => {
        if (moved.length > 0) loadEngines();
      })
      .catch(() => {});
  }, [loadEngines]);

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

  busyRef.current = busy;

  // ⌘K is usually how the palette opens, so there's no hover to preload
  // on; fetch it once the first load has settled.
  useEffect(() => {
    const idle = window.requestIdleCallback ?? ((cb: () => void) => window.setTimeout(cb, 2000));
    idle(() => CommandPalette.preload());
  }, []);

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
    return () => {
      window.removeEventListener('dragover', over);
      window.removeEventListener('drop', drop);
    };
  }, [mainView]);

  useEffect(() => {
    if (!githubReturn) return;
    setSettingsSection('integrations');
    openSettings();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useModalFocus(phone && drawerOpen, drawerRef, closeDrawer);

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

  shortcutEntriesRef.current = entries;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        e.repeat ||
        e.isComposing ||
        (e.target as HTMLElement)?.closest('[data-keybinding-capture]')
      )
        return;
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (command !== 'terminal.toggle') return;
      e.preventDefault();
      setTerminalOpen((v) => {
        try {
          localStorage.setItem('mira.terminal.open', v ? '0' : '1');
        } catch {
          /* private mode */
        }
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
      if (
        e.repeat ||
        e.isComposing ||
        (e.target as HTMLElement)?.closest('[data-keybinding-capture]')
      )
        return;
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

  // Something outside a turn changed files (a diff applied from a reply).
  useEffect(() => {
    window.addEventListener('mira:repo-changed', refreshRepo);
    return () => window.removeEventListener('mira:repo-changed', refreshRepo);
  }, [refreshRepo]);

  useEffect(() => {
    const c = connect(onMessage, (nextStatus) => {
      queueMutationsRef.current.setConnected(nextStatus === 'open');
      if (nextStatus !== 'open') {
        flushNativeFrames();
        flushTokens();
      }
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

  chatTitleRef.current = sessionTitle ?? titleFromEntries(entries);

  // Apply the reduce-motion class on boot (Settings → General → Appearance).
  useEffect(() => {
    applyReduceMotion();
  }, []);

  useEffect(() => {
    if (!firstPendingCallId) return;
    function onKey(e: KeyboardEvent) {
      const visibleCallId = visibleApprovalRef.current;
      if (!visibleCallId) return;
      const t = e.target as HTMLElement | null;
      if (t) {
        const tag = t.tagName;
        const queueCheckbox =
          t instanceof HTMLInputElement &&
          t.type === 'checkbox' &&
          t.dataset.approvalQueue === 'true';
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

  // Global app shortcuts, resolved through the keybinding engine
  // Modified app actions work from the composer; bare keys yield to editing.
  // Recording fields, dialogs, composition, and repeated keys retain ownership.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.defaultPrevented || e.repeat || e.isComposing) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest('[data-keybinding-capture]') || document.querySelector('[role=dialog]'))
        return;
      if (t) {
        const tag = t.tagName;
        if (
          (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t.isContentEditable) &&
          !e.metaKey &&
          !e.ctrlKey
        )
          return;
      }
      const command = resolveShortcutCommand(e, keybindings, { context: shortcutContext() });
      if (!command) return;
      if (command.startsWith('panel.slot')) {
        const id = [
          ...agentTabs,
          ...fileTabs.map((tab) => tab.id),
          ...toolTabs.map((tab) => tab.id),
        ][Number(command.at(-1)) - 1];
        if (id) {
          e.preventDefault();
          setMainView('chat');
          setActiveAgentTab(id);
        }
        return;
      }
      if (command.startsWith('chat.slot')) {
        e.preventDefault();
        const request = ++chatShortcutRequest.current;
        const origin = sessionIdRef.current;
        void listSessions({ all: true })
          .then((chats) => {
            if (request !== chatShortcutRequest.current || origin !== sessionIdRef.current) return;
            const chat = chats
              .filter((chat) => !chat.parent_id)
              .sort((a, b) => b.updated_at - a.updated_at)[Number(command.at(-1)) - 1];
            if (chat) {
              setMainView('chat');
              attachSession(chat.id);
            }
          })
          .catch((error) =>
            setEntries((prev) => [
              ...prev,
              { kind: 'error', text: `Could not switch chats: ${error.message}` },
            ]),
          );
        return;
      }
      switch (command) {
        case 'panel.tests':
        case 'panel.activity':
        case 'panel.devices':
        case 'panel.whiteboard':
        case 'panel.aside':
        case 'panel.devtools':
          e.preventDefault();
          setMainView('chat');
          openToolPane(command.slice(6) as ToolPaneKind);
          break;
        case 'panel.next':
        case 'panel.previous': {
          const tabs = [
            ...agentTabs,
            ...fileTabs.map((tab) => tab.id),
            ...toolTabs.map((tab) => tab.id),
          ];
          if (!tabs.length) return;
          e.preventDefault();
          setMainView('chat');
          const found = tabs.indexOf(activeAgentTab ?? '');
          const index = found >= 0 ? found : command === 'panel.next' ? -1 : 0;
          setActiveAgentTab(
            tabs[(index + (command === 'panel.next' ? 1 : tabs.length - 1)) % tabs.length],
          );
          break;
        }
        case 'panel.closeTab':
          if (!activeAgentTab) return;
          e.preventDefault();
          closeAnyTab(activeAgentTab);
          break;
        case 'chat.bottom':
        case 'chat.top':
          if (mainView !== 'chat') return;
          e.preventDefault();
          paneRef.current?.scrollTo({
            top: command === 'chat.top' ? 0 : paneRef.current.scrollHeight,
            behavior: 'auto',
          });
          break;
        case 'composer.attach':
          e.preventDefault();
          setMainView('chat');
          requestAnimationFrame(() =>
            document.querySelector<HTMLInputElement>('[data-composer-attachments]')?.click(),
          );
          break;
        case 'composer.focus':
          e.preventDefault();
          setMainView('chat');
          requestAnimationFrame(() =>
            document.querySelector<HTMLElement>('.mention-input[contenteditable=true]')?.focus(),
          );
          break;
        case 'chat.stop':
          if (mainView !== 'chat' || !busyRef.current) return;
          e.preventDefault();
          wsRef.current?.send({ type: 'interrupt' });
          setBusy(false);
          busyRef.current = false;
          setThinking(false);
          break;
        case 'panel.files':
        case 'panel.processes':
        case 'panel.browser':
          e.preventDefault();
          setMainView('chat');
          if (command === 'panel.files') setPanelFilePickerOpen(true);
          else openToolPane(command === 'panel.processes' ? 'processes' : 'browser');
          break;
        case 'panel.close':
          e.preventDefault();
          closeSubagentPanel();
          break;
        case 'settings.shortcuts':
          e.preventDefault();
          openSettings();
          setSettingsSection('keybindings');
          break;
        case 'chat.copyResponse':
        case 'chat.copyCode': {
          if (mainView !== 'chat') return;
          const responses = shortcutEntriesRef.current
            .filter(
              (entry): entry is MsgEntry =>
                entry.kind === 'msg' && entry.msg.role === 'assistant' && !!entry.msg.content,
            )
            .map((entry) => entry.msg.content!);
          const blocks = responses.flatMap((text) =>
            Array.from(text.matchAll(/```[^\n]*\n([\s\S]*?)```/g), (match) => match[1]),
          );
          const text = command === 'chat.copyCode' ? blocks.at(-1) : responses.at(-1);
          if (text) {
            e.preventDefault();
            void navigator.clipboard
              .writeText(text)
              .catch(() =>
                setEntries((prev) => [
                  ...prev,
                  { kind: 'error', text: 'Could not copy to the clipboard.' },
                ]),
              );
          }
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
      if (
        e.kind === 'tool' &&
        e.call.function.name === 'browser' &&
        (e.status === 'running' || e.status === 'pending')
      ) {
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
  return {};
}
