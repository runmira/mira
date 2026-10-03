import { Activity, Bug, FlaskConical, Globe2, MessageCircleQuestion, Pencil, Plus, Smartphone, SquareTerminal } from 'lucide-react';
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
export type ToolPaneKind =
  | 'new'
  | 'aside'
  | 'browser'
  | 'devices'
  | 'processes'
  | 'tests'
  | 'activity'
  | 'whiteboard'
  | 'devtools';

/** What the launcher, the "+" menu and the toolbar menu offer, in order. */
export const OPENABLE_PANES: ToolPaneKind[] = [
  'aside',
  'browser',
  'devices',
  'processes',
  'tests',
  'activity',
  'whiteboard',
  'devtools',
];

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
  return kind === 'new' || (OPENABLE_PANES as string[]).includes(kind)
    ? (kind as ToolPaneKind)
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
  aside: {
    title: 'Ask aside',
    blurb: 'Ask a question without interrupting',
    icon: <MessageCircleQuestion className="size-3.5" />,
  },
  devices: {
    title: 'Devices',
    blurb: 'Your app on phone, tablet and desktop',
    icon: <Smartphone className="size-3.5" />,
  },
  processes: {
    title: 'Processes',
    blurb: 'Dev servers, ports and logs',
    icon: <SquareTerminal className="size-3.5" />,
  },
  tests: {
    title: 'Tests',
    blurb: 'Run tests, send failures to the agent',
    icon: <FlaskConical className="size-3.5" />,
  },
  activity: {
    title: 'Activity',
    blurb: 'What changed each turn, with restore',
    icon: <Activity className="size-3.5" />,
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
