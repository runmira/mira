/**
 * Processes: what the agent left running (dev servers, watchers, long
 * builds started with `run_background`) and what's listening on this
 * machine's ports. Stop, restart, read the logs, hand them to the agent,
 * or preview a dev server on devices.
 *
 * Polls `/api/processes` while open; the expanded process polls its
 * output. Both stop when the pane unmounts.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ChevronRight,
  CornerDownLeft,
  ExternalLink,
  Play,
  RotateCw,
  Smartphone,
  Square,
  SquareTerminal,
  X,
} from 'lucide-react';
import { composeText } from '@/lib/attachBridge';
import {stopBackgroundProcess, type BackgroundProcess} from '@/lib/backgroundProcesses';
import { openExternal } from '@/lib/desktop';
import { sessionQuery } from '@/lib/ndjson';
import { setPreviewUrl } from '@/lib/previewUrl';
import { cn } from '@/lib/utils';
import { Collapse } from '../ui/Collapse';
import { PaneBar, PaneEmpty, PaneIconButton, PaneSection, StatusDot } from './paneUi';

type Proc = BackgroundProcess;

type Port = {
  port: number;
  pid: number;
  command: string;
  address: string;
  process_id: number | null;
  system: boolean;
};

type ListView = { processes: Proc[]; ports: Port[]; self_port: number };

async function post(url: string, body: unknown = {}) {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const j = (await r.json().catch(() => ({}))) as { error?: string };
  if (!r.ok) throw new Error(j.error ?? `HTTP ${r.status}`);
  return j;
}

export function ProcessesPane({
  sessionId,
  onPreview,
  selectedProcess,
}: {
  sessionId: string;
  selectedProcess?:{id:number;nonce:number};
  /** Open Device preview (the URL is already set). */
  onPreview: () => void;
}) {
  const [data, setData] = useState<ListView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const [command, setCommand] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [showApps, setShowApps] = useState(false);
  const q = sessionQuery(sessionId);
  const selectedRow=useRef<HTMLDivElement|null>(null);
  const revealedSelection=useRef<number|null>(null);
  useEffect(()=>{if(selectedProcess)setOpen(selectedProcess.id);},[selectedProcess]);
  useEffect(()=>{
    if(selectedProcess && selectedRow.current && revealedSelection.current!==selectedProcess.nonce){
      selectedRow.current.scrollIntoView({block:'nearest'});revealedSelection.current=selectedProcess.nonce;
    }
  },[selectedProcess,data]);

  const refresh = useCallback(async () => {
    try {
      const r = await fetch(`/api/processes${q}`);
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      setData((await r.json()) as ListView);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [q]);

  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), 2000);
    return () => clearInterval(t);
  }, [refresh]);

  async function act(key: string, run: () => Promise<unknown>) {
    setBusy(key);
    try {
      await run();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
      void refresh();
    }
  }

  async function start() {
    const c = command.trim();
    if (!c) return;
    await act('start', async () => {
      const r = (await post(`/api/processes${q}`, { command: c })) as { id?: number };
      setCommand('');
      if (r.id) setOpen(r.id);
    });
  }

  function preview(url: string) {
    setPreviewUrl(url);
    onPreview();
  }

  const procs = data?.processes ?? [];
  const unowned = (data?.ports ?? []).filter((p) => p.process_id === null && p.port !== data?.self_port);
  const apps = unowned.filter((p) => p.system).length;
  const others = showApps ? unowned : unowned.filter((p) => !p.system);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PaneBar>
        <div className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md border border-border/70 bg-background/60 px-2 focus-within:border-mira-blue/50">
          <SquareTerminal className="size-3.5 shrink-0 text-muted-foreground/70" />
          <input
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void start();
            }}
            placeholder="Run in the background… (npm run dev)"
            className="min-w-0 flex-1 bg-transparent font-mono text-[12px] text-foreground outline-none placeholder:font-sans placeholder:text-muted-foreground/60"
          />
        </div>
        <PaneIconButton title="Run" onClick={() => void start()} disabled={!command.trim() || busy === 'start'}>
          <Play className="size-3.5" />
        </PaneIconButton>
      </PaneBar>

      {error && (
        <div className="flex items-center gap-2 border-b border-red-500/20 bg-red-500/[0.06] px-3 py-1.5 text-[11.5px] text-red-600 dark:text-red-300">
          <span className="min-w-0 flex-1 truncate">{error}</span>
          <button type="button" onClick={() => setError(null)} className="shrink-0 opacity-70 hover:opacity-100">
            <X className="size-3" />
          </button>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto pb-3">
        {data === null ? null : procs.length === 0 && others.length === 0 ? (
          <PaneEmpty icon={<SquareTerminal className="size-4" />} title="Nothing running">
            Dev servers and watchers the agent starts show up here, with their ports and logs. You can
            start one yourself above.
          </PaneEmpty>
        ) : (
          <>
            <PaneSection title="Started in this chat" />
            {procs.length === 0 ? (
              <p className="px-3 text-[12px] text-muted-foreground/80">
                None yet. When the agent runs something in the background, it lands here.
              </p>
            ) : (
              <div className="flex flex-col gap-1.5 px-2">
                {procs.map((p) => (
                  <div key={p.id} ref={p.id===selectedProcess?.id?selectedRow:undefined}><ProcessRow
                    proc={p}
                    q={q}
                    expanded={open === p.id}
                    onToggle={() => setOpen(open === p.id ? null : p.id)}
                    busy={busy?.endsWith(`:${p.id}`) ?? false}
                    onStop={() => void act(`stop:${p.id}`, () => stopBackgroundProcess(sessionId,p.id))}
                    onRestart={() =>
                      void act(`restart:${p.id}`, async () => {
                        const r = (await post(`/api/processes/${p.id}/restart${q}`)) as { id?: number };
                        if (open === p.id && r.id) setOpen(r.id);
                      })
                    }
                    onForget={() =>
                      void act(`forget:${p.id}`, async () => {
                        const r = await fetch(`/api/processes/${p.id}${q}`, { method: 'DELETE' });
                        if (!r.ok) throw new Error(`HTTP ${r.status}`);
                      })
                    }
                    onPreview={preview}
                  /></div>
                ))}
              </div>
            )}

            {unowned.length > 0 && (
              <>
                <PaneSection
                  title="Other listening ports"
                  right={
                    apps > 0 && (
                      <button
                        type="button"
                        onClick={() => setShowApps((v) => !v)}
                        className="text-[11px] text-muted-foreground hover:text-foreground"
                      >
                        {showApps ? 'Hide apps' : `Show apps (${apps})`}
                      </button>
                    )
                  }
                />
                {others.length === 0 && (
                  <p className="px-3 text-[12px] text-muted-foreground/80">Only installed apps are listening.</p>
                )}
                {others.length > 0 && (
                <div className="mx-2 overflow-hidden rounded-lg border border-border/60">
                  {others.map((p, i) => (
                    <div
                      key={`${p.port}:${p.pid}`}
                      className={cn(
                        'group flex items-center gap-2.5 px-2.5 py-1.5 text-[12px]',
                        i > 0 && 'border-t border-border/40',
                      )}
                    >
                      <span className="w-12 shrink-0 font-mono font-medium tabular-nums text-foreground">
                        :{p.port}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-muted-foreground">
                        {p.command}
                        <span className="text-muted-foreground/60"> · pid {p.pid}</span>
                        {p.address !== '*' && <span className="text-muted-foreground/60"> · {p.address}</span>}
                      </span>
                      <div className="flex items-center gap-0.5 opacity-60 transition-opacity group-hover:opacity-100">
                        <PaneIconButton title="Preview on devices" onClick={() => preview(`http://localhost:${p.port}`)}>
                          <Smartphone className="size-3.5" />
                        </PaneIconButton>
                        <PaneIconButton title="Open in browser" onClick={() => void openExternal(`http://localhost:${p.port}`)}>
                          <ExternalLink className="size-3.5" />
                        </PaneIconButton>
                        <PaneIconButton
                          title={`Stop ${p.command} (pid ${p.pid})`}
                          tone="danger"
                          disabled={busy === `port:${p.pid}`}
                          onClick={() => {
                            if (!window.confirm(`Stop ${p.command} (pid ${p.pid}) listening on :${p.port}?`)) return;
                            void act(`port:${p.pid}`, () => post('/api/ports/stop', { pid: p.pid }));
                          }}
                        >
                          <Square className="size-3" />
                        </PaneIconButton>
                      </div>
                    </div>
                  ))}
                </div>
                )}
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function ProcessRow({
  proc: p,
  q,
  expanded,
  onToggle,
  busy,
  onStop,
  onRestart,
  onForget,
  onPreview,
}: {
  proc: Proc;
  q: string;
  expanded: boolean;
  onToggle: () => void;
  busy: boolean;
  onStop: () => void;
  onRestart: () => void;
  onForget: () => void;
  onPreview: (url: string) => void;
}) {
  const failed = !p.running && !p.stopped && p.exit_code !== 0;
  const state = p.running ? 'running' : failed ? 'failed' : 'ok';
  const url = p.url ?? (p.ports[0] ? `http://localhost:${p.ports[0]}` : null);
  const status = p.running
    ? `running ${elapsed(p.elapsed_secs)}`
    : p.stopped
      ? 'stopped'
      : `exited ${p.exit_code ?? '?'}`;

  return (
    <div
      className={cn(
        'overflow-hidden rounded-lg border transition-colors',
        expanded ? 'border-border bg-fg/[0.02]' : 'border-border/60 hover:border-border',
      )}
    >
      <div className="group flex items-center gap-2 px-2.5 py-2">
        <button type="button" onClick={onToggle} className="flex min-w-0 flex-1 items-center gap-2 text-left">
          <ChevronRight
            className={cn('size-3.5 shrink-0 text-muted-foreground/60 transition-transform', expanded && 'rotate-90')}
          />
          <StatusDot state={state} />
          <div className="min-w-0 flex-1">
            <div className="truncate font-mono text-[12px] text-foreground">{p.command}</div>
            <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
              <span className={cn('shrink-0', failed && 'text-red-600 dark:text-red-400')}>{status}</span>
              {p.ports.map((port) => (
                <span key={port} className="shrink-0 rounded bg-emerald-500/10 px-1 font-mono text-emerald-700 dark:text-emerald-300">
                  :{port}
                </span>
              ))}
              {!expanded && p.last_line && (
                <span className="min-w-0 truncate font-mono text-muted-foreground/60">{p.last_line}</span>
              )}
            </div>
          </div>
        </button>
        <div className="flex shrink-0 items-center gap-0.5">
          {url && p.running && (
            <>
              <PaneIconButton title={`Preview ${url} on devices`} onClick={() => onPreview(url)}>
                <Smartphone className="size-3.5" />
              </PaneIconButton>
              <PaneIconButton title={`Open ${url}`} onClick={() => void openExternal(url)}>
                <ExternalLink className="size-3.5" />
              </PaneIconButton>
            </>
          )}
          <PaneIconButton title="Restart" onClick={onRestart} disabled={busy}>
            <RotateCw className={cn('size-3.5', busy && 'animate-spin')} />
          </PaneIconButton>
          {p.running ? (
            <PaneIconButton title="Stop" onClick={onStop} disabled={busy} tone="danger">
              <Square className="size-3" />
            </PaneIconButton>
          ) : (
            <PaneIconButton title="Remove from list" onClick={onForget} disabled={busy}>
              <X className="size-3.5" />
            </PaneIconButton>
          )}
        </div>
      </div>
      <Collapse open={expanded}>
        <ProcessLog id={p.id} q={q} command={p.command} running={p.running} />
      </Collapse>
    </div>
  );
}

function ProcessLog({ id, q, command, running }: { id: number; q: string; command: string; running: boolean }) {
  const [lines, setLines] = useState<string[]>([]);
  const ref = useRef<HTMLPreElement | null>(null);
  const stick = useRef(true);

  useEffect(() => {
    let live = true;
    async function load() {
      try {
        const r = await fetch(`/api/processes/${id}/output${q ? `${q}&` : '?'}tail=500`);
        if (!r.ok) return;
        const j = (await r.json()) as { lines: string[] };
        if (live) setLines(j.lines);
      } catch {
        /* next tick */
      }
    }
    void load();
    if (!running) return () => void (live = false);
    const t = setInterval(load, 1000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [id, q, running]);

  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div className="border-t border-border/50">
      <pre
        ref={ref}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
        className="max-h-72 min-h-[60px] overflow-auto bg-[#fbfbfc] px-3 py-2 font-mono text-[11.5px] leading-[1.55] text-foreground/85 dark:bg-[#0b0b0d]"
      >
        {lines.length ? lines.join('\n') : <span className="text-muted-foreground/60">No output yet.</span>}
      </pre>
      <div className="flex items-center gap-2 border-t border-border/50 px-2.5 py-1.5">
        <span className="text-[11px] text-muted-foreground/70">{lines.length} lines</span>
        <span className="flex-1" />
        <button
          type="button"
          disabled={!lines.length}
          onClick={() =>
            composeText(
              `Output of \`${command}\` (last ${Math.min(lines.length, 80)} lines):\n\n\`\`\`\n${lines.slice(-80).join('\n')}\n\`\`\`\n\n`,
            )
          }
          className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground disabled:opacity-40"
        >
          <CornerDownLeft className="size-3" />
          Send logs to agent
        </button>
      </div>
    </div>
  );
}

function elapsed(s: number): string {
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}
