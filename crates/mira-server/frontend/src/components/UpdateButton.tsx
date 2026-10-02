/**
 * The toolbar's "update available" button and the popup behind it: a
 * green disc with a download arrow, shown only when the desktop app's
 * channel has a newer build. The popup says what shipped and installs it.
 */
import { Fragment, type ReactNode, useMemo, useState } from 'react';
import { ArrowDownToLine, Check, Loader2, RotateCw, Sparkles } from 'lucide-react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';
import { parseNotes, useAppUpdate, type UpdateInfo, type UpdatePhase } from '../lib/updates';
import miraLogo from '../assets/mira-logo.png';
import { cn } from '@/lib/utils';

export function UpdateButton() {
  const { update, phase, install } = useAppUpdate();
  const [open, setOpen] = useState(false);
  if (!update) return null;
  return (
    <>
      <div className="group relative" data-tauri-drag-region="false">
        <button
          type="button"
          onClick={() => setOpen(true)}
          aria-label={`Update to Mira ${update.version}`}
          className="relative grid size-8 place-items-center rounded-lg transition-colors hover:bg-secondary"
        >
          {/* A slow ping behind the disc, so it's noticed without nagging. */}
          <span className="absolute size-[22px] animate-[ping_2.4s_cubic-bezier(0,0,0.2,1)_infinite] rounded-full bg-emerald-400/35" />
          <span className="relative grid size-[22px] place-items-center rounded-full bg-emerald-500 text-white shadow-[0_0_0_1px_rgba(16,185,129,0.5),0_4px_12px_-2px_rgba(16,185,129,0.6)]">
            <ArrowDownToLine className="size-3.5" strokeWidth={2.5} />
          </span>
        </button>
        <span className="tooltip pointer-events-none absolute right-0 top-full z-30 mt-1.5 whitespace-nowrap opacity-0 transition-opacity delay-300 group-hover:opacity-100">
          Mira {update.version} is available
        </span>
      </div>
      <UpdateDialog open={open} onOpenChange={setOpen} update={update} phase={phase} onInstall={() => void install()} />
    </>
  );
}

