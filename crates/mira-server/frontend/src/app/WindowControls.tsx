import { PanelLeft } from 'lucide-react';
import { TRAFFIC_LIGHT_INSET } from '../lib/desktop';
import { shortcutLabelForCommand, type ResolvedKeybindingsConfig } from '../lib/keybindings';
import { cn } from '../lib/utils';

/** Height of the strip the traffic lights sit in. The sidebar's own header
 *  row is the same height, and with the sidebar hidden the main pane
 *  starts below it. */
export const WINDOW_STRIP = TRAFFIC_LIGHT_INSET.top + 8;

/**
 * The desktop app's window controls: the sidebar toggle, pinned beside the
 * macOS traffic lights the way a native Mac app does it.
 *
 * It stays put whatever the main pane shows — chat, Settings, plugins — and
 * whether the sidebar is open or not, so there is always one place to
 * reach for. With the sidebar hidden the strip spans the window and drags
 * it; the main pane drops below it (see `WorkspaceMain`) so the lights and
 * the toggle never sit on top of content.
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
      className={cn('fixed left-0 top-0 z-40', sidebarOpen ? 'pointer-events-none' : 'right-0')}
      style={{ height: WINDOW_STRIP, paddingLeft: TRAFFIC_LIGHT_INSET.left }}
    >
      {/* Centred on the traffic lights' line, the title bar's own height. */}
      <div className="flex items-center" style={{ height: TRAFFIC_LIGHT_INSET.top }}>
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
