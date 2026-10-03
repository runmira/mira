import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { Download, RefreshCw, Search, Trash2 } from 'lucide-react';
import {
  clear as clearNet,
  downloadCsv,
  getEntries,
  subscribe,
  type NetEntry,
} from '@/lib/netLog';
import { cn } from '@/lib/utils';

type UsageRow = {
  session_id: string;
  title: string | null;
  cwd: string;
  model: string;
  day: string;
  prompt_tokens: number;
  completion_tokens: number;
  cached_input_tokens: number;
};

type View = 'tokens' | 'network';

export function DevToolsPane() {
  const [view, setView] = useState<View>('tokens');
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-1 border-b border-border/60 px-2.5 py-1.5">
        <ViewTab active={view === 'tokens'} onClick={() => setView('tokens')}>
          Tokens
        </ViewTab>
        <ViewTab active={view === 'network'} onClick={() => setView('network')}>
          Network
        </ViewTab>
      </div>
      {view === 'tokens' ? <TokenTable /> : <NetworkLog />}
    </div>
  );
}

function ViewTab({
  children,
  active,
  onClick,
}: {
  children: React.ReactNode;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'rounded-md px-2.5 py-1 text-[12px] transition-colors',
        active
          ? 'bg-fg/[0.1] text-foreground'
          : 'text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      {children}
    </button>
  );
}

/* ------------------------------------------------------------------ */
/* Tokens                                                              */
/* ------------------------------------------------------------------ */

