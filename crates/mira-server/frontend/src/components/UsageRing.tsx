import { useEffect, useRef, useState } from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { formatDollars, shortNum } from '../lib/usage';
import { getContextBreakdown, type ContextPart } from '../api';
import { partColor } from '../lib/contextParts';

/** One plan or rate-limit window. */
export type UsageLimit = {
  label: string;
  /** Share used, 0–1. */
  used: number;
  /** When it resets, epoch ms. */
  resetsAt: number | null;
};

/** Everything the ring shows, for whichever engine drives the chat. */
export type UsageRingData = {
  /** Tokens in context now, and the window they fill. */
  used: number | null;
  window: number | null;
  /** Where the chat is summarized automatically, when Mira manages it. */
  compactAt: number | null;
  tokens: { prompt: number; completion: number; cached: number } | null;
  /** Why the provider's prompt cache last went unused when it should have
   *  been reused (a rewritten message, a model or tool change, expiry). */
  cacheMiss?: string | null;
  costUsd: number | null;
  limitsTitle: string | null;
  limits: UsageLimit[];
  onCompact?: () => void;
  /** Open the context inspector (what fills the window). */
  onInspect?: () => void;
};

/**
 * The composer's usage ring: how full the context is, at a glance, with the
 * detail one click away — context, when it compacts, plan limits and what
 * the session has spent. One control for providers and agents alike; each
 * engine fills in what it knows.
 */
export function UsageRing({ data }: { data: UsageRingData }) {
  const { used, window, compactAt } = data;
  const frac = used != null && window ? Math.min(1, used / window) : null;
  const hasAnything = frac != null || data.limits.length > 0 || data.tokens != null || data.costUsd != null;
  // Opens on hover. Leaving starts a short grace period, cancelled by
  // entering the popover, so the pointer can travel to "Compact session".
  const [open, setOpen] = useState(false);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hoverOpen = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    setOpen(true);
  };
  const hoverClose = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 180);
  };
  useEffect(() => () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
  }, []);
  // Re-render each minute so "resets in" stays true while the popover is open.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 60_000);
    return () => clearInterval(t);
  }, []);
  // What the context is made of, fetched as the card opens — the same
  // breakdown the inspector shows, for providers and agents alike.
  const [parts, setParts] = useState<ContextPart[] | null>(null);
  const canBreakDown = !!data.onInspect;
  useEffect(() => {
    if (!open || !canBreakDown) return;
    let live = true;
    getContextBreakdown()
      .then((v) => live && setParts(v.breakdown.parts.filter((p) => p.tokens > 0)))
      .catch(() => live && setParts(null));
    return () => {
      live = false;
    };
  }, [open, canBreakDown]);

  if (!hasAnything) return null;

  const pct = frac != null ? Math.round(frac * 100) : null;
  const tone = frac == null ? 'text-muted-foreground' : frac >= 0.9 ? 'text-red-400' : frac >= 0.7 ? 'text-amber-400' : 'text-mira-blue';

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={pct != null ? `Context ${pct}% full` : 'Usage'}
          onMouseEnter={hoverOpen}
          onMouseLeave={hoverClose}
          onFocus={hoverOpen}
          onClick={(e) => {
            if (!data.onInspect) return;
            // Hover shows the summary; a click goes straight to the detail.
            e.preventDefault();
            setOpen(false);
            data.onInspect();
          }}
          className="grid size-7 shrink-0 place-items-center rounded-md transition-colors hover:bg-fg/[0.05]"
        >
          <Ring frac={frac ?? 0} className={tone} />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side="top"
        sideOffset={8}
        className="w-80 rounded-xl border border-border/60 bg-popover/95 p-0 backdrop-blur"
        onMouseEnter={hoverOpen}
        onMouseLeave={hoverClose}
        // Hover-opened: focus stays where the user was typing.
        onOpenAutoFocus={(e) => e.preventDefault()}
      >
        {frac != null && used != null && window != null && (
          <Section>
            <Row
              label="Context window"
              value={`${shortNum(used)} / ${shortNum(window)} (${pct}%)`}
            />
            <Bar frac={frac} marker={compactAt && window ? compactAt / window : null} parts={parts} />
            {parts && parts.length > 0 && (
              <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                {parts.map((p, i) => (
                  <span key={p.id} className="flex items-center gap-1.5">
                    <span className="size-1.5 rounded-full" style={{ background: partColor(p.id, i) }} />
                    {p.label}
                  </span>
                ))}
              </div>
            )}
            {compactAt != null && (
              <div className="mt-2 text-[12px] text-muted-foreground">
                {used >= compactAt ? 'Compacts on the next turn' : `${shortNum(compactAt - used)} until auto-compact`}
              </div>
            )}
            {(data.onInspect || data.onCompact) && (
              // Own row, wrapping: beside the status line the two buttons
              // overflowed the card.
              <div className="mt-2.5 flex flex-wrap justify-end gap-1.5">
                {data.onInspect && (
                  <SmallButton
                    onClick={() => {
                      setOpen(false);
                      data.onInspect!();
                    }}
                  >
                    What&apos;s in it
                  </SmallButton>
                )}
                {data.onCompact && <SmallButton onClick={data.onCompact}>Compact session</SmallButton>}
              </div>
            )}
          </Section>
        )}

        {data.limits.length > 0 && (
          <Section>
            <div className="mb-2 text-[12px] text-muted-foreground">{data.limitsTitle ?? 'Usage limits'}</div>
            <div className="space-y-2.5">
              {data.limits.map((l) => (
                <div key={l.label}>
                  <div className="flex items-baseline justify-between gap-2 text-[12.5px]">
                    <span className="text-foreground/90">{l.label}</span>
                    <span className="text-[11.5px] text-muted-foreground">
                      {l.resetsAt ? `Resets ${resetPhrase(l.resetsAt)}` : ''}
                      <span className="ml-2 tabular-nums text-foreground/80">{Math.round(l.used * 100)}%</span>
                    </span>
                  </div>
                  <Bar frac={l.used} thin />
                </div>
              ))}
            </div>
          </Section>
        )}

        {(data.tokens || data.costUsd != null) && (
          <div className="flex items-center justify-between gap-2 px-3.5 py-2.5 text-[12px] text-muted-foreground">
            <span className="font-mono tabular-nums">
              {data.tokens
                ? `↑${shortNum(data.tokens.prompt)} ↓${shortNum(data.tokens.completion)}${data.tokens.cached ? ` · ${Math.round((data.tokens.cached / Math.max(data.tokens.prompt, 1)) * 100)}% from cache` : ''}`
                : 'This session'}
            </span>
            {data.costUsd != null && (
              <span className="font-mono font-semibold tabular-nums text-emerald-400">{formatDollars(data.costUsd)}</span>
            )}
          </div>
        )}
        {data.cacheMiss && (
          <p className="border-t border-border/50 px-3.5 py-2 text-[11.5px] text-amber-700 dark:text-amber-400">
            Last request paid full price: {data.cacheMiss}.
          </p>
        )}
      </PopoverContent>
    </Popover>
  );
}

