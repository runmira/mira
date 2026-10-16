/**
 * Settings → Usage: what the work cost and how many tokens it took, per
 * provider (Mira's own model, each external agent), plus each agent's plan
 * limits. Rows come from `/api/usage` (per session/day/model); costs are
 * API estimates — an agent's own figure when it reports one, otherwise the
 * price table — and tokens with no known price are counted but not costed.
 */
import { useEffect, useMemo, useState } from 'react';
import { Check, ChevronDown, Download, Info, RefreshCw } from 'lucide-react';
import { cn } from '@/lib/utils';
import { sortProviders, usageProvider } from '../lib/usageProviders';
import { AgentIcon } from './AgentIcon';
import { UsageLimits } from './usage/UsageLimits';
import { UsageProviderChart } from './usage/UsageProviderChart';
import { UsageShareBar } from './usage/UsageShareBar';
import {
  buildUsage,
  dayRange,
  formatDayShort,
  formatPercent,
  formatTokens,
  formatUsd,
  rowCost,
  type Totals,
  type UsageRow,
} from './usage/usageModel';
import { providerKey } from '../lib/usageProviders';

type Metric = 'tokens' | 'cost' | 'limits';
type Breakdown = 'model' | 'day' | 'project' | 'session';
const WINDOWS = [7, 30, 90] as const;

const PREF_METRIC = 'mira.usage.metric';
const PREF_DAYS = 'mira.usage.days';
const PREF_HIDDEN = 'mira.usage.hiddenProviders';

function readPref<T>(key: string, parse: (raw: string) => T | undefined, def: T): T {
  try {
    const raw = localStorage.getItem(key);
    return (raw != null ? parse(raw) : undefined) ?? def;
  } catch {
    return def;
  }
}
function writePref(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch { /* private mode */ }
}

function basename(p: string) {
  return p.replace(/\/+$/, '').split('/').pop() || p;
}

