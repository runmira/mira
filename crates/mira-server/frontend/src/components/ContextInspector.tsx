/**
 * The context inspector: what fills the context window, and ways to make
 * room — drop a big tool result, or compact with a focus.
 *
 * The parts are sized by the server and calibrated against the provider's
 * own count for the last request, so they add up to what the model was
 * actually sent (see `mira_harness::context`).
 */
import { useCallback, useEffect, useState } from 'react';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { Loader2 } from 'lucide-react';
import { cn } from '@/lib/utils';
import { shortNum } from '../lib/usage';
import {
  dropContextResult,
  getContextBreakdown,
  type ContextView,
  type DroppedResult,
} from '../api';

const PART_COLORS: Record<string, string> = {
  system: '#a78bfa',
  tools: '#60a5fa',
  memory: '#2dd4bf',
  conversation: '#f472b6',
  tool_results: '#f59e0b',
};

export function ContextInspector({
  open,
  onOpenChange,
  busy,
  onCompact,
  onDropped,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** A turn is running: nothing can be changed until it ends. */
  busy: boolean;
  onCompact: (focus: string) => void;
  /** A result was dropped — note it in the transcript. */
  onDropped: (callId: string, d: DroppedResult) => void;
}) {
  const [view, setView] = useState<ContextView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dropping, setDropping] = useState<string | null>(null);
  const [focus, setFocus] = useState('');

  const load = useCallback(() => {
    setError(null);
    getContextBreakdown()
      .then(setView)
      .catch((e) => setError((e as Error).message));
  }, []);
  useEffect(() => {
    if (open) load();
    else setFocus('');
  }, [open, load]);

  async function drop(callId: string) {
    setDropping(callId);
    try {
      onDropped(callId, await dropContextResult(callId));
      load();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setDropping(null);
    }
  }

  const b = view?.breakdown;
  const total = b?.total ?? 0;
  const window = view?.window ?? 0;
  const scale = Math.max(window, total, 1);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg gap-0 p-0">
        <div className="px-4 pb-3 pt-4">
          <DialogTitle className="text-[15px] font-semibold">Context window</DialogTitle>
          {b && (
            <div className="mt-0.5 text-[12px] text-muted-foreground">
              {shortNum(total)} of {shortNum(window)} tokens ({Math.round((total / scale) * 100)}%)
              {!b.calibrated && ' · estimated until the next reply'}
            </div>
          )}
        </div>

        {error && !b && (
          <div className="mx-4 mb-4 rounded-lg border border-border/60 bg-secondary/50 px-3 py-2 text-[12.5px] text-muted-foreground">
            {error}
          </div>
        )}
        {!b && !error && (
          <div className="flex items-center gap-2 px-4 pb-5 text-[12.5px] text-muted-foreground">
            <Loader2 className="size-3.5 animate-spin" /> Measuring…
          </div>
        )}

        {b && (
          <>
            {/* One bar: each part's share of the window, with where it compacts. */}
            <div className="px-4">
              <div className="relative flex h-2.5 overflow-hidden rounded-full bg-white/[0.06]">
                {b.parts.map((p) =>
                  p.tokens > 0 ? (
                    <div
                      key={p.id}
                      title={`${p.label}: ${shortNum(p.tokens)}`}
                      style={{ width: `${(p.tokens / scale) * 100}%`, background: PART_COLORS[p.id] }}
                    />
                  ) : null,
                )}
                {view?.compact_at != null && window > 0 && (
                  <div
                    className="absolute inset-y-0 w-px bg-white/70"
                    style={{ left: `${(view.compact_at / scale) * 100}%` }}
                    title="Compacts automatically here"
                  />
                )}
              </div>
              <div className="mt-3 grid grid-cols-2 gap-x-6 gap-y-1.5">
                {b.parts.map((p) => (
                  <div key={p.id} className="flex items-center gap-2 text-[12.5px]">
                    <span className="size-2 shrink-0 rounded-full" style={{ background: PART_COLORS[p.id] }} />
                    <span className="min-w-0 flex-1 truncate text-foreground/80">{p.label}</span>
                    <span className="shrink-0 tabular-nums text-muted-foreground">{shortNum(p.tokens)}</span>
                  </div>
                ))}
              </div>
            </div>

            {b.largest_results.length > 0 && (
              <div className="mt-4 border-t border-border/50 px-4 pt-3">
                <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/70">
                  Largest tool results
                </div>
                <div className="max-h-56 overflow-y-auto">
                  {b.largest_results.map((r) => {
                    const share = total ? r.tokens / total : 0;
                    return (
                      <div key={r.call_id} className="group flex items-center gap-2 py-1 text-[12.5px]">
                        <span className="w-16 shrink-0 truncate font-mono text-[11px] text-muted-foreground">{r.tool}</span>
                        <span className="min-w-0 flex-1 truncate text-foreground/80" title={r.label}>
                          {r.label}
                        </span>
                        <span className={cn('shrink-0 tabular-nums', share > 0.15 ? 'text-amber-300' : 'text-muted-foreground')}>
                          {shortNum(r.tokens)}
                        </span>
                        <button
                          type="button"
                          disabled={busy || dropping != null}
                          onClick={() => void drop(r.call_id)}
                          title={busy ? 'Wait for the reply to finish' : 'Replace it with a short note; the model can re-run the tool'}
                          className="shrink-0 rounded px-1.5 py-0.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-white/[0.06] hover:text-foreground disabled:opacity-40"
                        >
                          {dropping === r.call_id ? 'Dropping…' : 'Drop'}
                        </button>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {error && <div className="px-4 pt-2 text-[12px] text-amber-300">{error}</div>}

            <div className="mt-4 flex items-center gap-2 border-t border-border/50 px-4 py-3">
              <input
                value={focus}
                onChange={(e) => setFocus(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !busy) {
                    onCompact(focus.trim());
                    onOpenChange(false);
                  }
                }}
                placeholder="Keep in focus (optional)"
                className="min-w-0 flex-1 rounded-md bg-white/[0.05] px-2.5 py-1.5 text-[12.5px] outline-none placeholder:text-muted-foreground/60 focus:ring-1 focus:ring-white/15"
              />
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  onCompact(focus.trim());
                  onOpenChange(false);
                }}
                className="shrink-0 rounded-md bg-foreground px-3 py-1.5 text-[12px] font-medium text-background disabled:opacity-40"
              >
                Compact now
              </button>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
