import { useCallback, useEffect, useRef, useState } from 'react';
import { Plus, TerminalWindow, X } from '@phosphor-icons/react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { cn } from '@/lib/utils';

/**
 * Bottom terminal panel: real shells (a PTY on the server) in the
 * session folder, rendered with xterm.js. Shells outlive the page — tab
 * ids are remembered, so a reload reattaches and replays recent output.
 */

type Tab = { key: string; id: string | null; title: string };

const TABS_KEY = 'mira.terminal.tabs';
const HEIGHT_KEY = 'mira.terminal.height';

function loadTabs(): Tab[] {
  try {
    const ids = JSON.parse(localStorage.getItem(TABS_KEY) ?? '[]') as string[];
    return ids.map((id, i) => ({ key: id, id, title: `Terminal ${i + 1}` }));
  } catch {
    return [];
  }
}

function saveTabs(tabs: Tab[]) {
  try {
    localStorage.setItem(TABS_KEY, JSON.stringify(tabs.map((t) => t.id).filter(Boolean)));
  } catch { /* private mode */ }
}

export function TerminalPanel({ onClose }: { onClose: () => void }) {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [height, setHeight] = useState(() => {
    const h = Number(localStorage.getItem(HEIGHT_KEY));
    return Number.isFinite(h) && h >= 120 ? h : 280;
  });
  const counter = useRef(0);
  // Don't persist until the saved tabs have been restored — saving the
  // initial empty list would forget every shell on each reload.
  const restored = useRef(false);

  const newTab = useCallback(() => {
    counter.current += 1;
    const key = `new-${Date.now()}-${counter.current}`;
    setTabs((prev) => [...prev, { key, id: null, title: `Terminal ${prev.length + 1}` }]);
    setActive(key);
  }, []);

  // Reattach to shells still alive on the server; start one if none.
  useEffect(() => {
    fetch('/api/terminals')
      .then(async (r) => {
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(j.error ?? `terminals ${r.status}`);
        const live = new Set<string>((j.terminals ?? []).map((t: { id: string }) => t.id));
        const saved = loadTabs().filter((t) => t.id && live.has(t.id));
        restored.current = true;
        if (saved.length > 0) {
          setTabs(saved);
          setActive(saved[saved.length - 1].key);
        } else {
          newTab();
        }
      })
      .catch((e) => setError((e as Error).message));
  }, [newTab]);

  useEffect(() => {
    if (restored.current) saveTabs(tabs);
  }, [tabs]);

  function closeTab(tab: Tab) {
    if (tab.id) void fetch(`/api/terminals/${tab.id}`, { method: 'DELETE' });
    setTabs((prev) => {
      const next = prev.filter((t) => t.key !== tab.key);
      if (active === tab.key) setActive(next[next.length - 1]?.key ?? null);
      if (next.length === 0) onClose();
      return next;
    });
  }

  function startResize(e: React.PointerEvent) {
    e.preventDefault();
    const startY = e.clientY;
    const startH = height;
    const move = (ev: PointerEvent) => {
      const h = Math.min(window.innerHeight * 0.8, Math.max(120, startH + startY - ev.clientY));
      setHeight(h);
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      setHeight((h) => {
        try { localStorage.setItem(HEIGHT_KEY, String(Math.round(h))); } catch { /* ignore */ }
        return h;
      });
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  return (
    <div className="relative flex shrink-0 flex-col border-t border-border bg-[#0b0b0d]" style={{ height }}>
      <div
        onPointerDown={startResize}
        className="absolute inset-x-0 -top-1 z-10 h-2 cursor-row-resize"
        title="Drag to resize"
      />
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border px-2">
        <TerminalWindow className="mr-1 size-3.5 text-muted-foreground" />
        {tabs.map((t) => (
          <div
            key={t.key}
            className={cn(
              'group flex items-center gap-1 rounded-md px-2 py-1 text-[12px]',
              t.key === active ? 'bg-secondary text-foreground' : 'text-muted-foreground hover:text-foreground',
            )}
          >
            <button type="button" onClick={() => setActive(t.key)}>{t.title}</button>
            <button
              type="button"
              aria-label={`Close ${t.title}`}
              onClick={() => closeTab(t)}
              className="rounded p-0.5 opacity-50 hover:bg-accent/60 hover:opacity-100"
            >
              <X className="size-3" />
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={newTab}
          disabled={!!error}
          aria-label="New terminal"
          className="rounded-md p-1 text-muted-foreground hover:bg-secondary hover:text-foreground disabled:opacity-30"
        >
          <Plus className="size-3.5" />
        </button>
        <button
          type="button"
          onClick={onClose}
          aria-label="Hide terminal"
          title="Hide terminal (⌘J)"
          className="ml-auto rounded-md p-1 text-muted-foreground hover:bg-secondary hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      </div>
      <div className="relative min-h-0 flex-1">
        {error && <div className="p-4 text-[12.5px] text-muted-foreground">{error}</div>}
        {tabs.map((t) => (
          <TerminalView
            key={t.key}
            termId={t.id}
            visible={t.key === active}
            onId={(id) => setTabs((prev) => prev.map((x) => (x.key === t.key ? { ...x, id } : x)))}
          />
        ))}
      </div>
    </div>
  );
}

function TerminalView({
  termId,
  visible,
  onId,
}: {
  termId: string | null;
  visible: boolean;
  onId: (id: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const termRef = useRef<Terminal | null>(null);
  // Only the first attach decides the id; later renders mustn't reconnect.
  const idRef = useRef(termId);
  const onIdRef = useRef(onId);
  onIdRef.current = onId;

  useEffect(() => {
    const term = new Terminal({
      fontFamily: "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, monospace",
      fontSize: 12.5,
      cursorBlink: true,
      scrollback: 5000,
      theme: { background: '#0b0b0d', foreground: '#d4d4d8', cursor: '#d4d4d8', selectionBackground: '#3f3f46' },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current!);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;

    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const params = new URLSearchParams({ cols: String(term.cols), rows: String(term.rows) });
    if (idRef.current) params.set('id', idRef.current);
    const ws = new WebSocket(`${proto}//${location.host}/ws/terminal?${params}`);
    ws.binaryType = 'arraybuffer';
    const enc = new TextEncoder();

    ws.onmessage = (ev) => {
      if (typeof ev.data === 'string') {
        const msg = JSON.parse(ev.data) as { type: string; id?: string; code?: number | null };
        if (msg.type === 'ready' && msg.id) {
          idRef.current = msg.id;
          onIdRef.current(msg.id);
        } else if (msg.type === 'exit') {
          term.write(`\r\n\x1b[2m[process exited${msg.code != null ? ` with code ${msg.code}` : ''}]\x1b[0m\r\n`);
        }
      } else {
        term.write(new Uint8Array(ev.data as ArrayBuffer));
      }
    };
    ws.onclose = () => term.write('\r\n\x1b[2m[disconnected]\x1b[0m\r\n');
    const data = term.onData((d) => {
      if (ws.readyState === WebSocket.OPEN) ws.send(enc.encode(d));
    });
    const resize = term.onResize(({ cols, rows }) => {
      if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: 'resize', cols, rows }));
    });
    const ro = new ResizeObserver(() => {
      if (hostRef.current?.offsetParent) fit.fit();
    });
    ro.observe(hostRef.current!);

    return () => {
      ro.disconnect();
      data.dispose();
      resize.dispose();
      ws.close();
      term.dispose();
    };
  }, []);

  useEffect(() => {
    if (visible) {
      fitRef.current?.fit();
      termRef.current?.focus();
    }
  }, [visible]);

  return (
    <div
      ref={hostRef}
      className={cn('absolute inset-0 px-2 py-1', !visible && 'invisible')}
    />
  );
}
