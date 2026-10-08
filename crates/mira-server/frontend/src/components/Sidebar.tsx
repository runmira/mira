import { BranchElbow } from './BranchElbow';
import { useEffect, useMemo, useRef, useState } from 'react';
import { LazyBoundary } from './LazyBoundary';
import {
  Archive,
  ArchiveRestore,
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  Circle,
  Ellipsis,
  Folder,
  GitBranch,
  Loader,
  Pencil,
  PenLine,
  Pin,
  PinOff,
  Puzzle,
  Search,
  Sparkles,
  Timer,
  Trash2,
  X,
  GitFork,
} from 'lucide-react';
import {
  deleteSession,
  listSessions,
  loadSession,
  regenerateSessionTitle,
  renameSession,
  setSessionFlags,
} from '../api';
import type { BackgroundMode, SessionSummary } from '../types';
import type { WsStatus } from '../ws';
import { parseSentAttachments } from './Composer';
import { SETTINGS_SECTIONS, type SettingsSectionId } from './settings/sections';
import { PluginsPanel, PullRequestPanel, SessionPeek } from '../lazyViews';

import { UserCard } from './UserCard';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import type { PeekTarget } from './SessionPeek';
import { useLatch } from '../lib/lazy';
import { hasHiddenTitleBar, TRAFFIC_LIGHT_INSET } from '@/lib/desktop';
import { Collapse } from './ui/Collapse';
import { useFileIcons } from '@/lib/fileIcons';
import { AgentIcon, ModelIcon } from './AgentIcon';

/** Primary view rendered in the main pane. Sidebar nav items switch the
 *  active view; the App owns the state and hides the chat composer /
 *  transcript when a non-chat view is selected. `'settings'` is a
 *  first-class view (not a dialog) — when active, the sidebar swaps
 *  its default nav for the settings-tab list and shows a "Back to
 *  app" pill up top. */
export type MainView = 'chat' | 'plugins' | 'pull-request' | 'scheduled' | 'settings';

/** Human-readable session label. Strips the `## Attached files … ` block
 *  from `first_user_message` so an image-only first turn doesn't read as
 *  a wall of markdown; falls back to a filename summary when the user
 *  sent nothing but attachments. */
function sessionLabel(s: SessionSummary): string {
  if (s.title) return s.title;
  const raw = s.first_user_message ?? '';
  if (!raw) return 'Untitled';
  const { attachments, text } = parseSentAttachments(raw);
  const clean = text.trim();
  if (clean) return clean;
  if (attachments.length > 0) {
    const first = attachments[0].filename;
    const more = attachments.length - 1;
    return more > 0 ? `${first} + ${more} more` : first;
  }
  return raw;
}

