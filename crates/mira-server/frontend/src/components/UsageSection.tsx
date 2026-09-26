import { useEffect, useMemo, useState } from 'react';
import { Download } from 'lucide-react';
import { cn } from '@/lib/utils';
import { costUsd, formatDollars, shortNum } from '../lib/usage';

/**
 * Settings → Usage: tokens and cost across every stored session, by
 * day, model, project and session. Rows come from `/api/usage` (per
 * session/day/model); pricing uses the same table as the rest of the
 * UI. Unpriced models count tokens but never show a $0 cost.
 */

type Row = {
  session_id: string;
  title: string | null;
  cwd: string;
  model: string;
  day: string;
  prompt_tokens: number;
  completion_tokens: number;
  cached_input_tokens: number;
};

type Agg = { input: number; output: number; cost: number; unpriced: boolean };

const RANGES = [7, 30, 90] as const;
const BAR = '#7aa2f7'; // mira-blue — one series, one hue

function rowCost(r: Row): number | null {
  return costUsd(r.model, {
    prompt_tokens: r.prompt_tokens,
    completion_tokens: r.completion_tokens,
    cached_input_tokens: r.cached_input_tokens,
    rounds: 0,
  });
}

function aggregate<K>(rows: Row[], key: (r: Row) => K): Map<K, Agg> {
  const m = new Map<K, Agg>();
  for (const r of rows) {
    const k = key(r);
    const a = m.get(k) ?? { input: 0, output: 0, cost: 0, unpriced: false };
    a.input += r.prompt_tokens;
    a.output += r.completion_tokens;
    const c = rowCost(r);
    if (c == null) a.unpriced = true;
    else a.cost += c;
    m.set(k, a);
  }
  return m;
}

/** Cost text: never "$0.00" for tokens we couldn't price. */
function costLabel(a: Agg): string {
  if (a.cost > 0) return formatDollars(a.cost) + (a.unpriced ? '+' : '');
  return a.unpriced ? '—' : formatDollars(0);
}

function daysBetween(since: string, n: number): string[] {
  const out: string[] = [];
  const d = new Date(`${since}T00:00:00Z`);
  for (let i = 0; i < n; i++) {
    out.push(d.toISOString().slice(0, 10));
    d.setUTCDate(d.getUTCDate() + 1);
  }
  return out;
}

function basename(p: string) {
  return p.replace(/\/+$/, '').split('/').pop() || p;
}

