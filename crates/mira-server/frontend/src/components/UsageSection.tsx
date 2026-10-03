import { useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  ArrowDownLeft,
  ArrowUpRight,
  CalendarDays,
  ChartColumn,
  ChevronRight,
  Coins,
  Download,
  Info,
  MessagesSquare,
} from 'lucide-react';
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
  /** The external agent that ran these turns (`claude`, `codex`). */
  agent?: string | null;
  /** The agent's own cost estimate, used instead of the price table. */
  cost_usd?: number | null;
};

const AGENT_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex' };
/** Model label, with the agent that ran it so agent and provider spend
 *  on the same model stay apart. */
function modelLabel(r: Row): string {
  return r.agent ? `${r.model} · ${AGENT_NAMES[r.agent] ?? r.agent}` : r.model;
}

type Agg = { input: number; output: number; cost: number; unpriced: boolean };

const RANGES = [7, 30, 90] as const;
const BAR = '#7aa2f7'; // mira-blue — one series, one hue

function rowCost(r: Row): number | null {
  if (typeof r.cost_usd === 'number') return r.cost_usd;
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

/* ---------- accent wheel ---------- */

/**
 * Stable accent per breakdown row / session avatar: hash the key into a
 * Tokyo-night-friendly hue. Every class is a hard-coded literal —
 * template-interpolated classes never reach Tailwind's JIT scanner, so
 * `` `bg-${hue}-500/15` `` would silently render unstyled.
 */
const ACCENTS = {
  blue:    { dot: 'bg-blue-400',         bar: 'from-blue-400/90 to-blue-400/25',       chipBg: 'bg-blue-500/15',    chipText: 'text-blue-300' },
  violet:  { dot: 'bg-violet-400',       bar: 'from-violet-400/90 to-violet-400/25',   chipBg: 'bg-violet-500/15',  chipText: 'text-violet-300' },
  cyan:    { dot: 'bg-cyan-400',         bar: 'from-cyan-400/90 to-cyan-400/25',       chipBg: 'bg-cyan-500/15',    chipText: 'text-cyan-300' },
  emerald: { dot: 'bg-emerald-400',      bar: 'from-emerald-400/90 to-emerald-400/25', chipBg: 'bg-emerald-500/15', chipText: 'text-emerald-300' },
  amber:   { dot: 'bg-amber-400',        bar: 'from-amber-400/90 to-amber-400/25',     chipBg: 'bg-amber-500/15',   chipText: 'text-amber-300' },
  pink:    { dot: 'bg-pink-400',         bar: 'from-pink-400/90 to-pink-400/25',       chipBg: 'bg-pink-500/15',    chipText: 'text-pink-300' },
  orange:  { dot: 'bg-orange-400',       bar: 'from-orange-400/90 to-orange-400/25',   chipBg: 'bg-orange-500/15',  chipText: 'text-orange-300' },
  red:     { dot: 'bg-red-400',          bar: 'from-red-400/90 to-red-400/25',         chipBg: 'bg-red-500/15',     chipText: 'text-red-300' },
  teal:    { dot: 'bg-teal-400',         bar: 'from-teal-400/90 to-teal-400/25',       chipBg: 'bg-teal-500/15',    chipText: 'text-teal-300' },
  indigo:  { dot: 'bg-indigo-400',       bar: 'from-indigo-400/90 to-indigo-400/25',   chipBg: 'bg-indigo-500/15',  chipText: 'text-indigo-300' },
} as const;

const WHEEL = ['blue', 'violet', 'cyan', 'emerald', 'amber', 'pink', 'orange', 'red', 'teal', 'indigo'] as const;

type AccentKey = (typeof WHEEL)[number];
type Accent = (typeof ACCENTS)[AccentKey];

function accentFor(seed: string): Accent {
  let h = 5381;
  for (let i = 0; i < seed.length; i++) {
    h = (h * 33) ^ seed.charCodeAt(i);
  }
  return ACCENTS[WHEEL[Math.abs(h) % WHEEL.length]];
}

/* ---------- section ---------- */

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
  const byModel = useMemo(() => [...aggregate(rows, modelLabel)].sort((a, b) => b[1].cost - a[1].cost || b[1].input - a[1].input), [rows]);
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
  // The hero leads with spend; when nothing in range has a known price,
  // lead with total tokens instead so the number never reads as "$0".
  const heroIsCost = !!total && (total.cost > 0 || !total.unpriced);

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-end gap-3">
        <div>
          <h2 className="text-[18px] font-semibold tracking-tight">Usage</h2>
          <p className="mt-0.5 text-[12.5px] text-muted-foreground">Tokens and cost across every session. Days are in UTC.</p>
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
            className="inline-flex items-center gap-1.5 rounded-lg border border-border/70 bg-background/40 px-2.5 py-1.5 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-secondary/50 hover:text-foreground disabled:opacity-40"
          >
            <Download className="size-3.5" strokeWidth={1.75} /> CSV
          </button>
        </div>
      </div>

      {error && (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">
          {error}
        </div>
      )}
      {!data && !error && (
        <div className="flex flex-col gap-4">
          <div className="h-32 animate-pulse rounded-2xl bg-secondary/30" />
          <div className="h-52 animate-pulse rounded-xl bg-secondary/20" />
        </div>
      )}
      {data && rows.length === 0 && (
        <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-border/60 bg-secondary/20 px-4 py-14 text-center">
          <div className="inline-flex size-11 items-center justify-center rounded-full bg-mira-blue/10 text-mira-blue">
            <ChartColumn className="size-5" strokeWidth={1.75} />
          </div>
          <div>
            <div className="text-[13.5px] font-semibold text-foreground">No usage yet</div>
            <div className="mt-1 text-[12.5px] text-muted-foreground">
              Nothing was spent in the last {days} days. Start a chat and it will show up here.
            </div>
          </div>
        </div>
      )}

      {data && total && (
        <>
          {/* Hero: the one number people open this page for, plus the
              secondary stats tucked to the right so the tiles don't
              repeat as four identical grey boxes. */}
          <div className="relative overflow-hidden rounded-2xl border border-mira-blue/20 bg-mira-elev1/70 px-5 py-5">
            <div aria-hidden className="pointer-events-none absolute -right-10 -top-16 size-48 rounded-full bg-mira-blue/25 blur-3xl" />
            <div aria-hidden className="pointer-events-none absolute -bottom-20 left-1/3 size-44 rounded-full bg-mira-purple/15 blur-3xl" />
            <div className="relative flex flex-wrap items-end justify-between gap-x-8 gap-y-5">
              <div className="min-w-0">
                <div className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
                  <Coins className="size-3.5 text-mira-blue" strokeWidth={1.75} />
                  {heroIsCost ? 'Total spend' : 'Total tokens'}
                  <span className="normal-case tracking-normal text-muted-foreground/60">· last {days} days</span>
                </div>
                <div className="mt-2 text-[32px] font-semibold leading-none tracking-tight tabular-nums text-foreground">
                  {heroIsCost ? costLabel(total) : shortNum(total.input + total.output)}
                </div>
                <div className="mt-1.5 text-[11.5px] text-muted-foreground">
                  {heroIsCost
                    ? total.unpriced ? 'some models unpriced — see footnote' : 'across all models and projects'
                    : 'no priced models in this range'}
                </div>
              </div>
              <div className="flex items-stretch">
                <HeroStat
                  icon={<ArrowDownLeft className="size-4 text-mira-cyan" strokeWidth={1.75} />}
                  label="Input"
                  value={shortNum(total.input)}
                />
                <HeroStat
                  icon={<ArrowUpRight className="size-4 text-mira-user" strokeWidth={1.75} />}
                  label="Output"
                  value={shortNum(total.output)}
                  divider
                />
                <HeroStat
                  icon={<MessagesSquare className="size-4 text-mira-purple" strokeWidth={1.75} />}
                  label="Sessions"
                  value={String(sessions)}
                  divider
                />
              </div>
            </div>
          </div>

          {/* Daily chart */}
          <Card>
            <div className="flex flex-wrap items-center gap-2 border-b border-border/50 px-4 py-2.5">
              <CalendarDays className="size-3.5 text-muted-foreground" strokeWidth={1.75} />
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
            <div className="px-3 py-3">
              <DailyBars days={daysBetween(data.since, days)} byDay={byDay} metric={effectiveMetric} />
            </div>
          </Card>

          {/* Breakdowns — full-width bar lists: share bars read better
              and truncate far less than two side-by-side tables. */}
          <Breakdown
            title="By model"
            rows={byModel.map(([m, a]) => ({ key: m, label: m.split('/').pop() || m, title: m, a }))}
          />
          <Breakdown
            title="By project"
            rows={byProject.map(([p, a]) => ({ key: p, label: basename(p), title: p, a }))}
          />

          {/* Top sessions */}
          <Card>
            <div className="flex items-center gap-2 border-b border-border/50 px-4 py-2.5">
              <span className="text-[12.5px] font-medium">Top sessions</span>
              <span className="text-[11px] text-muted-foreground">· click to open</span>
            </div>
            <div className="flex flex-col p-1.5">
              {bySession.map((s) => {
                const label = s.title || 'Untitled';
                const accent = accentFor(s.cwd);
                const initial = (label.match(/[a-zA-Z0-9]/)?.[0] ?? '?').toUpperCase();
                return (
                  <button
                    key={s.id}
                    type="button"
                    onClick={() => window.dispatchEvent(new CustomEvent('mira:open-session', { detail: s.id }))}
                    className="group flex items-center gap-3 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-secondary/40 focus:outline-none focus-visible:ring-1 focus-visible:ring-mira-blue"
                    title="Open session"
                  >
                    <span
                      className={cn(
                        'inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-[13px] font-semibold',
                        accent.chipBg,
                        accent.chipText,
                      )}
                    >
                      {initial}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] font-medium text-foreground">{label}</span>
                      <span className="mt-0.5 block truncate text-[11.5px] text-muted-foreground">
                        {basename(s.cwd)} · {shortNum(s.a.input + s.a.output)} tokens
                      </span>
                    </span>
                    <span className="shrink-0 text-right text-[12.5px] tabular-nums text-foreground">
                      {costLabel(s.a)}
                    </span>
                    <ChevronRight
                      className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-70"
                      strokeWidth={1.75}
                    />
                  </button>
                );
              })}
            </div>
          </Card>

          {total.unpriced && (
            <p className="flex items-start gap-1.5 text-[11.5px] text-muted-foreground">
              <Info className="mt-0.5 size-3.5 shrink-0" strokeWidth={1.75} />
              <span>
                — means a model has no known price: its tokens are counted, but not costed. “+” marks a cost that
                leaves some unpriced tokens out.
              </span>
            </p>
          )}
        </>
      )}
    </div>
  );
}