function UpdateDialog({
  open,
  onOpenChange,
  update,
  phase,
  onInstall,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  update: UpdateInfo;
  phase: UpdatePhase;
  onInstall: () => void;
}) {
  const sections = useMemo(() => parseNotes(update.notes ?? ''), [update.notes]);
  const busy = phase.kind === 'downloading' || phase.kind === 'restarting';
  const product = update.channel === 'stable' ? 'Mira' : `Mira ${update.channel === 'alpha' ? 'Alpha' : 'Beta'}`;
  const date = update.date ? new Date(update.date) : null;

  return (
    <Dialog open={open} onOpenChange={(v) => !busy && onOpenChange(v)}>
      <DialogContent className="max-w-[520px] gap-0 overflow-hidden border-fg/[0.08] bg-popover dark:bg-[#111215] p-0">
        {/* Header: the mark on a green glow, the version, what you're on now. */}
        <div className="relative overflow-hidden px-7 pb-6 pt-8">
          <div className="pointer-events-none absolute -top-24 left-1/2 h-56 w-[420px] -translate-x-1/2 rounded-full bg-[radial-gradient(closest-side,rgba(16,185,129,0.22),transparent)]" />
          <div className="relative flex items-center gap-4">
            <span className="relative">
              <img src={miraLogo} alt="" className="size-12 rounded-full shadow-[0_8px_24px_-6px_rgba(30,58,255,0.6)]" draggable={false} />
              <span className="absolute -bottom-1 -right-1 grid size-5 place-items-center rounded-full border-2 border-popover dark:border-[#111215] bg-emerald-500 text-white">
                <ArrowDownToLine className="size-2.5" strokeWidth={3} />
              </span>
            </span>
            <div className="min-w-0">
              <span className="inline-flex items-center gap-1 rounded-full border border-emerald-400/25 bg-emerald-400/10 px-2 py-px text-[10.5px] font-semibold uppercase tracking-[0.08em] text-emerald-300">
                <Sparkles className="size-3" /> New version
              </span>
              <DialogTitle className="mt-1.5 text-[20px] font-semibold tracking-[-0.02em] text-foreground">
                {product} {update.version}
              </DialogTitle>
              <DialogDescription className="mt-0.5 text-[12.5px] text-muted-foreground">
                You're on {update.current}
                {date && !Number.isNaN(date.getTime()) && (
                  <> · released {date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}</>
                )}
              </DialogDescription>
            </div>
          </div>
        </div>

        {/* What shipped. */}
        <div className="max-h-[46vh] overflow-y-auto border-y border-fg/[0.06] bg-shade/20 px-7 py-5">
          {sections.length === 0 ? (
            <p className="text-[13px] leading-relaxed text-muted-foreground">
              {update.notes?.trim() || 'Fixes and improvements.'}
            </p>
          ) : (
            <div className="flex flex-col gap-5">
              {sections.map((s, i) => (
                <section key={i}>
                  {s.heading && (
                    <h3 className="mb-2.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/80">
                      {s.heading}
                    </h3>
                  )}
                  <ul className="flex flex-col gap-2.5">
                    {s.items.map((it, j) => (
                      <li key={j} className="flex gap-3">
                        <span className="mt-[3px] grid size-4 shrink-0 place-items-center rounded-full bg-emerald-500/15 text-emerald-400">
                          <Check className="size-2.5" strokeWidth={3.5} />
                        </span>
                        <span className="text-[13px] leading-relaxed text-muted-foreground">
                          {it.title && <span className="font-medium text-foreground">{inline(it.title)}. </span>}
                          {inline(it.body)}
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          )}
        </div>

        <Footer phase={phase} onLater={() => onOpenChange(false)} onInstall={onInstall} />
      </DialogContent>
    </Dialog>
  );
}

function Footer({ phase, onLater, onInstall }: { phase: UpdatePhase; onLater: () => void; onInstall: () => void }) {
  if (phase.kind === 'downloading' || phase.kind === 'restarting') {
    const pct =
      phase.kind === 'restarting' ? 100 : phase.total ? Math.min(100, (phase.downloaded / phase.total) * 100) : null;
    return (
      <div className="flex flex-col gap-2.5 px-7 py-5">
        <div className="flex items-center justify-between text-[12.5px]">
          <span className="flex items-center gap-2 text-foreground">
            {phase.kind === 'restarting' ? <RotateCw className="size-3.5 animate-spin" /> : <Loader2 className="size-3.5 animate-spin" />}
            {phase.kind === 'restarting' ? 'Restarting into the new version…' : 'Downloading the update…'}
          </span>
          {phase.kind === 'downloading' && (
            <span className="tabular-nums text-muted-foreground">
              {mb(phase.downloaded)}
              {phase.total ? ` / ${mb(phase.total)}` : ''}
            </span>
          )}
        </div>
        <div className="h-1.5 overflow-hidden rounded-full bg-fg/[0.08]">
          <div
            className={cn('h-full rounded-full bg-emerald-500 transition-[width] duration-300', pct === null && 'w-1/3 animate-pulse')}
            style={pct === null ? undefined : { width: `${pct}%` }}
          />
        </div>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-3 px-7 py-5">
      {phase.kind === 'error' && (
        <p className="rounded-lg border border-red-500/20 bg-red-500/[0.07] px-3 py-2 text-[12px] text-red-300">
          The update didn't install: {phase.message}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <span className="mr-auto text-[11.5px] text-muted-foreground/70">Your chats stay as they are.</span>
        <button
          type="button"
          onClick={onLater}
          className="h-9 rounded-lg px-3.5 text-[13px] text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
        >
          Later
        </button>
        <button
          type="button"
          onClick={onInstall}
          className="inline-flex h-9 items-center gap-2 rounded-lg bg-emerald-500 px-4 text-[13px] font-semibold text-white shadow-[0_8px_20px_-8px_rgba(16,185,129,0.8)] transition-[filter,transform] hover:brightness-110 active:translate-y-px"
        >
          <ArrowDownToLine className="size-4" strokeWidth={2.5} />
          {phase.kind === 'error' ? 'Try again' : 'Update now'}
        </button>
      </div>
    </div>
  );
}

function mb(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

/** `code` and **bold** inside a note line. */
function inline(text: string): ReactNode {
  return text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') && part.length > 2 ? (
      <code key={i} className="rounded bg-fg/[0.07] px-1 py-px font-mono text-[11.5px] text-foreground/85">
        {part.slice(1, -1)}
      </code>
    ) : part.startsWith('**') && part.endsWith('**') && part.length > 4 ? (
      <span key={i} className="font-medium text-foreground">
        {part.slice(2, -2)}
      </span>
    ) : (
      <Fragment key={i}>{part}</Fragment>
    ),
  );
}