function TokenTable() {
  const [rows, setRows] = useState<UsageRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [days, setDays] = useState(7);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let live = true;
    setError(null);
    fetch(`/api/usage?days=${days}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d) => {
        if (live) setRows(Array.isArray(d?.rows) ? d.rows : []);
      })
      .catch((e: Error) => {
        if (live) {
          setRows([]);
          setError(e.message);
        }
      });
    return () => {
      live = false;
    };
  }, [days, nonce]);

  // Aggregate per model — the same model shows up once per session per day,
  // so the raw rows are noise for a totals view.
  const byModel = useMemo(() => {
    const m = new Map<
      string,
      { prompt: number; completion: number; cached: number; sessions: Set<string> }
    >();
    for (const r of rows ?? []) {
      const cur =
        m.get(r.model) ??
        { prompt: 0, completion: 0, cached: 0, sessions: new Set<string>() };
      cur.prompt += r.prompt_tokens;
      cur.completion += r.completion_tokens;
      cur.cached += r.cached_input_tokens;
      cur.sessions.add(r.session_id);
      m.set(r.model, cur);
    }
    return [...m.entries()].sort(
      (a, b) =>
        b[1].prompt + b[1].completion - (a[1].prompt + a[1].completion),
    );
  }, [rows]);

  const totals = useMemo(
    () =>
      byModel.reduce(
        (acc, [, v]) => ({
          prompt: acc.prompt + v.prompt,
          completion: acc.completion + v.completion,
          cached: acc.cached + v.cached,
        }),
        { prompt: 0, completion: 0, cached: 0 },
      ),
    [byModel],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-1.5 border-b border-border/40 px-2.5 py-1.5">
        <select
          value={days}
          onChange={(e) => setDays(Number(e.target.value))}
          className="rounded-md border border-border/60 bg-transparent px-1.5 py-0.5 text-[12px] text-foreground"
        >
          <option value={1}>Last day</option>
          <option value={7}>Last 7 days</option>
          <option value={30}>Last 30 days</option>
          <option value={90}>Last 90 days</option>
        </select>
        <button
          type="button"
          title="Refresh"
          onClick={() => setNonce((n) => n + 1)}
          className="flex size-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
        >
          <RefreshCw className="size-3.5" />
        </button>
        <span className="ml-auto text-[11px] text-muted-foreground/70">
          per model, cached input counted separately
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {error && (
          <p className="p-3 text-[12.5px] text-destructive">Failed: {error}</p>
        )}
        {!error && rows === null && (
          <p className="p-3 text-[12.5px] text-muted-foreground">Loading…</p>
        )}
        {rows !== null && byModel.length === 0 && !error && (
          <p className="p-3 text-[12.5px] text-muted-foreground">
            No recorded usage in this window.
          </p>
        )}

        {byModel.length > 0 && (
          <table className="w-full border-collapse text-[12px]">
            <thead className="sticky top-0 bg-background">
              <tr className="border-b border-border/60 text-left text-[11px] uppercase tracking-wider text-muted-foreground/80">
                <th className="px-2.5 py-1.5 font-medium">Model</th>
                <th className="px-2.5 py-1.5 text-right font-medium">In</th>
                <th className="px-2.5 py-1.5 text-right font-medium">Cached</th>
                <th className="px-2.5 py-1.5 text-right font-medium">Out</th>
                <th className="px-2.5 py-1.5 text-right font-medium">Total</th>
              </tr>
            </thead>
            <tbody className="font-mono tabular-nums">
              {byModel.map(([model, v]) => (
                <tr key={model} className="border-b border-border/30 last:border-0">
                  <td className="max-w-[180px] truncate px-2.5 py-1.5 font-sans text-foreground/90">
                    {model}
                    <span className="ml-1.5 text-[10.5px] text-muted-foreground/60">
                      {v.sessions.size} session{v.sessions.size === 1 ? '' : 's'}
                    </span>
                  </td>
                  <td className="px-2.5 py-1.5 text-right">{n(v.prompt)}</td>
                  <td className="px-2.5 py-1.5 text-right text-mira-cyan/80">
                    {v.cached ? n(v.cached) : '—'}
                  </td>
                  <td className="px-2.5 py-1.5 text-right">{n(v.completion)}</td>
                  <td className="px-2.5 py-1.5 text-right text-foreground/90">
                    {n(v.prompt + v.completion)}
                  </td>
                </tr>
              ))}
            </tbody>
            <tfoot className="sticky bottom-0 bg-background font-mono tabular-nums">
              <tr className="border-t border-border/60 text-foreground">
                <td className="px-2.5 py-1.5 font-sans font-medium">Total</td>
                <td className="px-2.5 py-1.5 text-right">{n(totals.prompt)}</td>
                <td className="px-2.5 py-1.5 text-right text-mira-cyan/80">
                  {n(totals.cached)}
                </td>
                <td className="px-2.5 py-1.5 text-right">{n(totals.completion)}</td>
                <td className="px-2.5 py-1.5 text-right">
                  {n(totals.prompt + totals.completion)}
                </td>
              </tr>
            </tfoot>
          </table>
        )}
      </div>
    </div>
  );
}

function n(v: number): string {
  return v.toLocaleString();
}

/* ------------------------------------------------------------------ */
/* Network                                                             */
/* ------------------------------------------------------------------ */

const KIND_FILTERS: { id: NetEntry['kind'] | 'all'; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'api', label: 'API' },
  { id: 'asset', label: 'Assets' },
  { id: 'other', label: 'Other' },
];

function NetworkLog() {
  // Re-render on every recorded request.
  const entries = useSyncExternalStore(subscribe, getEntries, getEntries);
  const [kind, setKind] = useState<NetEntry['kind'] | 'all'>('api');
  const [q, setQ] = useState('');
  const [expanded, setExpanded] = useState<number | null>(null);

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return entries.filter(
      (e) =>
        (kind === 'all' || e.kind === kind) &&
        (!needle ||
          e.path.toLowerCase().includes(needle) ||
          e.method.toLowerCase().includes(needle)),
    );
  }, [entries, kind, q]);

  // Newest first — you're almost always looking at what just happened.
  const rows = [...filtered].reverse();

  const stats = useMemo(() => {
    const failed = filtered.filter((e) => e.error || (e.status !== null && e.status >= 400));
    const done = filtered.filter((e) => e.durationMs !== null);
    const avg = done.length
      ? Math.round(done.reduce((a, e) => a + (e.durationMs ?? 0), 0) / done.length)
      : 0;
    return { failed: failed.length, avg };
  }, [filtered]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border/40 px-2.5 py-1.5">
        <div className="flex items-center gap-1 rounded-md border border-border/60 px-1.5 py-0.5 focus-within:border-border">
          <Search className="size-3 shrink-0 text-muted-foreground" />
          <input
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Filter path or method"
            className="w-32 bg-transparent text-[12px] text-foreground outline-none placeholder:text-muted-foreground/60"
          />
        </div>
        {KIND_FILTERS.map((f) => (
          <button
            key={f.id}
            type="button"
            onClick={() => setKind(f.id)}
            className={cn(
              'rounded-md px-1.5 py-0.5 text-[11.5px] transition-colors',
              kind === f.id
                ? 'bg-fg/[0.1] text-foreground'
                : 'text-muted-foreground hover:bg-fg/[0.05] hover:text-foreground',
            )}
          >
            {f.label}
          </button>
        ))}
        <span className="flex-1" />
        <span className="text-[11px] tabular-nums text-muted-foreground/70">
          {filtered.length} · avg {stats.avg}ms
          {stats.failed > 0 && (
            <span className="ml-1.5 text-destructive">{stats.failed} failed</span>
          )}
        </span>
        <button
          type="button"
          title="Export CSV"
          onClick={downloadCsv}
          className="flex size-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
        >
          <Download className="size-3.5" />
        </button>
        <button
          type="button"
          title="Clear log"
          onClick={clearNet}
          className="flex size-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {rows.length === 0 && (
          <p className="p-3 text-[12.5px] text-muted-foreground">
            Nothing logged yet. Requests appear here as the app makes them.
          </p>
        )}
        {rows.map((e) => (
          <div key={e.id} className="border-b border-border/30 last:border-0">
            <button
              type="button"
              onClick={() => setExpanded(expanded === e.id ? null : e.id)}
              className="flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[11.5px] hover:bg-fg/[0.03]"
            >
              <span className="w-11 shrink-0 text-muted-foreground/80">
                {e.method}
              </span>
              <span className="min-w-0 flex-1 truncate text-foreground/90">
                {e.path}
              </span>
              <StatusPill e={e} />
              <span className="w-12 shrink-0 text-right tabular-nums text-muted-foreground/70">
                {e.durationMs === null ? '…' : `${e.durationMs}ms`}
              </span>
            </button>
            {expanded === e.id && (
              <dl className="grid grid-cols-[88px_minmax(0,1fr)] gap-x-3 gap-y-0.5 bg-shade/20 px-2.5 py-2 font-mono text-[11px]">
                <dt className="text-muted-foreground/70">url</dt>
                <dd className="break-all text-foreground/80">{e.url}</dd>
                <dt className="text-muted-foreground/70">started</dt>
                <dd className="text-foreground/80">
                  {new Date(e.startedAt).toLocaleTimeString()}
                </dd>
                <dt className="text-muted-foreground/70">status</dt>
                <dd className="text-foreground/80">{e.status ?? '—'}</dd>
                <dt className="text-muted-foreground/70">bytes</dt>
                <dd className="text-foreground/80">
                  {e.bytes === null ? '—' : n(e.bytes)}
                </dd>
                {e.error && (
                  <>
                    <dt className="text-muted-foreground/70">error</dt>
                    <dd className="break-words text-destructive">{e.error}</dd>
                  </>
                )}
              </dl>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function StatusPill({ e }: { e: NetEntry }) {
  if (e.error) {
    return <Pill className="bg-destructive/15 text-destructive">error</Pill>;
  }
  if (e.status === null) {
    return <Pill className="bg-mira-blue/15 text-mira-blue">…</Pill>;
  }
  if (e.status >= 500) {
    return <Pill className="bg-destructive/15 text-destructive">{e.status}</Pill>;
  }
  if (e.status >= 400) {
    return <Pill className="bg-amber-500/15 text-amber-400">{e.status}</Pill>;
  }
  return <Pill className="bg-emerald-500/15 text-emerald-400">{e.status}</Pill>;
}

function Pill({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        'w-12 shrink-0 rounded px-1 py-0.5 text-center text-[10.5px] tabular-nums',
        className,
      )}
    >
      {children}
    </span>
  );
}
