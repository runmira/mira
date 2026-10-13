import { appendAgentProgress } from '../lib/agentProgress';
import * as agentTerminal from '../lib/agentTerminal';
import { applyResolution } from '../state/requestReducer';
import { useAppRuntime } from '../state/RuntimeProvider';
import {
appendProgressLine,
appendReasoning,
applyTaskResult,
attachAskUserProposal,
attachPlanProposal,
attachToolResult,
finishCompaction,
sealThought,
settleTools,
stampLastTurn,
upsertToolStart,
type GoalEntry,
} from '../transcript/entries';
import type { ServerMsg,UsageTotals } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useTranscriptEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'recordTurnDiffs'
    | 'updateTranscript'
    | 'streams'
    | 'reportThinking'
    | 'clearThinkingIdle'
    | 'scheduleThinkingIdle'
    | 'pendingProposalsRef'
    | 'pendingAskUserRef'
    | 'isTurnRunning'
    | 'recordTasks'
    | 'setSidebarRefresh'
    | 'turnBaseRef'
    | 'getUsage'
    | 'recordTurnUsage'
    | 'reportTurnRunning'
    | 'playPing'
    | 'refreshRepo'
    | 'recordTurnTimings'
    | 'recordModel'
    | 'recordMode'
    | 'recordUsage'
    | 'recordCacheMiss'
    | 'recordProviderContext'
    | 'recordRateLimit'
    | 'recordGoal'
  >,
) {
  const appRuntime = useAppRuntime();
  const {
    recordTurnDiffs,
    updateTranscript,
    streams,
    reportThinking,
    clearThinkingIdle,
    scheduleThinkingIdle,
    pendingProposalsRef,
    pendingAskUserRef,
    isTurnRunning,
    recordTasks,
    setSidebarRefresh,
    turnBaseRef,
    getUsage,
    recordTurnUsage,
    reportTurnRunning,
    playPing,
    refreshRepo,
    recordTurnTimings,
    recordModel,
    recordMode,
    recordUsage,
    recordCacheMiss,
    recordProviderContext,
    recordRateLimit,
    recordGoal,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    const receivedAt = Date.now();
    switch (msg.type) {
      case 'turn_diffs':
        recordTurnDiffs(msg.summaries);
        break;

      case 'stream_activity':
        updateTranscript((previous) => appendAgentProgress(previous, msg));
        break;

      case 'reasoning':
        // The live thought block is its own "working" signal.
        reportThinking(false);
        clearThinkingIdle();
        updateTranscript((prev) => appendReasoning(prev, msg.text, receivedAt));
        break;

      case 'token':
        reportThinking(false);
        // Text is streaming — hide the indicator, but arm a short idle
        // timer so a silent gap (typically the model emitting tool-call
        // deltas after its assistant text ends) brings the indicator
        // back before `tool_start` finally fires.
        scheduleThinkingIdle();
        streams.queueToken(msg.text);
        break;

      case 'tool_start':
        reportThinking(false);
        clearThinkingIdle();
        agentTerminal.commandStart(
          msg.call.id,
          msg.call.function.name,
          msg.call.function.arguments,
        );
        {
          const planQ = pendingProposalsRef.current.get(msg.call.id);
          const askQ = pendingAskUserRef.current.get(msg.call.id);
          const resolution = appRuntime.sessionServices.resolutions().get(msg.call.id);
          pendingProposalsRef.current.delete(msg.call.id);
          pendingAskUserRef.current.delete(msg.call.id);
          updateTranscript(prev => {
            let next = upsertToolStart(prev, msg.call, receivedAt);
            if (planQ) next = attachPlanProposal(next, msg.call.id, planQ);
            if (askQ) next = attachAskUserProposal(next, msg.call.id, askQ);
            return resolution ? applyResolution(next, resolution) : next;
          });
        }
        break;

      case 'tool_end':
        appRuntime.requests.resolve(msg.result.call_id);
        // The model usually starts thinking again after a tool result comes
        // back before the next text token arrives — while the turn lasts.
        if (isTurnRunning()) reportThinking(true);
        updateTranscript((prev) => attachToolResult(prev, msg.result));
        agentTerminal.commandEnd(msg.result.call_id, msg.result.content, !!msg.result.is_error);
        // Piggyback: task_* tools ship the current task or full list in
        // `data`. Upsert so the Plan panel stays live without another
        // round-trip.
        recordTasks((prev) => applyTaskResult(prev, msg.result));
        break;

      case 'turn_complete':
        // A turn ended (assistant round complete). More may follow if there
        // were tool calls; if not, `done` will clear us right after. A late
        // one after Stop must not set the spinner going again.
        if (isTurnRunning()) reportThinking(true);
        // Belt-and-suspenders sidebar refresh — the onSend-triggered
        // refetch can race the harness's first checkpoint on a very
        // fresh session; this fires once the first assistant round has
        // definitively landed on disk.
        setSidebarRefresh((n) => n + 1);
        break;

      case 'done': {
        if (!appRuntime.acceptTerminal()) break;
        appRuntime.updateSessionData('activeInputId', null);
        const wasRunning = isTurnRunning();
        const started = turnBaseRef.current;
        const now = getUsage();
        if (started && now) {
          const d: UsageTotals = {
            prompt_tokens: now.prompt_tokens - started.base.prompt_tokens,
            completion_tokens: now.completion_tokens - started.base.completion_tokens,
            cached_input_tokens: now.cached_input_tokens - started.base.cached_input_tokens,
            rounds: now.rounds - started.base.rounds,
          };
          if (d.prompt_tokens + d.completion_tokens > 0) {
            recordTurnUsage((prev) => new Map(prev).set(started.turn, d));
          }
        }
        turnBaseRef.current = null;
        reportTurnRunning(false);
        reportThinking(false);
        clearThinkingIdle();
        updateTranscript((prev) => settleTools(sealThought(prev, receivedAt)));
        if (wasRunning) playPing();
        // Refresh git status, session diff, and branch PR after each turn.
        refreshRepo();
        // Close out the most recent turn's timing.
        recordTurnTimings((prev) => stampLastTurn(prev, receivedAt));
        setSidebarRefresh((n) => n + 1);
        break;
      }

      case 'tool_progress':
        updateTranscript((prev) => appendProgressLine(prev, msg.call_id, msg.line));
        agentTerminal.outputLine(msg.call_id, msg.line);
        break;

      case 'tool_preview':
        // The harness computes a diff preview for edit/write calls
        // just before execution and fires this frame regardless of
        // approval mode. Attach it to the matching in-flight tool
        // entry so auto-allowed writes get the same rich diff view
        // that approval-gated ones already do via ApprovalRequest.
        updateTranscript((prev) =>
          prev.map((e) => {
            if (e.kind !== 'tool' || e.call.id !== msg.call_id) return e;
            return { ...e, preview: msg.preview };
          }),
        );
        break;

      case 'model_changed':
        recordModel(msg.model);
        break;

      case 'mode_changed':
        recordMode(msg.mode);
        break;

      case 'usage':
        recordUsage(msg.totals);

        recordCacheMiss(msg.cache_miss ?? null);
        if (msg.context_window) {
          recordProviderContext({
            used: msg.round.prompt_tokens + msg.round.completion_tokens,
            window: msg.context_window,
            compactAt: msg.compact_at ?? null,
          });
        }
        break;

      case 'rate_limit':
        recordRateLimit({ rate_limit: msg.rate_limit, summary: msg.summary, at: receivedAt });
        break;

      case 'compacting':
        // One card per compaction: a running one already showing (the agent
        // path adds its own) isn't doubled.
        updateTranscript((prev) =>
          prev.some((e) => e.kind === 'compact' && e.state === 'running')
            ? prev
            : [
                ...prev,
                {
                  kind: 'compact',
                  state: 'running',
                  trigger: msg.trigger === 'auto' ? 'auto' : 'manual',
                  startedAt: receivedAt,
                  summarized: null,
                  summary: null,
                  tokensBefore: msg.tokens_before ?? null,
                },
              ],
        );
        break;

      case 'compacted':
        updateTranscript((prev) =>
          finishCompaction(prev, {
            state: 'done',
            summarized: msg.messages_removed,
            tokensBefore: msg.tokens_before ?? undefined,
            tokensAfter: msg.tokens_after ?? null,
          }, receivedAt),
        );
        break;

      case 'compaction_failed':
        updateTranscript((prev) => finishCompaction(prev, { state: 'failed', error: msg.error }, receivedAt));
        break;

      case 'goal_set':
        recordGoal(msg.goal);
        updateTranscript((prev) => [
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
        recordGoal(null);
        updateTranscript((prev) => [
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
        recordGoal((prev) =>
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
          updateTranscript((prev) => [
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
        recordGoal((prev) =>
          prev ? { ...prev, status: d.status, last_reason: d.reason ?? prev.last_reason } : prev,
        );
        updateTranscript((prev) => [
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
    }
  });
}