/* ---------- pieces ---------- */

function Card({ children }: { children: ReactNode }) {
  return <div className="overflow-hidden rounded-xl border border-border/60 bg-mira-elev1/50">{children}</div>;
}

function HeroStat({
  icon, label, value, divider,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  divider?: boolean;
}) {
  return (
    <div className={cn('px-5 first:pl-0', divider && 'border-l border-border/50')}>
      <div className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        {icon}
        {label}
      </div>
      <div className="mt-1.5 text-[16px] font-semibold leading-none tabular-nums text-foreground">{value}</div>
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
    <div className="inline-flex items-center rounded-full border border-border/70 bg-secondary/40 p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cn(
            'rounded-full px-2.5 py-0.5 text-[12px] font-medium transition-colors',
            o.value === value
              ? 'bg-mira-elev2 text-foreground shadow-sm ring-1 ring-fg/5'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * Breakdown as a bar list: one row per model/project, a share bar under
 * the name, input/output inline and cost right-aligned. Share is by cost
 * when anything in the card is priced, by total tokens otherwise — a
 * mixed metric would silently rank unpriced models as zero.
 */
function Breakdown({
  title,
  rows,
}: {
  title: string;
  rows: { key: string; label: string; title?: string; a: Agg }[];
}) {
  const byCost = rows.some((r) => r.a.cost > 0);
  const share = (a: Agg): number => (byCost ? a.cost : a.input + a.output);
  const max = Math.max(...rows.map((r) => share(r.a)), 0) || 1;

  return (
    <Card>
      <div className="flex items-center gap-2 border-b border-border/50 px-4 py-2.5">
        <span className="text-[12.5px] font-medium">{title}</span>
        <span className="text-[11px] text-muted-foreground">· by {byCost ? 'cost' : 'tokens'}</span>
      </div>
      <div className="flex flex-col divide-y divide-border/30 px-2 py-1.5">
        {rows.map((r) => {
          const accent = accentFor(r.key);
          const s = share(r.a);
          // 2% floor only for rows that actually have a nonzero share —
          // zero (unpriced) rows render an empty track instead of a
          // sliver that would read as a tiny cost.
          const pct = s > 0 ? Math.max(2, Math.round((s / max) * 100)) : 0;
          return (
            <div key={r.key} className="flex items-center gap-3 px-2.5 py-2" title={r.title ?? r.label}>
              <span className={cn('size-2 shrink-0 rounded-full', accent.dot)} />
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline justify-between gap-3">
                  <span className="truncate font-mono text-[12.5px] text-foreground">{r.label}</span>
                  <span className="shrink-0 text-[12px] tabular-nums text-muted-foreground">{costLabel(r.a)}</span>
                </div>
                <div className="mt-1.5 flex items-center gap-2.5">
                  <div className="h-1.5 w-full max-w-56 overflow-hidden rounded-full bg-secondary/50">
                    <div
                      className={cn('h-full rounded-full bg-gradient-to-r', accent.bar)}
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                  <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/80">
                    {shortNum(r.a.input)} in · {shortNum(r.a.output)} out
                  </span>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </Card>
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
        <defs>
          <linearGradient id="mira-usage-bar" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={BAR} stopOpacity={0.95} />
            <stop offset="100%" stopColor={BAR} stopOpacity={0.30} />
          </linearGradient>
        </defs>
        {ticks.map((t) => {
          const y = 8 + plotH - (t / max) * plotH;
          return (
            <g key={t}>
              <line
                x1={padL} x2={W} y1={y} y2={y}
                stroke="currentColor" className="text-border" strokeWidth={1}
                strokeDasharray={t === 0 ? undefined : '3 4'} opacity={t === 0 ? 1 : 0.6}
              />
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
                  fill="url(#mira-usage-bar)"
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
            className="tooltip tooltip-stack pointer-events-none absolute top-0 z-10 -translate-x-1/2"
            style={{ left: `${Math.min(88, Math.max(12, leftPct))}%` }}
          >
            <div className="font-medium">{d}</div>
            {a ? (
              <>
                <div className="mt-0.5 text-muted-foreground">{shortNum(a.input)} in · {shortNum(a.output)} out</div>
                <div className="mt-0.5">{costLabel(a)}</div>
              </>
            ) : (
              <div className="mt-0.5 text-muted-foreground">No usage</div>
            )}
          </div>
        );
      })()}
    </div>
  );
}
