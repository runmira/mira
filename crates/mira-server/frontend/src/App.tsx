import { useEffect, useMemo, useRef, useState } from 'react';
import { connect, type WsClient, type WsStatus } from './ws';
import { getSettings, newSession } from './api';
import { SettingsPanel } from './components/Settings';
import { Sidebar } from './components/Sidebar';
import { Composer } from './components/Composer';
import { FolderPicker } from './components/FolderPicker';
import { AssistantContent } from './components/AssistantContent';
import { ToolCard, type ToolStatus } from './components/ToolCard';
import { Thinking } from './components/Thinking';
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

function historyToEntries(history: Message[]): Entry[] {
  return history
    .filter((m) => m.role !== 'system')
    .map((msg) => ({ kind: 'msg', msg }) as Entry);
}

export default function App() {
  const [status, setStatus] = useState<WsStatus>('connecting');
  const [sessionId, setSessionId] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [mode, setMode] = useState<Mode>('manual');
  const [cwd, setCwd] = useState<string>('');
  const [entries, setEntries] = useState<Entry[]>([]);
  const [busy, setBusy] = useState<boolean>(false);
  const [thinking, setThinking] = useState<boolean>(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [sidebarRefresh, setSidebarRefresh] = useState(0);
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

  function onMessage(msg: ServerMsg) {
    switch (msg.type) {
      case 'ready':
        setSessionId(msg.session_id);
        setModel(msg.model);
        setMode(msg.mode);
        setCwd(msg.cwd);
        setEntries(historyToEntries(msg.history));
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
    setEntries((prev) => [...prev, { kind: 'msg', msg: { role: 'user', content: text } }]);
    setBusy(true);
    setThinking(true);
    wsRef.current?.send({ type: 'send', text });
  }

  function onSetMode(m: Mode) { wsRef.current?.send({ type: 'set_mode', mode: m }); }
  function onSetModel(m: string) { wsRef.current?.send({ type: 'set_model', model: m }); }

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

  return (
    <div className="app">
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

      <main className="main">
        <div className="topbar">
          <span className="title">{titleFromEntries(entries)}</span>
          <div className="seg">
            <button className="on" type="button">Chat</button>
            <button className="disabled" type="button" disabled title="Not implemented yet">Work</button>
          </div>
        </div>

        <div className="pane" ref={paneRef}>
          {configured === false && (
            <div className="notice">
              No provider configured — <button className="link" onClick={() => setSettingsOpen(true)}>open Settings</button> to add one.
            </div>
          )}

          {isEmpty ? (
            <EmptyState />
          ) : (
            <div className="transcript">
              {entries.map((e, i) => (
                <EntryView key={entryKey(e, i)} entry={e} onDecide={decideApproval} />
              ))}
              {thinking && (
                <div className="row row-left">
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
          onOpenPicker={() => setPickerOpen(true)}
          onInterrupt={() => wsRef.current?.send({ type: 'interrupt' })}
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

function entryKey(e: Entry, i: number): string {
  if (e.kind === 'tool') return `tool-${e.call.id}`;
  return `e-${i}`;
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

function EmptyState() {
  return (
    <div className="empty">
      <div className="logo" />
      <div className="prompt">What should we build today?</div>
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
          <div className="row row-right">
            <div className="bubble bubble-user">{content}</div>
          </div>
        );
      }
      return (
        <div className="row row-left">
          <div className="assistant-body"><AssistantContent text={content ?? ''} /></div>
        </div>
      );
    }
    case 'tool':
      return (
        <div className="row row-left">
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
      return <div className="row row-left"><div className="warning">! {entry.text}</div></div>;
    case 'error':
      return <div className="row row-left"><div className="error">error: {entry.text}</div></div>;
  }
}
