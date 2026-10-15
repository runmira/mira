import { PanelLeft, Search, SquarePen } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { TRAFFIC_LIGHT_INSET } from '../lib/desktop';
import {
  shortcutLabelForCommand,
  type MiraKeybindingCommand,
  type ResolvedKeybindingsConfig,
} from '../lib/keybindings';
import { cn } from '../lib/utils';

/** Height of the strip the traffic lights sit in. The sidebar's own header
 *  row is the same height. */
export const WINDOW_STRIP = TRAFFIC_LIGHT_INSET.top + 8;

const BUTTON = 28;
const GAP = 6;
const BUTTONS = 3;
/** Where the controls start: after the traffic lights, or at the window's
 *  edge in fullscreen, where macOS hides the lights. */
const controlsLeft = (lights: boolean) => (lights ? TRAFFIC_LIGHT_INSET.left : 14);
const controlsRight = (lights: boolean) => controlsLeft(lights) + BUTTONS * BUTTON + (BUTTONS - 1) * GAP;

/**
 * The desktop app's window controls, pinned beside the macOS traffic lights
 * the way a native Mac app does it: the sidebar toggle, a new thread, and
 * search (the command palette, which finds chats too).
 *
 * They stay put whatever the main pane shows — chat, Settings, plugins —
 * and whether the sidebar is open or not, so there is always one place to
 * reach for. With the sidebar hidden, the main pane keeps its full height
 * and curves around them instead: an inverted-L notch cut from its top-left
 * corner (see `notchedPath`), which drags the window like a title bar.
 */
export function WindowControls({
  lights,
  sidebarOpen,
  onToggleSidebar,
  onNewChat,
  onSearch,
  keybindings,
}: {
  /** The traffic lights are showing (not fullscreen). */
  lights: boolean;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onNewChat: () => void;
  onSearch: () => void;
  keybindings: ResolvedKeybindingsConfig;
}) {
  const [hint, setHint] = useState<{ label: string; shortcut: string | null } | null>(null);
  // Hints wait a beat, like native tooltips, and go at once.
  const hintTimer = useRef<number | undefined>(undefined);
  const showHint = (next: { label: string; shortcut: string | null }) => {
    window.clearTimeout(hintTimer.current);
    hintTimer.current = window.setTimeout(() => setHint(next), 350);
  };
  const hideHint = () => {
    window.clearTimeout(hintTimer.current);
    setHint(null);
  };
  useEffect(() => () => window.clearTimeout(hintTimer.current), []);
  const controls: {
    label: string;
    icon: React.ReactNode;
    command: MiraKeybindingCommand;
    onClick: () => void;
    expanded?: boolean;
  }[] = [
    {
      label: sidebarOpen ? 'Hide sidebar' : 'Show sidebar',
      icon: <PanelLeft className="size-4" strokeWidth={1.75} />,
      command: 'sidebar.toggle',
      onClick: onToggleSidebar,
      expanded: sidebarOpen,
    },
    {
      label: 'New thread',
      icon: <SquarePen className="size-[15px]" strokeWidth={1.75} />,
      command: 'chat.new',
      onClick: onNewChat,
    },
    {
      label: 'Search',
      icon: <Search className="size-[15px]" strokeWidth={1.75} />,
      command: 'palette.toggle',
      onClick: onSearch,
    },
  ];
  return (
    <div
      data-window-drag-region={sidebarOpen ? undefined : ''}
      className={cn('fixed left-0 top-0 z-40', sidebarOpen && 'pointer-events-none')}
      style={{ height: WINDOW_STRIP, width: notchWidth(lights), paddingLeft: controlsLeft(lights) }}
    >
      {/* Centred on the traffic lights' line. */}
      <div className="relative flex items-center" style={{ height: TRAFFIC_LIGHT_INSET.centerY * 2, gap: GAP }}>
        {controls.map((c) => (
          <button
            key={c.command}
            type="button"
            onClick={() => {
              hideHint();
              c.onClick();
            }}
            aria-label={c.label}
            aria-expanded={c.expanded}
            onPointerEnter={() => showHint({ label: c.label, shortcut: shortcutLabelForCommand(keybindings, c.command) })}
            onPointerLeave={hideHint}
            onBlur={hideHint}
            className="pointer-events-auto flex shrink-0 items-center justify-center rounded-md text-foreground/60 transition-colors hover:bg-fg/[0.07] hover:text-foreground"
            style={{ width: BUTTON, height: BUTTON }}
          >
            {c.icon}
          </button>
        ))}
        {/* One hint, beside the group rather than under it: below sits the
            sidebar's Chat/Work switch, which it would cover. */}
        {hint && (
          <span
            aria-hidden
            className="tooltip pointer-events-none absolute left-full top-1/2 z-50 ml-2 -translate-y-1/2 whitespace-nowrap"
          >
            {hint.label}
            {hint.shortcut && <span className="tooltip-tag">{hint.shortcut}</span>}
          </span>
        )}
      </div>
    </div>
  );
}