function Ring({ frac, className }: { frac: number; className?: string }) {
  const r = 6.5;
  const c = 2 * Math.PI * r;
  return (
    <svg viewBox="0 0 16 16" className={cn('size-4 -rotate-90', className)} aria-hidden>
      <circle cx="8" cy="8" r={r} fill="none" strokeWidth="2.2" className="stroke-muted-foreground/25" />
      <circle
        cx="8"
        cy="8"
        r={r}
        fill="none"
        strokeWidth="2.2"
        strokeLinecap="round"
        stroke="currentColor"
        strokeDasharray={c}
        strokeDashoffset={c * (1 - Math.max(0.02, frac))}
        className="transition-[stroke-dashoffset] duration-500"
      />
    </svg>
  );
}

function Bar({
  frac,
  marker,
  thin,
  parts,
}: {
  frac: number;
  marker?: number | null;
  thin?: boolean;
  /** What the fill is made of: split into colored segments, scaled to
   *  `frac` so the bar still matches the figure above it. */
  parts?: ContextPart[] | null;
}) {
  const tone = frac >= 0.9 ? 'bg-red-400' : frac >= 0.7 ? 'bg-amber-400' : 'bg-mira-blue';
  const fill = Math.min(1, frac);
  const sum = parts?.reduce((n, p) => n + p.tokens, 0) ?? 0;
  return (
    <div className={cn('relative mt-1.5 flex w-full overflow-hidden rounded-full bg-fg/[0.08]', thin ? 'h-1' : 'h-1.5')}>
      {parts && sum > 0 ? (
        parts.map((p, i) => (
          <div
            key={p.id}
            title={`${p.label}: ${shortNum(p.tokens)}`}
            className="h-full transition-[width] duration-500"
            style={{ width: `${(p.tokens / sum) * fill * 100}%`, background: partColor(p.id, i) }}
          />
        ))
      ) : (
        <div className={cn('h-full rounded-full transition-[width] duration-500', tone)} style={{ width: `${Math.round(fill * 100)}%` }} />
      )}
      {marker != null && marker > 0 && marker < 1 && (
        <div className="absolute top-0 h-full w-px bg-foreground/50" style={{ left: `${marker * 100}%` }} title="Auto-compact" />
      )}
    </div>
  );
}

function Section({ children }: { children: React.ReactNode }) {
  return <div className="border-b border-border/50 px-3.5 py-3">{children}</div>;
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2 text-[12.5px]">
      <span className="text-foreground/90">{label}</span>
      <span className="font-mono tabular-nums text-[12px] text-muted-foreground">{value}</span>
    </div>
  );
}

function SmallButton({ children, onClick }: { children: React.ReactNode; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="shrink-0 rounded-md bg-fg/[0.06] px-2 py-1 text-[11.5px] text-foreground/85 transition-colors hover:bg-fg/[0.1]"
    >
      {children}
    </button>
  );
}

/** "in 4 hr 38 min", or a weekday and time when it's days away. */
function resetPhrase(at: number): string {
  const ms = at - Date.now();
  if (ms <= 0) return 'now';
  const mins = Math.round(ms / 60_000);
  if (mins < 60) return `in ${Math.max(1, mins)} min`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `in ${hrs} hr ${mins % 60} min`;
  return new Date(at).toLocaleString(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' });
}
