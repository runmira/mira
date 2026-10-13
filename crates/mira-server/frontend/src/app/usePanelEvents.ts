import { applyReviewEvent,emptyReviewState } from '../components/ReviewPanel';
import {
appendDelegateStep,
appendToken,
attachToolResult,
updateSubagent,
upsertToolStart,
} from '../transcript/entries';
import type { ServerMsg } from '../types';

import { useStableCallback } from './shared';
import type { WorkspaceRuntime } from './WorkspaceRuntime';
export function usePanelEvents(
  context: Pick<
    WorkspaceRuntime,
    | 'setReviewState'
    | 'setReviewPanelOpen'
    | 'updateSubagentProjection'
    | 'setAgentTabs'
    | 'setActiveAgentTab'
    | 'updateTranscript'
    | 'setBrowserPing'
  >,
) {
  const {
    setReviewState,
    setReviewPanelOpen,
    updateSubagentProjection,
    setAgentTabs,
    setActiveAgentTab,
    updateTranscript,
    setBrowserPing,
  } = context;
  return useStableCallback((msg: ServerMsg) => {
    const receivedAt = Date.now();
    switch (msg.type) {
      case 'review_started':
        setReviewState(emptyReviewState(msg.run_id));
        setReviewPanelOpen(true);
        break;

      case 'review_progress':
        setReviewState((prev) => {
          // Guard against stale frames from a previous run bleeding in
          // after a new run started — only apply if the run_ids match.
          if (!prev || prev.runId !== msg.run_id) return prev;
          return applyReviewEvent(prev, msg.event);
        });
        break;

      case 'review_result':
        setReviewState((prev) => {
          if (!prev || prev.runId !== msg.run_id) return prev;
          return { ...prev, findings: msg.findings, done: true };
        });
        break;

      case 'review_error':
        setReviewState((prev) => {
          if (!prev || prev.runId !== msg.run_id) return prev;
          return { ...prev, error: msg.text, done: true };
        });
        break;

      case 'subagent_started':
        updateSubagentProjection((prev) => {
          const next = new Map(prev);
          const existing = next.get(msg.parent_call_id);
          next.set(msg.parent_call_id, {
            parentCallId: msg.parent_call_id,
            agentId: msg.agent_id,
            model: msg.model,
            prompt: msg.prompt,
            agentName: msg.agent_name ?? null,
            agentCategory: msg.agent_category ?? null,
            entries: existing?.entries ?? [],
            done: false,
            pendingReview: existing?.pendingReview ?? null,
          });
          return next;
        });
        // Auto-focus the newly-spawned agent in the panel so the user sees
        // its first tokens without having to click the row. Only if the
        // panel isn't already open on a different agent — respect the
        // user's manual selection when one exists.
        setAgentTabs((prev) =>
          prev.includes(msg.parent_call_id) ? prev : [...prev, msg.parent_call_id],
        );
        setActiveAgentTab((prev) => prev ?? msg.parent_call_id);
        break;

      case 'subagent_token':
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            entries: appendToken(s.entries, msg.text, receivedAt),
          })),
        );
        break;

      case 'subagent_tool_start':
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            entries: upsertToolStart(s.entries, msg.call, receivedAt),
          })),
        );
        break;

      case 'subagent_tool_end':
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            entries: attachToolResult(s.entries, msg.result),
          })),
        );
        break;

      case 'subagent_warning':
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            entries: [...s.entries, { kind: 'warning', text: msg.text }],
          })),
        );
        break;

      case 'subagent_progress':
        // Streaming intermediate summary — surfaces as a `[progress]`
        // chip in the SubagentPanel (styled distinctly from warnings so
        // the reader can tell "here's where I am" from "something's off").
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            entries: [...s.entries, { kind: 'warning', text: `[progress] ${msg.text}` }],
          })),
        );
        break;

      case 'subagent_review_request':
        // Auto-open the tab so the user can't miss the review — the
        // parent's turn is blocked until Approve/Deny lands. If it's
        // already open, we just annotate its state.
        setAgentTabs((prev) =>
          prev.includes(msg.parent_call_id) ? prev : [...prev, msg.parent_call_id],
        );
        setActiveAgentTab(msg.parent_call_id);
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            pendingReview: { promptId: msg.prompt_id, summary: msg.summary },
          })),
        );
        break;

      case 'subagent_done':
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            done: true,
          })),
        );
        break;

      case 'subagent_scratchpad_note':
        // Cross-subagent shared findings. Surface as a distinct chip in
        // the author's tab so a viewer can see who posted what, and keep
        // the raw text so a future "Shared notes" pane can dedupe by
        // (session, ts) if we surface it more prominently later.
        updateSubagentProjection((prev) =>
          updateSubagent(prev, msg.parent_call_id, (s) => ({
            ...s,
            entries: [
              ...s.entries,
              { kind: 'warning', text: `[note ${msg.entry.author}] ${msg.entry.text}` },
            ],
          })),
        );
        break;

      case 'delegate_progress':
        // A step the delegated child just took. Routes to its card by call id
        // (the parent's `delegate_task` id), with a fallback to the most
        // recent running delegation for providers that don't echo the id.
        updateTranscript((prev) => appendDelegateStep(prev, msg.call_id, msg.kind, msg.text, receivedAt));
        break;

      case 'browser_active':
        // An agent is driving Mira's browser through the tool server.
        setBrowserPing((n) => n + 1);
        break;

      case 'html_render':
        updateTranscript((prev) =>
          prev.some((e) => e.kind === 'html_render' && e.id === msg.id)
            ? prev
            : [...prev, { kind: 'html_render', id: msg.id, title: msg.title, html: msg.html }],
        );
        break;
    }
  });
}
