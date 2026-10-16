import { type AskUserDecision } from '../components/AskUserCard';
import { isAgentRequest } from '../lib/agentRequest';
import { callForAttention } from '../lib/attention';
import { useAppRuntime } from '../state/RuntimeProvider';
import {
attachAskUserProposal,
attachPlanProposal,
recordAskUserDecision,
recordPlanDecision,
sealThought,
upsertRuntimeQuestion,
} from '../transcript/entries';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
/** Secret requests already announced; a re-sent one doesn't ping again. */
const announcedSecrets = new Set<string>();

export function useApprovalEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'approvalStarted'
    | 'recordApprovalRules'
    | 'updateTranscript'
    | 'wsRef'
    | 'reportThinking'
    | 'clearThinkingIdle'
    | 'playPing'
    | 'chatTitleRef'
    | 'pendingProposalsRef'
    | 'recordSecretRequests'
    | 'pendingAskUserRef'
  >,
) {
  const appRuntime = useAppRuntime();
  const {
    approvalStarted,
    recordApprovalRules,
    updateTranscript,
    wsRef,
    reportThinking,
    clearThinkingIdle,
    playPing,
    chatTitleRef,
    pendingProposalsRef,
    recordSecretRequests,
    pendingAskUserRef,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    const receivedAt = Date.now();
    switch (msg.type) {
      case 'approval_rules':
        if (msg.error) appRuntime.requests.fail(msg.call_id, msg.error);
        if (approvalStarted.current[msg.call_id] === undefined) break;
        recordApprovalRules((previous) => ({
          ...previous,
          [msg.call_id]: { rules: msg.rules, error: msg.error },
        }));
        break;

      case 'approval_resolved':
        appRuntime.requests.resolve(msg.call_id);
        appRuntime.sessionServices.rememberResolution(msg.call_id, msg);
        updateTranscript((previous) =>
          previous.flatMap((entry) => {
            if (entry.kind !== 'tool' || entry.call.id !== msg.call_id) return [entry];
            if (isAgentRequest(entry.call)) return [];
            if (entry.status !== 'pending') return [entry];
            return [{ ...entry, status: msg.allow ? ('running' as const) : ('denied' as const) }];
          }),
        );
        break;

      case 'approval_request':
        if (appRuntime.requests.isResolved(msg.call.id) || appRuntime.readSession().entries.some(entry =>
          entry.kind === 'tool' && entry.call.id === msg.call.id && (entry.status === 'pending' || !!entry.result || entry.status === 'denied'))) break;
        approvalStarted.current[msg.call.id] = receivedAt;
        wsRef.current?.send({ type: 'approval_rules', call_id: msg.call.id });
        reportThinking(false);
        clearThinkingIdle();
        playPing();
        appRuntime.effect(() => callForAttention('approval', chatTitleRef.current));
        updateTranscript((prev) => [
          ...sealThought(prev, receivedAt),
          {
            kind: 'tool',
            call: msg.call,
            preview: msg.preview ?? null,
            needs: msg.needs ?? [],
            status: 'pending',
            result: null,
          },
        ]);
        break;

      case 'plan_request':
        if (appRuntime.requests.isResolved(msg.prompt_id) || appRuntime.readSession().entries.some(entry =>
          entry.kind === 'tool' && entry.call.id === msg.prompt_id && (entry.plan?.decision != null || entry.askUser?.decision != null))) break;
        // Server reuses the tool call id as the prompt id. Attach immediately
        // if the tool_start already arrived; otherwise stash the proposal so
        // tool_start can pick it up when it lands (see the tool_start case).
        // A waiting plan is re-sent whenever the chat is opened: only the
        // first sighting pings.
        if (!appRuntime.readSession().entries.some(e => e.kind === 'tool' && e.call.id === msg.prompt_id && e.plan?.decision === null)) {
          playPing();
          appRuntime.effect(() => callForAttention('plan', chatTitleRef.current));
        }
        if (!appRuntime.readSession().entries.some(e => e.kind === 'tool' && e.call.id === msg.prompt_id))
          pendingProposalsRef.current.set(msg.prompt_id, msg.plan);
        else updateTranscript(prev => attachPlanProposal(prev, msg.prompt_id, msg.plan));
        break;

      case 'prompt_resolved':
        appRuntime.requests.resolve(msg.prompt_id);
        appRuntime.sessionServices.rememberResolution(msg.prompt_id, msg);
        // Answered in another window (or tab, or device): close this copy
        // with the same answer instead of leaving it waiting.
        if (msg.kind === 'secret') {
          recordSecretRequests((prev) => prev.filter((r) => r.promptId !== msg.prompt_id));
        } else if (msg.kind === 'subagent_review') {
          appRuntime.updateSessionData('subagentState', previous => new Map([...previous].map(([id, stream]) =>
            [id, stream.pendingReview?.promptId === msg.prompt_id ? { ...stream, pendingReview: null } : stream])));
        } else if (msg.kind === 'ask_user') {
          const decision: AskUserDecision = msg.cancelled
            ? { cancelled: true }
            : { cancelled: false, answers: msg.answers ?? [] };
          updateTranscript((prev) => recordAskUserDecision(prev, msg.prompt_id, decision));
        } else if (msg.kind === 'plan') {
          updateTranscript((prev) =>
            recordPlanDecision(prev, msg.prompt_id, {
              approved: !!msg.approved,
              steps: msg.steps ?? undefined,
              note: msg.note ?? undefined,
            }),
          );
        }
        break;

      case 'runtime_request_updated':
        if (msg.request.response || ['delivered', 'cancelled'].includes(msg.request.delivery)) appRuntime.requests.resolve(msg.request.id);
        else if (appRuntime.requests.isResolved(msg.request.id)) break;
        updateTranscript((prev) => upsertRuntimeQuestion(prev, msg.request));
        break;

      case 'ask_user_request':
        if (appRuntime.requests.isResolved(msg.prompt_id) || appRuntime.readSession().entries.some(entry =>
          entry.kind === 'tool' && entry.call.id === msg.prompt_id && (entry.plan?.decision != null || entry.askUser?.decision != null))) break;
        // Same race-guard pattern as plan_request — attach immediately when
        // the tool_start already landed; stash otherwise. Re-sent while it
        // waits (each time the chat is opened): only the first one pings.
        if (!appRuntime.readSession().entries.some(e => e.kind === 'tool' && e.call.id === msg.prompt_id && e.askUser?.decision === null)) {
          playPing();
          appRuntime.effect(() => callForAttention('question', chatTitleRef.current));
        }
        if (!appRuntime.readSession().entries.some(e => e.kind === 'tool' && e.call.id === msg.prompt_id))
          pendingAskUserRef.current.set(msg.prompt_id, msg.proposal);
        else updateTranscript(prev => attachAskUserProposal(prev, msg.prompt_id, msg.proposal));
        break;

      case 'secret_request':
        if (appRuntime.requests.isResolved(msg.prompt_id) || appRuntime.readSession().entries.some(entry =>
          entry.kind === 'tool' && entry.call.id === msg.prompt_id && (entry.plan?.decision != null || entry.askUser?.decision != null))) break;
        // Re-sent while it waits (each time the chat is opened): ping once.
        if (!announcedSecrets.has(msg.prompt_id)) {
          announcedSecrets.add(msg.prompt_id);
          if (announcedSecrets.size > 200) announcedSecrets.delete(announcedSecrets.values().next().value!);
          playPing();
          appRuntime.effect(() => callForAttention('question', chatTitleRef.current));
        }
        recordSecretRequests((prev) =>
          prev.some((r) => r.promptId === msg.prompt_id)
            ? prev
            : [
                ...prev,
                { promptId: msg.prompt_id, name: msg.name, reason: msg.reason, dotenv: msg.dotenv },
              ],
        );
        break;
    }
  });
}
