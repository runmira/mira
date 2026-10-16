/** One quantity split into its parts: a segmented bar and a legend. */
import type { ReactNode } from 'react';
import { formatPercent } from './usageModel';

export type ShareSegment = { label: string; value: number; color: string };

export function UsageShareBar({
  label,
  segments,
  format,
  aside,
}: {
  label: string;
  segments: ShareSegment[];
  format: (value: number) => string;
  aside?: ReactNode;
}) {
  const visible = segments.filter((s) => s.value > 0);
  const total = visible.reduce((n, s) => n + s.value, 0);
  if (total <= 0) return null;
  return (
    <div className="flex min-w-0 flex-col gap-2.5">
      <div className="flex items-baseline justify-between gap-3">
        <h3 className="text-[13px] font-medium text-foreground">{label}</h3>
        {aside}
      </div>
      <div
        role="img"
        aria-label={`${label}: ${visible.map((s) => `${s.label} ${format(s.value)}`).join(', ')}`}
        className="flex h-2 gap-0.5"
      >
        {visible.map((s) => (
          <div
            key={s.label}
            title={`${s.label} · ${format(s.value)} · ${formatPercent(s.value / total)}`}
            className="h-full min-w-1 rounded-[2px] first:rounded-l-full last:rounded-r-full"
            style={{ flex: `${s.value} 1 0`, backgroundColor: s.color }}
          />
        ))}
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
        {visible.map((s) => (
          <span key={s.label} className="flex items-center gap-1.5">
            <span aria-hidden className="size-2 rounded-[2px]" style={{ backgroundColor: s.color }} />
            <span className="text-muted-foreground">{s.label}</span>
            <span className="tabular-nums text-foreground">{format(s.value)}</span>
          </span>
        ))}
      </div>
    </div>
  );
}
