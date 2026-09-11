import { useEffect, useMemo, useRef, useState } from 'react';
import { CaretDown, Target } from '@phosphor-icons/react';
import { cn } from './lib/utils';
import { connect, type WsClient, type WsStatus } from './ws';
import { appendMemory, applyUndo, getSessionHistory, getSettings, newSession, startReview } from './api';
import { extractAgentId } from './components/AgentCard';
import { SettingsPanel } from './components/Settings';
import { PluginsPanel } from './components/Plugins';
import { PullRequestPanel } from './components/PullRequestPanel';
import { Sidebar, type MainView } from './components/Sidebar';
import { Composer } from './components/Composer';
import { FolderPicker } from './components/FolderPicker';
import { AssistantContent } from './components/AssistantContent';
import { ToolCard, type ToolStatus } from './components/ToolCard';
import { Thinking } from './components/Thinking';
import {
  applyReviewEvent,
  emptyReviewState,
  ReviewPanel,
  type ReviewState,
} from './components/ReviewPanel';
import { PlanCard } from './components/PlanCard';
import { AgentCard, AgentGroup } from './components/AgentCard';
import { SubagentPanel, type SubagentTab } from './components/SubagentPanel';
import { TaskListPanel } from './components/TaskListPanel';
import { GoalPanel } from './components/GoalPanel';
import { ToolGroup } from './components/ToolGroup';
import type {
  DiffPreview,
  Goal,
  Message,
  Mode,
  PlanProposal,
  PlanStep,
  ServerMsg,
  SettingsView,
  TaskItem,
  ToolCall,
  ToolResult,
  UsageTotals,
} from './types';
import { formatUsage } from './lib/usage';

/** Live per-child state for the subagent panel — mirrors the shape of
 *  the parent's own transcript so the panel body can reuse EntryView-style
 *  grouping without special-casing. `done` flips when either an explicit
 *  `subagent_done` frame lands or the parent's ToolEnd for this call
 *  arrives (whichever is first). */
export type SubagentStreamState = {
  parentCallId: string;
  agentId?: string;
  model?: string;
  prompt?: string;
  entries: Entry[];
  done: boolean;
  /** When set, the child produced a summary that requires human review
   *  before it's returned to the parent. The SubagentPanel renders an
   *  inline card with Approve / Deny buttons; approving fires the
   *  server-bound `PromptResponse` and clears this field. Set from the
   *  `subagent_review_request` frame. */
  pendingReview: null | { promptId: string; summary: string };
};

type ToolEntry = {
  kind: 'tool';
  call: ToolCall;
  preview: DiffPreview | null;
  status: ToolStatus;
  result: ToolResult | null;
  /** Only set for the `plan` tool. `proposal` arrives on `plan_request`;
   *  `decision` fills in when the user approves/cancels. Rendered inline
   *  as a full-fledged plan card instead of the generic tool row. */
  plan?: {
    proposal: PlanProposal;
    decision: null | { approved: boolean; steps?: PlanStep[]; note?: string };
  };
};
type WarningEntry = { kind: 'warning'; text: string };
type ErrorEntry = { kind: 'error'; text: string };
type MsgEntry = { kind: 'msg'; msg: Message };
/** Structured goal event in the transcript. Rendered as its own
 *  purple-tinted card by [`EntryView`] so goal turns visually anchor
 *  the timeline instead of masquerading as generic warnings.
 *  `variant: 'set'` marks the initial `/goal` moment,
 *  `'cleared'` the user drop, `'progress'` each non-terminal
 *  evaluator verdict, and `'done'` the terminal transition. */
type GoalEntry = {
  kind: 'goal';
  variant: 'set' | 'cleared' | 'progress' | 'done';
  iteration: number | null;
  maxIterations: number | null;
  status: import('./types').GoalStatus | null;
  reason: string | null;
  /** Only set on `variant: 'set'`. */
  condition?: string | null;
};
export type Entry = MsgEntry | ToolEntry | WarningEntry | ErrorEntry | GoalEntry;

type TurnTiming = {
  startedAt: number;
  endedAt: number | null;
};

/**
 * Rebuild the transcript from a `Session::history()` snapshot (arrives in
 * the `Ready` frame on connect / reconnect / new-session / session-load).
 *
 * The wire history is a flat `[system, user, assistant, tool, assistant …]`
 * list where:
 *  - an assistant turn may carry text AND/OR `tool_calls`
 *  - a tool-role message answers exactly one `tool_call_id`
 *
 * Naively rendering every message as a bubble drops all the tool work
 * (the previous rendering had `if (role === 'tool') return null` and
 * skipped empty assistant messages too). This walk re-pairs assistant
 * calls with their tool results and emits `ToolEntry`s marked `complete`
 * so the compact tool rows show up again on reconnect.
 *
 * Caveats: `is_error` and diff previews aren't persisted, so we default
 * both to a "clean complete" render. Denials are still visible because
 * the harness writes a "denied by policy: …" content string into the tool
 * message.
 */
export function historyToEntries(history: Message[]): Entry[] {
  // First pass — index tool results by call_id so the assistant walk can
  // attach them in O(1) rather than re-scanning history for each call.
  const resultByCallId = new Map<string, ToolResult>();
  for (const m of history) {
    if (m.role === 'tool' && m.tool_call_id) {
      resultByCallId.set(String(m.tool_call_id), {
        call_id: String(m.tool_call_id),
        content: m.content ?? '',
        is_error: false,
      });
    }
  }

  const entries: Entry[] = [];
  for (const m of history) {
    if (m.role === 'system' || m.role === 'tool') continue;

    if (m.role === 'user') {
      entries.push({ kind: 'msg', msg: m });
      continue;
    }

    // Assistant: emit any text first, then a ToolEntry per tool_call.
    // Doing text-then-tools mirrors the live turn flow (`token…` then
    // `tool_start`) so a resumed transcript reads identically.
    if ((m.content ?? '').trim()) {
      entries.push({ kind: 'msg', msg: m });
    }
    for (const call of m.tool_calls ?? []) {
      entries.push({
        kind: 'tool',
        call,
        preview: null,
        status: 'complete',
        result: resultByCallId.get(call.id) ?? null,
      });
    }
  }
  return entries;
}

