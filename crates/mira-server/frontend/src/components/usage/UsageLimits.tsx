/**
 * Usage → Limits: each agent's usage-limit windows (a five-hour window, a
 * weekly one, …) as it last reported them, from `/api/usage/limits`.
 *
 * Each window shows what's left, with a line where an even pace through
 * the window would be, so "60% left" reads differently an hour in than an
 * hour before the reset. Agents only report while they run a turn, so
 * every provider shows when its numbers are from; a window whose reset has
 * passed shows as reset rather than with its stale share.
 */
import { Fragment, useEffect, useState } from 'react';
import { Gauge, Minus, TrendingDown, TrendingUp } from 'lucide-react';
import { usageProvider } from '../../lib/usageProviders';
import { elapsedShare, paceOf, resetsIn, windowLabel, type Pace } from '../../lib/usageLimitFormat';
import { AgentIcon } from '../AgentIcon';

type LimitWindow = { name: string; utilization: number; resets_at?: number | null };
type ProviderLimits = { instance: string; driver: string; windows: LimitWindow[]; reported_at: number };

const PACE: Record<Pace, { label: string; Icon: typeof TrendingUp }> = {
  ahead: { label: 'Using it faster than an even pace', Icon: TrendingUp },
  on: { label: 'On an even pace', Icon: Minus },
  behind: { label: 'Using it slower than an even pace', Icon: TrendingDown },
};

function ago(sec: number, nowSec: number): string {
  const s = Math.max(0, nowSec - sec);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

function WindowRow({ window: w, color, now }: { window: LimitWindow; color: string; now: number }) {
  const until = resetsIn(w.resets_at, now);
  const reset = w.resets_at != null && until === null;
  const used = reset ? 0 : Math.min(1, Math.max(0, w.utilization));
  const left = Math.round((1 - used) * 100);
  const elapsed = reset ? null : elapsedShare(w.name, w.resets_at, now);
  const timeLeft = elapsed === null ? null : Math.round((1 - elapsed) * 100);
  const pace = paceOf(used, elapsed);
  const resetAt = w.resets_at && !reset
    ? new Date(w.resets_at * 1000).toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' })
    : null;
  const title = [
    `${left}% left${timeLeft !== null ? ` · ${timeLeft}% of the window left` : ''}`,
    timeLeft !== null ? 'The line is where even spending would be.' : null,
    resetAt ? `Resets ${resetAt}${until ? ` · ${until}` : ''}` : null,
  ].filter(Boolean).join('\n');
  return (
    <Fragment>
      <span className="flex min-w-0 items-center gap-2 text-xs">
        <span className="truncate text-muted-foreground">{windowLabel(w.name)}</span>
        <span className="ms-auto shrink-0 font-medium tabular-nums text-foreground">{reset ? 'Reset' : `${left}% left`}</span>
      </span>
      <div role="img" aria-label={title.replace(/\n/g, ', ')} title={title} className="relative h-6 cursor-default rounded-full">
        <div className="absolute inset-x-0 inset-y-1.5 rounded-full bg-secondary/70" />
        {left > 0 && (
          <div className="absolute inset-y-1.5 left-0 rounded-full" style={{ width: `${left}%`, backgroundColor: color }} />
        )}
        {timeLeft !== null && (
          <span aria-hidden className="absolute inset-y-0.5 w-px -translate-x-1/2 bg-foreground/60" style={{ left: `${timeLeft}%` }} />
        )}
      </div>
      <span className="flex items-center gap-2 whitespace-nowrap text-xs tabular-nums text-muted-foreground">
        {pace && (() => {
          const { Icon, label } = PACE[pace];
          return <span title={label} aria-label={label} role="img" className="inline-flex"><Icon className="size-3.5" aria-hidden /></span>;
        })()}
        <span className="ms-auto shrink-0">{until ? `Resets ${until}` : ''}</span>
      </span>
    </Fragment>
  );
}

export function UsageLimits({ hidden, reload }: { hidden: ReadonlySet<string>; reload: number }) {
  const [limits, setLimits] = useState<ProviderLimits[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  useEffect(() => {
    setError(null);
    fetch('/api/usage/limits')
      .then(async (r) => {
        if (!r.ok) throw new Error(`limits ${r.status}`);
        setLimits((await r.json()) as ProviderLimits[]);
      })
      .catch((e) => setError((e as Error).message))
      .finally(() => setNow(Math.floor(Date.now() / 1000)));
  }, [reload]);
  // Countdowns and pace lines stay current while the page is open.
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(t);
  }, []);

  const shown = (limits ?? []).filter((l) => !hidden.has(usageProvider(l.driver).id));

  if (error) return <p className="text-sm text-destructive">{error}</p>;
  if (limits === null) {
    return (
      <div className="flex flex-col gap-6">
        {[0, 1].map((i) => (
          <div key={i} className="flex flex-col gap-3">
            <div className="h-4 w-32 animate-pulse rounded bg-secondary/50" />
            <div className="h-6 animate-pulse rounded-full bg-secondary/40" />
          </div>
        ))}
      </div>
    );
  }
  if (shown.length === 0) {
    return (
      <div className="flex flex-col items-center gap-3 py-16 text-center">
        <Gauge className="size-5 text-muted-foreground" strokeWidth={1.75} />
        <div className="text-sm text-foreground">No limits reported yet</div>
        <div className="max-w-sm text-xs text-muted-foreground">
          Limits show up here once Claude Code or Codex runs a turn in Mira on a plan that reports them.
        </div>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-8">
      {shown.map((l) => {
        const p = usageProvider(l.driver);
        return (
          <section key={l.instance} className="flex flex-col gap-3">
            <div className="flex items-center gap-2">
              {p.driver && <AgentIcon kind={p.driver} name={p.label} size="sm" tile={false} className="size-4" />}
              <h2 className="text-[13px] font-medium text-foreground">{p.label}</h2>
              {l.instance !== l.driver && (
                <span className="font-mono text-[11px] text-muted-foreground">{l.instance}</span>
              )}
              <span className="ms-auto text-xs text-muted-foreground">Reported {ago(l.reported_at, now)}</span>
            </div>
            <div className="grid grid-cols-[11rem_minmax(0,1fr)_7rem] items-center gap-x-4 gap-y-1">
              {l.windows.map((w) => (
                <WindowRow key={w.name} window={w} color={p.color} now={now} />
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
