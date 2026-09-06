import { useEffect, useMemo, useRef, useState } from 'react';
import { CaretDown, CaretRight } from '@phosphor-icons/react';
import { cn } from './lib/utils';
import { connect, type WsClient, type WsStatus } from './ws';
import { getSettings, newSession, startReview } from './api';
import { SettingsPanel } from './components/Settings';
import { Sidebar } from './components/Sidebar';
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
import type {
  DiffPreview,
  Message,
  Mode,
  ServerMsg,
  SettingsView,
  ToolCall,
  ToolResult,
} from './types';

type ToolEntry = {
  kind: 'tool';
  call: ToolCall;
  preview: DiffPreview | null;
  status: ToolStatus;
  result: ToolResult | null;
};
type WarningEntry = { kind: 'warning'; text: string };
type ErrorEntry = { kind: 'error'; text: string };
type MsgEntry = { kind: 'msg'; msg: Message };
export type Entry = MsgEntry | ToolEntry | WarningEntry | ErrorEntry;

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
function historyToEntries(history: Message[]): Entry[] {
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
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [sidebarRefresh, setSidebarRefresh] = useState(0);
  const [reviewPanelOpen, setReviewPanelOpen] = useState(false);
  const [reviewState, setReviewState] = useState<ReviewState | null>(null);
  // Force a re-render every second while a turn is active so the live
  // "Working…" counter ticks. Cheap; the tree is small and only mounts
  // when the browser tab is visible.
  const [, setNowTick] = useState(0);
  const wsRef = useRef<WsClient | null>(null);
  const paneRef = useRef<HTMLDivElement | null>(null);

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
        setBusy(false);
        setThinking(false);
        setSidebarRefresh((n) => n + 1);
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
        setEntries((prev) => upsertToolStart(prev, msg.call));
        break;
      case 'tool_end':
        // The model usually starts thinking again after a tool result comes
        // back before the next text token arrives.
        setThinking(true);
        setEntries((prev) => attachToolResult(prev, msg.result));
        break;
      case 'turn_complete':
        // A turn ended (assistant round complete). More may follow if there
        // were tool calls; if not, `done` will clear us right after.
        setThinking(true);
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
    }
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

  function decideApproval(callId: string, allow: boolean) {
    wsRef.current?.send({ type: 'approve', call_id: callId, allow });
    setEntries((prev) => updateTool(prev, callId, (t) => ({
      ...t,
      status: allow ? 'running' : 'denied',
    })));
  }

  function onSend(text: string) {
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

  function toggleTurn(idx: number) {
    setExpandedTurns((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx); else next.add(idx);
      return next;
    });
  }

  return (
    <div className="grid h-screen grid-cols-[260px_1fr] bg-background">
      <Sidebar
        status={status}
        cwd={cwd}
        activeSessionId={sessionId}
        refreshKey={sidebarRefresh}
        onNewChat={onNewChat}
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenPicker={() => setPickerOpen(true)}
        onSessionLoaded={() => { /* Ready broadcast refreshes */ }}
      />

      <main className="flex min-w-0 min-h-0 flex-col">
        <div className="flex min-h-[44px] items-center gap-3 border-b border-border/60 px-4 py-2">
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
            <div className="mx-auto flex max-w-3xl flex-col gap-4">
              {turns.map((turn, i) => (
                <TurnView
                  key={`turn-${i}`}
                  turn={turn}
                  timing={turnTimings.get(i) ?? null}
                  expanded={expandedTurns.has(i)}
                  onToggle={() => toggleTurn(i)}
                  onDecide={decideApproval}
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
          onSend={onSend}
          onSetMode={onSetMode}
          onSetModel={onSetModel}
          onSetEffort={onSetEffort}
          onOpenPicker={() => setPickerOpen(true)}
          onInterrupt={() => wsRef.current?.send({ type: 'interrupt' })}
          onNewChat={onNewChat}
          onOpenSettings={() => setSettingsOpen(true)}
          onRunReview={runReview}
        />
      </main>

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

function updateTool(prev: Entry[], callId: string, f: (t: ToolEntry) => ToolEntry): Entry[] {
  for (let i = prev.length - 1; i >= 0; i--) {
    const e = prev[i];
    if (e.kind === 'tool' && e.call.id === callId) {
      return [...prev.slice(0, i), f(e), ...prev.slice(i + 1)];
    }
  }
  return prev;
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
  turn, timing, expanded, isActive, onToggle, onDecide,
}: {
  turn: Turn;
  timing: TurnTiming | null;
  expanded: boolean;
  isActive: boolean;
  onToggle: () => void;
  onDecide: (callId: string, allow: boolean) => void;
}) {
  // Split the body into "intermediate work" and the final assistant text.
  // Rule: the LAST assistant text message with non-empty content is the
  // final answer; everything before it is intermediate. Tool cards + earlier
  // assistant text hide behind the "Worked for" chip when collapsed.
  const finalIdx = findFinalAssistantIndex(turn.body);
  const intermediate = finalIdx >= 0 ? turn.body.slice(0, finalIdx) : turn.body;
  const finalEntry = finalIdx >= 0 ? turn.body[finalIdx] : null;
  const trailing = finalIdx >= 0 ? turn.body.slice(finalIdx + 1) : [];

  const durationMs = timing
    ? (timing.endedAt ?? Date.now()) - timing.startedAt
    : null;
  const showWorkedChip = (intermediate.length > 0 || isActive) && durationMs != null;

  // While a turn is in flight, force the intermediate section open so
  // in-progress tool calls (esp. pending approval bubbles) stay visible.
  // The user can collapse it after `done` fires. A pending approval
  // requires a click to keep the model moving; hiding it would deadlock.
  const hasPendingApproval = intermediate.some(
    (e) => e.kind === 'tool' && e.status === 'pending',
  );
  const forceOpen = isActive || hasPendingApproval;
  const effectivelyExpanded = expanded || forceOpen;

  return (
    <>
      {turn.user && <EntryView entry={turn.user} onDecide={onDecide} />}

      {showWorkedChip && (
        <WorkedForChip
          durationMs={durationMs!}
          active={isActive && timing?.endedAt == null}
          expanded={effectivelyExpanded}
          locked={forceOpen}
          onToggle={onToggle}
        />
      )}

      {(effectivelyExpanded || !showWorkedChip) && intermediate.map((e, i) => (
        <EntryView key={`t-i-${i}`} entry={e} onDecide={onDecide} />
      ))}

      {finalEntry && <EntryView entry={finalEntry} onDecide={onDecide} />}
      {trailing.map((e, i) => (
        <EntryView key={`t-t-${i}`} entry={e} onDecide={onDecide} />
      ))}
    </>
  );
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
      {expanded
        ? <CaretDown className="size-3 text-muted-foreground/60" />
        : <CaretRight className="size-3 text-muted-foreground/60" />}
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

function EntryView({
  entry,
  onDecide,
}: {
  entry: Entry;
  onDecide: (callId: string, allow: boolean) => void;
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
    case 'warning':
      return (
        <div className="flex justify-start">
          <div className="font-mono text-xs text-mira-tool">! {entry.text}</div>
        </div>
      );
    case 'error':
      return (
        <div className="flex justify-start">
          <div className="font-mono text-xs text-destructive">error: {entry.text}</div>
        </div>
      );
  }
}