export default function App() {
  const [status, setStatus] = useState<WsStatus>('connecting');
  const [sessionId, setSessionId] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [mode, setMode] = useState<Mode>('manual');
  const [cwd, setCwd] = useState<string>('');
  const [entries, setEntries] = useState<Entry[]>([]);
  // Per-turn timing. Turn index = 0-based order of user messages in `entries`.
  // Only turns started in *this* session have timing (reloaded transcripts
  // have no wall-clock data, so their turns skip the "Worked for" header).
  const [turnTimings, setTurnTimings] = useState<Map<number, TurnTiming>>(new Map());
  const [expandedTurns, setExpandedTurns] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState<boolean>(false);
  const [thinking, setThinking] = useState<boolean>(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  // Which primary view fills the main pane. Sidebar nav items switch this;
  // starting a chat / loading a session snaps back to 'chat' so the user
  // isn't stranded on a management screen when the model streams a reply.
  const [mainView, setMainView] = useState<MainView>('chat');
  // Right-side subagent panel: `agentTabs` is the ordered list of open
  // agent call_ids; `activeAgentTab` is the visible one. Panel is open
  // iff `agentTabs.length > 0`. Tab bodies are derived from `entries`
  // in a useMemo so status/result updates flow through automatically.
  const [agentTabs, setAgentTabs] = useState<string[]>([]);
  const [activeAgentTab, setActiveAgentTab] = useState<string | null>(null);
  // Live subagent transcripts keyed by parent tool-call id. Same Entry
  // shape as the parent transcript so the panel body can reuse the same
  // grouping and rendering helpers. Populated by the `subagent_*` WS
  // frames the AgentTool broadcasts as its child streams events.
  const [subagentState, setSubagentState] = useState<Map<string, SubagentStreamState>>(new Map());
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [sidebarRefresh, setSidebarRefresh] = useState(0);
  const [reviewPanelOpen, setReviewPanelOpen] = useState(false);
  const [reviewState, setReviewState] = useState<ReviewState | null>(null);
  const [usage, setUsage] = useState<UsageTotals | null>(null);
  // Live task list — hydrated from `ready.tasks` on socket open and
  // upserted whenever a `task_*` tool result lands. Rendered as a
  // persistent "Plan" card near the top of the transcript.
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  // Standing `/goal`, if any. Populated from `ready.goal` on socket
  // open and mutated by `goal_set` / `goal_progress` / `goal_done` /
  // `goal_cleared` server frames. Absent = no autonomous run set.
  const [goal, setGoal] = useState<Goal | null>(null);
  // Force a re-render every second while a turn is active so the live
  // "Working…" counter ticks. Cheap; the tree is small and only mounts
  // when the browser tab is visible.
  const [, setNowTick] = useState(0);
  const wsRef = useRef<WsClient | null>(null);
  const paneRef = useRef<HTMLDivElement | null>(null);
  // Plan payloads that arrived *before* their tool_start (race between the
  // interactive tool's direct broadcast and the harness→WS forwarder). We
  // stash by call_id and drain on tool_start so no plan ever renders as a
  // plain running tool row.
  const pendingProposalsRef = useRef<Map<string, PlanProposal>>(new Map());

  useEffect(() => {
    const c = connect(onMessage, setStatus);
    wsRef.current = c;
    return () => c.close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    getSettings()
      .then((v) => {
        setConfigured(v.configured);
        if (!v.configured) setSettingsOpen(true);
      })
      .catch(() => setConfigured(false));
  }, []);

  useEffect(() => {
    const el = paneRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [entries, thinking]);

  // Tick the live counter while a turn is in flight. Stopping the interval
  // as soon as `busy` clears avoids a needless setInterval that runs forever.
  useEffect(() => {
    if (!busy) return;
    const id = window.setInterval(() => setNowTick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, [busy]);

  function onMessage(msg: ServerMsg) {
    switch (msg.type) {
      case 'ready':
        setSessionId(msg.session_id);
        setModel(msg.model);
        setMode(msg.mode);
        setCwd(msg.cwd);
        setEntries(historyToEntries(msg.history));
        // Server-persisted turn timing is aligned with user-message order
        // (turn 0 = first user msg). Rebuild the local Map so "Worked for"
        // chips render on reloaded transcripts.
        setTurnTimings(rebuildTurnTimings(msg.turns ?? []));
        setExpandedTurns(new Set());
        setUsage(msg.usage ?? null);
        setTasks(msg.tasks ?? []);
        setGoal(msg.goal ?? null);
        setBusy(false);
        setThinking(false);
        setSidebarRefresh((n) => n + 1);
        // A Ready frame means the harness swapped session context (new /
        // load / resume / reconnect). If the user was parked on Plugins
        // or another management view, jump back to chat so a fresh
        // transcript actually shows.
        setMainView('chat');
        break;
      case 'token':
        setThinking(false);
        setEntries((prev) => appendToken(prev, msg.text));
        break;
      case 'approval_request':
        setThinking(false);
        setEntries((prev) => [
          ...prev,
          { kind: 'tool', call: msg.call, preview: msg.preview ?? null, status: 'pending', result: null },
        ]);
        break;
      case 'tool_start':
        setThinking(false);
        setEntries((prev) => {
          const withStart = upsertToolStart(prev, msg.call);
          // If a plan_request arrived before this tool_start (race between
          // the tool's direct broadcast and the harness forwarder), drain
          // the queued proposal onto the fresh entry now.
          const queued = pendingProposalsRef.current.get(msg.call.id);
          if (queued) {
            pendingProposalsRef.current.delete(msg.call.id);
            return attachPlanProposal(withStart, msg.call.id, queued);
          }
          return withStart;
        });
        break;
      case 'tool_end':
        // The model usually starts thinking again after a tool result comes
        // back before the next text token arrives.
        setThinking(true);
        setEntries((prev) => attachToolResult(prev, msg.result));
        // Piggyback: task_* tools ship the current task or full list in
        // `data`. Upsert so the Plan panel stays live without another
        // round-trip.
        setTasks((prev) => applyTaskResult(prev, msg.result));
        break;
      case 'turn_complete':
        // A turn ended (assistant round complete). More may follow if there
        // were tool calls; if not, `done` will clear us right after.
        setThinking(true);
        // Belt-and-suspenders sidebar refresh — the onSend-triggered
        // refetch can race the harness's first checkpoint on a very
        // fresh session; this fires once the first assistant round has
        // definitively landed on disk.
        setSidebarRefresh((n) => n + 1);
        break;
      case 'done':
        setBusy(false);
        setThinking(false);
        // Close out the most recent turn's timing.
        setTurnTimings((prev) => stampLastTurn(prev, Date.now()));
        setSidebarRefresh((n) => n + 1);
        break;
      case 'warning':
        setEntries((prev) => [...prev, { kind: 'warning', text: msg.text }]);
        break;
      case 'error':
        setEntries((prev) => [...prev, { kind: 'error', text: msg.text }]);
        break;
      case 'model_changed':
        setModel(msg.model);
        break;
      case 'mode_changed':
        setMode(msg.mode);
        break;
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
      case 'session_title_updated':
        // Nickname landed on disk — refresh the sidebar so the row label
        // switches from the first-user-message fallback to the AI title.
        setSidebarRefresh((n) => n + 1);
        break;
      case 'usage':
        setUsage(msg.totals);
        break;
      case 'memory_learned':
        setEntries((prev) => [
          ...prev,
          {
            kind: 'warning',
            text: `[memory] remembered ${msg.count} thing${msg.count === 1 ? '' : 's'}`,
          },
        ]);
        break;
      case 'compacted':
        setEntries((prev) => [
          ...prev,
          {
            kind: 'warning',
            text: `[context] compacted ${msg.messages_removed} earlier message${msg.messages_removed === 1 ? '' : 's'} into a summary`,
          },
        ]);
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
      case 'subagent_started':
        setSubagentState((prev) => {
          const next = new Map(prev);
          const existing = next.get(msg.parent_call_id);
          next.set(msg.parent_call_id, {
            parentCallId: msg.parent_call_id,
            agentId: msg.agent_id,
            model: msg.model,
            prompt: msg.prompt,
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
        setAgentTabs((prev) => (prev.includes(msg.parent_call_id) ? prev : [...prev, msg.parent_call_id]));
        setActiveAgentTab((prev) => prev ?? msg.parent_call_id);
        break;
      case 'subagent_token':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: appendToken(s.entries, msg.text),
        })));
        break;
      case 'subagent_tool_start':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: upsertToolStart(s.entries, msg.call),
        })));
        break;
      case 'subagent_tool_end':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: attachToolResult(s.entries, msg.result),
        })));
        break;
      case 'subagent_warning':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: [...s.entries, { kind: 'warning', text: msg.text }],
        })));
        break;
      case 'subagent_progress':
        // Streaming intermediate summary — surfaces as a `[progress]`
        // chip in the SubagentPanel (styled distinctly from warnings so
        // the reader can tell "here's where I am" from "something's off").
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          entries: [...s.entries, { kind: 'warning', text: `[progress] ${msg.text}` }],
        })));
        break;
      case 'subagent_review_request':
        // Auto-open the tab so the user can't miss the review — the
        // parent's turn is blocked until Approve/Deny lands. If it's
        // already open, we just annotate its state.
        setAgentTabs((prev) => (prev.includes(msg.parent_call_id) ? prev : [...prev, msg.parent_call_id]));
        setActiveAgentTab(msg.parent_call_id);
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          pendingReview: { promptId: msg.prompt_id, summary: msg.summary },
        })));
        break;
      case 'subagent_done':
        setSubagentState((prev) => updateSubagent(prev, msg.parent_call_id, (s) => ({
          ...s,
          done: true,
        })));
        break;
      case 'plan_request':
        // Server reuses the tool call id as the prompt id. Attach immediately
        // if the tool_start already arrived; otherwise stash the proposal so
        // tool_start can pick it up when it lands (see the tool_start case).
        setEntries((prev) => {
          const hit = prev.some((e) => e.kind === 'tool' && e.call.id === msg.prompt_id);
          if (!hit) {
            pendingProposalsRef.current.set(msg.prompt_id, msg.plan);
            return prev;
          }
          return attachPlanProposal(prev, msg.prompt_id, msg.plan);
        });
        break;
    }
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
    setSubagentState((prev) => updateSubagent(prev, parentCallId, (s) => ({
      ...s,
      pendingReview: null,
    })));
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

  function decideApproval(callId: string, allow: boolean) {
    wsRef.current?.send({ type: 'approve', call_id: callId, allow });
    setEntries((prev) => updateTool(prev, callId, (t) => ({
      ...t,
      status: allow ? 'running' : 'denied',
    })));
  }

  function onSend(text: string) {
    // Belt-and-suspenders — the composer isn't visible on non-chat views,
    // but a keyboard-driven send would still land the message and it should
    // pull the user back to the transcript.
    setMainView('chat');
    // Refresh the sidebar immediately so a brand-new thread shows up in
    // the projects list on the first send, not after the model finishes
    // responding. The harness checkpoints the pushed user message at the
    // top of `run_loop` so this refetch sees the new row.
    setSidebarRefresh((n) => n + 1);
    const now = Date.now();
    setEntries((prev) => {
      const next: Entry[] = [...prev, { kind: 'msg', msg: { role: 'user', content: text } }];
      const turnIndex = countUserMessages(next) - 1;
      setTurnTimings((tt) => {
        const clone = new Map(tt);
        clone.set(turnIndex, { startedAt: now, endedAt: null });
        return clone;
      });
      return next;
    });
    setBusy(true);
    setThinking(true);
    wsRef.current?.send({ type: 'send', text });
  }

  function onSetMode(m: Mode) { wsRef.current?.send({ type: 'set_mode', mode: m }); }
  function onSetModel(m: string) { wsRef.current?.send({ type: 'set_model', model: m }); }
  function onSetEffort(e: string | null) { wsRef.current?.send({ type: 'set_effort', effort: e }); }

  /** Kick off (or replace) an autonomous run against a condition. The
   *  server broadcasts `goal_set` so the panel state syncs there — we
   *  don't set it locally to avoid a brief drift if the server rejects
   *  the request (e.g. empty condition). */
  function onSetGoal(condition: string, maxIterations?: number) {
    const trimmed = condition.trim();
    if (!trimmed) return;
    wsRef.current?.send({
      type: 'set_goal',
      condition: trimmed,
      max_iterations: maxIterations ?? null,
    });
  }

  function onClearGoal() {
    wsRef.current?.send({ type: 'clear_goal' });
  }

  async function onNewChat() {
    try {
      await newSession();
    } catch (e) {
      setEntries((prev) => [...prev, { kind: 'error', text: `new chat: ${(e as Error).message}` }]);
    }
  }

  const settingsHandler = (v: SettingsView) => {
    setConfigured(v.configured);
    if (v.default_model) setModel(v.default_model);
    if (v.default_mode) setMode(v.default_mode as Mode);
  };

  const isEmpty = useMemo(
    () => entries.every((e) => e.kind === 'msg' && !(e.msg.content || '').trim() && (e.msg.tool_calls?.length ?? 0) === 0),
    [entries],
  );

  const turns = useMemo(() => groupByTurn(entries), [entries]);

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

  function openAgentTab(callId: string) {
    setAgentTabs((prev) => (prev.includes(callId) ? prev : [...prev, callId]));
    setActiveAgentTab(callId);
    // Fire-and-forget: if we don't already have a live stream for this
    // agent (fresh page after a reload, or a resumed session), pull the
    // child's persisted history and reconstruct the transcript.
    void hydrateSubagentIfNeeded(callId);
  }

  /** If subagentState has no entries for `callId`, extract the child
   *  session id from the parent's tool_result marker and fetch the
   *  child's history from the backend. Populates subagentState so the
   *  panel body renders the full timeline. No-op when the marker is
   *  missing (older sessions before the marker landed) or when a live
   *  stream already exists. */
  async function hydrateSubagentIfNeeded(callId: string) {
    // Bail if we already have a stream in flight or on record for this call.
    const existing = subagentState.get(callId);
    if (existing && existing.entries.length > 0) return;
    // Find the parent's tool entry to read the tool_result content.
    const entry = entries.find(
      (e): e is ToolEntry => e.kind === 'tool' && e.call.id === callId,
    );
    if (!entry?.result) return;
    const agentId = extractAgentId(entry.result.content);
    if (!agentId) return;
    try {
      const view = await getSessionHistory(agentId);
      const rebuilt = historyToEntries(view.messages);
      setSubagentState((prev) => updateSubagent(prev, callId, (s) => ({
        ...s,
        agentId,
        model: view.model,
        entries: rebuilt,
        done: true,
      })));
    } catch (e) {
      // Non-fatal: panel falls back to the summary block. Log so
      // developers see the failure without breaking the user's flow.
      console.warn('subagent hydrate failed', callId, agentId, e);
    }
  }

  function closeAgentTab(callId: string) {
    setAgentTabs((prev) => {
      const next = prev.filter((id) => id !== callId);
      // If we just closed the active tab, jump to whatever's still open
      // (or clear when empty).
      if (activeAgentTab === callId) {
        setActiveAgentTab(next.length > 0 ? next[next.length - 1] : null);
      }
      return next;
    });
  }

  function closeSubagentPanel() {
    setAgentTabs([]);
    setActiveAgentTab(null);
  }

  const panelOpen = subagentTabs.length > 0;

  function toggleTurn(idx: number) {
    setExpandedTurns((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx); else next.add(idx);
      return next;
    });
  }

  return (
    <div
      className={cn(
        // grid-rows-1 forces the single row to `minmax(0, 1fr)` — without
        // it, tall cell content (the SubagentPanel's markdown body) would
        // grow the row past 100vh and push the composer inside <main>
        // below the viewport fold.
        'grid h-screen grid-rows-1 bg-background transition-[grid-template-columns] duration-150',
        panelOpen
          ? 'grid-cols-[300px_minmax(0,1fr)_minmax(340px,440px)]'
          : 'grid-cols-[300px_minmax(0,1fr)]',
      )}
    >
      <Sidebar
        status={status}
        cwd={cwd}
        activeSessionId={sessionId}
        activeBusy={busy}
        refreshKey={sidebarRefresh}
        activeView={mainView}
        onNavigate={setMainView}
        onNewChat={async () => {
          setMainView('chat');
          await onNewChat();
        }}
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenPicker={() => setPickerOpen(true)}
        onSessionLoaded={() => { /* Ready broadcast refreshes + jumps to chat */ }}
      />

      <main className="flex min-w-0 min-h-0 flex-col">
        {mainView === 'chat' && (
          <>
            <div className="flex h-11 shrink-0 items-center gap-3 border-b border-border/60 px-4">
              <span className="min-w-0 flex-1 truncate text-[13.5px] text-foreground">
                {titleFromEntries(entries)}
              </span>
              <div className="inline-flex rounded-full border border-border bg-secondary/60 p-0.5">
                <button
                  type="button"
                  className="rounded-full bg-secondary px-3.5 py-1 text-[12.5px] font-medium text-foreground"
                >
                  Chat
                </button>
                <button
                  type="button"
                  disabled
                  title="Not implemented yet"
                  className="rounded-full px-3.5 py-1 text-[12.5px] text-muted-foreground/40 cursor-not-allowed"
                >
                  Work
                </button>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto px-5 pb-5 pt-4" ref={paneRef}>
              {configured === false && (
                <div className="mx-auto mb-4 max-w-3xl rounded-lg border border-amber-500/30 bg-amber-500/[0.08] px-3 py-2 text-[13px] text-amber-200">
                  No provider configured —{' '}
                  <button
                    className="underline underline-offset-2 hover:text-amber-100"
                    onClick={() => setSettingsOpen(true)}
                  >
                    open Settings
                  </button>{' '}
                  to add one.
                </div>
              )}

              {isEmpty ? (
                <EmptyState />
              ) : (
                <div className="mx-auto flex max-w-3xl flex-col gap-2">
                  {goal && (
                    <GoalPanel
                      goal={goal}
                      busy={busy}
                      activity={goalActivity(entries)}
                      onClear={onClearGoal}
                      onRestart={(condition, maxIter) => onSetGoal(condition, maxIter)}
                    />
                  )}
                  {tasks.length > 0 && <TaskListPanel tasks={tasks} />}
                  {turns.map((turn, i) => (
                    <TurnView
                      key={`turn-${i}`}
                      turn={turn}
                      timing={turnTimings.get(i) ?? null}
                      expanded={expandedTurns.has(i)}
                      onToggle={() => toggleTurn(i)}
                      onDecide={decideApproval}
                      onPlanReply={replyToPlan}
                      onOpenAgent={openAgentTab}
                      isActive={busy && i === turns.length - 1}
                    />
                  ))}
                  {thinking && (
                    <div className="flex justify-start">
                      <Thinking />
                    </div>
                  )}
                </div>
              )}
            </div>

            <Composer
              disabled={status !== 'open'}
              busy={busy}
              mode={mode}
              model={model}
              cwd={cwd}
              usage={formatUsage(model, usage)}
              onSend={onSend}
              onSetMode={onSetMode}
              onSetModel={onSetModel}
              onSetEffort={onSetEffort}
              onOpenPicker={() => setPickerOpen(true)}
              onInterrupt={() => wsRef.current?.send({ type: 'interrupt' })}
              onNewChat={onNewChat}
              onOpenSettings={() => setSettingsOpen(true)}
              onRunReview={runReview}
              onSetGoal={onSetGoal}
              onClearGoal={onClearGoal}
              goal={goal}
              onRemember={async (scope, text) => {
                const r = await appendMemory(scope, text);
                return `remembered → ${r.path}`;
              }}
              onUndo={async (count) => {
                const r = await applyUndo(count);
                if (r.applied.length === 0) return 'nothing to undo';
                return `reverted ${r.applied.length} write${r.applied.length === 1 ? '' : 's'}`;
              }}
            />
          </>
        )}

        {mainView === 'plugins' && (
          <div className="flex-1 min-h-0 overflow-y-auto">
            <PluginsPanel />
          </div>
        )}

        {mainView === 'pull-request' && (
          <PullRequestPanel
            onOpenSettings={() => setSettingsOpen(true)}
            onReviewPr={runPrReview}
          />
        )}
        {mainView === 'scheduled' && <ComingSoon label="Scheduled" />}
      </main>

      {panelOpen && (
        <SubagentPanel
          tabs={subagentTabs}
          activeCallId={activeAgentTab}
          onSelectTab={setActiveAgentTab}
          onCloseTab={closeAgentTab}
          onClose={closeSubagentPanel}
          onReview={replyToSubagentReview}
        />
      )}

      <SettingsPanel
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onSaved={settingsHandler}
      />
      <FolderPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onPicked={() => { /* Ready broadcast refreshes */ }}
      />
      <ReviewPanel
        open={reviewPanelOpen}
        state={reviewState}
        onClose={() => setReviewPanelOpen(false)}
      />
    </div>
  );
}

