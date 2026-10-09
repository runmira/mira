import * as agentTerminal from '../lib/agentTerminal';
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
import type { ServerMsg, UsageTotals } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useTranscriptEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'setTurnDiffs'
    | 'setEntries'
    | 'setThinking'
    | 'clearThinkingIdle'
    | 'scheduleThinkingIdle'
    | 'tokenBufRef'
    | 'tokenRafRef'
    | 'drainTokens'
    | 'pendingProposalsRef'
    | 'pendingAskUserRef'
    | 'busyRef'
    | 'setTasks'
    | 'setSidebarRefresh'
    | 'turnBaseRef'
    | 'usageRef'
    | 'setTurnUsage'
    | 'setBusy'
    | 'playPing'
    | 'refreshRepo'
    | 'setTurnTimings'
    | 'drainQueuedMessage'
    | 'setModel'
    | 'setMode'
    | 'setUsage'
    | 'setCacheMiss'
    | 'setProviderContext'
    | 'setRateLimit'
    | 'setGoal'
  >,
) {
  const {
    setTurnDiffs,
    setEntries,
    setThinking,
    clearThinkingIdle,
    scheduleThinkingIdle,
    tokenBufRef,
    tokenRafRef,
    drainTokens,
    pendingProposalsRef,
    pendingAskUserRef,
    busyRef,
    setTasks,
    setSidebarRefresh,
    turnBaseRef,
    usageRef,
    setTurnUsage,
    setBusy,
    playPing,
    refreshRepo,
    setTurnTimings,
    drainQueuedMessage,
    setModel,
    setMode,
    setUsage,
    setCacheMiss,
    setProviderContext,
    setRateLimit,
    setGoal,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    switch (msg.type) {
      case 'turn_diffs':
        setTurnDiffs(msg.summaries);
        break;

      case 'stream_activity':
        setEntries((previous) => [
          ...previous,
          { kind: 'activity', activityKind: msg.kind, title: msg.title, detail: msg.detail },
        ]);
        break;

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

      case 'tool_start':
        setThinking(false);
        clearThinkingIdle();
        agentTerminal.commandStart(
          msg.call.id,
          msg.call.function.name,
          msg.call.function.arguments,
        );
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

      case 'model_changed':
        setModel(msg.model);
        break;

      case 'mode_changed':
        setMode(msg.mode);
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

      case 'compacting':
        // One card per compaction: a running one already showing (the agent
        // path adds its own) isn't doubled.
        setEntries((prev) =>
          prev.some((e) => e.kind === 'compact' && e.state === 'running')
            ? prev
            : [
                ...prev,
                {
                  kind: 'compact',
                  state: 'running',
                  trigger: msg.trigger === 'auto' ? 'auto' : 'manual',
                  startedAt: Date.now(),
                  summarized: null,
                  summary: null,
                  tokensBefore: msg.tokens_before ?? null,
                },
              ],
        );
        break;

      case 'compacted':
        setEntries((prev) =>
          finishCompaction(prev, {
            state: 'done',
            summarized: msg.messages_removed,
            tokensBefore: msg.tokens_before ?? undefined,
            tokensAfter: msg.tokens_after ?? null,
          }),
        );
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
    }
  });
}
