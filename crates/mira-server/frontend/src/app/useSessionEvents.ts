import { getContextBreakdown, listCommands, listSkills } from '../api';
import * as agentTerminal from '../lib/agentTerminal';
import { getBoolPref, PREF_KEYS } from '../lib/prefs';
import { hasTranscriptPosition } from '../lib/transcriptPosition';
import {
  rebuildTurnModels,
  rebuildTurnTimings,
  rebuildTurnUsage,
  sealThought,
  upsertRuntimeQuestion,
  type Entry,
  type SubagentStreamState,
} from '../transcript/entries';
import { entriesForTranscriptPage, historyToEntries, interleaveReplay } from '../transcript/replay';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useSessionEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'queueMutationsRef'
    | 'updateSessionActivity'
    | 'sessionIdRef'
    | 'historyRequestRef'
    | 'setHistoryLoading'
    | 'setHistoryError'
    | 'paneRef'
    | 'followRef'
    | 'historyOffsetRef'
    | 'setHistoryCursor'
    | 'setEntries'
    | 'historyPreviewsRef'
    | 'readingAnchorRef'
    | 'setRecoveryBySession'
    | 'setQueuedForSession'
    | 'sendNow'
    | 'activityTitlesRef'
    | 'pendingSteerRef'
    | 'setGitStatus'
    | 'setSessionDiff'
    | 'setSessionCommitted'
    | 'setBranchPr'
    | 'setSessionId'
    | 'approvalStarted'
    | 'setApprovalRules'
    | 'setRuleEditorCallId'
    | 'wsRef'
    | 'setSessionTitle'
    | 'setProviderContext'
    | 'setAcpUsage'
    | 'setRestoreNote'
    | 'setRestoreAsk'
    | 'setModel'
    | 'setMode'
    | 'setCwd'
    | 'applyEngine'
    | 'onMessage'
    | 'setTurnDiffs'
    | 'providerTurnCursorRef'
    | 'setSubagentState'
    | 'setTurnTimings'
    | 'setTurnUsage'
    | 'setTurnModels'
    | 'setExpandedTurns'
    | 'setUsage'
    | 'setCacheMiss'
    | 'setRateLimit'
    | 'setTasks'
    | 'setGoal'
    | 'nativeWorkRef'
    | 'nativeTurnsRef'
    | 'setBusy'
    | 'busyRef'
    | 'setThinking'
    | 'clearThinkingIdle'
    | 'setSidebarRefresh'
    | 'setSkills'
    | 'setCommands'
    | 'loadEngines'
    | 'setEnvSwitching'
    | 'refreshRepo'
    | 'setMainView'
    | 'drainQueuedMessage'
  >,
) {
  const {
    queueMutationsRef,
    updateSessionActivity,
    sessionIdRef,
    historyRequestRef,
    setHistoryLoading,
    setHistoryError,
    paneRef,
    followRef,
    historyOffsetRef,
    setHistoryCursor,
    setEntries,
    historyPreviewsRef,
    readingAnchorRef,
    setRecoveryBySession,
    setQueuedForSession,
    sendNow,
    activityTitlesRef,
    pendingSteerRef,
    setGitStatus,
    setSessionDiff,
    setSessionCommitted,
    setBranchPr,
    setSessionId,
    approvalStarted,
    setApprovalRules,
    setRuleEditorCallId,
    wsRef,
    setSessionTitle,
    setProviderContext,
    setAcpUsage,
    setRestoreNote,
    setRestoreAsk,
    setModel,
    setMode,
    setCwd,
    applyEngine,
    onMessage,
    setTurnDiffs,
    providerTurnCursorRef,
    setSubagentState,
    setTurnTimings,
    setTurnUsage,
    setTurnModels,
    setExpandedTurns,
    setUsage,
    setCacheMiss,
    setRateLimit,
    setTasks,
    setGoal,
    nativeWorkRef,
    nativeTurnsRef,
    setBusy,
    busyRef,
    setThinking,
    clearThinkingIdle,
    setSidebarRefresh,
    setSkills,
    setCommands,
    loadEngines,
    setEnvSwitching,
    refreshRepo,
    setMainView,
    drainQueuedMessage,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
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

      case 'history_page': {
        if (msg.session_id !== sessionIdRef.current || msg.request_id !== historyRequestRef.current)
          break;
        historyRequestRef.current = null;
        setHistoryLoading(false);
        setHistoryError(msg.error);
        if (msg.page) {
          const page = msg.page;
          const pane = paneRef.current;
          const beforeHeight = pane?.scrollHeight ?? 0;
          const beforeTop = pane?.scrollTop ?? 0;
          followRef.current = false;
          historyOffsetRef.current = page.first_turn;
          setHistoryCursor(page.next_cursor);
          setEntries((previous) => [
            ...entriesForTranscriptPage(page, historyPreviewsRef.current),
            ...previous,
          ]);
          requestAnimationFrame(() => {
            if (pane) {
              pane.scrollTop = beforeTop + pane.scrollHeight - beforeHeight;
              readingAnchorRef.current = pane.scrollTop;
            }
          });
        }
        break;
      }

      case 'queue_updated': {
        setRecoveryBySession((prev) => ({
          ...prev,
          [msg.session_id]: msg.items.filter((item) => item.recovery),
        }));
        setQueuedForSession(msg.session_id, () =>
          msg.items
            .filter((item) => !item.recovery)
            .map((item) => ({
              id: item.id,
              text: item.text,
              images: item.images,
              fingerprint: item.fingerprint,
              steering: item.dispatching && !item.error,
              error: item.error ?? undefined,
            })),
        );
        break;
      }

      case 'queue_delivery': {
        if (msg.session_id === sessionIdRef.current)
          sendNow(msg.item.text, msg.item.images, false, msg.item.id);
        break;
      }

      case 'ready': {
        if (msg.session_activity) updateSessionActivity({ snapshot: msg.session_activity });
        if (msg.title) {
          activityTitlesRef.current.set(msg.session_id, msg.title);
          if (activityTitlesRef.current.size > 1000)
            activityTitlesRef.current.delete(activityTitlesRef.current.keys().next().value!);
        }
        setRecoveryBySession((prev) => ({
          ...prev,
          [msg.session_id]: (msg.queued_inputs ?? []).filter((item) => item.recovery),
        }));
        setQueuedForSession(msg.session_id, () =>
          (msg.queued_inputs ?? [])
            .filter((item) => !item.recovery)
            .map((item) => ({
              id: item.id,
              text: item.text,
              images: item.images,
              fingerprint: item.fingerprint,
              steering: item.dispatching && !item.error,
              error: item.error ?? undefined,
            })),
        );
        pendingSteerRef.current.delete(msg.session_id);
        if (msg.session_id === sessionIdRef.current && !followRef.current)
          readingAnchorRef.current = paneRef.current?.scrollTop ?? null;
        else if (msg.session_id !== sessionIdRef.current) {
          readingAnchorRef.current = null;
          followRef.current =
            !hasTranscriptPosition(msg.session_id) && getBoolPref(PREF_KEYS.transcriptFollow, true);
        }
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
          const isAgent = msg.engine
            ? msg.engine.kind === 'agent'
            : !!(msg.agent_kind || msg.agent_configured);
          if (!isAgent) {
            getContextBreakdown()
              .then((v) => {
                if (sessionIdRef.current !== opened || v.source !== 'mira') return;
                setProviderContext(
                  (cur) =>
                    cur ?? { used: v.breakdown.total, window: v.window, compactAt: v.compact_at },
                );
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
              ? {
                  kind: 'agent',
                  driver: msg.agent_kind ?? msg.agent_configured,
                  display_name: msg.agent_kind ?? msg.agent_configured ?? 'agent',
                  status: msg.agent_kind ? 'ready' : 'idle',
                }
              : {
                  kind: 'provider',
                  instance: msg.instance ?? null,
                  display_name: msg.instance ?? 'Mira',
                  model: msg.model,
                  status: 'ready',
                }),
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
        setHistoryLoading(false);
        setHistoryError(null);
        historyPreviewsRef.current = msg.previews ?? {};
        setTurnDiffs(msg.turn_diffs ?? []);
        setHistoryCursor(msg.transcript_page?.next_cursor ?? null);
        historyOffsetRef.current = msg.transcript_page?.first_turn ?? 0;
        providerTurnCursorRef.current =
          msg.turns?.length ??
          msg.history.filter(
            (message) => message.role === 'user' && message.input_intent !== 'steer',
          ).length;
        const readyEntries = msg.transcript_page
          ? entriesForTranscriptPage(msg.transcript_page, msg.previews)
          : historyToEntries(msg.history, msg.previews);
        setEntries(
          (msg.runtime_requests ?? []).reduce(
            (entries, request) => upsertRuntimeQuestion(entries, request),
            msg.transcript_page
              ? readyEntries
              : interleaveReplay(msg.history, msg.previews, msg.agent_transcript ?? []),
          ),
        );
        // Seed subagent state from history so the ContextPanel shows agents
        // on session reload (live subagent_started frames don't replay).
        setSubagentState(() => {
          const seeded = new Map<string, SubagentStreamState>();
          for (const e of readyEntries) {
            if (e.kind !== 'tool' || e.call.function.name !== 'agent') continue;
            try {
              const args = JSON.parse(e.call.function.arguments) as {
                prompt?: string;
                type?: string;
              };
              seeded.set(e.call.id, {
                parentCallId: e.call.id,
                prompt: args.prompt ?? '',
                agentName: args.type ?? null,
                agentCategory: null,
                entries: [],
                done: true,
                pendingReview: null,
              });
            } catch {
              /* skip malformed args */
            }
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
        listSkills()
          .then(setSkills)
          .catch(() => setSkills([]));
        listCommands()
          .then(setCommands)
          .catch(() => setCommands([]));
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

      case 'steer_result': {
        const pending = pendingSteerRef.current.get(msg.session_id);
        if (pending?.id === msg.request_id) {
          pendingSteerRef.current.delete(msg.session_id);
          setQueuedForSession(msg.session_id, (items) =>
            msg.error
              ? items.map((item) =>
                  item.id === msg.request_id
                    ? { ...item, steering: false, error: msg.error ?? undefined }
                    : item,
                )
              : items.filter((item) => item.id !== msg.request_id),
          );
        }
        if (msg.session_id !== sessionIdRef.current) break;
        if (!busyRef.current) window.setTimeout(drainQueuedMessage, 0);
        setEntries((entries) => {
          const index = entries.findIndex(
            (entry) => entry.kind === 'msg' && entry.steerRequestId === msg.request_id,
          );
          if (!msg.message) return index < 0 ? entries : entries.filter((_, i) => i !== index);
          const entry: Entry = { kind: 'msg', msg: msg.message, steerRequestId: msg.request_id };
          return index < 0
            ? [...sealThought(entries), entry]
            : entries.map((previous, i) => (i === index ? entry : previous));
        });
        break;
      }

      case 'session_title_updated':
        activityTitlesRef.current.delete(msg.session_id);
        activityTitlesRef.current.set(msg.session_id, msg.title);
        if (activityTitlesRef.current.size > 1000)
          activityTitlesRef.current.delete(activityTitlesRef.current.keys().next().value!);
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
    }
  });
}