/* ---------- helpers ---------- */

/** Immutable Map update helper — replaces the entry for `parentCallId`
 *  (or seeds a fresh one) with the result of `f`. Returns a new Map so
 *  React sees the state change. */
function updateSubagent(
  prev: Map<string, SubagentStreamState>,
  parentCallId: string,
  f: (s: SubagentStreamState) => SubagentStreamState,
): Map<string, SubagentStreamState> {
  const cur: SubagentStreamState = prev.get(parentCallId) ?? {
    parentCallId,
    entries: [],
    done: false,
    pendingReview: null,
  };
  const next = new Map(prev);
  next.set(parentCallId, f(cur));
  return next;
}

function appendToken(prev: Entry[], text: string): Entry[] {
  const last = prev[prev.length - 1];
  if (last && last.kind === 'msg' && last.msg.role === 'assistant') {
    const updated: Entry = {
      kind: 'msg',
      msg: { ...last.msg, content: (last.msg.content ?? '') + text },
    };
    return [...prev.slice(0, -1), updated];
  }
  return [...prev, { kind: 'msg', msg: { role: 'assistant', content: text } }];
}

// A tool_start arrives after either (a) a user-approved approval_request, in
// which case an entry already exists — leave it alone, or (b) an auto-approved
// call the policy let through with no prompt — add a fresh entry.
function upsertToolStart(prev: Entry[], call: ToolCall): Entry[] {
  const existing = prev.findIndex((e) => e.kind === 'tool' && e.call.id === call.id);
  if (existing >= 0) return prev;
  return [...prev, { kind: 'tool', call, preview: null, status: 'running', result: null }];
}

