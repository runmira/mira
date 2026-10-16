/**
 * Daily usage per provider: one smoothed line per provider over a faint
 * area, each measured from zero (overlaid, not stacked, so a small series
 * stays readable next to a big one). Hovering shows a guide line and every
 * provider's figure for that day, plus the total.
 */
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { cn } from '@/lib/utils';
import { usageProvider } from '../../lib/usageProviders';
import { AgentIcon } from '../AgentIcon';
import { formatDayShort, formatTokens, formatUsd, type DayTotals } from './usageModel';

const VIEW_W = 960;
const VIEW_H = 260;
const PLOT_TOP = 8;
const TICKS = 4;

type Point = { x: number; y: number };

/** A round axis maximum and its gridlines. */
export function niceScale(peak: number, count = TICKS): { max: number; ticks: number[] } {
  if (peak <= 0) return { max: 0, ticks: [0] };
  const raw = peak / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const n = raw / mag;
  const step = (n > 5 ? 10 : n > 2 ? 5 : n > 1 ? 2 : 1) * mag;
  const max = Math.ceil(peak / step) * step;
  const ticks: number[] = [];
  for (let v = 0; v <= max + step * 1e-6; v += step) ticks.push(v);
  return { max, ticks };
}

/**
 * A smooth path through the points that never overshoots them (monotone
 * cubic interpolation), so a day with no usage never dips below zero.
 */
export function smoothPath(points: Point[]): string {
  const n = points.length;
  if (n === 0) return '';
  if (n === 1) return `M${points[0].x},${points[0].y}`;
  const dx: number[] = [];
  const slope: number[] = [];
  for (let i = 0; i < n - 1; i++) {
    dx.push(points[i + 1].x - points[i].x);
    slope.push(dx[i] === 0 ? 0 : (points[i + 1].y - points[i].y) / dx[i]);
  }
  const tangent: number[] = [slope[0]];
  for (let i = 1; i < n - 1; i++) {
    tangent.push(slope[i - 1] * slope[i] <= 0 ? 0 : (slope[i - 1] + slope[i]) / 2);
  }
  tangent.push(slope[n - 2]);
  for (let i = 0; i < n - 1; i++) {
    if (slope[i] === 0) {
      tangent[i] = 0;
      tangent[i + 1] = 0;
      continue;
    }
    const a = tangent[i] / slope[i];
    const b = tangent[i + 1] / slope[i];
    const h = a * a + b * b;
    if (h > 9) {
      const t = 3 / Math.sqrt(h);
      tangent[i] = t * a * slope[i];
      tangent[i + 1] = t * b * slope[i];
    }
  }
  let d = `M${points[0].x},${points[0].y}`;
  for (let i = 0; i < n - 1; i++) {
    const p = points[i];
    const q = points[i + 1];
    const h = dx[i] / 3;
    d += ` C${p.x + h},${p.y + tangent[i] * h} ${q.x - h},${q.y - tangent[i + 1] * h} ${q.x},${q.y}`;
  }
  return d;
}

