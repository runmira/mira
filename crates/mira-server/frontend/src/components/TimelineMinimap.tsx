/**
 * Timeline minimap: a tick stack on a faint spine, docked left of the
 * chat column — one short dash per user turn, the active one long and
 * bright. Hover a tick for a message preview, click (or Enter) to jump
 * there, arrow keys step between turns.
 *
 * Matches the reference UI's minimap visual 1:1 (tick sizes, spacing,
 * spine, preview card); only the reveal differs — ours is always
 * visible instead of hover-gated.
 */
import { useCallback, useEffect, useState, type RefObject } from 'react';

import { cn } from '@/lib/utils';

export interface MinimapItem {
  readonly id: string;
  readonly userText: string;
  readonly assistantText: string | null;
}

const MIN_ITEMS = 3;
/** Vertical px per turn gap; the strip caps at MAX_STRIP_PX. */
const ITEM_SPACING_PX = 8;
const MAX_STRIP_PX = 420;

function compact(text: string | null | undefined): string {
  return (text ?? '').replace(/\s+/g, ' ').trim();
}

export function TimelineMinimap({
  items,
  paneRef,
  onSelect,
}: {
  items: ReadonlyArray<MinimapItem>;
  /** The scrollable transcript pane. Tracked for the active tick. */
  paneRef: RefObject<HTMLDivElement | null>;
  /** Jump the pane to the turn with this id. */
  onSelect: (id: string) => void;
}) {
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  // Measured left edge: comfortably left of the actual text column,
  // clamped into the pane. Guessing from max-w-3xl breaks whenever the
  // pane is narrower than the full column width (panel open, small window).
  const [left, setLeft] = useState<number | null>(null);
  /** Peek rail: the pointer is on it. */
  const [engaged, setEngaged] = useState(false);
  /** Peek rail: the pane scrolled a moment ago. */
  const [awake, setAwake] = useState(false);

  useEffect(() => {
    const pane = paneRef.current;
    if (!pane) return;
    const update = () => {
      const col = pane.querySelector('[data-transcript-column]');
      const host = pane.parentElement;
      if (!(col instanceof HTMLElement) || !host) return;
      const gap =
        col.getBoundingClientRect().left - host.getBoundingClientRect().left - 72;
      // No room beside the column: hide rather than clamp onto the text.
      // Clamping to the pane edge drew the ticks over the transcript
      // (they read as a stray "☰" on top of "Worked for …").
      setLeft(gap >= 8 ? gap : -1);
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(pane);
    // The column may not exist on the first pass (the transcript renders
    // after the minimap mounts); watch it too once it does.
    const col = pane.querySelector('[data-transcript-column]');
    if (col) ro.observe(col);
    window.addEventListener('resize', update);
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', update);
    };
  }, [paneRef, items.length]);

  // Active tick follows the scroll position (fraction of total scroll).
  useEffect(() => {
    const pane = paneRef.current;
    if (!pane || items.length === 0) return;
    const update = () => {
      const max = pane.scrollHeight - pane.clientHeight;
      const frac = max <= 0 ? 0 : Math.min(1, Math.max(0, pane.scrollTop / max));
      setActiveIndex(Math.round(frac * (items.length - 1)));
    };
    update();
    // Scrolling wakes the peek rail briefly, so it is discoverable without
    // ever sitting on screen at rest.
    let sleep: ReturnType<typeof setTimeout> | undefined;
    const onScroll = () => {
      update();
      setAwake(true);
      clearTimeout(sleep);
      sleep = setTimeout(() => setAwake(false), 1400);
    };
    pane.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      clearTimeout(sleep);
      pane.removeEventListener('scroll', onScroll);
    };
  }, [paneRef, items.length]);

  const jump = useCallback(
    (index: number) => {
      const item = items[index];
      if (item) onSelect(item.id);
    },
    [items, onSelect],
  );

  const step = useCallback(
    (delta: number) => {
      const base = hoverIndex ?? activeIndex ?? 0;
      const next = Math.max(0, Math.min(items.length - 1, base + delta));
      jump(next);
    },
    [hoverIndex, activeIndex, items.length, jump],
  );

  if (items.length < MIN_ITEMS) return null;
  // Unmeasured: render nothing rather than guess a position — a guessed
  // position is what put the ticks on top of the transcript.
  if (left === null) return null;
  // No room beside the column: the compact "peek" rail instead.
  const peek = left < 0;

  const stripPx = Math.min(MAX_STRIP_PX, Math.max(1, (items.length - 1) * ITEM_SPACING_PX));
  const topPercent = (index: number) =>
    items.length === 1 ? 50 : (index / (items.length - 1)) * 100;

  // The preview is hover-only: it must vanish on mouse-leave, never
  // linger on the active tick.
  const previewItem = hoverIndex == null ? null : (items[hoverIndex] ?? null);
  // Proximity sizing may fall back to the active tick (visual only).
  const focusIndex = hoverIndex ?? activeIndex;

  // Peek rail: hugs the pane's left edge, inside its padding, so it can
  // never sit on the text. Invisible at rest; a faint glimmer while the
  // pane scrolls; on hover it opens into a frosted-glass rail.
  const peekVisible = engaged || awake;

  return (
    <div
      data-testid="timeline-minimap"
      className={cn(
        'absolute top-1/2 z-10 -translate-y-1/2 select-none',
        peek
          ? cn(
              'left-0.5 block rounded-full transition-[width,opacity,background-color,box-shadow] duration-200',
              engaged
                ? 'w-8 border border-white/10 bg-background/55 opacity-100 shadow-[0_6px_24px_rgba(0,0,0,0.35),inset_0_1px_0_rgba(255,255,255,0.08)] backdrop-blur-md'
                : peekVisible
                  ? 'w-3 opacity-60'
                  : 'w-3 opacity-0',
            )
          : 'hidden w-8 sm:block',
      )}
      // Measured beside the text column; the peek rail rides the edge.
      style={peek ? { height: stripPx + (engaged ? 16 : 0) } : { height: stripPx, left }}
      onMouseEnter={() => peek && setEngaged(true)}
      onMouseLeave={() => {
        setHoverIndex(null);
        setEngaged(false);
      }}
    >
      <div className={cn('relative h-full w-full', peek && engaged && 'my-2 h-[calc(100%-16px)]')}>
        {/* Faint spine */}
        {!peek && <div className="absolute left-3 top-0 h-full w-px bg-border/15" aria-hidden />}

        {/* Ticks */}
        {items.map((item, index) => {
          const distance = focusIndex == null ? null : Math.abs(index - focusIndex);
          return (
            <button
              key={item.id}
              type="button"
              aria-label={`Jump to: ${compact(item.userText).slice(0, 80) || 'User message'}`}
              onMouseEnter={() => setHoverIndex(index)}
              onFocus={() => setHoverIndex(index)}
              onBlur={() => setHoverIndex(null)}
              onClick={() => jump(index)}
              onKeyDown={(event) => {
                if (event.key === 'ArrowDown') {
                  event.preventDefault();
                  step(1);
                } else if (event.key === 'ArrowUp') {
                  event.preventDefault();
                  step(-1);
                } else if (event.key === 'Home') {
                  event.preventDefault();
                  jump(0);
                } else if (event.key === 'End') {
                  event.preventDefault();
                  jump(items.length - 1);
                }
              }}
              className={cn(
                'group absolute -translate-y-1/2 rounded-full focus-visible:outline-none',
                peek ? (engaged ? 'left-0 px-2 py-1' : 'left-0 px-0.5 py-1') : 'left-0 px-3 py-1.5',
              )}
              style={{ top: `${topPercent(index)}%` }}
            >
              <span
                aria-hidden
                className={cn(
                  'block h-0.5 rounded-full transition-[width,background-color] duration-150',
                  peek && !engaged
                    ? distance === 0
                      ? 'w-2 bg-foreground/80'
                      : 'w-1 bg-muted-foreground/60'
                    : distance === 0
                      ? 'w-6 bg-muted-foreground/75'
                      : distance === 1
                        ? 'w-4 bg-muted-foreground/35 group-hover:bg-foreground/70'
                        : distance === 2
                          ? 'w-2.5 bg-muted-foreground/35 group-hover:bg-foreground/70'
                          : 'w-2 bg-muted-foreground/35 group-hover:bg-foreground/70',
                )}
              />
            </button>
          );
        })}

        {/* Hover preview */}
        {previewItem && hoverIndex != null ? (
          <div
            className={cn('pointer-events-none absolute z-20 w-80', peek ? 'left-10' : 'left-8')}
            style={{
              top: `${topPercent(hoverIndex)}%`,
              transform: 'translateY(-12%)',
            }}
          >
            <div className="block rounded-xl border border-border bg-popover p-3 text-left shadow-xl shadow-black/25">
              <div className="block max-w-full overflow-hidden text-ellipsis whitespace-nowrap text-sm font-medium leading-5 text-foreground">
                {compact(previewItem.userText) || 'User message'}
              </div>
              {previewItem.assistantText ? (
                <div
                  className="mt-1 max-h-[3.75rem] overflow-hidden text-sm leading-5 text-muted-foreground"
                  style={{
                    display: '-webkit-box',
                    WebkitBoxOrient: 'vertical',
                    WebkitLineClamp: 3,
                  }}
                >
                  {compact(previewItem.assistantText)}
                </div>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
