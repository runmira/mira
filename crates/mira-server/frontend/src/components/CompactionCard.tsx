import { useId, useState } from 'react';
import { ChevronDown, CircleAlert, Minimize2 } from 'lucide-react';
import { ActivityShimmer } from './ActivityShimmer';
import { cn } from '@/lib/utils';

export type CompactionEntry = {
  kind: 'compact';
  /** Native lifecycle receipts retain running/terminal state on replay. */
  state?: 'running' | 'done' | 'failed' | 'cancelled';
  nativeCompactionId?: string;
  /** `manual` (/compact or the context ring), `auto` (the history outgrew
   *  the window mid-turn), or `agent` (an external agent compacting). */
  trigger?: 'manual' | 'auto' | 'agent';
  startedAt?: number | null;
  endedAt?: number | null;
  summarized: number | null;
  summary: string | null;
  tokensBefore?: number | null;
  tokensAfter?: number | null;
  error?: string | null;
};


const tokens = (n: number) => n >= 1000 ? `${Math.round(n / 1000)}k` : `${n}`;

/** A quiet transcript boundary, following T3's compaction timeline row. */
export function CompactionCard({ entry }: { entry: CompactionEntry }) {
  const [open, setOpen] = useState(false);
  const detailId = useId();
  const state = entry.state ?? 'done';
  const running = state === 'running';
  const failed = state === 'failed';
  const label = running ? 'Compacting context' : failed ? 'Compaction failed' : state === 'cancelled' ? 'Compaction cancelled' : 'Context compacted';
  const facts: string[] = [];
  if (state === 'done') {
    if (entry.summarized != null) facts.push(`${entry.summarized} messages`);
    if (entry.tokensBefore != null && entry.tokensAfter != null) facts.push(`~${tokens(entry.tokensBefore)} → ~${tokens(entry.tokensAfter)} tokens`);
  }
  const error = failed ? entry.error ?? 'The agent could not finish compaction.' : null;
  const hasDetails = !!entry.summary || !!error;
  const content = <>
    <span className="grid size-4 shrink-0 place-items-center" aria-hidden>
      {failed ? <CircleAlert className="size-3" /> : <Minimize2 className="size-3" />}
    </span>
    <ActivityShimmer active={running} className={failed ? 'text-destructive' : undefined}>{label}</ActivityShimmer>
    {facts.length > 0 && <span className="hidden text-[11px] text-muted-foreground/70 sm:inline">· {facts.join(' · ')}</span>}
    {hasDetails && <ChevronDown className={cn('size-3 shrink-0 text-muted-foreground/70 transition-transform',open && 'rotate-180')} aria-hidden />}
  </>;
  return <div className="min-w-0 py-1" data-compaction-state={state}>
    <div className={cn('flex min-w-0 items-center gap-3 text-xs text-muted-foreground',failed && 'text-destructive')}>
      <span aria-hidden className="h-px min-w-3 flex-1 bg-border/70" />
      {hasDetails ? <button type="button" onClick={()=>setOpen(value=>!value)} data-transcript-disclosure
        aria-expanded={open} aria-controls={detailId} aria-label={`${label}: ${open ? 'Hide' : 'Show'} details`}
        className="inline-flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-fg/[0.04] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        {content}
      </button> : <span role={running ? 'status' : 'separator'} aria-label={label} className="inline-flex min-w-0 items-center gap-1.5 px-1.5 py-1">{content}</span>}
      <span aria-hidden className="h-px min-w-3 flex-1 bg-border/70" />
    </div>
    {open && hasDetails && <div id={detailId} className="mt-2 min-w-0 rounded-md border border-border/60 bg-fg/[0.02] px-3 py-2.5 text-[12px] leading-relaxed">
      {error && <p className="whitespace-pre-wrap break-words text-destructive">{error}</p>}
      {entry.summary && <>
        <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">Retained context{running ? ' · updating' : ''}</p>
        <pre data-live-output className="max-h-60 overflow-auto overscroll-contain whitespace-pre-wrap break-words font-sans text-foreground/85">{entry.summary}</pre>
      </>}
    </div>}
  </div>;
}
