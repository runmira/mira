import { cn } from '@/lib/utils';
import { useEffect, useRef, useState } from 'react';
import type { Origin } from '../../api';
import { originIconSrc, type PaletteGroup, type SlashCommand } from '../commands';
/* ---------- slash palette ---------- */

export function SlashPalette({
  groups,
  activeIdx,
  onHover,
  onPick,
}: {
  /** Sections in display order; `activeIdx` counts across all of them. */
  groups: PaletteGroup[];
  activeIdx: number;
  onHover: (i: number) => void;
  onPick: (cmd: SlashCommand) => void;
}) {
  const activeRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: 'nearest' });
  }, [activeIdx]);
  // Headers only help when there's more than one section.
  const showHeaders = groups.length > 1 || groups.some((g) => g.origin);
  let index = -1;

  return (
    <div
      className={cn(
        'absolute bottom-full left-0 right-0 z-10 mb-2 mx-auto max-w-xl overflow-hidden',
        'rounded-2xl border border-fg/[0.07] bg-popover/95 dark:bg-[#1f2024]/95 backdrop-blur-md',
        'shadow-[0_20px_50px_-16px_rgba(0,0,0,0.85)] ring-1 ring-shade/40',
        'animate-fade-in',
      )}
      role="listbox"
    >
      <div className="max-h-[24rem] overflow-y-auto py-1.5">
        {groups.map((g, gi) => (
          <div key={g.key} role="group" aria-label={g.label || undefined}>
            {showHeaders && g.label && (
              <div
                className={cn(
                  'flex items-center gap-2 px-4 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground/70',
                  gi > 0 && 'mt-1 border-t border-fg/[0.05]',
                )}
              >
                {g.origin && <OriginIcon origin={g.origin} />}
                <span className="normal-case tracking-normal text-[12px] text-foreground/80">
                  {g.label}
                </span>
                {g.hint && (
                  <span className="font-normal normal-case tracking-normal text-muted-foreground/50">
                    {g.hint}
                  </span>
                )}
              </div>
            )}
            {g.items.map((cmd) => {
              index += 1;
              const i = index;
              const Icon = cmd.icon;
              const active = i === activeIdx;
              return (
                <button
                  key={`${g.key}:${cmd.name}`}
                  ref={active ? activeRef : undefined}
                  type="button"
                  onMouseEnter={() => onHover(i)}
                  onClick={() => onPick(cmd)}
                  role="option"
                  aria-selected={active}
                  className={cn(
                    'flex w-full items-baseline gap-3 px-4 py-1.5 text-left transition-colors',
                    active ? 'bg-fg/[0.06]' : 'hover:bg-fg/[0.035]',
                  )}
                >
                  <Icon
                    className={cn(
                      'size-[15px] shrink-0 self-center transition-colors',
                      active ? 'text-foreground/85' : 'text-foreground/55',
                    )}
                  />
                  <span
                    className={cn(
                      'shrink-0 text-[13.5px] font-medium tracking-tight',
                      active ? 'text-foreground' : 'text-foreground/90',
                    )}
                  >
                    {displayName(cmd)}
                  </span>
                  <span
                    className={cn(
                      'min-w-0 flex-1 truncate text-[13px]',
                      active ? 'text-muted-foreground' : 'text-muted-foreground/70',
                    )}
                  >
                    {cmd.description}
                  </span>
                  {cmd.kindLabel && (
                    <span className="shrink-0 self-center rounded border border-fg/[0.08] px-1.5 text-[10px] uppercase tracking-wide text-muted-foreground/70">
                      {cmd.kindLabel}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

/** A plugin or server's icon, falling back to a letter badge. */
export function OriginIcon({
  origin,
  fallback,
}: {
  origin: Origin;
  fallback?: SlashCommand['icon'];
}) {
  const src = originIconSrc(origin);
  const [failed, setFailed] = useState(false);
  if (src && !failed) {
    return (
      <img
        src={src}
        alt=""
        className="size-3.5 shrink-0 rounded-[3px]"
        onError={() => setFailed(true)}
      />
    );
  }
  if (fallback) {
    const F = fallback;
    return <F className="size-3.5 shrink-0 text-muted-foreground" />;
  }
  return (
    <span className="flex size-3.5 shrink-0 items-center justify-center rounded-[3px] bg-fg/10 text-[9px] font-semibold text-foreground/80">
      {origin.label.charAt(0).toUpperCase()}
    </span>
  );
}

/** Human-facing label for a command. Falls back to Title-Case of the
 *  `name` so single-word commands ("new" → "New") don't need a hand-
 *  written label, while multi-word commands ("pull-request") can
 *  override with a proper display string ("Pull request"). */
export function displayName(cmd: SlashCommand): string {
  const n = cmd.label ?? cmd.name;
  if (!n) return n;
  return n.charAt(0).toUpperCase() + n.slice(1).replace(/-/g, ' ');
}

/* ---------- slash button (opens the palette by seeding "/") ---------- */

/**
 * Custom slash glyph — Phosphor's `*-Slash` icons are all "crossed-out"
 * variants (BellSlash, ChatSlash, …), and a bare `/` character is too
 * thin to sit alongside chunky filled icons in the composer bar.
 *
 * SVG: rounded, italic-slanted forward slash with a thick stroke.
 * Sized like the other 16px composer icons; `currentColor` inherits the
 * button's text colour so the muted → foreground hover transition still
 * works.
 */
export function SlashGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="2.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M15.5 4.5 L8.5 19.5" />
    </svg>
  );
}

export function SlashButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title="Slash commands"
      aria-label="Open slash palette"
      className="inline-flex size-8 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
    >
      <SlashGlyph className="size-4" />
    </button>
  );
}
