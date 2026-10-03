import { useRef, useState } from 'react';
import { ChevronDown, PanelRight } from 'lucide-react';
import { cn } from '@/lib/utils';
import { Menu, MenuItem, MenuLabel } from './ui/Menu';
import {
  TOOL_PANE_DEFS,
  type ToolPaneKind,
  type ToolPaneTab,
} from './panes/toolPanes';

const KINDS: ToolPaneKind[] = ['browser', 'whiteboard', 'devtools'];

/**
 * Split button in the chat toolbar: the main half focuses (or opens) the
 * right panel, the chevron half opens a menu of which pane to put there.
 * One control, so the panel is discoverable from where you already are
 * rather than hidden behind a per-pane entry point.
 */
export function RightPanelButton({
  open,
  /** Which kind is showing, so the menu can mark it. Null when closed. */
  activeKind,
  onOpen,
  onOpenPane,
}: {
  open: boolean;
  activeKind: ToolPaneKind | null;
  /** Main-action handler. Opens the panel on the New tab, or collapses it
   *  when the New tab is already what's showing. */
  onOpen: () => void;
  onOpenPane: (kind: ToolPaneKind) => void;
}) {
  const [menu, setMenu] = useState(false);
  const wrap = useRef<HTMLDivElement | null>(null);

  // Matches the "Open in…" control beside it: same `bg-secondary/40` fill
  // and the same hairline stroke, with the active state lifting the fill
  // rather than dropping the border.
  const shell = (active: boolean) =>
    cn(
      'flex h-8 items-center text-muted-foreground transition-colors',
      active
        ? 'bg-secondary/70 text-foreground'
        : 'bg-secondary/40 hover:bg-secondary hover:text-foreground',
    );

  return (
    // One bordered shell split into two hit targets. The hairline on the
    // left half's right edge is the divider; putting `overflow-hidden` and
    // a single border on the wrapper keeps the divider from double up with
    // an outer stroke on the seam.
    <div
      ref={wrap}
      className="relative flex items-center overflow-hidden rounded-lg border border-border/60"
    >
      <button
        type="button"
        onClick={onOpen}
        title="Open the right panel"
        className={cn(
          shell(open),
          'gap-1.5 border-r border-border/60 px-1.5 text-[12.5px]',
        )}
      >
        <PanelRight className="size-3.5" />
        <span className="hidden lg:inline">
          {open && activeKind ? TOOL_PANE_DEFS[activeKind].title : 'Panel'}
        </span>
      </button>
      <button
        type="button"
        onClick={() => setMenu((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={menu}
        title="Choose what to open"
        className={cn(shell(open), 'pl-1 pr-1')}
      >
        <ChevronDown className="size-3" />
      </button>

      <Menu
        open={menu}
        onOpenChange={setMenu}
        label="Open a panel"
        minWidth={224}
      >
        <MenuLabel>Open</MenuLabel>
        {KINDS.map((kind) => (
          <MenuItem
            key={kind}
            label={TOOL_PANE_DEFS[kind].title}
            hint={TOOL_PANE_DEFS[kind].blurb}
            icon={TOOL_PANE_DEFS[kind].icon}
            active={open && activeKind === kind}
            onSelect={() => onOpenPane(kind)}
          />
        ))}
      </Menu>
    </div>
  );
}

/**
 * The "+" at the end of the panel's own tab strip — the other way in.
 *
 * The toolbar button opens a pane; this one covers the full set of things a
 * right-hand tab can be, including opening a file, so you don't have to
 * reach for the toolbar once you're already working in the panel.
 */
export function PanelNewTabButton({
  toolTabs,
  activeId,
  onOpenPane,
  onOpenFile,
}: {
  toolTabs: ToolPaneTab[];
  activeId: string | null;
  onOpenPane: (kind: ToolPaneKind) => void;
  onOpenFile: () => void;
}) {
  const [menu, setMenu] = useState(false);

  return (
    <div className="relative shrink-0">
      <button
        type="button"
        onClick={() => setMenu((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={menu}
        title="New tab"
        className="flex size-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground"
      >
        <PlusIcon />
      </button>

      <Menu open={menu} onOpenChange={setMenu} label="New tab" minWidth={224}>
        <MenuLabel>Open</MenuLabel>
        <MenuItem
          label="File…"
          hint="Browse the project and open a file"
          icon={<FileIcon />}
          onSelect={onOpenFile}
        />
        <MenuLabel>Open</MenuLabel>
        {KINDS.map((kind) => (
          <MenuItem
            key={kind}
            label={TOOL_PANE_DEFS[kind].title}
            hint={TOOL_PANE_DEFS[kind].blurb}
            icon={TOOL_PANE_DEFS[kind].icon}
            active={activeId === `tool:${kind}`}
            onSelect={() => onOpenPane(kind)}
          />
        ))}
        {toolTabs.length > 0 && (
          <>
            <MenuLabel>Open tabs</MenuLabel>
            {toolTabs.map((t) => (
              <MenuItem
                key={t.id}
                label={t.title}
                icon={TOOL_PANE_DEFS[t.kind].icon}
                active={activeId === t.id}
                onSelect={() => onOpenPane(t.kind)}
              />
            ))}
          </>
        )}
      </Menu>
    </div>
  );
}

function PlusIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-3.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      aria-hidden
    >
      <path d="M8 3.5v9M3.5 8h9" />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-3.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M9 1.75H4.5A1.25 1.25 0 0 0 3.25 3v10A1.25 1.25 0 0 0 4.5 14.25h7A1.25 1.25 0 0 0 12.75 13V5.5L9 1.75Z" />
      <path d="M9 1.75V5.5h3.75" />
    </svg>
  );
}