function downloadCsv(rows: Row[]) {
  const head = 'day,session_id,title,project,model,input_tokens,output_tokens,cached_input_tokens,cost_usd';
  const esc = (v: string) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
  const lines = rows.map((r) => {
    const c = rowCost(r);
    return [
      r.day, r.session_id, esc(r.title ?? ''), esc(r.cwd), esc(r.model),
      r.prompt_tokens, r.completion_tokens, r.cached_input_tokens, c == null ? '' : c.toFixed(6),
    ].join(',');
  });
  const blob = new Blob([[head, ...lines].join('\n')], { type: 'text/csv' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = `mira-usage-${new Date().toISOString().slice(0, 10)}.csv`;
  a.click();
  URL.revokeObjectURL(a.href);
}

export function UsageSection() {
  const [days, setDays] = useState<(typeof RANGES)[number]>(30);
  const [data, setData] = useState<{ since: string; rows: Row[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [metric, setMetric] = useState<'cost' | 'tokens'>('cost');

  useEffect(() => {
    setData(null);
    setError(null);
    fetch(`/api/usage?days=${days}`)
      .then(async (r) => {
        const j = await r.json();
        if (!r.ok) throw new Error(j.error ?? `usage ${r.status}`);
        setData(j);
      })
      .catch((e) => setError((e as Error).message));
  }, [days]);

  const rows = data?.rows ?? [];
  const total = useMemo(() => aggregate(rows, () => 'all').get('all'), [rows]);
  const byDay = useMemo(() => aggregate(rows, (r) => r.day), [rows]);
  const byModel = useMemo(() => [...aggregate(rows, (r) => r.model)].sort((a, b) => b[1].cost - a[1].cost || b[1].input - a[1].input), [rows]);
  const byProject = useMemo(() => [...aggregate(rows, (r) => r.cwd)].sort((a, b) => b[1].cost - a[1].cost || b[1].input - a[1].input), [rows]);
  const bySession = useMemo(() => {
    const titles = new Map(rows.map((r) => [r.session_id, { title: r.title, cwd: r.cwd }]));
    return [...aggregate(rows, (r) => r.session_id)]
      .sort((a, b) => b[1].cost - a[1].cost || b[1].input + b[1].output - (a[1].input + a[1].output))
      .slice(0, 10)
      .map(([id, a]) => ({ id, a, ...titles.get(id)! }));
  }, [rows]);
  const sessions = useMemo(() => new Set(rows.map((r) => r.session_id)).size, [rows]);
  // Tokens-only when nothing in range can be priced.
  const effectiveMetric = total && total.cost === 0 ? 'tokens' : metric;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <div>
          <h2 className="text-[15px] font-semibold">Usage</h2>
          <p className="text-[12.5px] text-muted-foreground">Tokens and cost across every session. Days are in UTC.</p>
        </div>
        <div className="ml-auto flex items-center gap-2">
          <Segmented
            value={String(days)}
            options={RANGES.map((d) => ({ value: String(d), label: `${d}d` }))}
            onChange={(v) => setDays(Number(v) as (typeof RANGES)[number])}
          />
          <button
            type="button"
            onClick={() => downloadCsv(rows)}
            disabled={rows.length === 0}
            className="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[12px] text-muted-foreground hover:text-foreground disabled:opacity-40"
          >
            <Download className="size-3.5" strokeWidth={1.75} /> CSV
          </button>
        </div>
      </div>

      {error && <div className="text-[12.5px] text-destructive">{error}</div>}
      {!data && !error && <div className="h-40 animate-pulse rounded-lg bg-secondary/40" />}
      {data && rows.length === 0 && (
        <div className="rounded-lg border border-border py-12 text-center text-[13px] text-muted-foreground">
          No usage in the last {days} days.
        </div>
      )}

      {data && total && (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Tile label="Cost" value={costLabel(total)} note={total.unpriced ? 'some models unpriced' : undefined} />
            <Tile label="Input tokens" value={shortNum(total.input)} />
            <Tile label="Output tokens" value={shortNum(total.output)} />
            <Tile label="Sessions" value={String(sessions)} />
          </div>

          <div className="rounded-lg border border-border p-4">
            <div className="mb-3 flex items-center">
              <span className="text-[12.5px] font-medium">
                {effectiveMetric === 'cost' ? 'Cost per day' : 'Tokens per day'}
              </span>
              {total.cost > 0 && (
                <div className="ml-auto">
                  <Segmented
                    value={metric}
                    options={[{ value: 'cost', label: 'Cost' }, { value: 'tokens', label: 'Tokens' }]}
                    onChange={(v) => setMetric(v as 'cost' | 'tokens')}
                  />
                </div>
              )}
            </div>
            <DailyBars days={daysBetween(data.since, days)} byDay={byDay} metric={effectiveMetric} />
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <Breakdown
              title="By model"
              rows={byModel.map(([m, a]) => ({ key: m, label: m.split('/').pop() || m, title: m, a }))}
            />
            <Breakdown
              title="By project"
              rows={byProject.map(([p, a]) => ({ key: p, label: basename(p), title: p, a }))}
            />
          </div>

          <div className="rounded-lg border border-border">
            <div className="border-b border-border px-4 py-2.5 text-[12.5px] font-medium">Top sessions</div>
            <table className="w-full table-fixed text-[12.5px]">
              <colgroup>
                <col />
                <col className="w-32" />
                <col className="w-20" />
                <col className="w-24" />
              </colgroup>
              <tbody>
                {bySession.map((s) => (
                  <tr key={s.id} className="border-b border-border/60 last:border-0 hover:bg-secondary/40">
                    <td className="max-w-0 px-4 py-2">
                      <button
                        type="button"
                        onClick={() => window.dispatchEvent(new CustomEvent('mira:open-session', { detail: s.id }))}
                        className="block w-full truncate text-left hover:underline"
                        title="Open session"
                      >
                        {s.title || 'Untitled'}
                      </button>
                    </td>
                    <td className="px-2 py-2 text-muted-foreground">{basename(s.cwd)}</td>
                    <td className="px-2 py-2 text-right tabular-nums text-muted-foreground">
                      {shortNum(s.a.input + s.a.output)}
                    </td>
                    <td className="px-4 py-2 text-right tabular-nums">{costLabel(s.a)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {total.unpriced && (
            <p className="text-[11.5px] text-muted-foreground">
              — means a model has no known price: its tokens are counted, but not costed. “+” marks a cost that
              leaves some unpriced tokens out.
            </p>
          )}
        </>
      )}
    </div>
  );
}

function Segmented({
  value,
  options,
  onChange,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <div className="inline-flex rounded-md border border-border p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cn(
            'rounded px-2 py-0.5 text-[12px]',
            o.value === value ? 'bg-secondary text-foreground' : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Tile({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="rounded-lg border border-border px-4 py-3">
      <div className="text-[11.5px] text-muted-foreground">{label}</div>
      <div className="mt-0.5 text-[20px] font-semibold tabular-nums">{value}</div>
      {note && <div className="text-[11px] text-muted-foreground">{note}</div>}
    </div>
  );
}

function Breakdown({
  title,
  rows,
}: {
  title: string;
  rows: { key: string; label: string; title?: string; a: Agg }[];
}) {
  return (
    <div className="rounded-lg border border-border">
      <div className="border-b border-border px-4 py-2.5 text-[12.5px] font-medium">{title}</div>
      <table className="w-full table-fixed text-[12.5px]">
        <colgroup>
          <col />
          <col className="w-16" />
          <col className="w-16" />
          <col className="w-20" />
        </colgroup>
        <thead>
          <tr className="text-[11px] text-muted-foreground">
            <th className="px-4 py-1.5 text-left font-normal">Name</th>
            <th className="px-2 py-1.5 text-right font-normal">Input</th>
            <th className="px-2 py-1.5 text-right font-normal">Output</th>
            <th className="px-4 py-1.5 text-right font-normal">Cost</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.key} className="border-t border-border/60">
              <td className="max-w-0 truncate px-4 py-1.5 font-mono text-[12px]" title={r.title ?? r.label}>{r.label}</td>
              <td className="px-2 py-1.5 text-right tabular-nums text-muted-foreground">{shortNum(r.a.input)}</td>
              <td className="px-2 py-1.5 text-right tabular-nums text-muted-foreground">{shortNum(r.a.output)}</td>
              <td className="px-4 py-1.5 text-right tabular-nums">{costLabel(r.a)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** One series of daily bars on one axis, with a per-bar hover tooltip. */
function DailyBars({
  days,
  byDay,
  metric,
}: {
  days: string[];
  byDay: Map<string, Agg>;
  metric: 'cost' | 'tokens';
}) {
  const [hover, setHover] = useState<number | null>(null);
  const W = 720;
  const H = 180;
  const padL = 44;
  const padB = 22;
  const plotW = W - padL;
  const plotH = H - padB - 8;
  const value = (d: string) => {
    const a = byDay.get(d);
    if (!a) return 0;
    return metric === 'cost' ? a.cost : a.input + a.output;
  };
  const max = Math.max(...days.map(value), 0) || 1;
  const step = plotW / days.length;
  const barW = Math.max(2, Math.min(24, step - 2));
  const fmt = (v: number) => (metric === 'cost' ? formatDollars(v) : shortNum(v));
  const ticks = [0, 0.5, 1].map((t) => t * max);
  const labelEvery = Math.ceil(days.length / 8);

  return (
    <div className="relative">
      <svg viewBox={`0 0 ${W} ${H}`} className="w-full" role="img" aria-label={`${metric} per day`}>
        {ticks.map((t) => {
          const y = 8 + plotH - (t / max) * plotH;
          return (
            <g key={t}>
              <line x1={padL} x2={W} y1={y} y2={y} stroke="currentColor" className="text-border" strokeWidth={1} />
              <text x={padL - 6} y={y + 3} textAnchor="end" className="fill-muted-foreground" fontSize={10}>
                {t === 0 ? '0' : fmt(t)}
              </text>
            </g>
          );
        })}
        {days.map((d, i) => {
          const v = value(d);
          const h = (v / max) * plotH;
          const x = padL + i * step + (step - barW) / 2;
          const y = 8 + plotH - h;
          return (
            <g key={d} onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
              {/* Hit target wider and taller than the bar. */}
              <rect x={padL + i * step} y={8} width={step} height={plotH} fill="transparent" />
              {v > 0 && (
                <path
                  d={`M${x},${8 + plotH} V${y + Math.min(4, h)} Q${x},${y} ${x + Math.min(4, barW / 2)},${y} H${x + barW - Math.min(4, barW / 2)} Q${x + barW},${y} ${x + barW},${y + Math.min(4, h)} V${8 + plotH} Z`}
                  fill={BAR}
                  opacity={hover == null || hover === i ? 1 : 0.45}
                />
              )}
              {i % labelEvery === 0 && (
                <text x={padL + i * step + step / 2} y={H - 6} textAnchor="middle" className="fill-muted-foreground" fontSize={10}>
                  {d.slice(5)}
                </text>
              )}
            </g>
          );
        })}
      </svg>
      {hover != null && (() => {
        const d = days[hover];
        const a = byDay.get(d);
        const leftPct = ((padL + hover * step + step / 2) / W) * 100;
        return (
          <div
            className="pointer-events-none absolute top-0 z-10 -translate-x-1/2 whitespace-nowrap rounded-md border border-border bg-popover px-2.5 py-1.5 text-[11.5px] shadow-lg"
            style={{ left: `${Math.min(88, Math.max(12, leftPct))}%` }}
          >
            <div className="font-medium">{d}</div>
            {a ? (
              <>
                <div className="text-muted-foreground">{shortNum(a.input)} in · {shortNum(a.output)} out</div>
                <div>{costLabel(a)}</div>
              </>
            ) : (
              <div className="text-muted-foreground">No usage</div>
            )}
          </div>
        );
      })()}
    </div>
  );
}