function attachToolResult(prev: Entry[], result: ToolResult): Entry[] {
  return updateTool(prev, result.call_id, (t) => ({
    ...t,
    result,
    // Preserve "denied" state; otherwise mark done.
    status: t.status === 'denied' ? 'denied' : 'complete',
  }));
}

/**
 * Merge a task_* tool result into the live task list.
 *
 * Contract: task_create / task_update / task_get emit
 * `{ task: FullItem }`; task_list emits `{ tasks: FullItem[] }`. Any
 * other tool (or an error result) → no-op.
 */
function applyTaskResult(prev: TaskItem[], result: ToolResult): TaskItem[] {
  if (result.is_error || !result.data || typeof result.data !== 'object') return prev;
  const d = result.data as { task?: TaskItem; tasks?: TaskItem[] };
  if (Array.isArray(d.tasks)) return d.tasks;
  if (d.task && typeof d.task.id === 'number') {
    const idx = prev.findIndex((t) => t.id === d.task!.id);
    if (idx === -1) return [...prev, d.task];
    return [...prev.slice(0, idx), d.task, ...prev.slice(idx + 1)];
  }
  return prev;
}

function updateTool(prev: Entry[], callId: string, f: (t: ToolEntry) => ToolEntry): Entry[] {
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && e.call.id === callId) {
      return [...prev.slice(0, i), f(e), ...prev.slice(i + 1)];
    }
  }
  return prev;
}

