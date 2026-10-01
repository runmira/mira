import { Bug, Globe2, Pencil, Plus } from 'lucide-react';
import type { ReactNode } from 'react';

/**
 * The panes that live alongside subagent and file tabs.
 *
 * `new` is the launcher: it's a real tab, not an empty-panel state, so
 * there is always something to click in the strip and the panel can never
 * be a dead end. Each kind is a singleton — asking for a second browser
 * just focuses the one you already have, which is what you want ninety-nine
 * times out of a hundred and much cheaper than tab bookkeeping.
 */
export type ToolPaneKind = 'new' | 'browser' | 'whiteboard' | 'devtools';

export type ToolPaneTab = {
  /** `tool:<kind>`. The prefix keeps these from ever colliding with a
   *  file path (file tab ids are paths) or a subagent call id. */
  id: string;
  kind: ToolPaneKind;
  /** Tab-strip label; the browser pane updates it with the live hostname. */
  title: string;
};

export const TOOL_PANE_PREFIX = 'tool:';

export function toolPaneId(kind: ToolPaneKind): string {
  return `${TOOL_PANE_PREFIX}${kind}`;
}

export function isToolPaneId(id: string): boolean {
  return id.startsWith(TOOL_PANE_PREFIX);
}

export function toolPaneKindOf(id: string): ToolPaneKind | null {
  if (!isToolPaneId(id)) return null;
  const kind = id.slice(TOOL_PANE_PREFIX.length);
  return kind === 'new' ||
    kind === 'browser' ||
    kind === 'whiteboard' ||
    kind === 'devtools'
    ? kind
    : null;
}

export const TOOL_PANE_DEFS: Record<
  ToolPaneKind,
  { title: string; blurb: string; icon: ReactNode }
> = {
  new: {
    title: 'New',
    blurb: 'Choose what to open here',
    icon: <Plus className="size-3.5" />,
  },
  browser: {
    title: 'Browser',
    blurb: 'Drive a local Chrome',
    icon: <Globe2 className="size-3.5" />,
  },
  whiteboard: {
    title: 'Whiteboard',
    blurb: 'Sketch and send to agent',
    icon: <Pencil className="size-3.5" />,
  },
  devtools: {
    title: 'Developer tools',
    blurb: 'Tokens and request log',
    icon: <Bug className="size-3.5" />,
  },
};
