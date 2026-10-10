import type { BackgroundMode, SessionSummary } from '../../types';
import type { WsStatus } from '../../ws';
import { type SettingsSectionId } from '../settings/sections';
/** Primary view rendered in the main pane. Sidebar nav items switch the
 *  active view; the App owns the state and hides the chat composer /
 *  transcript when a non-chat view is selected. `'settings'` is a
 *  first-class view (not a dialog) — when active, the sidebar swaps
 *  its default nav for the settings-tab list and shows a "Back to
 *  app" pill up top. */
export type MainView = 'chat' | 'plugins' | 'pull-request' | 'scheduled' | 'settings';

export type Props = {
  status: WsStatus;
  cwd: string;
  activeSessionId: string;
  /** True when the active session is currently streaming — drives the
   *  pulsing indicator on that row. */
  activeBusy: boolean;
  runningSessions: ReadonlySet<string> | null;
  completedSessions: ReadonlyMap<string, number>;
  refreshKey: number;
  /** Highlights the matching nav item in the sidebar. */
  activeView: MainView;
  /** Switch the main pane to a different primary view. */
  onNavigate: (view: MainView) => void;
  onNewChat: () => void;
  onOpenSettings: () => void;
  onOpenPicker: () => void;
  onSessionLoaded: () => void;
  /** WS-native session switch. When present, the sidebar sends
   *  `Attach { session_id }` instead of hitting `POST /load` — no HTTP
   *  round-trip, no reload flash on the transcript, and the previously
   *  attached slot keeps running in the background. */
  onAttachSession?: (id: string) => void;
  /** Change a session's background mode. Provided by the app so the
   *  RowMenu can call `PUT /api/sessions/:id/background`. */
  onSetBackgroundMode?: (id: string, mode: BackgroundMode) => Promise<void>;
  /** The open chat's PR, when the app already knows it. Shown on the
   *  hover card of that chat only — a GitHub lookup per hovered row
   *  would put a network call in front of a glance. */
  activePr?: { number: number; title?: string } | null;
  /** Settings-mode state. Ignored unless `activeView === 'settings'`,
   *  in which case the sidebar renders the section tabs + a "Back to
   *  app" pill instead of the default nav. */
  settingsSection?: SettingsSectionId;
  onSettingsSectionChange?: (id: SettingsSectionId) => void;
  onExitSettings?: () => void;
};

/* ---------- session row ---------- */

/** Codex-style session row. Title on top, muted subline of
 *  `short-id · time · optional branch · optional provider-dot`, and a
 *  right-side status circle (running / merged / idle) — the row background
 *  itself stays quiet even when active so the sidebar doesn't shout. */
/** A chat and the chats forked from it ("Fork from here") or launched
 *  from it (`thread_launch`), newest first at each level. A fork whose original isn't in the list (archived, or
 *  another project) stands on its own. */
export type ForkNode = { session: SessionSummary; forks: ForkNode[] };

/* ---------- grouping ---------- */

export type Group = {
  cwd: string;
  label: string;
  isCurrent: boolean;
  sessions: SessionSummary[];
};