function attachPlanProposal(prev: Entry[], callId: string, proposal: PlanProposal): Entry[] {
  return updateTool(prev, callId, (t) => ({
    ...t,
    plan: { proposal, decision: null },
  }));
}

function recordPlanDecision(
  prev: Entry[],
  callId: string,
  decision: { approved: boolean; steps?: PlanStep[]; note?: string },
): Entry[] {
  return updateTool(prev, callId, (t) => {
    if (!t.plan) return t;
    return { ...t, plan: { ...t.plan, decision } };
  });
}

function countUserMessages(entries: Entry[]): number {
  let n = 0;
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user') n++;
  }
  return n;
}

function rebuildTurnTimings(serverTurns: { started_at: number; ended_at?: number | null }[]): Map<number, TurnTiming> {
  const out = new Map<number, TurnTiming>();
  serverTurns.forEach((t, i) => {
    out.set(i, {
      startedAt: t.started_at,
      endedAt: t.ended_at ?? null,
    });
  });
  return out;
}

function stampLastTurn(prev: Map<number, TurnTiming>, endedAt: number): Map<number, TurnTiming> {
  // The `done` frame closes out the most recently started turn — find the
  // highest turn index that's still marked "in flight" and stamp it.
  let target = -1;
  for (const [idx, t] of prev) {
    if (t.endedAt == null && idx > target) target = idx;
  }
  if (target < 0) return prev;
  const clone = new Map(prev);
  const t = clone.get(target)!;
  clone.set(target, { ...t, endedAt });
  return clone;
}

/** Pull the most recent "here's what mira is doing" line off the end
 *  of the entry list. Priority: newest running tool call > newest
 *  tool result > latest assistant fragment head. Empty string when
 *  there's nothing recognizable to show — the GoalPanel renders a
 *  generic "Working" in that case. */
function goalActivity(entries: Entry[]): string {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i];
    if (e.kind === 'tool' && e.status === 'running') {
      return `Running ${e.call.function.name}`;
    }
    if (e.kind === 'tool' && e.status === 'complete' && !e.result?.is_error) {
      return `Ran ${e.call.function.name}`;
    }
    if (e.kind === 'msg' && e.msg.role === 'assistant' && (e.msg.content ?? '').trim()) {
      const line = (e.msg.content ?? '').trim().split('\n')[0];
      return line.length > 90 ? line.slice(0, 90) + '…' : line;
    }
  }
  return '';
}

