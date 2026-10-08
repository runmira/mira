/**
 * The toolbar's update button and the popup behind it. Always there: a
 * quiet disc when Mira is up to date (the popup says so and can check
 * again), a green one with a soft ping when a newer build is out (the
 * popup says what shipped and installs it — or, for the CLI behind a
 * browser, gives the command).
 */
import { useState } from 'react';
import { ArrowDownToLine, Loader2 } from 'lucide-react';
import { useAppUpdate } from '../lib/updates';
import { lazyNamed, preloadOnIntent, useLatch } from '../lib/lazy';
import { LazyBoundary } from './LazyBoundary';

const UpdateDialogs = lazyNamed(() => import('./UpdateDialogs'), 'UpdateDialogs');

export function UpdateButton() {
  const u = useAppUpdate();
  const [open, setOpen] = useState(false);
  // Mounted from the first open on, so closing still animates.
  const shown = useLatch(open);
  const update = u.update;
  const tip = update
    ? `Mira ${update.version} is available`
    : u.current
      ? `Mira ${u.current} · up to date`
      : 'Updates';
  return (
    <>
      <div className="group relative" data-tauri-drag-region="false" {...preloadOnIntent(UpdateDialogs.preload)}>
        <button
          type="button"
          onClick={() => setOpen(true)}
          aria-label={update ? `Update to Mira ${update.version}` : 'Mira is up to date'}
          className="relative grid size-8 place-items-center rounded-lg transition-colors hover:bg-secondary"
        >
          {update ? (
            <>
              {/* A slow ping behind the disc, so it's noticed without nagging. */}
              <span className="absolute size-[22px] animate-[ping_2.4s_cubic-bezier(0,0,0.2,1)_infinite] rounded-full bg-emerald-400/35" />
              <span className="relative grid size-[22px] place-items-center rounded-full bg-emerald-500 text-white shadow-[0_0_0_1px_rgba(16,185,129,0.5),0_4px_12px_-2px_rgba(16,185,129,0.6)]">
                <ArrowDownToLine className="size-3.5" strokeWidth={2.5} />
              </span>
            </>
          ) : (
            <span className="relative grid size-[22px] place-items-center rounded-full bg-fg/[0.06] text-muted-foreground ring-1 ring-fg/[0.08] transition-colors group-hover:text-foreground">
              {u.checking ? <Loader2 className="size-3.5 animate-spin" /> : <ArrowDownToLine className="size-3.5" strokeWidth={2.25} />}
            </span>
          )}
        </button>
        {/* Above: the button sits at the bottom of the sidebar. */}
        <span className="tooltip pointer-events-none absolute bottom-full right-0 z-30 mb-1.5 whitespace-nowrap opacity-0 transition-opacity delay-300 group-hover:opacity-100">
          {tip}
        </span>
      </div>
      {shown && (
        <LazyBoundary>
          <UpdateDialogs u={u} open={open} onOpenChange={setOpen} />
        </LazyBoundary>
      )}
    </>
  );
}
