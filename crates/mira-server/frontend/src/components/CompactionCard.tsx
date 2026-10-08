import { useEffect, useState } from 'react';
import { ChevronDown, CircleAlert, ListCollapse } from 'lucide-react';
import { ActivityShimmer } from './ActivityShimmer';
import { cn } from '@/lib/utils';

/** One compaction in the chat: where older messages were summarized so the
 *  conversation fits the model's context window. */
export type CompactionEntry = {
  kind: 'compact';
  /** `running` while the summary is being written. Restored history and
   *  older frames are always `done`. */
  state?: 'running' | 'done' | 'failed';
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

const tokens = (n: number) => (n >= 1000 ? `${Math.round(n / 1000)}k` : `${n}`);

function seconds(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

/** A live clock while running. */
function useElapsed(from: number | null | undefined, live: boolean): number | null {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [live]);
  return from ? now - from : null;
}

export function CompactionCard({ entry }: { entry: CompactionEntry }) {
  const [open, setOpen] = useState(false);
  const state = entry.state ?? 'done';
  const running = state === 'running';
  const elapsed = useElapsed(entry.startedAt, running);
  const took = entry.startedAt && entry.endedAt ? entry.endedAt - entry.startedAt : null;

  const doing =
    entry.trigger === 'agent'
      ? 'The agent is summarizing older messages to free up context'
      : entry.trigger === 'auto'
        ? 'Context nearly full: summarizing older messages'
        : 'Summarizing older messages to free up context';
  const facts: string[] = [];
  if (state === 'done') {
    if (entry.summarized != null) facts.push(`${entry.summarized} earlier message${entry.summarized === 1 ? '' : 's'} summarized`);
    if (entry.tokensBefore && entry.tokensAfter != null && entry.tokensAfter < entry.tokensBefore) {
      facts.push(`~${tokens(entry.tokensBefore)} → ~${tokens(entry.tokensAfter)} tokens`);
    }
    if (took != null) facts.push(`in ${seconds(took)}`);
  }

  return (
    <div className="flex flex-col gap-2 py-1.5" role={running ? 'status' : undefined}>
      <div
        className={cn(
          'relative mx-auto flex w-full max-w-[560px] items-center gap-3 overflow-hidden rounded-xl border px-3.5 py-2.5',
          running
            ? 'border-mira-blue/25 bg-mira-blue/[0.05]'
            : state === 'failed'
              ? 'border-destructive/25 bg-destructive/[0.04]'
              : 'border-border/60 bg-fg/[0.02]',
        )}
      >
        {/* A sweep across the card while the summary is written. */}
        {running && <span aria-hidden className="compaction-sweep pointer-events-none absolute inset-0" />}
        <span
          className={cn(
            'relative flex size-7 shrink-0 items-center justify-center rounded-lg',
            running ? 'bg-mira-blue/12 text-mira-blue' : state === 'failed' ? 'bg-destructive/10 text-destructive' : 'bg-fg/[0.06] text-muted-foreground',
          )}
        >
          {state === 'failed' ? (
            <CircleAlert className="size-3.5" aria-hidden />
          ) : (
            <ListCollapse className={cn('size-3.5', running && 'compaction-fold')} aria-hidden />
          )}
        </span>
        <span className="relative min-w-0 flex-1">
          <span className="flex items-baseline gap-2 text-[12.5px] font-medium">
            {running ? (
              <ActivityShimmer active>Compacting the conversation</ActivityShimmer>
            ) : state === 'failed' ? (
              <span className="text-destructive">Couldn't compact the conversation</span>
            ) : (
              <span className="text-foreground/85">Conversation compacted</span>
            )}
            {running && elapsed != null && (
              <span className="font-mono text-[11px] tabular-nums text-muted-foreground/70">{seconds(elapsed)}</span>
            )}
          </span>
          <span className="mt-0.5 block truncate text-[11.5px] text-muted-foreground">
            {running
              ? `${doing}${entry.tokensBefore ? ` · ~${tokens(entry.tokensBefore)} tokens` : ''}`
              : state === 'failed'
                ? `${entry.error ?? 'The summary call failed'}. The chat continues with its full history.`
                : facts.join(' · ') || 'Older messages were summarized to free up context.'}
          </span>
        </span>
        {entry.summary && !running && (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            className="relative inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[11.5px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
          >
            Summary
            <ChevronDown className={cn('size-3 transition-transform', open && 'rotate-180')} />
          </button>
        )}
      </div>
      {open && entry.summary && (
        <pre className="mx-auto max-h-96 w-full max-w-[560px] overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border/60 bg-muted/30 px-3 py-2 font-sans text-[12.5px] leading-relaxed text-foreground/85">
          {entry.summary}
        </pre>
      )}
    </div>
  );
}
