import { type AskUserDecision } from '../components/AskUserCard';
import { isAgentRequest } from '../lib/agentRequest';
import { callForAttention } from '../lib/attention';
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
export function useApprovalEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'approvalStarted'
    | 'setApprovalRules'
    | 'setEntries'
    | 'wsRef'
    | 'setThinking'
    | 'clearThinkingIdle'
    | 'playPing'
    | 'chatTitleRef'
    | 'pendingProposalsRef'
    | 'setSecretRequests'
    | 'pendingAskUserRef'
  >,
) {
  const {
    approvalStarted,
    setApprovalRules,
    setEntries,
    wsRef,
    setThinking,
    clearThinkingIdle,
    playPing,
    chatTitleRef,
    pendingProposalsRef,
    setSecretRequests,
    pendingAskUserRef,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    switch (msg.type) {
      case 'approval_rules':
        if (approvalStarted.current[msg.call_id] === undefined) break;
        setApprovalRules((previous) => ({
          ...previous,
          [msg.call_id]: { rules: msg.rules, error: msg.error },
        }));
        break;

      case 'approval_resolved':
        setEntries((previous) =>
          previous.flatMap((entry) => {
            if (entry.kind !== 'tool' || entry.call.id !== msg.call_id) return [entry];
            if (isAgentRequest(entry.call)) return [];
            if (entry.status !== 'pending') return [entry];
            return [{ ...entry, status: msg.allow ? ('running' as const) : ('denied' as const) }];
          }),
        );
        break;

      case 'approval_request':
        approvalStarted.current[msg.call.id] = Date.now();
        wsRef.current?.send({ type: 'approval_rules', call_id: msg.call.id });
        setThinking(false);
        clearThinkingIdle();
        playPing();
        callForAttention('approval', chatTitleRef.current);
        setEntries((prev) => [
          ...sealThought(prev),
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
        // Server reuses the tool call id as the prompt id. Attach immediately
        // if the tool_start already arrived; otherwise stash the proposal so
        // tool_start can pick it up when it lands (see the tool_start case).
        playPing();
        callForAttention('plan', chatTitleRef.current);
        setEntries((prev) => {
          const hit = prev.some((e) => e.kind === 'tool' && e.call.id === msg.prompt_id);
          if (!hit) {
            pendingProposalsRef.current.set(msg.prompt_id, msg.plan);
            return prev;
          }
          return attachPlanProposal(prev, msg.prompt_id, msg.plan);
        });
        break;

      case 'prompt_resolved':
        // Answered in another window (or tab, or device): close this copy
        // with the same answer instead of leaving it waiting.
        if (msg.kind === 'secret') {
          setSecretRequests((prev) => prev.filter((r) => r.promptId !== msg.prompt_id));
        } else if (msg.kind === 'ask_user') {
          const decision: AskUserDecision = msg.cancelled
            ? { cancelled: true }
            : { cancelled: false, answers: msg.answers ?? [] };
          setEntries((prev) => recordAskUserDecision(prev, msg.prompt_id, decision));
        } else if (msg.kind === 'plan') {
          setEntries((prev) =>
            recordPlanDecision(prev, msg.prompt_id, {
              approved: !!msg.approved,
              steps: msg.steps ?? undefined,
              note: msg.note ?? undefined,
            }),
          );
        }
        break;

      case 'runtime_request_updated':
        setEntries((prev) => upsertRuntimeQuestion(prev, msg.request));
        break;

      case 'ask_user_request':
        // Same race-guard pattern as plan_request — attach immediately when
        // the tool_start already landed; stash otherwise.
        playPing();
        callForAttention('question', chatTitleRef.current);
        setEntries((prev) => {
          const hit = prev.some((e) => e.kind === 'tool' && e.call.id === msg.prompt_id);
          if (!hit) {
            pendingAskUserRef.current.set(msg.prompt_id, msg.proposal);
            return prev;
          }
          return attachAskUserProposal(prev, msg.prompt_id, msg.proposal);
        });
        break;

      case 'secret_request':
        playPing();
        callForAttention('question', chatTitleRef.current);
        setSecretRequests((prev) =>
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
