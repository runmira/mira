/**
 * A download's progress: a blue bar that fills as the server reports it,
 * with a soft sweep while it's moving. With no number yet it glides.
 */
import { cn } from '@/lib/utils';

export function InstallProgress({ label, value, className }: { label: string; value: number | null; className?: string }) {
  const pct = value === null ? null : Math.round(value * 100);
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <div className="flex items-baseline justify-between gap-3 text-[12px]">
        <span className="text-foreground/85">{label}</span>
        {pct !== null && <span className="tabular-nums text-muted-foreground">{pct}%</span>}
      </div>
      <div
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct ?? undefined}
        className="relative h-1.5 overflow-hidden rounded-full bg-fg/[0.08]"
      >
        {pct === null ? (
          <span className="progress-glide absolute inset-y-0 left-0 w-2/5 rounded-full bg-gradient-to-r from-transparent via-mira-blue to-transparent" />
        ) : (
          <span
            className="absolute inset-y-0 left-0 overflow-hidden rounded-full bg-gradient-to-r from-mira-blue/80 to-mira-blue shadow-[0_0_12px_rgba(122,162,247,0.55)] transition-[width] duration-500 ease-out"
            style={{ width: `${Math.max(pct, 3)}%` }}
          >
            {pct < 100 && <span className="compaction-sweep absolute inset-0" />}
          </span>
        )}
      </div>
    </div>
  );
}
