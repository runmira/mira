import { startReview } from '../api';
import { type AskUserDecision } from '../components/AskUserCard';
import { isAgentRequest } from '../lib/agentRequest';
import { useAppRuntime } from '../state/RuntimeProvider';
import type { ApprovalScope,PlanStep } from '../types';

import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function useApprovalCommands(
  context: Pick<
    WorkspaceRuntime,
    | 'wsRef'
    | 'setReviewState'
    | 'setReviewPanelOpen'
    | 'setRuleEditorCallId'
  >,
) {
  const appRuntime = useAppRuntime();
  const submit = (id: string, message: import('../types').ClientMsg) =>
    appRuntime.requests.submit(id, message, frame => wsRef.current?.sendImmediate(frame) ?? false);
  const {
    wsRef,
    setReviewState,
    setReviewPanelOpen,
    setRuleEditorCallId,
  } = context;

  /** Answer an agent's secret prompt. `null` declines. The value goes only
   *  to the server, which stores it privately; it is not kept here. */
  function replyToSecret(promptId: string, value: string | null) {
    submit(promptId, {
      type: 'prompt_response',
      prompt_id: promptId,
      kind: 'secret',
      value,
      cancelled: value == null,
    });
  }

  function replyToPlan(callId: string, approved: boolean, steps?: PlanStep[], note?: string) {
    submit(callId, {
      type: 'prompt_response',
      prompt_id: callId,
      kind: 'plan',
      approved,
      steps,
      note,
    });
    // Record the decision locally so the card switches to its resolved state
    // immediately, without waiting for tool_end to round-trip.

  }

  /** Send answers (or a skip) for an `ask_user` prompt back to the
   *  server and flip the card into its resolved state locally. */
  function replyToAskUser(callId: string, decision: AskUserDecision) {
    if (decision.cancelled) {
      submit(callId, {
        type: 'prompt_response',
        prompt_id: callId,
        kind: 'ask_user',
        answers: [],
        cancelled: true,
      });
    } else {
      submit(callId, {
        type: 'prompt_response',
        prompt_id: callId,
        kind: 'ask_user',
        answers: decision.answers,
        cancelled: false,
      });
    }

  }

  /** Answer a subagent's review-required prompt. Clears `pendingReview`
   *  locally so the SubagentPanel immediately drops the review card; the
   *  backend's tool_end will land shortly after with the final result. */
  function replyToSubagentReview(
    _parentCallId: string,
    promptId: string,
    approved: boolean,
    note?: string,
  ) {
    submit(promptId, {
      type: 'prompt_response',
      prompt_id: promptId,
      kind: 'subagent_review',
      approved,
      note,
    });

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
    const pendingCall = appRuntime.readSession().entries.find((entry) => entry.kind === 'tool' && entry.call.id === callId);
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
    submit(callId, { type: 'approve', call_id: callId, allow, scope, rules });
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