function downloadCsv(rows: UsageRow[]) {
  const head = 'day,session_id,title,project,provider,model,input_tokens,output_tokens,cached_input_tokens,cost_usd';
  const esc = (v: string) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
  const lines = rows.map((r) => {
    const c = rowCost(r);
    return [
      r.day, r.session_id, esc(r.title ?? ''), esc(r.cwd), usageProvider(r.agent).label, esc(r.model),
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
  const [metric, setMetricState] = useState<Metric>(() =>
    readPref(PREF_METRIC, (v) => (v === 'tokens' || v === 'cost' || v === 'limits' ? v : undefined), 'cost'));
  const [days, setDaysState] = useState<(typeof WINDOWS)[number]>(() =>
    readPref(PREF_DAYS, (v) => WINDOWS.find((d) => String(d) === v), 30));
  const [hidden, setHiddenState] = useState<ReadonlySet<string>>(() =>
    readPref(PREF_HIDDEN, (v) => new Set(JSON.parse(v) as string[]), new Set<string>()));
  const setMetric = (m: Metric) => { setMetricState(m); writePref(PREF_METRIC, m); };
  const setDays = (d: (typeof WINDOWS)[number]) => { setDaysState(d); writePref(PREF_DAYS, String(d)); };
  const setHidden = (next: ReadonlySet<string>) => { setHiddenState(next); writePref(PREF_HIDDEN, JSON.stringify([...next])); };
  const [breakdown, setBreakdown] = useState<Breakdown>('model');

  const [data, setData] = useState<{ since: string; rows: UsageRow[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    setError(null);
    setRefreshing(true);
    fetch(`/api/usage?days=${days}`)
      .then(async (r) => {
        const j = await r.json();
        if (!r.ok) throw new Error(j.error ?? `usage ${r.status}`);
        setData(j);
      })
      .catch((e) => setError((e as Error).message))
      .finally(() => setRefreshing(false));
  }, [days, reload]);

  const showingLimits = metric === 'limits';
  const valueMetric: 'tokens' | 'cost' = metric === 'tokens' ? 'tokens' : 'cost';
  const allRows = data?.rows ?? [];
  const providerIds = useMemo(() => sortProviders(allRows.map((r) => providerKey(r.agent))), [allRows]);
  const rows = useMemo(() => allRows.filter((r) => !hidden.has(providerKey(r.agent))), [allRows, hidden]);
  const dayList = useMemo(() => (data ? dayRange(data.since, days) : []), [data, days]);
  const usage = useMemo(() => buildUsage(rows, dayList), [rows, dayList]);
  const active = providerIds.filter((id) => !hidden.has(id) && usage.byProvider.has(id));
  const value = (t: Totals | undefined) => (!t ? 0 : valueMetric === 'cost' ? t.costUsd : t.totalTokens);
  const format = valueMetric === 'cost' ? formatUsd : formatTokens;
  const totalValue = value(usage.total);
  const windowLabel = dayList.length ? `${formatDayShort(dayList[0])} to ${formatDayShort(dayList[dayList.length - 1])}` : '';

  return (
    <div className="flex flex-col gap-6">
      {/* Top bar: title and provider filter, then metric, period and refresh. */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h2 className="text-[18px] font-semibold tracking-tight">Usage</h2>
        <span className="text-muted-foreground/50">/</span>
        <ProviderFilter ids={providerIds} hidden={hidden} onChange={setHidden} />
        {!showingLimits && <span className="hidden truncate text-xs text-muted-foreground xl:block">{windowLabel}</span>}
        <div className="ms-auto flex items-center gap-2">
          <Segmented
            label="Usage metric"
            value={metric}
            options={[{ value: 'tokens', label: 'Tokens' }, { value: 'cost', label: 'Cost' }, { value: 'limits', label: 'Limits' }]}
            onChange={(v) => setMetric(v as Metric)}
          />
          <Segmented
            label="Usage period"
            value={String(days)}
            disabled={showingLimits}
            options={WINDOWS.map((d) => ({ value: String(d), label: `${d}d` }))}
            onChange={(v) => setDays(Number(v) as (typeof WINDOWS)[number])}
          />
          <button
            type="button"
            onClick={() => setReload((n) => n + 1)}
            aria-label={showingLimits ? 'Refresh limits' : 'Refresh usage'}
            aria-busy={refreshing}
            className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            <RefreshCw className={cn('size-3.5', refreshing && 'animate-spin')} strokeWidth={1.75} />
          </button>
          {!showingLimits && (
            <button
              type="button"
              onClick={() => downloadCsv(rows)}
              disabled={rows.length === 0}
              aria-label="Download CSV"
              title="Download CSV"
              className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground disabled:opacity-40"
            >
              <Download className="size-3.5" strokeWidth={1.75} />
            </button>
          )}
        </div>
      </div>

      {showingLimits ? (
        <UsageLimits hidden={hidden} reload={reload} />
      ) : error ? (
        <p className="py-12 text-center text-sm text-muted-foreground">{error}</p>
      ) : !data ? (
        <UsageSkeleton />
      ) : (
        <div aria-busy={refreshing} className={cn('flex flex-col gap-6 transition-opacity', refreshing && 'opacity-60 delay-150')}>
          <section className="grid gap-6 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
            <div className="flex min-w-0 flex-col gap-5">
              <div className="flex flex-col gap-1">
                <span className="text-4xl font-semibold tabular-nums text-foreground">{format(totalValue)}</span>
                <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                  <span>
                    {usage.total.sessions.toLocaleString('en-US')} sessions
                    {valueMetric === 'cost' && ' · API estimate'}
                  </span>
                  {valueMetric === 'cost' && (usage.total.unpricedTokens > 0 || usage.total.subscriptionUsd > 0) && (
                    <span
                      className="inline-flex cursor-help"
                      title={[
                        usage.total.unpricedTokens > 0
                          ? `Excludes ${formatTokens(usage.total.unpricedTokens)} tokens from models with no known price.`
                          : null,
                        usage.total.subscriptionUsd > 0
                          ? `${formatUsd(usage.total.subscriptionUsd)} of it ran on a subscription — covered by the plan, not billed.`
                          : null,
                      ].filter(Boolean).join('\n')}
                    >
                      <Info className="size-3" aria-hidden />
                    </span>
                  )}
                </span>
              </div>
              {active.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {allRows.length > 0 ? 'Every provider with usage is filtered out.' : `No usage in the last ${days} days.`}
                </p>
              ) : (
                [...active]
                  .sort((a, b) => value(usage.byProvider.get(b)) - value(usage.byProvider.get(a)))
                  .map((id) => {
                    const t = usage.byProvider.get(id)!;
                    const p = usageProvider(id);
                    const share = totalValue > 0 ? value(t) / totalValue : 0;
                    return (
                      <div key={id} className="flex flex-col gap-1">
                        <div className="flex items-baseline justify-between gap-4">
                          <span className="flex min-w-0 items-center gap-2 text-sm text-foreground">
                            <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: p.color }} />
                            {p.driver && <AgentIcon kind={p.driver} name={p.label} size="sm" tile={false} className="size-4" />}
                            <span className="flex min-w-0 items-baseline gap-1.5">
                              <span className="truncate">{p.label}</span>
                              <span className="shrink-0 whitespace-nowrap text-[11px] tabular-nums text-muted-foreground">
                                {t.sessions} {t.sessions === 1 ? 'session' : 'sessions'}
                              </span>
                            </span>
                          </span>
                          <span className="shrink-0 text-sm font-medium tabular-nums text-foreground">{format(value(t))}</span>
                        </div>
                        <span className="text-xs text-muted-foreground">
                          {valueMetric === 'cost'
                            ? `${formatPercent(share)} of cost · ${formatTokens(t.totalTokens)} tokens`
                            : `${formatPercent(share)} of tokens · ${formatUsd(t.costUsd)}`}
                        </span>
                      </div>
                    );
                  })
              )}
            </div>
            <div className="flex min-w-0 flex-col gap-3">
              <h3 className="text-[13px] font-medium text-foreground">
                Daily {valueMetric === 'tokens' ? 'processed tokens' : 'cost'}
              </h3>
              <UsageProviderChart providers={active} daily={usage.daily} metric={valueMetric} />
            </div>
          </section>

          <section className="flex flex-col gap-2">
            <h3 className="text-[13px] font-medium text-foreground">Totals</h3>
            <div className="grid grid-cols-2 gap-x-6 gap-y-4 py-1 md:grid-cols-5">
              <Figure label="Processed tokens" value={formatTokens(usage.total.totalTokens)} />
              <Figure label="Cached input" value={formatTokens(usage.total.cachedInput)} />
              <Figure label="Uncached input" value={formatTokens(usage.total.uncachedInput)} />
              <Figure label="Output" value={formatTokens(usage.total.output)} />
              <Figure label="Cache savings" value={formatUsd(usage.cacheSavingsUsd)} />
            </div>
          </section>

          {usage.total.totalTokens > 0 && (
            <section className="grid gap-x-12 gap-y-8 lg:grid-cols-2">
              {valueMetric === 'tokens' ? (
                <UsageShareBar
                  label="Tokens by type"
                  format={formatTokens}
                  segments={[
                    { label: 'Uncached input', value: usage.total.uncachedInput, color: '#7aa2f7' },
                    { label: 'Cached input', value: usage.total.cachedInput, color: '#7dcfff' },
                    { label: 'Output', value: usage.total.output, color: '#bb9af7' },
                  ]}
                />
              ) : (
                <UsageShareBar
                  label="Cost by type"
                  format={formatUsd}
                  segments={[
                    { label: 'Uncached input', value: usage.costByType.uncachedInput, color: '#7aa2f7' },
                    { label: 'Cached input', value: usage.costByType.cachedInput, color: '#7dcfff' },
                    { label: 'Output', value: usage.costByType.output, color: '#bb9af7' },
                  ]}
                />
              )}
              <UsageShareBar
                label={valueMetric === 'tokens' ? 'Tokens by provider' : 'Cost by provider'}
                format={format}
                segments={active.map((id) => ({
                  label: usageProvider(id).label,
                  value: value(usage.byProvider.get(id)),
                  color: usageProvider(id).color,
                }))}
              />
            </section>
          )}

          <section className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-[13px] font-medium text-foreground">Breakdown</h3>
              <Segmented
                label="Usage breakdown"
                value={breakdown}
                options={[
                  { value: 'model', label: 'Model' },
                  { value: 'day', label: 'Day' },
                  { value: 'project', label: 'Project' },
                  { value: 'session', label: 'Session' },
                ]}
                onChange={(v) => setBreakdown(v as Breakdown)}
              />
            </div>
            {breakdown === 'day' ? (
              <DayTable usage={usage} providers={active} metric={valueMetric} />
            ) : (
              <RankedTable
                rows={rankedRows(breakdown, rows, usage)}
                column={breakdown === 'model' ? 'Model' : breakdown === 'project' ? 'Project' : 'Session'}
                metric={valueMetric}
              />
            )}
          </section>
        </div>
      )}
    </div>
  );
}

/* ---------- breakdown ---------- */

type Ranked = Totals & { key: string; label: string; title?: string; provider?: string; open?: string };

function rankedRows(kind: Exclude<Breakdown, 'day'>, rows: UsageRow[], usage: ReturnType<typeof buildUsage>): Ranked[] {
  if (kind === 'model') {
    return usage.models.map((m) => ({ ...m, label: m.model, title: m.model, provider: m.provider }));
  }
  const group = kind === 'project' ? (r: UsageRow) => r.cwd : (r: UsageRow) => r.session_id;
  const groups = new Map<string, UsageRow[]>();
  for (const r of rows) {
    const k = group(r);
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k)!.push(r);
  }
  return [...groups].map(([k, rs]) => {
    const t = buildUsage(rs, []).total;
    return kind === 'project'
      ? { ...t, key: k, label: basename(k), title: k }
      : { ...t, key: k, label: rs[0].title || 'Untitled', title: basename(rs[0].cwd), open: k };
  }).sort((a, b) => b.costUsd - a.costUsd || b.totalTokens - a.totalTokens);
}