export function UsageProviderChart({
  providers,
  daily,
  metric,
}: {
  providers: string[];
  daily: DayTotals[];
  metric: 'cost' | 'tokens';
}) {
  const [hover, setHover] = useState<number | null>(null);
  const plotRef = useRef<HTMLDivElement | null>(null);
  const tipRef = useRef<HTMLDivElement | null>(null);
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const valueOf = useCallback(
    (d: DayTotals, p: string) => {
      const t = d.byProvider.get(p);
      return !t ? 0 : metric === 'cost' ? t.costUsd : t.totalTokens;
    },
    [metric],
  );
  const format = metric === 'cost' ? formatUsd : formatTokens;

  const { scale, stepX, series } = useMemo(() => {
    // Not the sum: each series measures from zero, so a combined peak would
    // leave the plot half empty.
    const peak = Math.max(0, ...daily.flatMap((d) => providers.map((p) => valueOf(d, p))));
    const scale = niceScale(peak);
    const stepX = daily.length < 2 ? 0 : VIEW_W / (daily.length - 1);
    const toY = (v: number) => (scale.max === 0 ? VIEW_H : VIEW_H - (v / scale.max) * (VIEW_H - PLOT_TOP));
    const series = providers
      .map((p) => {
        const line = smoothPath(daily.map((d, i) => ({ x: i * stepX, y: toY(valueOf(d, p)) })));
        return {
          provider: p,
          line,
          area: line ? `${line} L${VIEW_W},${VIEW_H} L0,${VIEW_H} Z` : '',
          total: daily.reduce((n, d) => n + valueOf(d, p), 0),
        };
      })
      // The biggest series underneath, so smaller ones stay on top.
      .sort((a, b) => b.total - a.total);
    return { scale, stepX, series };
  }, [daily, providers, valueOf]);
  const toY = (v: number) => (scale.max === 0 ? VIEW_H : VIEW_H - (v / scale.max) * (VIEW_H - PLOT_TOP));

  const place = useCallback(() => {
    const plot = plotRef.current;
    const tip = tipRef.current;
    const at = pointer.current;
    if (!plot || !tip || !at) return;
    const gap = 12;
    const w = tip.offsetWidth;
    const h = tip.offsetHeight;
    const left = at.x + gap + w <= plot.clientWidth ? at.x + gap : at.x - gap - w;
    const top = at.y + gap + h <= plot.clientHeight ? at.y + gap : at.y - gap - h;
    tip.style.left = `${Math.min(Math.max(0, left), Math.max(0, plot.clientWidth - w))}px`;
    tip.style.top = `${Math.min(Math.max(0, top), Math.max(0, plot.clientHeight - h))}px`;
  }, []);
  useLayoutEffect(() => {
    if (hover !== null) place();
  }, [hover, place]);

  const onMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const plot = plotRef.current;
    if (!plot || daily.length === 0) return;
    const b = plot.getBoundingClientRect();
    if (b.width === 0) return;
    const x = Math.min(b.width, Math.max(0, e.clientX - b.left));
    const y = Math.min(b.height, Math.max(0, e.clientY - b.top));
    pointer.current = { x, y };
    place();
    setHover(Math.min(daily.length - 1, Math.max(0, Math.round((x / b.width) * (daily.length - 1)))));
  };

  const hovered = hover === null ? undefined : daily[hover];
  const mid = daily[Math.floor(daily.length / 2)];

  return (
    <div className="flex flex-col gap-1">
      <div className="flex gap-2">
        <div className="relative h-56 w-14 shrink-0">
          {scale.ticks.map((t) => (
            <span
              key={t}
              className="absolute right-0 -translate-y-1/2 text-[10px] tabular-nums text-muted-foreground"
              style={{ top: `${(toY(t) / VIEW_H) * 100}%` }}
            >
              {t === 0 ? '0' : format(t)}
            </span>
          ))}
        </div>
        <div
          ref={plotRef}
          className="relative h-56 flex-1"
          onMouseMove={onMove}
          onMouseLeave={() => {
            pointer.current = null;
            setHover(null);
          }}
        >
          <svg
            className="h-full w-full"
            viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
            preserveAspectRatio="none"
            role="img"
            aria-label={`Daily ${metric === 'cost' ? 'cost' : 'processed tokens'} by provider`}
          >
            {scale.ticks.map((t) => (
              <line
                key={t}
                x1={0}
                x2={VIEW_W}
                y1={toY(t)}
                y2={toY(t)}
                stroke="currentColor"
                strokeWidth={1}
                className="text-border"
                vectorEffect="non-scaling-stroke"
              />
            ))}
            {series.map((s) => (
              <path key={`a-${s.provider}`} d={s.area} fill={usageProvider(s.provider).color} fillOpacity={0.12} />
            ))}
            {series.map((s) => (
              <path
                key={`l-${s.provider}`}
                d={s.line}
                fill="none"
                stroke={usageProvider(s.provider).color}
                strokeWidth={2}
                vectorEffect="non-scaling-stroke"
              />
            ))}
            {hover !== null && (
              <line
                x1={hover * stepX}
                x2={hover * stepX}
                y1={PLOT_TOP}
                y2={VIEW_H}
                stroke="currentColor"
                strokeWidth={1}
                className="text-muted-foreground"
                vectorEffect="non-scaling-stroke"
              />
            )}
          </svg>
          {hovered && (
            <div
              ref={tipRef}
              className="pointer-events-none absolute z-10 min-w-36 max-w-full rounded-xl border border-border/50 bg-popover/90 px-2.5 py-2 text-xs shadow-lg backdrop-blur"
            >
              <div className="mb-1 text-muted-foreground">{formatDayShort(hovered.day)}</div>
              {providers.map((p) => {
                const pr = usageProvider(p);
                return (
                  <div key={p} className="flex items-center justify-between gap-3">
                    <span className="flex items-center gap-1.5 text-muted-foreground">
                      {pr.driver ? (
                        <AgentIcon kind={pr.driver} name={pr.label} size="sm" tile={false} className="size-3" />
                      ) : (
                        <span className="size-2 rounded-full" style={{ background: pr.color }} />
                      )}
                      {pr.label}
                    </span>
                    <span className="tabular-nums text-foreground">{format(valueOf(hovered, p))}</span>
                  </div>
                );
              })}
              <div className="mt-1 flex items-center justify-between gap-3 border-t border-border pt-1">
                <span className="text-muted-foreground">Total</span>
                <span className="tabular-nums text-foreground">
                  {format(providers.reduce((n, p) => n + valueOf(hovered, p), 0))}
                </span>
              </div>
            </div>
          )}
        </div>
      </div>
      <div className={cn('flex justify-between pl-16 text-[10px] uppercase text-muted-foreground')}>
        <span>{daily[0] ? formatDayShort(daily[0].day) : ''}</span>
        <span>{mid ? formatDayShort(mid.day) : ''}</span>
        <span>{daily.length ? formatDayShort(daily[daily.length - 1].day) : ''}</span>
      </div>
    </div>
  );
}
