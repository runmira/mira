import { startReview } from '../api';
import { type AskUserDecision } from '../components/AskUserCard';
import { isAgentRequest } from '../lib/agentRequest';
import {
  recordAskUserDecision,
  recordPlanDecision,
  updateSubagent,
  updateTool,
} from '../transcript/entries';
import type { ApprovalScope, PlanStep } from '../types';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useApprovalCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'wsRef'
    | 'setSecretRequests'
    | 'setEntries'
    | 'setSubagentState'
    | 'setReviewState'
    | 'setReviewPanelOpen'
    | 'entries'
    | 'setRuleEditorCallId'
  >,
) {
  const {
    wsRef,
    setSecretRequests,
    setEntries,
    setSubagentState,
    setReviewState,
    setReviewPanelOpen,
    entries,
    setRuleEditorCallId,
  } = context;

  /** Answer an agent's secret prompt. `null` declines. The value goes only
   *  to the server, which stores it privately; it is not kept here. */
  function replyToSecret(promptId: string, value: string | null) {
    wsRef.current?.send({
      type: 'prompt_response',
      prompt_id: promptId,
      kind: 'secret',
      value,
      cancelled: value == null,
    });
    setSecretRequests((prev) => prev.filter((r) => r.promptId !== promptId));
  }

  function replyToPlan(callId: string, approved: boolean, steps?: PlanStep[], note?: string) {
    wsRef.current?.send({
      type: 'prompt_response',
      prompt_id: callId,
      kind: 'plan',
      approved,
      steps,
      note,
    });
    // Record the decision locally so the card switches to its resolved state
    // immediately, without waiting for tool_end to round-trip.
    setEntries((prev) => recordPlanDecision(prev, callId, { approved, steps, note }));
  }

  /** Send answers (or a skip) for an `ask_user` prompt back to the
   *  server and flip the card into its resolved state locally. */
  function replyToAskUser(callId: string, decision: AskUserDecision) {
    if (decision.cancelled) {
      wsRef.current?.send({
        type: 'prompt_response',
        prompt_id: callId,
        kind: 'ask_user',
        answers: [],
        cancelled: true,
      });
    } else {
      wsRef.current?.send({
        type: 'prompt_response',
        prompt_id: callId,
        kind: 'ask_user',
        answers: decision.answers,
        cancelled: false,
      });
    }
    if (!callId.startsWith('async-'))
      setEntries((prev) => recordAskUserDecision(prev, callId, decision));
  }

  /** Answer a subagent's review-required prompt. Clears `pendingReview`
   *  locally so the SubagentPanel immediately drops the review card; the
   *  backend's tool_end will land shortly after with the final result. */
  function replyToSubagentReview(
    parentCallId: string,
    promptId: string,
    approved: boolean,
    note?: string,
  ) {
    wsRef.current?.send({
      type: 'prompt_response',
      prompt_id: promptId,
      kind: 'subagent_review',
      approved,
      note,
    });
    setSubagentState((prev) =>
      updateSubagent(prev, parentCallId, (s) => ({
        ...s,
        pendingReview: null,
      })),
    );
  }

  async function runReview(args: string) {
    // Kicks off the server run; the `review_started` frame that comes back
    // over WS opens the panel + wipes prior state, so we don't seed the
    // ReviewState here.
    try {
      await startReview({ range: args || undefined });
    } catch (e) {
      const text = (e as Error).message;
      // If the POST itself failed there's no run_id — synthesize a state
      // so the panel opens and shows the error rather than swallowing it.
      setReviewState({
        runId: 'local-error',
        status: 'Review failed',
        progressPct: null,
        findings: null,
        verdicts: [],
        error: text,
        done: true,
      });
      setReviewPanelOpen(true);
    }
  }

  /** Kick off a review of a remote GitHub PR. Diff comes from the REST API
   *  using the stored `GITHUB_TOKEN`; stage-2 verify is skipped because the
   *  files referenced by the diff live on GitHub, not in the session cwd. */
  async function runPrReview(owner: string, repo: string, number: number) {
    try {
      await startReview({ owner, repo, pr: number, no_verify: true });
    } catch (e) {
      const text = (e as Error).message;
      setReviewState({
        runId: 'local-error',
        status: 'Review failed',
        progressPct: null,
        findings: null,
        verdicts: [],
        error: text,
        done: true,
      });
      setReviewPanelOpen(true);
    }
  }

  function decideApproval(
    callId: string,
    allow: boolean,
    scope: ApprovalScope = 'once',
    rules?: string[],
  ) {
    const pendingCall = entries.find((entry) => entry.kind === 'tool' && entry.call.id === callId);
    if (
      allow &&
      scope === 'always' &&
      !rules &&
      pendingCall?.kind === 'tool' &&
      !isAgentRequest(pendingCall.call)
    ) {
      setRuleEditorCallId(callId);
      return;
    }
    wsRef.current?.send({ type: 'approve', call_id: callId, allow, scope, rules });
    if (rules) return; // Keep the editor visible if server validation rejects the rule.
    // An agent's request is only the question; the agent's own tool card
    // shows the call running. Keeping the request would leave a second
    // card that never finishes.
    const decided = entries.find((e) => e.kind === 'tool' && e.call.id === callId);
    if (decided && decided.kind === 'tool' && isAgentRequest(decided.call)) {
      setEntries((prev) => prev.filter((e) => !(e.kind === 'tool' && e.call.id === callId)));
      return;
    }
    setEntries((prev) =>
      updateTool(prev, callId, (t) => ({
        ...t,
        status: allow ? 'running' : 'denied',
      })),
    );
  }
  return {
    replyToSecret,
    replyToPlan,
    replyToAskUser,
    replyToSubagentReview,
    runReview,
    runPrReview,
    decideApproval,
  };
}