type Props = {
  status: WsStatus;
  cwd: string;
  activeSessionId: string;
  /** True when the active session is currently streaming — drives the
   *  pulsing indicator on that row. */
  activeBusy: boolean;
  runningSessions: ReadonlySet<string> | null;
  completedSessions:ReadonlyMap<string,number>;
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

const COLLAPSED_KEY = 'mira.sidebar.collapsed-projects';
const PER_GROUP_LIMIT = 5;

export function Sidebar({
  status, cwd, activeSessionId, activeBusy, refreshKey, activeView, onNavigate,
  onNewChat, onOpenSettings, onSessionLoaded, onAttachSession,
  onSetBackgroundMode, activePr, runningSessions, completedSessions,
  settingsSection = 'provider',
  onSettingsSectionChange,
  onExitSettings,
}: Props) {
  const hiddenTitleBar = hasHiddenTitleBar();
  const [storedSessions, setSessions] = useState<SessionSummary[]>([]);
  const sessions = useMemo(() => runningSessions == null ? storedSessions
    : storedSessions.map(session => ({...session, running:runningSessions.has(session.id)})), [storedSessions,runningSessions]);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadCollapsed());
  // Chats whose forks are folded away (the fork pill on the row).
  const [forksHidden, setForksHidden] = useState<Set<string>>(() => new Set());
  const toggleForks = (id: string) =>
    setForksHidden((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const [showMore, setShowMore] = useState<Set<string>>(() => new Set());
  const [renaming, setRenaming] = useState<SessionSummary | null>(null);
  // Pin/archive/bulk actions bump this so the list refetches without
  // waiting for the App to change `refreshKey`.
  const [localVersion, setLocalVersion] = useState(0);
  // Archived view — fetched lazily on first expand, then kept fresh.
  const [showArchived, setShowArchived] = useState(false);
  const [archived, setArchived] = useState<SessionSummary[] | null>(null);
  // Bulk-select mode: clicking rows toggles checkboxes instead of
  // opening sessions; a floating bar offers archive/delete/cancel.
  const [selecting, setSelecting] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);
  const [confirmBulkDelete, setConfirmBulkDelete] = useState(false);
  // Hover peek — one card for the whole list, anchored to whichever row
  // the pointer is resting on. State lives here rather than in the row
  // so sliding the pointer down the list moves one card rather than
  // mounting twenty.
  const [peek, setPeek] = useState<PeekTarget | null>(null);
  // Hover card chunk loads on the first hover, then stays mounted.
  const peekMounted = useLatch(peek !== null);
  // Sidebar search: filters every folder at once, and opens them while it
  // has text so a match is never hidden behind a collapsed folder.
  const [query, setQuery] = useState('');
  const searching = query.trim().length > 0;

  useEffect(() => {
    // Fetch all non-archived sessions across every folder — grouped
    // in-memory below. The server excludes archived ones here; they come
    // through the separate archived fetch when the user opens that view.
    let cancelled = false;
    listSessions({ all: true })
      .then((s) => { if (!cancelled) {setSessions(s); setError(null);} })
      .catch((e) => {if (!cancelled) setError(String(e.message ?? e));});
    return () => {cancelled = true;};
  }, [refreshKey, localVersion]);

  useEffect(() => {
    if (!showArchived) return;
    listSessions({ all: true, archived: true })
      .then((s) => { setArchived(s); setError(null); })
      .catch((e) => setError(String(e.message ?? e)));
  }, [showArchived, refreshKey, localVersion]);

  const refresh = () => setLocalVersion((n) => n + 1);

  // Background chats only report through this list, so while any of them
  // is mid-turn, look again every few seconds — that's how a spinner turns
  // into "finished" without the user having to open the chat.
  // Also how a chat that starts waiting on you gets its badge: that change
  // isn't pushed, and a chat waiting on you is mid-turn, so it's covered.
  const anyRunning = sessions.some((s) => (s.running === true || s.needs_attention === true) && s.id !== activeSessionId);
  useEffect(() => {
    if (!anyRunning) return;
    const id = window.setInterval(refresh, 4000);
    return () => window.clearInterval(id);
  }, [anyRunning]);

  // Chats that finished while you were looking at something else get a
  // dot until you open them. Remembered across reloads.
  const [unread, setUnread] = useState<Set<string>>(() => loadUnread());
  const wasRunning = useRef<Set<string>>(new Set());
  const seenCompletions=useRef(new Map<string,number>());
  useEffect(() => {
    const next = new Set(unread);
    let changed = false;
    for(const [id,revision] of completedSessions){
      if(seenCompletions.current.get(id)===revision)continue;
      seenCompletions.current.set(id,revision);
      if(id!==activeSessionId&&!next.has(id)){next.add(id);changed=true;}
    }
    while(seenCompletions.current.size>500)seenCompletions.current.delete(seenCompletions.current.keys().next().value!);
    for (const s of sessions) {
      if (s.running) continue;
      if (wasRunning.current.has(s.id) && s.id !== activeSessionId && !next.has(s.id)) {
        next.add(s.id);
        changed = true;
      }
    }
    wasRunning.current = new Set(sessions.filter((s) => s.running).map((s) => s.id));
    if (activeSessionId && next.delete(activeSessionId)) changed = true;
    if (changed) {
      setUnread(next);
      saveUnread(next);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions, activeSessionId, completedSessions]);

  /** Flip one session's pin/archive flag and refresh. Errors surface in
   *  the same slot as fetch errors. */
  async function flagSession(id: string, flags: { pinned?: boolean; archived?: boolean }) {
    try {
      await setSessionFlags(id, flags);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
    refresh();
  }

  function toggleSelectMode() {
    setSelecting((v) => !v);
    setSelected(new Set());
  }

  function toggleSelected(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }

  async function bulkArchive() {
    setBulkBusy(true);
    try {
      await Promise.all([...selected].map((id) => setSessionFlags(id, { archived: true })));
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
    setBulkBusy(false);
    setSelected(new Set());
    setSelecting(false);
    refresh();
  }

  /** Confirm happens in the dialog (`confirmBulkDelete`) — deleting is
   *  irreversible, so bulk delete never fires straight from the bar. */
  async function bulkDelete() {
    setBulkBusy(true);
    try {
      await Promise.all([...selected].map((id) => deleteSession(id)));
      setError(null);
      // Optimistic drop, mirroring removeSession.
      setSessions((prev) => prev.filter((s) => !selected.has(s.id)));
    } catch (e) {
      setError(String((e as Error).message));
    }
    setBulkBusy(false);
    setSelected(new Set());
    setSelecting(false);
    refresh();
  }

  const groups = useMemo(
    () => groupByCwd(searching ? sessions.filter((s) => matchesQuery(s, query)) : sessions, cwd),
    [sessions, cwd, query, searching],
  );

  async function pickSession(id: string) {
    // Prefer WS attach — no HTTP round-trip, no reload flash, and the
    // previously attached slot keeps making progress in the background.
    // Fall back to POST /load if the WS helper isn't wired (shouldn't
    // happen in the shipped app, but keeps the component reusable).
    if (onAttachSession) {
      try {
        onAttachSession(id);
        onSessionLoaded();
        return;
      } catch (e) {
        setError(String((e as Error).message));
        return;
      }
    }
    try { await loadSession(id); onSessionLoaded(); }
    catch (e) { setError(String((e as Error).message)); }
  }

  async function removeSession(id: string) {
    try {
      await deleteSession(id);
      // Optimistic drop from local state — the parent's next refresh via
      // the Ready broadcast (fired if this was the active session) will
      // re-sync the rest.
      setSessions((prev) => prev.filter((s) => s.id !== id));
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  async function removeAllInProject(cwd: string) {
    const ids = sessions.filter((s) => s.cwd === cwd).map((s) => s.id);
    try {
      await Promise.all(ids.map((id) => deleteSession(id)));
      setSessions((prev) => prev.filter((s) => s.cwd !== cwd));
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  /** Optimistically stamp the title into local state so the sidebar row
   *  updates before the server broadcast round-trips. The server also
   *  fires `SessionTitleUpdated` on the WS channel — App handles that and
   *  triggers a refresh, so this is belt-and-suspenders. */
  function applyLocalTitle(id: string, title: string) {
    setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, title } : s)));
  }

  async function applyManualRename(id: string, title: string) {
    try {
      const res = await renameSession(id, title);
      applyLocalTitle(id, res.title);
      setRenaming(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  /** AI rename returns the generated title so the dialog can display it and
   *  let the user accept / re-generate / edit. Returns the string to the
   *  caller; errors bubble so the dialog shows them inline. */
  async function applyAiRename(id: string): Promise<string> {
    const res = await regenerateSessionTitle(id);
    applyLocalTitle(id, res.title);
    return res.title;
  }

  function toggleCollapsed(key: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      persistCollapsed(next);
      return next;
    });
  }

  function toggleShowMore(key: string) {
    setShowMore((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  }

  return (
    <>
    <aside
      className={cn(
        'flex h-full min-w-0 flex-col',
        // On the desktop build the window is transparent and macOS paints
        // vibrancy behind it, so the sidebar contributes *no* fill of its
        // own — the OS material is the sidebar background. An earlier pass
        // laid a dark tint over the vibrancy to keep text readable, which
        // defeated the point: the material got multiplied down to near-black
        // and read as another flat dark panel. Contrast comes from macOS
        // darkening the material in dark appearance, not from us.
        hiddenTitleBar ? 'bg-transparent' : 'bg-panel',
      )}
    >
      {/* When the native title bar is hidden, this row *is* the title bar:
          it sits in the strip beside the traffic lights, carries the
          wordmark, and is the window's drag region. Otherwise it's a normal
          padded header. */}
      <div
        data-tauri-drag-region
        className={cn(
          'flex items-center justify-between px-3',
          hiddenTitleBar
            ? 'shrink-0'
            : 'pb-2 pt-3.5',
        )}
        style={
          hiddenTitleBar
            ? {
                // Beside the traffic lights, not below them.
                height: TRAFFIC_LIGHT_INSET.top + 8,
                paddingLeft: TRAFFIC_LIGHT_INSET.left,
              }
            : undefined
        }
      >
        {/* No wordmark: the sidebar's own nav labels identify the app, and
            a title here just crowds the traffic lights. The row stays as
            the drag region and as the inset that keeps the lights clear. */}
      </div>

      {activeView !== 'settings' && (
        <div className="px-3 pb-2">
          {/* A raised control sitting *on* the vibrancy, so it needs enough
              body to read as a surface: a translucent fill, a backdrop blur
              to keep the wallpaper from muddying the label, and a hairline.
              The selected half is near-opaque so the active tab is
              unambiguous without relying on colour alone. */}
          <div className="grid grid-cols-2 gap-0.5 rounded-full border border-border/80 bg-shade/25 p-0.5 backdrop-blur-md">
            <span className="rounded-full bg-foreground/90 px-3.5 py-1 text-center text-[12.5px] font-medium text-background">
              Chat
            </span>
            <span
              title="Not implemented yet"
              className="cursor-not-allowed rounded-full px-3.5 py-1 text-center text-[12.5px] text-foreground/45"
            >
              Work
            </span>
          </div>
        </div>
      )}

      {activeView === 'settings' ? (
        // Settings mode — the sidebar becomes the section picker. The
        // "Back to app" pill up top pops the user back to whatever view
        // they were on before entering settings; the rest of the app
        // (projects, folder chip, status footer) is hidden to keep the
        // context single-purpose while they're configuring things.
        <div className="min-h-0 flex-1">
        <div className="h-full overflow-y-auto px-3 pb-2 pt-1">
          <button
            type="button"
            onClick={() => onExitSettings?.()}
            className="mb-3 inline-flex items-center gap-1.5 rounded-full border border-border bg-secondary/60 px-3 py-1.5 text-[12.5px] text-foreground/90 transition-colors hover:bg-secondary hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            Back to app
          </button>
          <div className="mb-1.5 px-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
            Settings
          </div>
          <nav className="flex flex-col gap-0.5">
            {SETTINGS_SECTIONS.map((s) => {
              const active = settingsSection === s.id;
              const Icon = s.icon;
              return (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => onSettingsSectionChange?.(s.id)}
                  className={cn(
                    'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13.5px] transition-colors',
                    active
                      ? 'bg-fg/[0.1] text-foreground'
                      : 'text-foreground/80 hover:bg-fg/[0.05] hover:text-foreground',
                  )}
                >
                  <Icon className={cn('size-4 shrink-0', active ? 'text-mira-blue' : 'text-muted-foreground')} />
                  <span>{s.label}</span>
                </button>
              );
            })}
          </nav>
        </div>
        </div>
      ) : (
      <div className="min-h-0 flex-1">
      <div className="h-full overflow-y-auto px-1.5 pb-2">
        <nav className="flex flex-col gap-0.5 px-0.5">
          <NavItem icon={<PenLine className="size-3.5" />} onClick={onNewChat}>
            New thread
          </NavItem>
          <NavItem
            icon={<GitBranch className="size-3.5" />}
            active={activeView === 'pull-request'}
            onClick={() => onNavigate('pull-request')}
            onIntent={PullRequestPanel.preload}
          >
            Pull request
          </NavItem>
          <NavItem
            icon={<Puzzle className="size-3.5" />}
            active={activeView === 'plugins'}
            onClick={() => onNavigate('plugins')}
            onIntent={PluginsPanel.preload}
          >
            Plugins
          </NavItem>
          <NavItem
            icon={<Timer className="size-3.5" />}
            disabled
            active={activeView === 'scheduled'}
          >
            Scheduled
          </NavItem>
        </nav>

        <div className="mt-4 flex flex-col gap-1 px-0.5">
          <div className="flex items-center justify-between px-2.5 py-1">
            <div className="text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
              Projects
            </div>
            {!selecting && groups.length > 0 && (
              <button
                type="button"
                onClick={toggleSelectMode}
                className="text-[11.5px] font-medium text-muted-foreground/80 transition-colors hover:text-foreground"
              >
                Select
              </button>
            )}
          </div>

          {!selecting && (sessions.length > 0 || searching) && (
            <label className="mx-1 mb-1 flex items-center gap-2 rounded-lg border border-border/60 bg-shade/10 px-2 py-1 text-[12.5px] focus-within:border-border">
              <Search className="size-3.5 shrink-0 text-muted-foreground/70" aria-hidden />
              {/* Plain text, not type=search: that adds the browser's own
                  clear button beside ours. */}
              <input
                type="text"
                enterKeyHint="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Escape') setQuery(''); }}
                placeholder="Search chats"
                aria-label="Search chats"
                className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground/60"
              />
              {searching && (
                <button type="button" onClick={() => setQuery('')} aria-label="Clear search" className="text-muted-foreground/70 hover:text-foreground">
                  <X className="size-3.5" />
                </button>
              )}
            </label>
          )}

          {selecting && (
            <div className="sticky top-0 z-10 flex items-center gap-1 rounded-lg border border-border/70 bg-mira-elev1/95 px-2.5 py-1.5 shadow-sm backdrop-blur">
              <span className="text-[12px] font-medium text-foreground">
                {selected.size} selected
              </span>
              <div className="ml-auto flex items-center gap-0.5">
                <button
                  type="button"
                  onClick={() => void bulkArchive()}
                  disabled={bulkBusy || selected.size === 0}
                  className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-foreground/85 transition-colors hover:bg-fg/[0.05] hover:text-foreground disabled:opacity-40"
                >
                  <Archive className="size-3.5" /> Archive
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmBulkDelete(true)}
                  disabled={bulkBusy || selected.size === 0}
                  className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-red-400/90 transition-colors hover:bg-red-500/10 hover:text-red-300 disabled:opacity-40"
                >
                  <Trash2 className="size-3.5" /> Delete
                </button>
                <button
                  type="button"
                  onClick={toggleSelectMode}
                  disabled={bulkBusy}
                  title="Cancel selection"
                  className="inline-flex items-center rounded-md p-1 text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground disabled:opacity-40"
                >
                  <X className="size-3.5" />
                </button>
              </div>
            </div>
          )}

          {error && <Empty>error: {error}</Empty>}
          {!error && groups.length === 0 && <Empty>{searching ? 'No chats match' : 'No saved chats yet'}</Empty>}

          {groups.map((g) => {
            const isCollapsed = !searching && collapsed.has(g.cwd);
            const expandedAll = searching || showMore.has(g.cwd);
            const roots = forkForest(g.sessions);
            const visible = expandedAll ? roots : roots.slice(0, PER_GROUP_LIMIT);
            const waiting = g.sessions.filter((x) => x.needs_attention && x.id !== activeSessionId).length;
            // Date sections only when they say something: rows spanning
            // more than one period.
            const sections = new Set(visible.map((r) => dateSection(r.session))).size > 1;
            const overflow = roots.length - visible.length;
            const labelOf = (id: string | null | undefined) => {
              const p = g.sessions.find((x) => x.id === id);
              return p ? sessionLabel(p) : undefined;
            };

            return (
              <div key={g.cwd} className="flex flex-col">
                <div
                  className="group flex items-center gap-1.5 rounded-md px-2 py-1 text-[14.5px] font-semibold text-foreground/90 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
                  title={g.cwd}
                >
                  <button
                    type="button"
                    onClick={() => toggleCollapsed(g.cwd)}
                    className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                  >
                    {isCollapsed ? (
                      <ChevronRight className="size-3 shrink-0 text-muted-foreground/60" />
                    ) : (
                      <ChevronDown className="size-3 shrink-0 text-muted-foreground/60" />
                    )}
                    <SidebarFolderIcon
                      name={g.label}
                      open={!isCollapsed}
                      className="size-3.5"
                      current={g.isCurrent}
                    />
                    <span className={cn('truncate', g.isCurrent && 'text-foreground')}>{g.label}</span>
                    {isCollapsed && waiting > 0 && (
                      <span
                        className="ml-0.5 inline-flex items-center gap-0.5 rounded-full bg-amber-500/15 px-1.5 text-[10.5px] font-medium tabular-nums text-amber-600 dark:text-amber-400"
                        title={`${waiting} chat${waiting === 1 ? '' : 's'} waiting on you`}
                      >
                        <CircleAlert className="size-2.5" />
                        {waiting}
                      </span>
                    )}
                  </button>
                  <RowMenu
                    items={[
                      {
                        label: 'Delete all sessions',
                        danger: true,
                        confirm: `Delete all ${g.sessions.length} session${g.sessions.length === 1 ? '' : 's'} in ${g.label}?`,
                        icon: <Trash2 className="size-3.5" />,
                        onSelect: () => removeAllInProject(g.cwd),
                      },
                    ]}
                  />
                </div>

                <Collapse open={!isCollapsed}>
                  <div className="flex flex-col gap-0.5 pl-1">
                    {visible.map((root, rootIndex) => {
                      const section = dateSection(root.session);
                      const header =
                        sections && (rootIndex === 0 || dateSection(visible[rootIndex - 1].session) !== section) ? (
                          <div key={`h-${section}`} className="px-2 pb-0.5 pt-2 text-[10.5px] font-medium uppercase tracking-wider text-muted-foreground/55 first:pt-0.5">
                            {section}
                          </div>
                        ) : null;
                      const renderNode = (node: ForkNode, depth: number): React.ReactNode => {
                        const s = node.session;
                        const open = !forksHidden.has(s.id);
                        return (
                          <div key={s.id} className="flex flex-col gap-0.5">
                            <SessionRow
                        session={s}
                        depth={depth}
                        forkedFromLabel={labelOf(s.forked_from)}
                        forkCount={node.forks.length}
                        forksOpen={!forksHidden.has(s.id)}
                        onToggleForks={() => toggleForks(s.id)}
                        // One selection model across the sidebar: the
                        // session highlight only shows while the chat
                        // view is actually open — picking Plugins /
                        // PR / Scheduled leaves the row highlighted
                        // otherwise (and New thread swaps in a fresh
                        // id server-side, which drops it naturally).
                        active={s.id === activeSessionId && activeView === 'chat'}
                        activeBusy={runningSessions == null && activeBusy}
                        unread={unread.has(s.id)}
                        launched={depth > 0 && !s.forked_from && !!s.launched_by}
                        selecting={selecting}
                        checked={selected.has(s.id)}
                        onToggleSelect={() => toggleSelected(s.id)}
                        onPeek={(rect) => setPeek(rect ? { session: s, rect } : null)}
                        onPick={() => pickSession(s.id)}
                        onRename={() => setRenaming(s)}
                        onDelete={() => removeSession(s.id)}
                        onPin={(p) => void flagSession(s.id, { pinned: p })}
                        onArchive={() => void flagSession(s.id, { archived: true })}
                        onSetBackgroundMode={
                          onSetBackgroundMode
                            ? (mode) => onSetBackgroundMode(s.id, mode).catch((e) => setError(String(e.message ?? e)))
                            : undefined
                        }
                      />
                            <Collapse open={node.forks.length > 0 && open}>
                              {/* Branch lines: a rail down from the chat's icon,
                                  with a rounded elbow into each fork. */}
                              <div className="ml-[15px] flex flex-col gap-0.5 pl-3">
                                {node.forks.map((f, i) => (
                                  <div key={f.session.id} className="relative">
                                    {i < node.forks.length - 1 && (
                                      <span
                                        aria-hidden
                                        className="pointer-events-none absolute -bottom-0.5 -left-3 top-0 border-l border-fg/15"
                                      />
                                    )}
                                    <BranchElbow className="-left-3 top-0" />
                                    {renderNode(f, depth + 1)}
                                  </div>
                                ))}
                              </div>
                            </Collapse>
                          </div>
                        );
                      };
                      return (
                        <div key={root.session.id} className="flex flex-col gap-0.5">
                          {header}
                          {renderNode(root, 0)}
                        </div>
                      );
                    })}
                    {overflow > 0 && (
                      <button
                        onClick={() => toggleShowMore(g.cwd)}
                        className="px-2 py-1 text-left text-[12.5px] text-muted-foreground/80 transition-colors hover:text-foreground"
                      >
                        Show {overflow} more
                      </button>
                    )}
                    {expandedAll && g.sessions.length > PER_GROUP_LIMIT && (
                      <button
                        onClick={() => toggleShowMore(g.cwd)}
                        className="px-2 py-1 text-left text-[12.5px] text-muted-foreground/80 transition-colors hover:text-foreground"
                      >
                        Show less
                      </button>
                    )}
                  </div>
                </Collapse>
              </div>
            );
          })}

          {/* Archived view (issue #58) — sessions hidden from the main
              list live here. Clicking a row opens it; Restore puts it back
              in its folder; Delete is permanent. */}
          {!selecting && !searching && (
            <div className="mt-4 flex flex-col gap-0.5 px-0.5">
              <button
                type="button"
                onClick={() => setShowArchived((v) => !v)}
                className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
              >
                {showArchived ? (
                  <ChevronDown className="size-3 shrink-0 text-muted-foreground/60" />
                ) : (
                  <ChevronRight className="size-3 shrink-0 text-muted-foreground/60" />
                )}
                <Archive className="size-3 shrink-0" />
                Archived
                {archived !== null && archived.length > 0 && (
                  <span className="rounded-full bg-secondary/80 px-1.5 text-[10.5px] font-medium normal-case tracking-normal text-muted-foreground">
                    {archived.length}
                  </span>
                )}
              </button>
              {showArchived && (
                <div className="flex flex-col gap-0.5 pl-1">
                  {archived === null && <Empty>Loading…</Empty>}
                  {archived !== null && archived.length === 0 && <Empty>Nothing archived</Empty>}
                  {archived?.map((s) => (
                    <ArchivedRow
                      key={s.id}
                      session={s}
                      active={s.id === activeSessionId && activeView === 'chat'}
                      onOpen={() => void pickSession(s.id)}
                      onRestore={() => void flagSession(s.id, { archived: false })}
                      onDelete={() => removeSession(s.id)}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

      </div>
      </div>
      )}

      <UserCard status={status} onOpenSettings={onOpenSettings} />
      {/* One hover card for every row in the list. */}
      {peekMounted && (
        <LazyBoundary>
          <SessionPeek
            target={peek}
            pr={peek && peek.session.id === activeSessionId ? activePr ?? null : null}
            onDismiss={() => setPeek(null)}
          />
        </LazyBoundary>
      )}
    </aside>
    <RenameDialog
      session={renaming}
      onClose={() => setRenaming(null)}
      onManual={applyManualRename}
      onAi={applyAiRename}
    />
    {/* Bulk delete confirm (issue #58 acceptance: bulk actions confirm
        before deleting). Archive is reversible, so it fires directly. */}
    <Dialog open={confirmBulkDelete} onOpenChange={(o) => !o && setConfirmBulkDelete(false)}>
      <DialogContent className="max-w-sm p-0 gap-0">
        <div className="flex flex-col gap-4 p-5">
          <div>
            <div className="text-[15px] font-semibold text-foreground">
              Delete {selected.size} session{selected.size === 1 ? '' : 's'}?
            </div>
            <div className="mt-1 text-[12.5px] text-muted-foreground">
              This permanently removes the transcripts. This cannot be undone.
            </div>
          </div>
          <div className="flex items-center justify-end gap-2">
            <Button variant="outline" onClick={() => setConfirmBulkDelete(false)} disabled={bulkBusy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void bulkDelete()} disabled={bulkBusy}>
              {bulkBusy ? 'Deleting…' : 'Delete'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
    </>
  );
}

/* ---------- session row ---------- */

/** Codex-style session row. Title on top, muted subline of
 *  `short-id · time · optional branch · optional provider-dot`, and a
 *  right-side status circle (running / merged / idle) — the row background
 *  itself stays quiet even when active so the sidebar doesn't shout. */
/** A chat and the chats forked from it ("Fork from here") or launched
 *  from it (`thread_launch`), newest first at each level. A fork whose original isn't in the list (archived, or
 *  another project) stands on its own. */
type ForkNode = { session: SessionSummary; forks: ForkNode[] };

function forkForest(sessions: SessionSummary[]): ForkNode[] {
  const ids = new Set(sessions.map((s) => s.id));
  const nodes = new Map(sessions.map((s) => [s.id, { session: s, forks: [] as ForkNode[] }]));
  const roots: ForkNode[] = [];
  for (const s of sessions) {
    const node = nodes.get(s.id)!;
    // A fork nests under its original; a launched thread under the chat
    // that launched it.
    const up = s.forked_from ?? s.launched_by ?? null;
    const parent = up && up !== s.id && ids.has(up) ? nodes.get(up) : undefined;
    if (parent) parent.forks.push(node);
    else roots.push(node);
  }
  // A cycle would leave nodes unreachable; promote them so nothing vanishes.
  const seen = new Set<string>();
  const mark = (n: ForkNode) => {
    if (seen.has(n.session.id)) return;
    seen.add(n.session.id);
    n.forks.forEach(mark);
  };
  roots.forEach(mark);
  for (const n of nodes.values()) {
    if (!seen.has(n.session.id)) {
      roots.push(n);
      mark(n);
    }
  }
  return roots;
}

/** What a fork row says. Forks of one chat share its title, so the
 *  message each one branched at is what tells them apart — unless the
 *  fork has been renamed (its title no longer ends in "(fork)"). */
function forkLabel(s: SessionSummary): string {
  const label = sessionLabel(s);
  if (s.forked_at && /\(fork\)$/.test(label)) return s.forked_at;
  return label.replace(/\s*\(fork\)$/, '');
}

function SessionRow({
  session,
  depth = 0,
  forkedFromLabel,
  forkCount = 0,
  forksOpen = true,
  onToggleForks,
  active,
  activeBusy,
  unread = false,
  launched = false,
  onPeek,
  onPick,
  onRename,
  onDelete,
  onSetBackgroundMode,
  onPin,
  onArchive,
  selecting = false,
  checked = false,
  onToggleSelect,
}: {
  session: SessionSummary;
  /** Fork nesting level: 0 for a chat, 1 for a fork of it, … */
  depth?: number;
  /** The label of the chat this one was forked from, for the tooltip. */
  forkedFromLabel?: string;
  /** How many chats were forked from this one, and whether they show. */
  forkCount?: number;
  forksOpen?: boolean;
  onToggleForks?: () => void;
  active: boolean;
  activeBusy: boolean;
  /** Finished in the background since you last looked. */
  unread?: boolean;
  /** A thread another chat launched (rather than a fork of it). */
  launched?: boolean;
  onPick: () => void;
  /** Pointer entered the row with its viewport rect (so the hover card
   *  can sit beside it), or left it with `null`. Omitted in bulk-select
   *  mode, where hovering is about choosing, not looking. */
  onPeek?: (rect: DOMRect | null) => void;
  onRename: () => void;
  onDelete: () => void;
  /** Optional — when provided, the RowMenu shows a "Background mode ▸"
   *  submenu with Deny / AutoApprove / Park. Only meaningful for slots
   *  the server has materialized (persisted-but-not-loaded sessions
   *  have no runtime; the endpoint 404s until the session is loaded). */
  onSetBackgroundMode?: (mode: BackgroundMode) => void;
  /** Optional pin toggle — wires the row menu's Pin/Unpin item. */
  onPin?: (pinned: boolean) => void;
  /** Optional archive action — wires the row menu's Archive item. */
  onArchive?: () => void;
  /** Bulk-select mode: rows toggle a checkbox instead of opening. */
  selecting?: boolean;
  checked?: boolean;
  onToggleSelect?: () => void;
}) {
  // A session is "running" from the sidebar's POV either because it's the
  // active session with a live in-flight turn (activeBusy), OR because the
  // server reports its slot has a background turn still going. The second
  // case is what makes multi-session actually visible — you can leave a
  // tab, watch a different session, and this row keeps its spinner.
  const running = (active && activeBusy) || session.running === true;
  return (
    <div
      // Hover tooltip prefers the cleaned-up label (no `## Attached
      // files` markdown blob) so an attachment-only turn still hovers
      // sensibly. Falls back to session id when everything is empty.
      title={
        selecting
          ? undefined
          : session.forked_from
            ? `${sessionLabel(session) || session.id}\nForked from “${forkedFromLabel ?? 'another chat'}” at “${session.forked_at ?? ''}”`
            : sessionLabel(session) || session.id
      }
      role={selecting ? 'checkbox' : 'button'}
      tabIndex={0}
      aria-checked={selecting ? checked : undefined}
      onClick={selecting ? onToggleSelect : onPick}
      onMouseEnter={
        onPeek && !selecting ? (e) => onPeek(e.currentTarget.getBoundingClientRect()) : undefined
      }
      // Leaving the row hands the sidebar a null target; the card's own
      // short linger means sliding straight onto the next row doesn't
      // make it blink.
      onMouseLeave={onPeek && !selecting ? () => onPeek(null) : undefined}
      onKeyDown={(e) => {
        if (e.key !== 'Enter' && e.key !== ' ') return;
        e.preventDefault();
        if (selecting) onToggleSelect?.();
        else onPick();
      }}
      className={cn(
        'group grid w-full cursor-pointer items-center gap-1.5 rounded-lg px-2 py-2 transition-colors',
        selecting ? 'grid-cols-[auto_1fr_auto] cursor-pointer' : 'grid-cols-[1fr_auto]',
        selecting && checked
          ? 'bg-fg/[0.1] text-foreground'
          // Same active treatment as the nav items (New thread / PR /
          // Plugins / Scheduled): 10% white band, full foreground.
          : active
            ? 'bg-fg/[0.1] text-foreground'
            : 'text-foreground/90 hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      {selecting && (
        <span className="flex size-4 items-center justify-center self-center text-mira-blue">
          {checked ? <CircleCheck className="size-4" /> : <Circle className="size-3.5" />}
        </span>
      )}
      {/* One centre line for everything in the row. The icon is a 16px box
       *  — the height of the label's line — so it can't sit above or below
       *  the text the way a taller badge on a baseline row did. */}
      <span className="flex min-w-0 items-center gap-2 text-left">
        {!selecting &&
          (depth > 0 && !launched ? (
            <GitFork className="size-3.5 shrink-0 text-mira-blue/70" aria-label="Fork" />
          ) : (
            <EngineBadge session={session} />
          ))}
        <span
          className={cn(
            'min-w-0 flex-1 truncate leading-4',
            depth > 0 ? 'text-[13px]' : 'text-[13.5px]',
            active && !selecting
              ? 'font-semibold text-foreground'
              : depth > 0
                ? 'text-foreground/80'
                : 'font-medium text-foreground/90',
          )}
        >
          {depth > 0 && !launched ? forkLabel(session) : sessionLabel(session)}
        </span>
        {/* Just the mark: the title gets the width, the branch is on hover
            and in the peek. */}
        {!selecting && session.worktree_branch && session.worktree_status !== 'merged' && (
          <span className="flex shrink-0 text-muted-foreground/60" title={`Worktree branch ${session.worktree_branch}`}>
            <GitBranch className="size-3" aria-label={`Branch ${session.worktree_branch}`} />
          </span>
        )}
        {!selecting && forkCount > 0 && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onToggleForks?.();
            }}
            title={`${forksOpen ? 'Hide' : 'Show'} ${forkCount} fork${forkCount === 1 ? '' : 's'} and thread${forkCount === 1 ? '' : 's'}`}
            className={cn(
              'inline-flex h-4 shrink-0 items-center gap-0.5 rounded-full px-1.5 text-[10.5px] font-medium tabular-nums transition-colors',
              forksOpen
                ? 'bg-mira-blue/12 text-mira-blue hover:bg-mira-blue/20'
                : 'bg-fg/[0.07] text-muted-foreground hover:bg-fg/[0.12] hover:text-foreground',
            )}
          >
            <GitFork className="size-2.5" />
            {forkCount}
          </button>
        )}
        {!selecting && session.pinned && (
          <Pin className="size-3 shrink-0 rotate-45 text-mira-blue/70" aria-label="Pinned" />
        )}
        <span className="shrink-0 text-[10.5px] leading-4 tabular-nums text-muted-foreground/50">
          {timeAgo(session.updated_at)}
        </span>
      </span>
      {/* Single far-right slot. Status circle sits underneath the row
       *  menu — both share the same absolute box so the layout never
       *  shifts when the menu appears on hover. Space is reserved even
       *  when the menu is hidden. Hidden while selecting. */}
      {!selecting && (
        <div className="relative flex size-5 items-center justify-center">
          <span
            className={cn(
              'absolute inset-0 flex items-center justify-center transition-opacity',
              'group-hover:opacity-0',
            )}
          >
            <SessionStatus
              running={running}
              unread={unread}
              merged={session.worktree_status === 'merged'}
              waiting={!active && session.needs_attention === true}
            />
          </span>
          <span
            className={cn(
              'absolute inset-0 flex items-center justify-center opacity-0 transition-opacity',
              'group-hover:opacity-100 focus-within:opacity-100',
            )}
          >
            <RowMenu
              items={backgroundMenuItems({
                session,
                onRename,
                onDelete,
                onSetBackgroundMode,
                onPin,
                onArchive,
              })}
            />
          </span>
        </div>
      )}
    </div>
  );
}

/** What runs this session: its agent's mark, or the vendor mark of the
 *  model its provider serves. Every row gets one, so agent and provider
 *  sessions read alike at a glance and the name is on hover. */
function EngineBadge({ session }: { session: SessionSummary }) {
  if (session.agent_driver) {
    return (
      <span title={`Runs on ${agentName(session.agent_driver)}`} className="flex shrink-0">
        <AgentIcon kind={session.agent_driver} name={agentName(session.agent_driver)} size="xs" tile={false} />
      </span>
    );
  }
  return (
    <span title={session.model || 'Mira provider'} className="flex shrink-0">
      <ModelIcon model={session.model} size="xs" />
    </span>
  );
}

function agentName(kind: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code', codex: 'Codex', cursor: 'Cursor', grok: 'Grok',
    opencode: 'OpenCode', antigravity: 'Antigravity',
  };
  return known[kind] ?? kind;
}

/** The right-side status affordance. Priority: running (spinner) > merged
 *  (green check) > idle (empty circle outline). Matches Codex's row-status
 *  ring — quiet by default, expressive when there's a state worth noting. */
function SessionStatus({ running, unread, merged, waiting = false }: { running: boolean; unread: boolean; merged: boolean; waiting?: boolean }) {
  // Waiting on you beats everything: it's the one state that stalls
  // without you.
  if (waiting) {
    return (
      <span
        className="inline-flex size-4 items-center justify-center text-amber-500"
        title="Waiting on you: an approval, a question or a plan"
        aria-label="needs you"
      >
        <CircleAlert className="size-3.5" />
      </span>
    );
  }
  if (unread && !running) {
    return (
      <span
        className="inline-flex size-4 items-center justify-center"
        title="Finished while you were away"
        aria-label="finished, unread"
      >
        <span className="size-2 rounded-full bg-mira-blue shadow-[0_0_6px_rgba(96,165,250,0.7)]" />
      </span>
    );
  }
  if (running) {
    // Same spinner the transcript uses in <Thinking /> and every
    // ToolGroup / AgentCard while a call is in flight — size-3 mira-blue
    // CircleNotch, no size-3.5 outlier.
    return (
      <span
        className="inline-flex size-4 items-center justify-center"
        title="Streaming"
        aria-label="working"
      >
        <Loader className="size-3 shrink-0 animate-spin text-mira-blue" />
      </span>
    );
  }
  if (merged) {
    return (
      <span
        className="inline-flex size-4 items-center justify-center text-emerald-500"
        title="Merged"
        aria-label="merged"
      >
        <CircleCheck className="size-4" />
      </span>
    );
  }
  return (
    <span
      className="inline-flex size-4 items-center justify-center text-muted-foreground/40"
      aria-label="idle"
    >
      <Circle className="size-3.5" />
    </span>
  );
}

/* ---------- archived row + bulk delete confirm ---------- */

/** One archived session. Clicking the row opens it where it is; Restore
 *  puts it back in its folder; Delete is permanent and confirms. */
function ArchivedRow({
  session, active, onOpen, onRestore, onDelete,
}: {
  session: SessionSummary;
  active: boolean;
  onOpen: () => void;
  onRestore: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      className={cn(
        'group grid w-full grid-cols-[1fr_auto] items-center gap-1.5 rounded-lg px-2 py-1.5 transition-colors',
        active ? 'bg-fg/[0.1] text-foreground' : 'text-foreground/70 hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      <button type="button" onClick={onOpen} className="flex min-w-0 items-center gap-2 text-left" title="Open">
        <EngineBadge session={session} />
        <span className="min-w-0 flex-1 truncate text-[13px] leading-4 font-medium">
          {sessionLabel(session)}
        </span>
        <span className="shrink-0 text-[10.5px] tabular-nums text-muted-foreground/50">
          {timeAgo(session.updated_at)}
        </span>
      </button>
      <div className="flex items-center gap-0.5">
        <button
          type="button"
          onClick={onRestore}
          title="Restore to its folder"
          aria-label="Restore"
          className="rounded-sm p-0.5 text-muted-foreground/60 opacity-0 transition-opacity hover:bg-fg/[0.05] hover:text-foreground group-hover:opacity-100 focus:opacity-100"
        >
          <ArchiveRestore className="size-3.5" />
        </button>
        <RowMenu
          items={[
            { label: 'Restore', icon: <ArchiveRestore className="size-3.5" />, onSelect: onRestore },
            {
              label: 'Delete permanently',
              danger: true,
              confirm: 'Delete this session? This cannot be undone.',
              icon: <Trash2 className="size-3.5" />,
              onSelect: onDelete,
            },
          ]}
        />
      </div>
    </div>
  );
}

/* ---------- rename dialog ---------- */

function RenameDialog({
  session, onClose, onManual, onAi,
}: {
  session: SessionSummary | null;
  onClose: () => void;
  onManual: (id: string, title: string) => Promise<void>;
  onAi: (id: string) => Promise<string>;
}) {
  const [value, setValue] = useState('');
  const [aiBusy, setAiBusy] = useState(false);
  const [saveBusy, setSaveBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [aiApplied, setAiApplied] = useState<string | null>(null);

  // Hydrate the input each time the dialog opens for a different session.
  useEffect(() => {
    if (!session) return;
    // Seed the rename input with the cleaned-up label so the user isn't
    // editing a raw "## Attached files …" markdown blob when their first
    // turn was an attachment.
    setValue(session.title ?? (session.first_user_message ? sessionLabel(session) : ''));
    setAiBusy(false);
    setSaveBusy(false);
    setErr(null);
    setAiApplied(null);
  }, [session]);

  if (!session) return null;

  async function submitManual(e: React.FormEvent) {
    e.preventDefault();
    if (!session) return;
    const trimmed = value.trim();
    if (!trimmed) return;
    setSaveBusy(true);
    setErr(null);
    try {
      await onManual(session.id, trimmed);
    } catch (e2) {
      setErr(String((e2 as Error).message));
    } finally {
      setSaveBusy(false);
    }
  }

  async function submitAi() {
    if (!session) return;
    setAiBusy(true);
    setErr(null);
    setAiApplied(null);
    try {
      const generated = await onAi(session.id);
      // Populate the input with the AI result. Don't auto-close so the
      // user can see what was generated (and re-run / edit / cancel).
      // The rename is already committed server-side by the time we get
      // here; clicking Cancel now would leave the AI title in place.
      setValue(generated);
      setAiApplied(generated);
    } catch (e2) {
      setErr(String((e2 as Error).message));
    } finally {
      setAiBusy(false);
    }
  }

  return (
    <Dialog open={!!session} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md p-0 gap-0 overflow-hidden">
        <form onSubmit={submitManual} className="flex flex-col gap-4 p-5">
          <div>
            <div className="text-[15px] font-semibold text-foreground">Rename session</div>
            <div className="mt-1 text-[12px] text-muted-foreground">
              Type a new name below, or let the model pick one from the conversation.
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <label htmlFor="rename-input" className="text-[12.5px] font-medium text-foreground/85">
              Title
            </label>
            <Input
              id="rename-input"
              value={value}
              onChange={(e) => {
                setValue(e.target.value);
                setAiApplied(null);
              }}
              placeholder="Session name"
              spellCheck={false}
              autoFocus
              disabled={saveBusy || aiBusy}
            />
          </div>

          {aiApplied && (
            <div className="rounded-md border border-emerald-500/40 bg-emerald-500/[0.06] px-3 py-2 text-[12.5px] text-emerald-400">
              AI applied: <span className="font-mono">{aiApplied}</span> — click Done to close, or edit + Save.
            </div>
          )}
          {err && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12.5px] text-destructive">
              {err}
            </div>
          )}

          <div className="flex items-center justify-between gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={submitAi}
              disabled={saveBusy || aiBusy}
              className="gap-1.5"
            >
              <Sparkles className="size-3.5" />
              {aiBusy ? 'Generating…' : aiApplied ? 'Regenerate' : 'Rename with AI'}
            </Button>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={onClose}
                disabled={saveBusy || aiBusy}
              >
                {aiApplied ? 'Done' : 'Cancel'}
              </Button>
              <Button type="submit" disabled={saveBusy || aiBusy || !value.trim()}>
                {saveBusy ? 'Saving…' : 'Save'}
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/* ---------- grouping ---------- */

type Group = {
  cwd: string;
  label: string;
  isCurrent: boolean;
  sessions: SessionSummary[];
};

function groupByCwd(sessions: SessionSummary[], currentCwd: string): Group[] {
  const byCwd = new Map<string, SessionSummary[]>();
  for (const s of sessions) {
    const bucket = byCwd.get(s.cwd);
    if (bucket) bucket.push(s); else byCwd.set(s.cwd, [s]);
  }
  // Pinned sessions float to the top of their folder (issue #58); a
  // stable sort keeps the backend's newest-first order within each tier.
  for (const list of byCwd.values()) {
    list.sort((a, b) => Number(b.pinned ?? false) - Number(a.pinned ?? false));
  }
  const groups: Group[] = [];
  for (const [cwd, list] of byCwd) {
    // Backend already sorts by updated_at DESC across all cwds, so this
    // per-bucket order is already correct.
    groups.push({
      cwd,
      label: basename(cwd),
      isCurrent: cwd === currentCwd,
      sessions: list,
    });
  }
  // Current project first, then by "most recent activity in that project"
  // (which the first session's updated_at approximates cheaply).
  groups.sort((a, b) => {
    if (a.isCurrent !== b.isCurrent) return a.isCurrent ? -1 : 1;
    const at = a.sessions[0]?.updated_at ?? 0;
    const bt = b.sessions[0]?.updated_at ?? 0;
    return bt - at;
  });
  return groups;
}

/** Search: title, first message, folder and branch, case-insensitive. */
function matchesQuery(s: SessionSummary, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [sessionLabel(s), s.first_user_message, s.worktree_branch, basename(s.cwd), s.agent_driver]
    .some((field) => (field ?? '').toLowerCase().includes(q));
}

/** Which period a row falls in, for the date sections. Pinned chats sit
 *  at the top of their folder, so they get their own section. */
function dateSection(s: SessionSummary): string {
  if (s.pinned) return 'Pinned';
  if (!s.updated_at) return 'Older';
  const then = new Date(s.updated_at * 1000);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const day = 86_400_000;
  const t = then.getTime();
  if (t >= today.getTime()) return 'Today';
  if (t >= today.getTime() - day) return 'Yesterday';
  if (t >= today.getTime() - 6 * day) return 'This week';
  if (t >= today.getTime() - 29 * day) return 'This month';
  return 'Older';
}

function basename(p: string): string {
  if (!p) return 'unknown';
  const trimmed = p.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i >= 0 ? trimmed.slice(i + 1) || '/' : trimmed;
}

/* ---------- collapsed-state persistence ---------- */

function loadCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return new Set();
    const arr = JSON.parse(raw);
    return new Set(Array.isArray(arr) ? arr : []);
  } catch { return new Set(); }
}

function persistCollapsed(set: Set<string>) {
  try { localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set])); }
  catch { /* private-mode etc; ignore */ }
}

/** Themed folder icon for a project row, falling back to the lucide glyph
 *  while the icon set loads. */
function SidebarFolderIcon({
  name,
  open,
  className,
  current,
}: {
  name: string;
  open: boolean;
  className?: string;
  current?: boolean;
}) {
  const { folderIcon } = useFileIcons();
  const dataUri = folderIcon(name, open);
  if (dataUri) {
    return (
      <img
        src={dataUri}
        alt=""
        className={cn('shrink-0', className, !current && 'opacity-80')}
        draggable={false}
      />
    );
  }
  return (
    <Folder
      className={cn(
        'shrink-0',
        className,
        current ? 'text-mira-blue' : 'text-muted-foreground/70',
      )}
    />
  );
}

/* ---------- little helpers ---------- */

function NavItem({
  icon, disabled, active, onClick, onIntent, children,
}: {
  icon: React.ReactNode;
  disabled?: boolean;
  /** True when this item's view is currently rendered in the main pane —
   *  gets the same accent treatment as an active session row. */
  active?: boolean;
  onClick?: () => void;
  /** Hover/focus: preload whatever the click opens. */
  onIntent?: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      onPointerEnter={onIntent}
      onFocus={onIntent}
      title={disabled ? 'Not implemented yet' : undefined}
      className={cn(
        'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[14.5px] transition-colors',
        disabled
          ? 'text-muted-foreground/40 cursor-not-allowed'
          : active
            ? 'bg-fg/[0.1] text-foreground'
            : 'text-foreground hover:bg-fg/[0.05]',
      )}
    >
      <span
        className={cn(
          'shrink-0',
          disabled
            ? 'text-muted-foreground/40'
            : active
              ? 'text-foreground'
              : 'text-muted-foreground',
        )}
      >
        {icon}
      </span>
      <span>{children}</span>
    </button>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <div className="px-2.5 py-1 text-[13px] text-muted-foreground/60">{children}</div>;
}

function timeAgo(unixSecs: number): string {
  // No timestamp (0) is unknown, not 1970: show nothing rather than "2961w".
  if (!unixSecs || unixSecs <= 0) return '';
  const now = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, now - unixSecs);
  if (diff < 60) return `${diff}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  const days = Math.floor(diff / 86400);
  if (days < 7) return `${days}d`;
  if (days < 60) return `${Math.floor(days / 7)}w`;
  if (days < 365) return `${Math.floor(days / 30)}mo`;
  return `${Math.floor(days / 365)}y`;
}


/* ---------- row overflow menu (extensible) ---------- */

type RowMenuItem = {
  label: string;
  icon?: React.ReactNode;
  danger?: boolean;
  /** Confirmation prompt shown before running `onSelect`. Skips confirm if null. */
  confirm?: string | null;
  /** Optional check-mark rendered on the right — used by radio-style
   *  sub-items so the current background mode reads at a glance. */
  checked?: boolean;
  onSelect: () => void | Promise<void>;
};

/** RowMenu items for a session row. Renders rename + pin/archive (when
 *  wired) + delete plus, when the caller wired a background-mode handler,
 *  three radio-style items for the current per-slot policy. The three
 *  modes always render (rather than hiding when the slot isn't loaded) so
 *  the user can see the choice; clicking on a persisted-but-not-loaded
 *  row 404s — callers should typically attach first. */
function backgroundMenuItems(args: {
  session: SessionSummary;
  onRename: () => void;
  onDelete: () => void;
  onSetBackgroundMode?: (mode: BackgroundMode) => void;
  onPin?: (pinned: boolean) => void;
  onArchive?: () => void;
}): RowMenuItem[] {
  const items: RowMenuItem[] = [
    {
      label: 'Rename session',
      icon: <Pencil className="size-3.5" />,
      onSelect: args.onRename,
    },
  ];
  if (args.onPin) {
    items.push({
      label: args.session.pinned ? 'Unpin session' : 'Pin session',
      icon: args.session.pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />,
      onSelect: () => args.onPin!(!args.session.pinned),
    });
  }
  if (args.onArchive) {
    items.push({
      label: 'Archive session',
      icon: <Archive className="size-3.5" />,
      onSelect: args.onArchive,
    });
  }
  if (args.onSetBackgroundMode) {
    const current = args.session.background_mode ?? null;
    const modes: Array<{ mode: BackgroundMode; label: string; desc: string }> = [
      { mode: 'deny', label: 'Background: Auto-deny', desc: 'Safe default — approvals silently fail if you leave.' },
      { mode: 'auto_approve', label: 'Background: Auto-approve', desc: 'Trust this session to keep going without you.' },
      { mode: 'park', label: 'Background: Wait for me', desc: 'Park approvals until you re-attach.' },
    ];
    for (const m of modes) {
      items.push({
        label: m.label,
        checked: current === m.mode,
        onSelect: () => args.onSetBackgroundMode!(m.mode),
      });
    }
  }
  items.push({
    label: 'Delete session',
    danger: true,
    confirm: 'Delete this session? This cannot be undone.',
    icon: <Trash2 className="size-3.5" />,
    onSelect: args.onDelete,
  });
  return items;
}

function RowMenu({ items }: { items: RowMenuItem[] }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState<RowMenuItem | null>(null);

  function pick(item: RowMenuItem) {
    if (item.confirm) {
      setConfirming(item);
    } else {
      setOpen(false);
      void item.onSelect();
    }
  }

  return (
    <Popover
      open={open}
      onOpenChange={(v) => {
        setOpen(v);
        if (!v) setConfirming(null);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Row menu"
          onClick={(e) => e.stopPropagation()}
          className={cn(
            'shrink-0 rounded-sm p-0.5 text-muted-foreground/60 transition-opacity hover:bg-fg/[0.05] hover:text-foreground',
            open ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus:opacity-100',
          )}
        >
          <Ellipsis className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="w-56 p-1"
        align="end"
        // Prevent the enclosing row's onClick from firing when the user
        // clicks anywhere inside the menu popover.
        onClick={(e) => e.stopPropagation()}
      >
        {confirming ? (
          <div className="flex flex-col gap-2 px-2 py-1.5">
            <div className="text-[12.5px] text-foreground">{confirming.confirm}</div>
            <div className="flex justify-end gap-1.5">
              <button
                type="button"
                onClick={() => setConfirming(null)}
                className="rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-fg/[0.05]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={async () => {
                  const item = confirming;
                  setConfirming(null);
                  setOpen(false);
                  await item.onSelect();
                }}
                className={cn(
                  'rounded-md px-2 py-1 text-[12px] font-medium',
                  confirming.danger
                    ? 'bg-destructive text-white hover:opacity-90'
                    : 'bg-foreground text-background hover:opacity-90',
                )}
              >
                {confirming.danger ? 'Delete' : 'OK'}
              </button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col">
            {items.map((it, i) => (
              <button
                key={i}
                type="button"
                onClick={() => pick(it)}
                className={cn(
                  'flex items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] transition-colors',
                  it.danger
                    ? 'text-destructive hover:bg-destructive/10'
                    : 'text-foreground hover:bg-fg/[0.05]',
                )}
              >
                {it.icon && <span className="shrink-0 text-muted-foreground">{it.icon}</span>}
                <span className="flex-1">{it.label}</span>
                {it.checked && (
                  <CircleCheck
                    className="size-3.5 shrink-0 text-emerald-500"
                  />
                )}
              </button>
            ))}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

const UNREAD_KEY = 'mira.sidebar.unread';
function loadUnread(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(UNREAD_KEY) ?? '[]');
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
  } catch {
    return new Set();
  }
}
function saveUnread(ids: Set<string>) {
  try {
    localStorage.setItem(UNREAD_KEY, JSON.stringify([...ids].slice(-200)));
  } catch {
    /* private mode */
  }
}
