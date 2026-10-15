import { PanelLeft } from 'lucide-react';
import { useLayoutEffect, useState } from 'react';
import { TRAFFIC_LIGHT_INSET } from '../lib/desktop';
import { shortcutLabelForCommand, type ResolvedKeybindingsConfig } from '../lib/keybindings';
import { cn } from '../lib/utils';

/** Height of the strip the traffic lights sit in. The sidebar's own header
 *  row is the same height. */
export const WINDOW_STRIP = TRAFFIC_LIGHT_INSET.top + 8;

/**
 * The desktop app's window controls: the sidebar toggle, pinned beside the
 * macOS traffic lights the way a native Mac app does it.
 *
 * It stays put whatever the main pane shows — chat, Settings, plugins — and
 * whether the sidebar is open or not, so there is always one place to
 * reach for. With the sidebar hidden, the main pane keeps its full height
 * and curves around them instead: an inverted-L notch cut from its top-left
 * corner (see `NOTCH`), which drags the window like a title bar.
 */
export function WindowControls({
  sidebarOpen,
  onToggleSidebar,
  keybindings,
}: {
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  keybindings: ResolvedKeybindingsConfig;
}) {
  const label = sidebarOpen ? 'Hide sidebar' : 'Show sidebar';
  const shortcut = shortcutLabelForCommand(keybindings, 'sidebar.toggle');
  return (
    <div
      data-window-drag-region={sidebarOpen ? undefined : ''}
      className={cn('fixed left-0 top-0 z-40', sidebarOpen && 'pointer-events-none')}
      style={{ height: WINDOW_STRIP, width: NOTCH.width, paddingLeft: TRAFFIC_LIGHT_INSET.left }}
    >
      {/* Centred on the traffic lights' line. */}
      <div className="flex items-center" style={{ height: TRAFFIC_LIGHT_INSET.centerY * 2 }}>
        <div className="group pointer-events-auto relative">
          <button
            type="button"
            onClick={onToggleSidebar}
            aria-label={label}
            aria-expanded={sidebarOpen}
            className="flex size-7 items-center justify-center rounded-md text-foreground/60 transition-colors hover:bg-fg/[0.07] hover:text-foreground"
          >
            <PanelLeft className="size-4" strokeWidth={1.75} />
          </button>
          <span className="tooltip pointer-events-none absolute left-0 top-full z-50 mt-1 whitespace-nowrap opacity-0 transition-opacity delay-300 group-hover:opacity-100">
            {label}
            {shortcut && <span className="tooltip-tag">{shortcut}</span>}
          </span>
        </div>
      </div>
    </div>
  );
}

/** The notch the hidden-sidebar pane leaves for the traffic lights and the
 *  toggle, in the pane's own coordinates (its top-left corner is 0,0). It
 *  reaches past the toggle and is exactly as tall as the pane's toolbar
 *  (`h-11`), so its ledge and the toolbar's divider are one line. */
export const NOTCH = { width: 116, height: 44, radius: 12 } as const;

/** Outline of a `w`×`h` rounded rectangle with an inverted-L notch cut from
 *  its top-left: the pane's top edge starts after the notch, its left edge
 *  below it.
 *
 *  The notch's corners are eased rather than circular: each curve starts
 *  well before the corner and leans into it gradually (the "smooth corner"
 *  of Apple's own shapes), so the step down around the lights reads as one
 *  soft S instead of two arcs joined by a line. The pane's outer corners
 *  stay plain arcs, matching every other card. */
export function notchedPath(w: number, h: number): string {
  const { width: nw, height: nh, radius: r } = NOTCH;
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
export function NotchOutline({ w, h }: { w: number; h: number }) {
  return (
    <svg
      aria-hidden
      className="pointer-events-none absolute inset-0 z-30 text-border"
      width={w}
      height={h}
      viewBox={`0 0 ${w} ${h}`}
    >
      <path d={notchedPath(w, h)} fill="none" stroke="currentColor" strokeWidth={2} />
    </svg>
  );
}