function titleFromEntries(entries: Entry[]): string {
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user' && e.msg.content?.trim()) {
      const t = e.msg.content.trim().split('\n')[0];
      return t.length > 60 ? t.slice(0, 60) + '…' : t;
    }
  }
  return 'New chat';
}

/* ---------- turn grouping ---------- */

type Turn = {
  /** The user's message that opened this turn. `null` for any pre-user
   *  entries (e.g. a system-emitted warning before the first send). */
  user: Entry | null;
  /** Everything after `user` up to the next user message. */
  body: Entry[];
};

function groupByTurn(entries: Entry[]): Turn[] {
  const turns: Turn[] = [];
  let current: Turn = { user: null, body: [] };
  for (const e of entries) {
    if (e.kind === 'msg' && e.msg.role === 'user') {
      // Close previous turn if it had anything.
      if (current.user || current.body.length) turns.push(current);
      current = { user: e, body: [] };
    } else {
      current.body.push(e);
    }
  }
  if (current.user || current.body.length) turns.push(current);
  return turns;
}

/* ---------- turn renderer ---------- */

function TurnView({
  turn, timing, expanded, isActive, onToggle, onDecide, onPlanReply, onOpenAgent,
}: {
  turn: Turn;
  timing: TurnTiming | null;
  expanded: boolean;
  isActive: boolean;
  onToggle: () => void;
  onDecide: (callId: string, allow: boolean) => void;
  onPlanReply: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  /** Opens (or focuses) the right-side SubagentPanel tab for the given
   *  agent call. Wired from App.tsx via `openAgentTab`. */
  onOpenAgent: (callId: string) => void;
}) {
  // Split the body into "intermediate work" and the final assistant text.
  // Rule: the LAST assistant text message with non-empty content is the
  // final answer; everything before it is intermediate. Tool cards + earlier
  // assistant text hide behind the "Worked for" chip when collapsed.
  const finalIdx = findFinalAssistantIndex(turn.body);
  const intermediateRaw = finalIdx >= 0 ? turn.body.slice(0, finalIdx) : turn.body;
  const finalEntry = finalIdx >= 0 ? turn.body[finalIdx] : null;
  const trailing = finalIdx >= 0 ? turn.body.slice(finalIdx + 1) : [];

  // Fold consecutive `agent` tool entries into a single group so a parallel
  // spawn ("N agents working") reads as one bar instead of N loud cards.
  // Non-agent entries pass through unchanged.
  const intermediate = useMemo(() => groupAgentRuns(intermediateRaw), [intermediateRaw]);

  const durationMs = timing
    ? (timing.endedAt ?? Date.now()) - timing.startedAt
    : null;
  const showWorkedChip = (intermediate.length > 0 || isActive) && durationMs != null;

  // While a turn is in flight, force the intermediate section open so
  // in-progress tool calls (esp. pending approval bubbles) stay visible.
  // The user can collapse it after `done` fires. A pending approval
  // requires a click to keep the model moving; hiding it would deadlock.
  const hasPendingApproval = intermediateRaw.some(
    (e) => e.kind === 'tool' && e.status === 'pending',
  );
  const forceOpen = isActive || hasPendingApproval;
  const effectivelyExpanded = expanded || forceOpen;

  return (
    <>
      {turn.user && <EntryView entry={turn.user} onDecide={onDecide} onPlanReply={onPlanReply} onOpenAgent={onOpenAgent} />}

      {showWorkedChip && (
        <WorkedForChip
          durationMs={durationMs!}
          active={isActive && timing?.endedAt == null}
          expanded={effectivelyExpanded}
          locked={forceOpen}
          onToggle={onToggle}
        />
      )}

      {(effectivelyExpanded || !showWorkedChip) && intermediate.map((item, i) => {
        if (item.kind === 'agent-group') {
          return (
            <div key={`t-a-${i}`} className="flex justify-start">
              <AgentGroup
                entries={item.entries.map((e) => ({
                  call: e.call,
                  status: e.status,
                  result: e.result,
                }))}
                onOpen={onOpenAgent}
              />
            </div>
          );
        }
        if (item.kind === 'tool-group') {
          return (
            <div key={`t-g-${i}`} className="flex justify-start">
              <ToolGroup
                entries={item.entries.map((e) => ({
                  call: e.call,
                  preview: e.preview,
                  status: e.status,
                  result: e.result,
                }))}
              />
            </div>
          );
        }
        return (
          <EntryView key={`t-i-${i}`} entry={item.entry} onDecide={onDecide} onPlanReply={onPlanReply} onOpenAgent={onOpenAgent} />
        );
      })}

      {finalEntry && <EntryView entry={finalEntry} onDecide={onDecide} onPlanReply={onPlanReply} onOpenAgent={onOpenAgent} />}
      {trailing.map((e, i) => (
        <EntryView key={`t-t-${i}`} entry={e} onDecide={onDecide} onPlanReply={onPlanReply} onOpenAgent={onOpenAgent} />
      ))}
    </>
  );
}

/** Item produced by `groupAgentRuns`: either a single passthrough entry,
 *  a run of `>=2` consecutive `agent` tool entries folded into a group,
 *  or a run of `>=2` consecutive same-name non-agent tool entries folded
 *  into a group (e.g. three `read_file`s → one "Read × 3" chip).
 *  A run of length 1 stays as a single entry — the individual card is
 *  enough on its own without the group chrome. */
export type GroupItem =
  | { kind: 'entry'; entry: Entry }
  | { kind: 'agent-group'; entries: (Entry & { kind: 'tool' })[] }
  | { kind: 'tool-group'; entries: (Entry & { kind: 'tool' })[] };

/** Types that render as their own cards (agent, plan) — never fold into
 *  a generic tool-group. Agent has its own AgentGroup path. */
const SPECIAL_TOOLS = new Set(['agent', 'plan']);