/** How far the hidden-sidebar pane's notch reaches, in window
 *  coordinates: past the controls, with a little room. */
export function notchWidth(lights: boolean): number {
  return controlsRight(lights) + 10;
}

/** The notch is exactly as tall as the pane's toolbar (`h-11`), so its
 *  ledge and the toolbar's divider are one line. */
export const NOTCH = { height: 44, radius: 12 } as const;

/** Outline of a `w`×`h` rounded rectangle with an inverted-L notch cut from
 *  its top-left: the pane's top edge starts after the notch, its left edge
 *  below it.
 *
 *  The notch's corners are eased rather than circular: each curve starts
 *  well before the corner and leans into it gradually (the "smooth corner"
 *  of Apple's own shapes), so the step down around the lights reads as one
 *  soft S instead of two arcs joined by a line. The pane's outer corners
 *  stay plain arcs, matching every other card. */
export function notchedPath(w: number, h: number, nw: number): string {
  const { height: nh, radius: r } = NOTCH;
  // How far each eased curve reaches along its edges, capped so the two on
  // the notch's vertical edge meet in the middle rather than overlap.
  const e = Math.min(r * 1.6, nh / 2);
  const k = 0.42; // how near the corner the curve bends: higher is softer
  const p = (x: number, y: number) => `${round(x)} ${round(y)}`;
  return [
    `M ${p(0, nh + e)}`,
    // Left edge turning onto the ledge under the lights.
    `C ${p(0, nh + e * k)} ${p(e * k, nh)} ${p(e, nh)}`,
    `L ${p(nw - e, nh)}`,
    // The notch's own corner: the pane curves in around the lights...
    `C ${p(nw - e * k, nh)} ${p(nw, nh - e * k)} ${p(nw, nh - e)}`,
    // ...and straight back out onto the top edge.
    `C ${p(nw, e * k)} ${p(nw + e * k, 0)} ${p(nw + e, 0)}`,
    `L ${p(w - r, 0)}`,
    `A ${r} ${r} 0 0 1 ${p(w, r)}`,
    `L ${p(w, h - r)}`,
    `A ${r} ${r} 0 0 1 ${p(w - r, h)}`,
    `L ${p(r, h)}`,
    `A ${r} ${r} 0 0 1 ${p(0, h - r)}`,
    'Z',
  ].join(' ');
}

function round(n: number): number {
  return Math.round(n * 100) / 100;
}

/** The pane's size while `enabled`, for cutting the notch. */
export function usePaneSize(ref: React.RefObject<HTMLElement | null>, enabled: boolean) {
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!enabled || !node) {
      setSize(null);
      return;
    }
    const measure = () => setSize({ w: node.offsetWidth, h: node.offsetHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [ref, enabled]);
  return size;
}

/** The pane's border, drawn along the notched outline (a CSS border can't
 *  follow a clip-path). The stroke is centred on the clip edge, so twice
 *  the width leaves a 1px line inside. */
export function NotchOutline({ w, h, nw }: { w: number; h: number; nw: number }) {
  return (
    <svg
      aria-hidden
      className="pointer-events-none absolute inset-0 z-30 text-border"
      width={w}
      height={h}
      viewBox={`0 0 ${w} ${h}`}
    >
      <path d={notchedPath(w, h, nw)} fill="none" stroke="currentColor" strokeWidth={2} />
    </svg>
  );
}
