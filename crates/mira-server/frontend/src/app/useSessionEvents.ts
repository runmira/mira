import { replayExternalStreams, projectExternalWork, upsertExternalWork } from '../lib/externalSubagents';
import * as agentTerminal from '../lib/agentTerminal';
import { getBoolPref,PREF_KEYS } from '../lib/prefs';
import { hasTranscriptPosition } from '../lib/transcriptPosition';
import { reconcileQueue } from '../state/queueReconciliation';
import { useAppRuntime } from '../state/RuntimeProvider';
import {
rebuildTurnModels,
rebuildTurnTimings,
rebuildTurnUsage,
sealThought,
upsertRuntimeQuestion,
type Entry,
} from '../transcript/entries';
import { entriesForTranscriptPage,historyToEntries,interleaveReplay } from '../transcript/replay';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useSessionEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'queueMutationsRef'
    | 'updateSessionActivity'
    | 'getSessionId'
    | 'historyRequestRef'
    | 'recordHistoryLoading'
    | 'recordHistoryError'
    | 'paneRef'
    | 'followRef'
    | 'historyOffsetRef'
    | 'recordHistoryCursor'
    | 'updateTranscript'
    | 'historyPreviewsRef'
    | 'readingAnchorRef'
    | 'recordQueueRecovery'
    | 'updateSessionQueue'
    | 'sendNow'
    | 'activityTitlesRef'
    | 'pendingSteerRef'
    | 'setSessionCommitted'
    | 'approvalStarted'
    | 'recordApprovalRules'
    | 'setRuleEditorCallId'
    | 'wsRef'
    | 'recordSessionTitle'
    | 'recordProviderContext'
    | 'recordAgentUsage'
    | 'setRestoreNote'
    | 'setRestoreAsk'
    | 'recordModel'
    | 'recordMode'
    | 'recordCwd'
    | 'applyEngine'
    | 'onMessage'
    | 'recordTurnDiffs'
    | 'providerTurnCursorRef'
    | 'updateSubagentProjection'
    | 'recordTurnTimings'
    | 'recordTurnUsage'
    | 'recordTurnModels'
    | 'setExpandedTurns'
    | 'recordUsage'
    | 'recordCacheMiss'
    | 'recordRateLimit'
    | 'recordTasks'
    | 'recordGoal'
    | 'turnCommands'
    | 'clearThinkingIdle'
    | 'setSidebarRefresh'
    | 'loadEngines'
    | 'recordEnvironmentTransition'
    | 'refreshRepo'
    | 'setMainView'
    | 'attachFromRef'
  >,
) {
  const appRuntime = useAppRuntime();
  const {
    queueMutationsRef,
    updateSessionActivity,
    getSessionId,
    historyRequestRef,
    recordHistoryLoading,
    recordHistoryError,
    paneRef,
    followRef,
    historyOffsetRef,
    recordHistoryCursor,
    updateTranscript,
    historyPreviewsRef,
    readingAnchorRef,
    recordQueueRecovery,
    updateSessionQueue,
    sendNow,
    activityTitlesRef,
    pendingSteerRef,
    setSessionCommitted,
    approvalStarted,
    recordApprovalRules,
    setRuleEditorCallId,
    wsRef,
    recordSessionTitle,
    recordProviderContext,
    recordAgentUsage,
    setRestoreNote,
    setRestoreAsk,
    recordModel,
    recordMode,
    recordCwd,
    applyEngine,
    onMessage,
    recordTurnDiffs,
    providerTurnCursorRef,
    updateSubagentProjection,
    recordTurnTimings,
    recordTurnUsage,
    recordTurnModels,
    setExpandedTurns,
    recordUsage,
    recordCacheMiss,
    recordRateLimit,
    recordTasks,
    recordGoal,
    turnCommands,
    clearThinkingIdle,
    setSidebarRefresh,
    loadEngines,
    recordEnvironmentTransition,
    refreshRepo,
    setMainView,
    attachFromRef,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    const receivedAt = Date.now();
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
        if (msg.session_id !== getSessionId() || msg.request_id !== historyRequestRef.current ||
          appRuntime.sessionServices.ref('historyGeneration').current !== appRuntime.sessionState.get().attachment)
          break;
        historyRequestRef.current = null;
        recordHistoryLoading(false);
        recordHistoryError(msg.error);
        if (msg.page) {
          const page = msg.page;
          followRef.current = false;
          historyOffsetRef.current = page.first_turn;
          recordHistoryCursor(page.next_cursor);
          updateTranscript((previous) => [
            ...entriesForTranscriptPage(page, historyPreviewsRef.current),
            ...previous,
          ]);
        }
        break;
      }

      case 'queue_updated': {
        if (msg.session_id === getSessionId() && msg.items.some(item =>
          item.id === appRuntime.readSession().activeInputId && !!item.error)) {
          appRuntime.turnCommands.reportTurnRunning(false);
          appRuntime.updateSessionData('activeInputId', null);
        }
        for (const item of msg.items) appRuntime.effect(() => appRuntime.drafts.confirmDelivery(item.id));
        recordQueueRecovery((prev) => ({
          ...prev,
          [msg.session_id]: msg.items.filter((item) => item.recovery),
        }));
        updateSessionQueue(msg.session_id, previous => reconcileQueue(previous,
          msg.items
            .filter((item) => !item.recovery && !item.dispatching && !item.delivered)
            .map((item) => ({
              id: item.id,
              text: item.text,
              images: item.images,
              fingerprint: item.fingerprint,
              steering: item.dispatching && !item.error,
              error: item.error ?? undefined,
            })), new Set(msg.items.filter(item => item.dispatching || item.delivered).map(item => item.id)))
        );
        break;
      }

      case 'queue_delivery': {
        appRuntime.effect(() => appRuntime.drafts.confirmDelivery(msg.item.id));
        if (msg.session_id === getSessionId())
          sendNow(msg.item.text, msg.item.images, false, msg.item.id);
        break;
      }

      case 'ready': {
        // Which chat we came from: `attachSession` records it before
        // `suspend()` flips the selection, because by the time this frame
        // arrives `getSessionId()` already reports the new chat. Falls back
        // to the current selection for readies that follow no attach (a
        // reconnect replays the same chat, whose scroll position must stay).
        const previousSession = attachFromRef.current ?? getSessionId();
        attachFromRef.current = null;
        for (const item of msg.queued_inputs ?? []) appRuntime.effect(() => appRuntime.drafts.confirmDelivery(item.id));
        for (const message of msg.history) if (message.input_id) appRuntime.effect(() => appRuntime.drafts.confirmDelivery(message.input_id!));
        appRuntime.resources.cancelContext();
        turnCommands.hydrate(msg.session_id, !!msg.running, msg.runtime_work ?? []);
        appRuntime.reconcileTerminals(!!msg.running, msg.turns);
        if (msg.session_activity) updateSessionActivity({ snapshot: msg.session_activity });
        if (msg.title) {
          activityTitlesRef.current.set(msg.session_id, msg.title);
          if (activityTitlesRef.current.size > 1000)
            activityTitlesRef.current.delete(activityTitlesRef.current.keys().next().value!);
        }
        recordQueueRecovery((prev) => ({
          ...prev,
          [msg.session_id]: (msg.queued_inputs ?? []).filter((item) => item.recovery),
        }));
        updateSessionQueue(msg.session_id, previous => reconcileQueue(previous,
          (msg.queued_inputs ?? [])
            .filter((item) => !item.recovery && !item.dispatching && !item.delivered)
            .map((item) => ({
              id: item.id,
              text: item.text,
              images: item.images,
              fingerprint: item.fingerprint,
              steering: item.dispatching && !item.error,
              error: item.error ?? undefined,
            })), new Set(msg.history.flatMap(message => message.input_id ? [message.input_id] : [])))
        );
        pendingSteerRef.current.delete(msg.session_id);
        if (msg.session_id === previousSession && !followRef.current)
          readingAnchorRef.current = paneRef.current?.scrollTop ?? null;
        else if (msg.session_id !== previousSession) {
          readingAnchorRef.current = null;
          followRef.current =
            !hasTranscriptPosition(JSON.stringify(appRuntime.resourceKey('scroll', msg.session_id))) && getBoolPref(PREF_KEYS.transcriptFollow, true);
        }
        agentTerminal.reset();
        setSessionCommitted(false);
        approvalStarted.current = {};
        recordApprovalRules({});
        setRuleEditorCallId(null);
        wsRef.current?.setSession(msg.session_id);

        recordSessionTitle(msg.title ?? null);
        // Context is per chat; the agent's plan limits are per account and
        // stay. The session's own usage frames (replayed below) restore it.
        recordProviderContext(null);
        recordAgentUsage(null);
        setRestoreNote(null);
        setRestoreAsk(null);
        recordModel(msg.model);
        recordMode(msg.mode);
        recordCwd(msg.cwd);
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
        recordHistoryLoading(false);
        recordHistoryError(null);
        historyPreviewsRef.current = msg.previews ?? {};
        recordTurnDiffs(msg.turn_diffs ?? []);
        recordHistoryCursor(msg.transcript_page?.next_cursor ?? null);
        historyOffsetRef.current = msg.transcript_page?.first_turn ?? 0;
        providerTurnCursorRef.current =
          msg.turns?.length ??
          msg.history.filter(
            (message) => message.role === 'user' && message.input_intent !== 'steer',
          ).length;
        const readyEntries = msg.transcript_page
          ? entriesForTranscriptPage(msg.transcript_page, msg.previews)
          : historyToEntries(msg.history, msg.previews);
        // The Ready activity snapshot is authoritative for resumed children.
        const resumedWork = (msg.runtime_work ?? []).map(work => ({ ...work, subagent: work.kind === 'subagent'
          ? { name: null, prompt: null, result: null, ...work.subagent, reopen: true } : work.subagent }));
        updateTranscript(
          (msg.runtime_requests ?? []).reduce(
            (entries, request) => upsertRuntimeQuestion(entries, request),
            resumedWork.reduce(upsertExternalWork,
              msg.transcript_page ? readyEntries : interleaveReplay(msg.history, msg.previews, msg.agent_transcript ?? [])),
          ),
        );
        // Seed subagent state from history so the ContextPanel shows agents
        // on session reload (live subagent_started frames don't replay).
        updateSubagentProjection(previous => {
          const replayLines = msg.transcript_page ? msg.transcript_page.items.flatMap(item => item.line ? [item.line] : []) : msg.agent_transcript ?? [];
          const seeded = resumedWork.reduce(projectExternalWork, replayExternalStreams(replayLines));
          for (const e of readyEntries) {
            if (e.kind !== 'tool' || e.call.function.name !== 'agent') continue;
            if (seeded.has(e.call.id)) continue;
            try {
              const args = JSON.parse(e.call.function.arguments) as {
                prompt?: string;
                type?: string;
              };
              if (previous.has(e.call.id)) { seeded.set(e.call.id, previous.get(e.call.id)!); continue; }
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
          for (const [id, stream] of previous) if ((!stream.done || stream.pendingReview) && !seeded.has(id)) seeded.set(id, stream);
          return seeded;
        });
        // Server-persisted turn timing is aligned with user-message order
        // (turn 0 = first user msg). Rebuild the local Map so "Worked for"
        // chips render on reloaded transcripts.
        recordTurnTimings(rebuildTurnTimings(msg.turns ?? []));
        // The reply hover row's tokens and cost, for turns from before this
        // page loaded — the server keeps them per turn.
        recordTurnUsage(rebuildTurnUsage(msg.turns ?? []));
        recordTurnModels(rebuildTurnModels(msg.turns ?? []));
        setExpandedTurns(new Set());
        recordUsage(msg.usage ?? null);
        recordCacheMiss(null);
        recordRateLimit(null);
        recordTasks(msg.tasks ?? []);
        recordGoal(msg.goal ?? null);
        // Opened mid-turn: show Stop and the working indicator until the
        // turn's end arrives, instead of an idle composer over a live reply.
        clearThinkingIdle();
        setSidebarRefresh((n) => n + 1);
        // Refresh the skill roster on every Ready — a cwd swap may
        // change the project tier (~/.mira vs. <cwd>/.mira). Silent on
        // failure; the palette just shows built-in commands.
        for (const resource of ['skills', 'commands'] as const) void appRuntime.resources.refresh(resource).catch(() => {});
        loadEngines();
        // Each session (and worktree) has its own environment; ask for it.
        recordEnvironmentTransition(null);
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
          updateSessionQueue(msg.session_id, (items) =>
            msg.error
              ? items.map((item) =>
                  item.id === msg.request_id
                    ? { ...item, steering: false, error: msg.error ?? undefined }
                    : item,
                )
              : items.filter((item) => item.id !== msg.request_id),
          );
        }
        if (msg.session_id !== getSessionId()) break;
        updateTranscript((entries) => {
          const index = entries.findIndex(
            (entry) => entry.kind === 'msg' && entry.steerRequestId === msg.request_id,
          );
          if (!msg.message) return index < 0 ? entries : entries.filter((_, i) => i !== index);
          const entry: Entry = { kind: 'msg', msg: msg.message, steerRequestId: msg.request_id };
          return index < 0
            ? [...sealThought(entries, receivedAt), entry]
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
        if (msg.session_id === getSessionId()) recordSessionTitle(msg.title);
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
