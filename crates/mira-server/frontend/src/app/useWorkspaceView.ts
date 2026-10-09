import { useCallback, useMemo } from 'react';
import { getBranchPr, getGitStatus, getSessionDiff, listEngines } from '../api';
import { parseSentAttachments, type PendingApproval } from '../components/Composer';
import { CONTEXT_PANEL_RESERVE, contextPanelHasContent } from '../components/ContextPanel';
import type { ActivityTurn } from '../components/panes/ActivityPane';
import type { SubagentTab } from '../components/SubagentPanel';
import { type MinimapItem } from '../components/TimelineMinimap';
import { categoryFor } from '../components/tools/toolInfo';
import { type MessageActions } from '../components/transcript/EntryView';
import type { UsageRingData } from '../components/UsageRing';
import { acpOptionsToDescriptors } from '../lib/acpOptions';
import { mapPosturesToModes, POSTURES } from '../lib/agentPostures';
import { useBackgroundProcesses } from '../lib/backgroundProcesses';
import { hasHiddenTitleBar } from '../lib/desktop';
import { useLatch } from '../lib/lazy';
import { getBoolPref, PREF_KEYS } from '../lib/prefs';
import { playTurnSound } from '../lib/sound';
import { groupByTurn } from '../lib/turnActivity';
import { costUsd } from '../lib/usage';
import { type Entry, type ToolEntry } from '../transcript/entries';
import type { Mode, SettingsView } from '../types';