function RankedTable({ rows, column, metric }: { rows: Ranked[]; column: string; metric: 'cost' | 'tokens' }) {
  const value = (r: Ranked) => (metric === 'tokens' ? r.totalTokens : r.costUsd);
  const peak = Math.max(0, ...rows.map(value));
  const sum = rows.reduce((n, r) => n + value(r), 0);
  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="border-b border-border text-right text-xs text-muted-foreground">
          <th className="py-2 pr-3 text-left font-normal">#</th>
          <th className="w-full py-2 text-left font-normal">{column}</th>
          <th className="py-2 pl-6 font-normal">Cost</th>
          <th className="hidden py-2 pl-6 font-normal sm:table-cell">Share</th>
          <th className="py-2 pl-6 font-normal">Tokens</th>
        </tr>
      </thead>
      <tbody>
        {rows.length === 0 ? (
          <tr><td colSpan={5} className="py-6 text-center text-muted-foreground">No activity in this window.</td></tr>
        ) : rows.map((r, i) => {
          const v = value(r);
          const color = r.provider ? usageProvider(r.provider).color : '#7aa2f7';
          const p = r.provider ? usageProvider(r.provider) : null;
          const name = (
            <span className="flex items-center gap-2 text-left text-foreground">
              {p?.driver && <AgentIcon kind={p.driver} name={p.label} size="sm" tile={false} className="size-3.5" />}
              {p && !p.driver && <span className="size-2 rounded-full" style={{ background: p.color }} />}
              <span className="truncate" title={r.title}>{r.label}</span>
              {r.open && <span className="truncate text-xs text-muted-foreground">{r.title}</span>}
            </span>
          );
          return (
            <tr
              key={r.key}
              className="relative border-b border-border/50 whitespace-nowrap text-right tabular-nums text-muted-foreground transition-colors hover:bg-secondary/40"
            >
              <td className="py-2.5 pr-3 text-left text-xs">{i + 1}</td>
              <td className="max-w-0 py-2.5 text-left whitespace-normal">
                {r.open ? (
                  <button
                    type="button"
                    onClick={() => window.dispatchEvent(new CustomEvent('mira:open-session', { detail: r.open }))}
                    className="w-full outline-none after:absolute after:inset-0"
                    title="Open session"
                  >
                    {name}
                  </button>
                ) : name}
                <div aria-hidden className="mt-1.5 h-0.5 max-w-48">
                  <div
                    className="h-full rounded-full"
                    style={{ width: v > 0 && peak > 0 ? `max(0.5rem, ${(v / peak) * 100}%)` : 0, backgroundColor: color }}
                  />
                </div>
              </td>
              <td className="py-2.5 pl-6 text-foreground">
                {r.costUsd === 0 && r.unpricedTokens > 0 ? <span className="text-muted-foreground">Unpriced</span> : formatUsd(r.costUsd)}
              </td>
              <td className="hidden py-2.5 pl-6 sm:table-cell">{sum > 0 ? formatPercent(v / sum) : ''}</td>
              <td className="py-2.5 pl-6">{formatTokens(r.totalTokens)}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function DayTable({ usage, providers, metric }: { usage: ReturnType<typeof buildUsage>; providers: string[]; metric: 'cost' | 'tokens' }) {
  const days = [...usage.daily].reverse().filter((d) => d.total.totalTokens > 0);
  const format = metric === 'cost' ? formatUsd : formatTokens;
  const value = (t: Totals | undefined) => (!t ? 0 : metric === 'cost' ? t.costUsd : t.totalTokens);
  return (
    <table className="w-full table-fixed text-sm">
      <colgroup>
        <col className="w-2/5" />
        {providers.map((p) => <col key={p} />)}
        <col />
        <col />
      </colgroup>
      <thead>
        <tr className="border-b border-border text-left text-xs text-muted-foreground">
          <th className="py-2 font-normal">Day</th>
          {providers.map((p) => <th key={p} className="py-2 text-right font-normal">{usageProvider(p).label}</th>)}
          <th className="py-2 text-right font-normal">Total</th>
          <th className="py-2 text-right font-normal">{metric === 'cost' ? 'Tokens' : 'Cost'}</th>
        </tr>
      </thead>
      <tbody>
        {days.length === 0 ? (
          <tr><td colSpan={providers.length + 3} className="py-6 text-center text-muted-foreground">No activity in this window.</td></tr>
        ) : days.map((d) => (
          <tr key={d.day} className="border-b border-border/50 transition-colors hover:bg-secondary/40">
            <td className="py-2 text-foreground">{formatDayShort(d.day)}</td>
            {providers.map((p) => (
              <td key={p} className="py-2 text-right tabular-nums text-muted-foreground">{format(value(d.byProvider.get(p)))}</td>
            ))}
            <td className="py-2 text-right tabular-nums text-foreground">{format(value(d.total))}</td>
            <td className="py-2 text-right tabular-nums text-muted-foreground">
              {metric === 'cost' ? formatTokens(d.total.totalTokens) : formatUsd(d.total.costUsd)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/* ---------- pieces ---------- */

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-base font-medium tabular-nums text-foreground">{value}</span>
    </div>
  );
}

function Segmented({
  label,
  value,
  options,
  onChange,
  disabled = false,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  return (
    <div role="radiogroup" aria-label={label} className={cn('inline-flex items-center rounded-lg bg-secondary/60 p-0.5', disabled && 'opacity-50')}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          disabled={disabled}
          onClick={() => onChange(o.value)}
          className={cn(
            'rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
            o.value === value ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Show or hide providers across the page (Usage and Limits). */
function ProviderFilter({
  ids,
  hidden,
  onChange,
}: {
  ids: string[];
  hidden: ReadonlySet<string>;
  onChange: (next: ReadonlySet<string>) => void;
}) {
  const [open, setOpen] = useState(false);
  const all = sortProviders([...ids, ...hidden]);
  const shown = all.filter((id) => !hidden.has(id));
  const label = hidden.size === 0 || all.length === 0
    ? 'All providers'
    : shown.length === 1 ? usageProvider(shown[0]).label : `${shown.length} providers`;
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-sm text-foreground transition-colors hover:bg-secondary/60"
      >
        {label}
        <ChevronDown className="size-3.5 text-muted-foreground" strokeWidth={2} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={() => setOpen(false)} />
          <div className="absolute left-0 z-30 mt-1 min-w-48 rounded-xl border border-border/70 bg-popover p-1 shadow-lg">
            {all.length === 0 && <div className="px-2.5 py-1.5 text-xs text-muted-foreground">No providers yet</div>}
            {all.map((id) => {
              const p = usageProvider(id);
              const on = !hidden.has(id);
              return (
                <button
                  key={id}
                  type="button"
                  onClick={() => {
                    const next = new Set(hidden);
                    if (on) next.add(id); else next.delete(id);
                    onChange(next);
                  }}
                  className="flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm hover:bg-secondary/60"
                >
                  <span className="inline-flex size-4 items-center justify-center">
                    {on && <Check className="size-3.5" strokeWidth={2.5} />}
                  </span>
                  {p.driver ? (
                    <AgentIcon kind={p.driver} name={p.label} size="sm" tile={false} className="size-4" />
                  ) : (
                    <span className="size-2 rounded-full" style={{ background: p.color }} />
                  )}
                  {p.label}
                </button>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}

function UsageSkeleton() {
  return (
    <div className="flex flex-col gap-6">
      <section className="grid gap-6 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
        <div className="flex flex-col gap-4">
          <div className="h-9 w-32 animate-pulse rounded bg-secondary/50" />
          {[0, 1, 2].map((i) => <div key={i} className="h-8 animate-pulse rounded bg-secondary/40" />)}
        </div>
        <div className="h-60 animate-pulse rounded-lg bg-secondary/30" />
      </section>
      <section className="flex flex-col gap-2">
        <h3 className="text-[13px] font-medium text-foreground">Totals</h3>
        <div className="grid grid-cols-2 gap-x-6 gap-y-4 md:grid-cols-5">
          {['Processed tokens', 'Cached input', 'Uncached input', 'Output', 'Cache savings'].map((l) => (
            <div key={l} className="flex flex-col gap-1">
              <span className="text-xs text-muted-foreground">{l}</span>
              <span className="h-5 w-16 animate-pulse rounded bg-secondary/50" />
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