export function groupAgentRuns(entries: Entry[]): GroupItem[] {
  const out: GroupItem[] = [];
  let i = 0;
  while (i < entries.length) {
    const e = entries[i];

    // Agent run: consume all consecutive `agent` entries into a single
    // AgentGroup regardless of individual state (pending is auto-approved
    // anyway since AgentTool is Pure).
    if (isAgentEntry(e)) {
      const run: (Entry & { kind: 'tool' })[] = [];
      while (i < entries.length && isAgentEntry(entries[i])) {
        run.push(entries[i] as Entry & { kind: 'tool' });
        i++;
      }
      out.push(
        run.length >= 2
          ? { kind: 'agent-group', entries: run }
          : { kind: 'entry', entry: run[0] },
      );
      continue;
    }

    // Generic tool run: any consecutive tool entries, none pending. Mixing
    // tool names is fine — ToolGroup renders a "Working/Worked" umbrella
    // with per-entry verbs in the preview when the run isn't homogeneous.
    // Pending calls must render individually so the approval UI is visible
    // and unmissable — folding them into a group would hide the y/n prompt.
    if (isGroupableTool(e)) {
      const run: (Entry & { kind: 'tool' })[] = [];
      while (i < entries.length && isGroupableTool(entries[i])) {
        run.push(entries[i] as Entry & { kind: 'tool' });
        i++;
      }
      out.push(
        run.length >= 2
          ? { kind: 'tool-group', entries: run }
          : { kind: 'entry', entry: run[0] },
      );
      continue;
    }

    out.push({ kind: 'entry', entry: e });
    i++;
  }
  return out;
}

function isAgentEntry(e: Entry): boolean {
  return e.kind === 'tool' && e.call.function.name === 'agent';
}

/** A tool entry is groupable when it isn't a special one-off renderer
 *  (agent/plan) and isn't currently awaiting user approval. */
function isGroupableTool(e: Entry): boolean {
  if (e.kind !== 'tool') return false;
  if (SPECIAL_TOOLS.has(e.call.function.name)) return false;
  if (e.status === 'pending') return false;
  return true;
}

function findFinalAssistantIndex(body: Entry[]): number {
  for (let i = body.length - 1; i >= 0; i--) {
    const e = body[i];
    if (e.kind === 'msg' && e.msg.role === 'assistant' && (e.msg.content ?? '').trim()) {
      return i;
    }
  }
  return -1;
}

function WorkedForChip({
  durationMs, active, expanded, locked, onToggle,
}: {
  durationMs: number;
  active: boolean;
  expanded: boolean;
  /** Force-open due to in-flight work or a pending approval — the chip
   *  goes non-interactive so the user can't collapse away important state. */
  locked: boolean;
  onToggle: () => void;
}) {
  const label = active ? `Working… ${formatDuration(durationMs)}` : `Worked for ${formatDuration(durationMs)}`;
  return (
    <button
      type="button"
      onClick={locked ? undefined : onToggle}
      disabled={locked}
      title={locked ? 'Auto-expanded while in progress' : undefined}
      className={cn(
        'flex w-fit items-center gap-1.5 rounded-md px-2 py-1 text-[12.5px] text-muted-foreground transition-colors',
        locked ? 'cursor-default opacity-80' : 'hover:bg-accent/40 hover:text-foreground',
      )}
    >
      <CaretDown
        weight="bold"
        className={cn(
          'size-3 text-muted-foreground/60 transition-transform',
          !expanded && '-rotate-90',
        )}
      />
      <span>{label}</span>
      {active && <span className="size-1.5 animate-pulse rounded-full bg-mira-blue" />}
    </button>
  );
}

function formatDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m}m` : `${m}m ${rem}s`;
}

function EmptyState() {
  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-3 text-muted-foreground">
      <div className="size-10 rounded-full border border-border bg-secondary" />
      <div className="text-[22px] font-normal tracking-tight text-foreground">
        What should we build today?
      </div>
    </div>
  );
}

/** Placeholder view rendered for sidebar entries that don't have a real
 *  panel yet (Pull request, Scheduled). Keeps the shell responsive while
 *  we build out the actual features. */
function ComingSoon({ label }: { label: string }) {
  return (
    <div className="flex flex-1 min-h-0 flex-col items-center justify-center gap-2 text-muted-foreground">
      <div className="text-[18px] font-medium text-foreground/80">{label}</div>
      <div className="text-[13px]">Coming soon.</div>
    </div>
  );
}

function EntryView({
  entry,
  onDecide,
  onPlanReply,
  onOpenAgent,
}: {
  entry: Entry;
  onDecide: (callId: string, allow: boolean) => void;
  onPlanReply: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  onOpenAgent: (callId: string) => void;
}) {
  switch (entry.kind) {
    case 'msg': {
      const { role, content } = entry.msg;
      if (role === 'tool') return null;
      const body = (content ?? '').trim();
      if (!body) return null;
      if (role === 'user') {
        return (
          <div className="flex justify-end">
            <div className="max-w-[78%] whitespace-pre-wrap break-words rounded-2xl rounded-br-md bg-secondary px-4 py-2.5 text-[14.5px]">
              {content}
            </div>
          </div>
        );
      }
      return (
        <div className="flex justify-start">
          <div className="max-w-[90%]">
            <AssistantContent text={content ?? ''} />
          </div>
        </div>
      );
    }
    case 'tool':
      // The `plan` tool gets a dedicated inline card with an editable step
      // list; the `agent` tool gets a compact per-agent card so parallel
      // spawns don't dominate the transcript. Everything else falls
      // through to the generic tool row.
      if (entry.plan) {
        return (
          <div className="flex justify-start">
            <PlanCard
              proposal={entry.plan.proposal}
              decision={entry.plan.decision}
              onApprove={(steps) => onPlanReply(entry.call.id, true, steps)}
              onCancel={(note) => onPlanReply(entry.call.id, false, undefined, note || undefined)}
            />
          </div>
        );
      }
      if (entry.call.function.name === 'agent') {
        return (
          <div className="flex justify-start">
            <AgentCard
              call={entry.call}
              status={entry.status}
              result={entry.result}
              onOpen={onOpenAgent}
            />
          </div>
        );
      }
      return (
        <div className="flex justify-start">
          <ToolCard
            call={entry.call}
            preview={entry.preview}
            status={entry.status}
            result={entry.result}
            onDecide={(allow) => onDecide(entry.call.id, allow)}
          />
        </div>
      );
    case 'warning': {
      // Special channels riding the warning stream get their own chip:
      //   `[undo] ...`          → green success chip
      //   `[file-conflict] ...` → amber warning chip
      //   `[verify] … passed`   → green success chip
      //   `[verify] … failed`   → amber warning chip
      //   `[verify] running`    → blue in-flight chip
      //   `[progress] ...`      → blue "in-flight status" chip (subagent
      //                            emit_progress → SubagentPanel stream)
      //   `[memory] ...`        → violet "learned" chip
      //   `[context] ...`       → slate "compacted" chip
      // Everything else stays the compact monospace `! …` line.
      const undo = entry.text.match(/^\[undo\]\s*(.*)$/);
      const conflict = entry.text.match(/^\[file-conflict\]\s*(.*)$/);
      const verify = entry.text.match(/^\[verify\]\s*(.*)$/);
      const progress = entry.text.match(/^\[progress\]\s*(.*)$/);
      const memory = entry.text.match(/^\[memory\]\s*(.*)$/);
      const context = entry.text.match(/^\[context\]\s*(.*)$/);
      if (undo) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-emerald-500/25 bg-emerald-500/[0.06] px-3 py-1.5 text-[12.5px] text-emerald-300">
              <span className="font-semibold">↩ undo</span>
              <span className="min-w-0 break-words text-emerald-200/90">{undo[1]}</span>
            </div>
          </div>
        );
      }
      if (conflict) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-[12.5px] text-amber-200">
              <span className="font-semibold">⚠ file conflict</span>
              <span className="min-w-0 break-words text-amber-100/90">{conflict[1]}</span>
            </div>
          </div>
        );
      }
      if (progress) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-mira-blue/25 bg-mira-blue/[0.06] px-3 py-1.5 text-[12.5px] text-mira-blue">
              <span className="font-semibold">… progress</span>
              <span className="min-w-0 break-words opacity-90">{progress[1]}</span>
            </div>
          </div>
        );
      }
      if (verify) {
        const body = verify[1];
        const passed = /passed/i.test(body);
        const running = /running/i.test(body);
        const cls = passed
          ? 'border-emerald-500/25 bg-emerald-500/[0.06] text-emerald-300'
          : running
            ? 'border-mira-blue/25 bg-mira-blue/[0.06] text-mira-blue'
            : 'border-amber-500/30 bg-amber-500/10 text-amber-200';
        return (
          <div className="flex justify-start">
            <div className={cn('inline-flex items-start gap-2 rounded-md border px-3 py-1.5 text-[12.5px]', cls)}>
              <span className="font-semibold">✓ verify</span>
              <span className="min-w-0 break-words opacity-90">{body}</span>
            </div>
          </div>
        );
      }
      if (memory) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-violet-500/25 bg-violet-500/[0.06] px-3 py-1.5 text-[12.5px] text-violet-300">
              <span className="font-semibold">✦ memory</span>
              <span className="min-w-0 break-words text-violet-200/90">{memory[1]}</span>
            </div>
          </div>
        );
      }
      if (context) {
        return (
          <div className="flex justify-start">
            <div className="inline-flex items-start gap-2 rounded-md border border-slate-500/25 bg-slate-500/[0.06] px-3 py-1.5 text-[12.5px] text-slate-300">
              <span className="font-semibold">≡ context</span>
              <span className="min-w-0 break-words text-slate-200/90">{context[1]}</span>
            </div>
          </div>
        );
      }
      return (
        <div className="flex justify-start">
          <div className="font-mono text-xs text-mira-tool">! {entry.text}</div>
        </div>
      );
    }
    case 'error':
      return (
        <div className="flex justify-start">
          <div className="font-mono text-xs text-destructive">error: {entry.text}</div>
        </div>
      );
    case 'goal':
      return <GoalTranscriptChip entry={entry} />;
  }
}

/** Goal lifecycle events in the transcript.
 *
 * Design principles:
 *  - No border, no left accent bar, no pill. The event is anchored by
 *    a colored Target icon + a status-tinted headline; that's enough
 *    to distinguish a goal beat from surrounding assistant text
 *    without a second visual layer of chrome.
 *  - The reason (evaluator note or original condition) breathes below
 *    the headline in muted body text, indented to align under the
 *    headline for a clean two-tier read.
 *  - Terminal statuses use short, positive English ("Goal met") not
 *    protocol-speak ("goal_done · met"). */
function GoalTranscriptChip({ entry }: { entry: GoalEntry }) {
  const tint = goalChipTint(entry.status, entry.variant);
  const headline = goalChipHeadline(entry);
  const body = entry.reason ?? entry.condition ?? null;
  return (
    <div className="flex justify-start">
      <div className="w-full max-w-2xl py-1.5">
        <div className="flex items-baseline gap-2">
          <Target
            weight="fill"
            className={cn(
              // Baseline-align the icon with the headline text — the
              // `translate-y-[1px]` nudges it visually onto the x-height
              // instead of floating above the cap-line.
              'size-3.5 shrink-0 translate-y-[1px]',
              tint.icon,
            )}
          />
          <span
            className={cn(
              'text-[13px] font-semibold tracking-tight',
              tint.head,
            )}
          >
            {headline}
          </span>
        </div>
        {body && (
          <div className="mt-1 pl-[22px] whitespace-pre-wrap break-words text-[13px] leading-relaxed text-muted-foreground">
            {body}
          </div>
        )}
      </div>
    </div>
  );
}

function goalChipHeadline(entry: GoalEntry): string {
  const iter = entry.iteration && entry.maxIterations
    ? ` · iteration ${entry.iteration} of ${entry.maxIterations}`
    : '';
  switch (entry.variant) {
    case 'set':
      return 'Goal set';
    case 'cleared':
      return 'Goal cleared';
    case 'progress':
      return `Goal · still working${iter}`;
    case 'done': {
      const status = entry.status;
      if (status === 'met') return 'Goal met';
      if (status === 'impossible') return 'Goal is impossible';
      if (status === 'needs_user') return 'Goal needs your input';
      if (status === 'exhausted') return 'Goal hit its iteration cap';
      if (status === 'cleared') return 'Goal cleared';
      return 'Goal finished';
    }
  }
}

/** Icon + headline color per (variant, status). No bg / bar / border
 *  fields — those live in the render function above and are always
 *  transparent by design. */
function goalChipTint(
  status: GoalEntry['status'],
  variant: GoalEntry['variant'],
) {
  if (variant === 'set' || variant === 'progress') {
    return { icon: 'text-mira-purple', head: 'text-mira-purple' };
  }
  if (variant === 'cleared' || status === 'cleared') {
    return { icon: 'text-muted-foreground', head: 'text-muted-foreground' };
  }
  switch (status) {
    case 'met':
      return { icon: 'text-emerald-400', head: 'text-emerald-300' };
    case 'impossible':
      return { icon: 'text-red-400', head: 'text-red-300' };
    case 'needs_user':
    case 'exhausted':
      return { icon: 'text-amber-400', head: 'text-amber-300' };
    default:
      return { icon: 'text-mira-purple', head: 'text-mira-purple' };
  }
}