import { MIN_STREAM_WIDTH, useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useWorkspaceView(
  context: Pick<
    WorkspaceRuntime,
    | 'tasks'
    | 'gitStatus'
    | 'branchPr'
    | 'sessionCommitted'
    | 'engine'
    | 'acpConfig'
    | 'acpAgents'
    | 'setEngines'
    | 'setAcpStatusPending'
    | 'setAcpError'
    | 'wsRef'
    | 'inspectOpen'
    | 'paletteOpen'
    | 'pickerOpen'
    | 'phone'
    | 'setDrawerOpen'
    | 'setSidebarOpen'
    | 'setCtxOpen'
    | 'reviewOpen'
    | 'setTerminalOpen'
    | 'sessionId'
    | 'status'
    | 'mainView'
    | 'chatColObserver'
    | 'setChatColWidth'
    | 'repoSeqRef'
    | 'setGitStatus'
    | 'setSessionDiff'
    | 'setBranchPr'
    | 'lastScrollInputRef'
    | 'followRef'
    | 'setShowJump'
    | 'paneRef'
    | 'entries'
    | 'approvalStarted'
    | 'approvalRules'
    | 'acpModes'
    | 'acpUsage'
    | 'acpLimits'
    | 'compactAcpAgent'
    | 'setInspectOpen'
    | 'rateLimit'
    | 'providerContext'
    | 'usage'
    | 'model'
    | 'onCompact'
    | 'cacheMiss'
    | 'toggleTurn'
    | 'decideApproval'
    | 'replyToPlan'
    | 'replyToAskUser'
    | 'openAgentTab'
    | 'openFileTab'
    | 'onSetMode'
    | 'openSettings'
    | 'setSettingsSection'
    | 'onResend'
    | 'askRestore'
    | 'forkAt'
    | 'setLightbox'
    | 'busy'
    | 'keybindings'
    | 'onNewChat'
    | 'setPickerOpen'
    | 'setMainView'
    | 'setReviewOpen'
    | 'runReview'
    | 'terminalOpen'
    | 'openToolPane'
    | 'setImportOpen'
    | 'setConfigured'
    | 'setProviderName'
    | 'setModel'
    | 'setMode'
    | 'historyOffsetRef'
    | 'sessionDiff'
    | 'secondOpinionSeen'
    | 'setSecondOpinionSeen'
    | 'ctxFits'
    | 'chatColWidth'
    | 'ctxOpen'
    | 'agentTabs'
    | 'subagentState'
    | 'fileTabs'
    | 'toolTabs'
    | 'sidebarOpen'
    | 'rightPanelWidth'
  >,
) {
  const {
    tasks,
    gitStatus,
    branchPr,
    sessionCommitted,
    engine,
    acpConfig,
    acpAgents,
    setEngines,
    setAcpStatusPending,
    setAcpError,
    wsRef,
    inspectOpen,
    paletteOpen,
    pickerOpen,
    phone,
    setDrawerOpen,
    setSidebarOpen,
    setCtxOpen,
    reviewOpen,
    setTerminalOpen,
    sessionId,
    status,
    mainView,
    chatColObserver,
    setChatColWidth,
    repoSeqRef,
    setGitStatus,
    setSessionDiff,
    setBranchPr,
    lastScrollInputRef,
    followRef,
    setShowJump,
    paneRef,
    entries,
    approvalStarted,
    approvalRules,
    acpModes,
    acpUsage,
    acpLimits,
    compactAcpAgent,
    setInspectOpen,
    rateLimit,
    providerContext,
    usage,
    model,
    onCompact,
    cacheMiss,
    toggleTurn,
    decideApproval,
    replyToPlan,
    replyToAskUser,
    openAgentTab,
    openFileTab,
    onSetMode,
    openSettings,
    setSettingsSection,
    onResend,
    askRestore,
    forkAt,
    setLightbox,
    busy,
    setConfigured,
    setProviderName,
    setModel,
    setMode,
    historyOffsetRef,
    sessionDiff,
    secondOpinionSeen,
    setSecondOpinionSeen,
    ctxFits,
    chatColWidth,
    ctxOpen,
    agentTabs,
    subagentState,
    fileTabs,
    toolTabs,
    sidebarOpen,
    rightPanelWidth,
  } = context;

  // Respects Settings → General → "Sound when a reply finishes".
  const playPing = useCallback(() => playTurnSound(), []);

  /** The agent this session runs on (picked, starting or running), or
   *  null on a provider. Agent mode starts the moment one is picked. */
  const acpDriver = engine?.kind === 'agent' ? (engine.driver ?? null) : null;

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
  const startAcpAgent = useCallback(
    (kind: string, resume?: string | null, model?: string | null, instance?: string | null) => {
      setAcpError(null);
      wsRef.current?.send({
        type: 'acp_start',
        driver: kind,
        instance: instance ?? null,
        resume: resume || null,
        model: model || null,
      });
    },
    [],
  );

  // Lazy dialogs: fetched on first open, then kept mounted so their close
  // transitions still run.
  const inspectOpenMounted = useLatch(inspectOpen);

  const paletteOpenMounted = useLatch(paletteOpen);

  const pickerOpenMounted = useLatch(pickerOpen);

  const toggleSidebar = () => (phone ? setDrawerOpen((v) => !v) : setSidebarOpen((v) => !v));

  const closeDrawer = useCallback(() => setDrawerOpen(false), []);

  const onCtxOpenChange = (v: boolean) => {
    setCtxOpen(v);
    try {
      localStorage.setItem('mira.context.open', v ? '1' : '0');
    } catch {
      /* private mode */
    }
  };

  // Lazy chunk: fetched on first open, then kept mounted so draft review
  // comments survive closing the drawer.
  const reviewMounted = useLatch(reviewOpen);

  const setTerminal = useCallback((v: boolean) => {
    setTerminalOpen(v);
    try {
      localStorage.setItem('mira.terminal.open', v ? '1' : '0');
    } catch {
      /* private mode */
    }
  }, []);

  const backgroundProcesses = useBackgroundProcesses(
    sessionId,
    status === 'open' && mainView === 'chat',
  );

  const chatColRef = useCallback((el: HTMLDivElement | null) => {
    chatColObserver.current?.disconnect();
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setChatColWidth(Math.round(e.contentRect.width)));
    ro.observe(el);
    chatColObserver.current = ro;
  }, []);

  const refreshRepo = useCallback(() => {
    const seq = ++repoSeqRef.current;
    const live = () => seq === repoSeqRef.current;
    getGitStatus()
      .then((s) => live() && setGitStatus(s))
      .catch(() => {});
    getSessionDiff()
      .then((d) => live() && setSessionDiff(d))
      .catch(() => {});
    getBranchPr()
      .then((pr) => live() && setBranchPr(pr))
      .catch(() => live() && setBranchPr(null));
  }, []);

  const noteScrollInput = () => {
    lastScrollInputRef.current = Date.now();
  };

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
    if (Date.now() - lastScrollInputRef.current < 1000)
      followRef.current = atBottom && getBoolPref(PREF_KEYS.transcriptFollow, true);
    setShowJump(!atBottom);
  }, []);

  const pendingApprovals = useMemo<PendingApproval[]>(
    () =>
      entries
        .filter(
          (e): e is Extract<Entry, { kind: 'tool' }> => e.kind === 'tool' && e.status === 'pending',
        )
        .map((e) => ({
          callId: e.call.id,
          call: e.call,
          preview: e.preview,
          needs: e.needs,
          startedAt: approvalStarted.current[e.call.id],
          rulePreview: approvalRules[e.call.id],
        })),
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
        return [
          {
            posture,
            modeId: m.mode_id,
            modeName: m.mode_name,
            modeDescription: m.mode_description ?? null,
            current: m.current,
          },
        ];
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
        limitsTitle: acpLimits.length
          ? `Plan usage limits · ${engine?.display_name ?? 'agent'}`
          : null,
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
          return [
            {
              label:
                k === 'requests'
                  ? 'Requests'
                  : k === 'tokens'
                    ? 'Tokens'
                    : k === 'input_tokens'
                      ? 'Input tokens'
                      : 'Output tokens',
              used: 1 - b.remaining / b.limit,
              resetsAt: b.reset_secs != null ? rateLimit!.at + b.reset_secs * 1000 : null,
            },
          ];
        })
      : [];
    return {
      used: providerContext?.used ?? null,
      window: providerContext?.window ?? null,
      compactAt: providerContext?.compactAt ?? null,
      tokens:
        usage && (usage.prompt_tokens || usage.completion_tokens)
          ? {
              prompt: usage.prompt_tokens,
              completion: usage.completion_tokens,
              cached: usage.cached_input_tokens,
            }
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
  const pendingPlans = useMemo(
    () =>
      entries.flatMap((e) =>
        e.kind === 'tool' && e.plan?.decision === null
          ? [{ callId: e.call.id, proposal: e.plan.proposal }]
          : [],
      ),
    [entries],
  );

  const pendingQuestions = useMemo(
    () =>
      entries.flatMap((e) =>
        e.kind === 'tool' && e.askUser?.decision === null
          ? [{ callId: e.call.id, proposal: e.askUser.proposal }]
          : [],
      ),
    [entries],
  );

  // Global approval shortcuts for the first pending approval, resolved
  // through the keybinding engine (Settings → Keybindings). Rebinds when
  // the head-of-queue call changes so back-to-back approvals each
  // pick up their own listener. Skipped while the user is typing so
  // approval keys in the composer/settings don't fire the decision.
  const firstPendingCallId = pendingApprovals[0]?.callId ?? null;

  // Stable identities for everything handed to the transcript, so a
  // streamed token re-renders only the turn it lands in.
  const stableToggleTurn = useStableCallback((idx: number) => toggleTurn(idx));

  const stableDecide = useStableCallback(decideApproval);

  const stablePlanReply = useStableCallback(replyToPlan);

  const stableAskUserReply = useStableCallback(replyToAskUser);

  const stableOpenAgent = useStableCallback(openAgentTab);

  const stableOpenFile = useStableCallback(openFileTab);

  const stableSetMode = useStableCallback(onSetMode);

  const stableFailureSettings = useStableCallback(() => {
    openSettings();
    setSettingsSection(acpDriver ? 'agents' : 'provider');
  });

  const editMessage = useStableCallback((entry: Entry, text: string) =>
    onResend(entries.indexOf(entry), text),
  );

  const restoreMessage = useStableCallback(
    (entry: Entry) => void askRestore(entries.indexOf(entry)),
  );

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

  /** Everything ⌘K can do. Each entry runs exactly what its button or
   *  shortcut does, so the palette never drifts from the rest of the app. */

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
    () =>
      entries.every(
        (e) =>
          e.kind === 'msg' &&
          !(e.msg.content || '').trim() &&
          (e.msg.tool_calls?.length ?? 0) === 0,
      ),
    [entries],
  );

  const turns = useMemo(() => groupByTurn(entries), [entries]);

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
    !!acpDriver &&
    !busy &&
    lastTurnIdx >= 0 &&
    lastTurnEdited &&
    sessionDiff.files.length > 0 &&
    !secondOpinionSeen.has(secondOpinionKey);

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
        assistantText:
          assistant && assistant.kind === 'msg' ? (assistant.msg.content ?? null) : null,
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
    if (!(node instanceof HTMLElement)) {
      followRef.current = false;
      window.dispatchEvent(new CustomEvent('mira:transcript-jump', { detail: id }));
      return;
    }
    const delta = node.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    pane.scrollTo({ top: pane.scrollTop + delta - 12, behavior: 'smooth' });
  }, []);

  // Keep the transcript and composer clear of the context card only while
  // it's open; the collapsed pill floats over the corner.
  const ctxHasContent =
    ctxFits &&
    contextPanelHasContent({
      processes: backgroundProcesses.processes,
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

  const panelOpen = subagentTabs.length > 0 || fileTabs.length > 0 || toolTabs.length > 0;

  /** True in the desktop app on macOS, where the native title bar is gone
   *  and our own header has to stand in for it. */
  const hiddenTitleBar = hasHiddenTitleBar();

  const sidebarCol = sidebarOpen ? '300px' : '0px';

  const rightCol = panelOpen ? `${rightPanelWidth}px` : '0px';
  return {
    playPing,
    acpDriver,
    acpModelOption,
    acpDescriptors,
    acpDriverName,
    loadEngines,
    requestAcpStatus,
    startAcpAgent,
    inspectOpenMounted,
    paletteOpenMounted,
    pickerOpenMounted,
    toggleSidebar,
    closeDrawer,
    onCtxOpenChange,
    reviewMounted,
    setTerminal,
    backgroundProcesses,
    chatColRef,
    refreshRepo,
    noteScrollInput,
    jumpToLatest,
    onPaneScroll,
    pendingApprovals,
    agentPostures,
    usageRing,
    pendingPlans,
    pendingQuestions,
    firstPendingCallId,
    stableToggleTurn,
    stableDecide,
    stablePlanReply,
    stableAskUserReply,
    stableOpenAgent,
    stableOpenFile,
    stableSetMode,
    stableFailureSettings,
    editMessage,
    restoreMessage,
    forkMessage,
    retryMessage,
    messageActions,

    settingsHandler,
    isEmpty,
    turns,
    lastTurnIdx,
    secondOpinionKey,
    lastTurnEdited,
    showSecondOpinion,
    retireSecondOpinion,
    minimapItems,
    activityTurns,
    jumpToMinimapTurn,
    ctxHasContent,
    ctxRoomy,
    ctxReserve,
    ctxPill,
    subagentTabs,
    panelOpen,
    hiddenTitleBar,
    sidebarCol,
    rightCol,
  };
}
