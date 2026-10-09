import { m, useReducedMotion } from 'framer-motion';
import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import { matchesSidebar, rowStatus, settledPeriod, type SidebarFilter } from '../lib/sidebar';
import { BranchElbow } from './BranchElbow';
import { useEffect, useMemo, useRef, useState } from 'react';
import { LazyBoundary } from './LazyBoundary';
import {
  Archive,
  ArchiveRestore,
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  CheckCheck,
  CircleAlert,
  CircleCheck,
  Circle,
  Ellipsis,
  Folder,
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
  History,
  Loader,
  Pencil,
  PenLine,
  Pin,
  PinOff,
  Puzzle,
  Search,
  Sparkles,
  Split,
  Timer,
  Trash2,
  X,
} from 'lucide-react';
import {
  deleteSession,
  listSessions,
  loadSession,
  regenerateSessionTitle,
  renameSession,
  setSessionFlags,
} from '../api';
import type { BackgroundMode, SessionPr, SessionSummary } from '../types';
import { isSettled, settledOrder } from '../lib/settled';
import { openExternal } from '@/lib/desktop';
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
const RECENTS_OPEN_KEY = 'mira.sidebar.recents-open';
const SETTLED_OPEN_KEY = 'mira.sidebar.settled-open';
const PER_GROUP_LIMIT = 5;
const RECENTS_LIMIT = 3;
const SHOW_MORE_KEY = 'mira.sidebar.expanded-projects';
const FORKS_KEY = 'mira.sidebar.hidden-forks';
const SCROLL_KEY = 'mira.sidebar.scroll';
const NEEDS_MORE_KEY = 'mira.sidebar.needs-more';
const RECENTS_MORE_KEY = 'mira.sidebar.recents-more';
/** The settled shelf is history: a page at a time. */
const SETTLED_PAGE = 10;

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
  const [forksHidden, setForksHidden] = useState<Set<string>>(() => loadSet(FORKS_KEY));
  const toggleForks = (id: string) =>
    setForksHidden((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      saveSet(FORKS_KEY, next);
      return next;
    });
  const [showMore, setShowMore] = useState<Set<string>>(() => loadSet(SHOW_MORE_KEY));
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
  const livePeek = useMemo(() => peek ? {
    ...peek, session: sessions.find((s) => s.id === peek.session.id) ?? peek.session,
  } : null, [peek, sessions]);
  // Sidebar search: filters every folder at once, and opens them while it
  // has text so a match is never hidden behind a collapsed folder.
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<SidebarFilter>('all');
  const searching = query.trim().length > 0 || filter !== 'all';
  const [needsMore, setNeedsMore] = useState(() => loadFlag(NEEDS_MORE_KEY, false));
  const [recentsMore, setRecentsMore] = useState(() => loadFlag(RECENTS_MORE_KEY, false));
  const scrollRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const projectsToolbarRef = useRef<HTMLDivElement>(null);
  const [hasScrolled, setHasScrolled] = useState(false);
  const [projectsPinned, setProjectsPinned] = useState(false);
  const savedScroll = useRef(loadScroll());
  const scrollRestored = useRef(false);
  useEffect(() => {
    if (activeView === 'settings') { scrollRestored.current = false; return; }
    const node = scrollRef.current;
    if (!node || scrollRestored.current || sessions.length === 0) return;
    // Wait for collapsible sections to reach their final height.
    const id = window.setTimeout(() => {
      node.scrollTop = savedScroll.current;
      scrollRestored.current = true;
    }, 280);
    return () => window.clearTimeout(id);
  }, [sessions.length, activeView]);

  // The two shelves around the project list, each remembered across
  // reloads: Recents starts open, Settled (history) starts closed.
  useEffect(() => {
    const node = scrollRef.current;
    if (!node || !scrollRestored.current) return;
    node.scrollTop = searching ? 0 : savedScroll.current;
  }, [searching]);

  const [recentsOpen, setRecentsOpen] = useState(() => loadFlag(RECENTS_OPEN_KEY, true));
  const [settledOpen, setSettledOpen] = useState(() => loadFlag(SETTLED_OPEN_KEY, false));
  const [settledShown, setSettledShown] = useState(SETTLED_PAGE);

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

  // Refresh worktree PRs even before the first answer arrives, and after
  // a PR closes: GitHub changes and slow initial fetches are not pushed.
  const anyWorktree = sessions.some((s) => !!s.worktree_branch);
  useEffect(() => {
    if (!anyWorktree) return;
    const id = window.setInterval(refresh, 60_000);
    return () => window.clearInterval(id);
  }, [anyWorktree]);

  // Chats that finished while you were looking at something else get a
  // dot until you open them. Remembered across reloads.
  const [unread, setUnread] = useState<Set<string>>(() => loadUnread());
  const wasRunning = useRef<Set<string>>(new Set());
  const seenCompletions=useRef(new Map<string,number>());
  useEffect(() => {
    const next = new Set(unread);
    let changed = false;
    let newCompletion = false;
    for(const [id,revision] of completedSessions){
      if(seenCompletions.current.get(id)===revision)continue;
      seenCompletions.current.set(id,revision);
      newCompletion = true;
      if(id!==activeSessionId&&!next.has(id)){next.add(id);changed=true;}
    }
    if (newCompletion) refresh();
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
  async function flagSession(id: string, flags: { pinned?: boolean; archived?: boolean; settled?: boolean }) {
    try {
      await setSessionFlags(id, flags);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
    refresh();
  }

  async function changeBackgroundMode(id: string, mode: BackgroundMode) {
    try {
      await onSetBackgroundMode?.(id, mode);
      setSessions((prev) => prev.map((s) => s.id === id ? { ...s, background_mode: mode } : s));
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
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

  const [arrangedSessions, setArrangedSessions] = useState<SessionSummary[]>([]);
  useEffect(() => {
    if (!peek) setArrangedSessions(sessions);
  }, [sessions, peek]);
  const currentSession = (s: SessionSummary) => sessions.find((current) => current.id === s.id) ?? s;
  const needsYou = arrangedSessions.filter((s) => s.needs_attention || s.failure_reason)
    .sort((a, b) => b.updated_at - a.updated_at);
  // Settled chats leave their folders for the shelf at the bottom — except
  // while searching, which looks everywhere at once.
  const { live, settled } = useMemo(() => {
    const live: SessionSummary[] = [];
    const settled: SessionSummary[] = [];
    for (const s of arrangedSessions) (isSettled(s) ? settled : live).push(s);
    settled.sort(settledOrder);
    return { live, settled };
  }, [arrangedSessions]);
  const recents = useMemo(
    () => [...live].sort((a, b) => b.updated_at - a.updated_at).filter((s) => !s.needs_attention && !s.failure_reason).slice(0, recentsMore ? 8 : RECENTS_LIMIT),
    [live, recentsMore],
  );
  const groups = useMemo(
    () => groupByCwd(searching ? arrangedSessions.filter((s) => matchesSidebar(s, query, filter)) : live, cwd),
    [arrangedSessions, live, cwd, query, searching, filter],
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

  function toggleShelf(key: string, set: (fn: (v: boolean) => boolean) => void) {
    set((v) => {
      saveFlag(key, !v);
      return !v;
    });
  }

  /** A row outside the folder tree (Recents, Settled): no forks under it. */
  function flatRow(s: SessionSummary, where: string) {
    return (
      <SessionRow
        key={`${where}-${s.id}`}
        session={currentSession(s)}
        settled={isSettled(currentSession(s))}
        active={s.id === activeSessionId && activeView === 'chat'}
        activeBusy={runningSessions == null && activeBusy}
        unread={unread.has(s.id)}
        selecting={selecting}
        checked={selected.has(s.id)}
        onToggleSelect={() => toggleSelected(s.id)}
        onPeek={(rect) => setPeek(rect ? { session: s, rect } : null)}
        onPick={() => pickSession(s.id)}
        onRename={() => setRenaming(s)}
        onDelete={() => removeSession(s.id)}
        onPin={(p) => void flagSession(s.id, { pinned: p })}
        onArchive={() => void flagSession(s.id, { archived: true })}
        onSettle={(v) => void flagSession(s.id, { settled: v })}
        onSetBackgroundMode={
          onSetBackgroundMode
            ? (mode) => void changeBackgroundMode(s.id, mode)
            : undefined
        }
      />
    );
  }

  function toggleShowMore(key: string) {
    setShowMore((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      saveSet(SHOW_MORE_KEY, next);
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
                      ? 'sidebar-active bg-fg/[0.1] text-foreground'
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
      <div className="flex min-h-0 flex-1 flex-col">
      <div className="relative z-30 shrink-0 bg-panel px-2 pb-1">
        <NavItem icon={<PenLine className="size-3.5" />} onClick={onNewChat}>
          New thread
        </NavItem>
        {hasScrolled && !projectsPinned && <span aria-hidden className="sidebar-scroll-fade" />}
      </div>
      <div ref={scrollRef} onScroll={(e) => {
        const node = e.currentTarget;
        setPeek(null);
        setHasScrolled(node.scrollTop > 0);
        setProjectsPinned(!!projectsToolbarRef.current && projectsToolbarRef.current.getBoundingClientRect().top <= node.getBoundingClientRect().top + 1);
        if (!scrollRestored.current || searching) return;
        savedScroll.current = node.scrollTop;
        try { localStorage.setItem(SCROLL_KEY, String(node.scrollTop)); } catch { /* optional storage */ }
      }} className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2 [overflow-anchor:none]">
        <nav className="flex flex-col gap-0.5 px-0.5">
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

        {!searching && needsYou.length > 0 && (
          <div className="mt-4 flex flex-col gap-0.5 px-0.5">
            <div className="flex items-center gap-1.5 px-2 py-1 text-[11.5px] font-semibold text-amber-700 dark:text-amber-400">
              <CircleAlert className="size-3" /> Needs you
              <span className="ml-auto text-[10.5px] tabular-nums opacity-70">{needsYou.length}</span>
            </div>
            <div className="flex flex-col gap-0.5 pl-1">
              {(needsMore ? needsYou : needsYou.slice(0, 2)).map((s) => flatRow(s, 'attention'))}
              {needsYou.length > 2 && (
                <button type="button" onClick={() => { saveFlag(NEEDS_MORE_KEY, !needsMore); setNeedsMore(!needsMore); }}
                  className="px-2 py-1 text-left text-[11px] text-muted-foreground hover:text-foreground">
                  {needsMore ? 'Show less' : `View ${needsYou.length - 2} more chats`}
                </button>
              )}
            </div>
          </div>
        )}

        {!searching && recents.length > 0 && (
          <div className="mt-4 flex flex-col gap-0.5 px-0.5">
            <ShelfHeader
              icon={<History className="size-3 shrink-0" />}
              label="Recents"
              open={recentsOpen}
              onToggle={() => toggleShelf(RECENTS_OPEN_KEY, setRecentsOpen)}
            />
            <Collapse open={recentsOpen}>
              <div className="flex flex-col gap-0.5 pl-1">
                {recents.map((s) => flatRow(s, 'recent'))}
                {live.filter((s) => !s.needs_attention && !s.failure_reason).length > RECENTS_LIMIT && (
                  <button type="button" onClick={() => {
                    saveFlag(RECENTS_MORE_KEY, !recentsMore); setRecentsMore(!recentsMore);
                  }} className="px-2 py-1 text-left text-[11px] text-muted-foreground hover:text-foreground">
                    {recentsMore ? 'Show less' : 'More recent chats'}
                  </button>
                )}
              </div>
            </Collapse>
          </div>
        )}

        <div className="mt-4 flex flex-col gap-1 px-0.5">
          <div ref={projectsToolbarRef} className="sticky top-0 z-20 -mx-0.5 bg-panel px-0.5 pb-1">
          <div className="mb-1 flex items-center justify-between px-2 py-1.5">
            <div className="flex items-center gap-2 text-[14px] font-semibold tracking-tight text-foreground">
              <Folder className="size-3.5 text-muted-foreground" aria-hidden />
              Projects
            </div>
            {!selecting && groups.length > 0 && (
              <button
                type="button"
                onClick={toggleSelectMode}
                className="sidebar-select rounded-full border border-border/60 px-2.5 py-1 text-[11px] font-medium text-foreground/75 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
              >
                Select
              </button>
            )}
          </div>

          {!selecting && (sessions.length > 0 || searching) && (
            <label className="sidebar-search mx-1 mb-1 flex items-center gap-1.5 rounded-xl border border-border/60 px-1.5 py-1.5 text-[12px] transition-shadow focus-within:ring-2 focus-within:ring-mira-blue/15">
              <button type="button" aria-label="Focus chat search" onClick={() => searchRef.current?.focus()}
                className="flex size-6 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-fg/[0.05] hover:text-foreground">
                <Search className="size-3.5" aria-hidden />
              </button>
              {/* Plain text, not type=search: that adds the browser's own
                  clear button beside ours. */}
              <input
                ref={searchRef}
                type="text"
                enterKeyHint="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Escape') setQuery(''); }}
                placeholder="Search chats, branches, PRs"
                aria-label="Search chats"
                className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground/60"
              />
              {query && (
                <button type="button" onClick={() => setQuery('')} aria-label="Clear search" className="flex size-6 shrink-0 items-center justify-center rounded-full text-muted-foreground/70 transition-colors hover:bg-fg/[0.06] hover:text-foreground">
                  <X className="size-3.5" />
                </button>
              )}
            </label>
          )}

          {!selecting && sessions.length > 0 && (
            <div className="mx-1 mb-2 flex gap-1" aria-label="Filter chats">
              {(['all', 'running', 'waiting', 'settled'] as SidebarFilter[]).map((value) => (
                <button type="button" key={value} aria-pressed={filter === value}
                  onClick={() => setFilter(value)}
                  className={cn('rounded-full px-2 py-1 text-[10.5px] transition-colors',
                    filter === value ? 'sidebar-active bg-fg/[0.08] font-medium text-foreground' : 'text-muted-foreground hover:bg-fg/[0.04] hover:text-foreground')}>
                  {{ all: 'All', running: 'Working', waiting: 'Needs you', settled: 'Settled' }[value]}
                </button>
              ))}
            </div>
          )}

          {projectsPinned && <span aria-hidden className="sidebar-scroll-fade" />}
          </div>
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


            return (
              <div key={g.cwd} className="flex flex-col">
                <div
                  className="group flex items-center gap-1.5 rounded-md px-2 py-1 text-[14.5px] font-semibold text-foreground/90 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
                  title={g.cwd}
                >
                  <button
                    type="button"
                    onClick={() => toggleCollapsed(g.cwd)}
                    aria-expanded={!isCollapsed}
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
                        session={currentSession(s)}
                        depth={depth}
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
                        settled={isSettled(currentSession(s))}
                        onSettle={(v) => void flagSession(s.id, { settled: v })}
                        onSetBackgroundMode={
                          onSetBackgroundMode
                            ? (mode) => void changeBackgroundMode(s.id, mode)
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

          {/* Settled: chats whose work is done — their PR merged or closed,
              or the user put them away. Out of the folders, still a click
              away; writing to one brings it back. */}
          {!searching && settled.length > 0 && (
            <div className="mt-4 flex flex-col gap-0.5 px-0.5">
              <ShelfHeader
                icon={<CheckCheck className="size-3 shrink-0" />}
                label="Settled"
                count={settled.length}
                open={settledOpen}
                onToggle={() => toggleShelf(SETTLED_OPEN_KEY, setSettledOpen)}
              />
              <Collapse open={settledOpen}>
                <div className="flex flex-col gap-0.5 pl-1">
                  {settled.slice(0, settledShown).map((s, i, rows) => (
                    <div key={s.id}>
                      {(i === 0 || settledPeriod(s) !== settledPeriod(rows[i - 1])) && (
                        <div className="px-2 pb-1 pt-2 text-[10.5px] font-medium text-muted-foreground/60">{settledPeriod(s)}</div>
                      )}
                      {flatRow(s, 'settled')}
                    </div>
                  ))}
                  {settled.length > settledShown && (
                    <button
                      onClick={() => setSettledShown((n) => n + SETTLED_PAGE * 2)}
                      className="px-2 py-1 text-left text-[12.5px] text-muted-foreground/80 transition-colors hover:text-foreground"
                    >
                      Show {Math.min(settled.length - settledShown, SETTLED_PAGE * 2)} more
                    </button>
                  )}
                </div>
              </Collapse>
            </div>
          )}

          {/* Archived view (issue #58) — sessions hidden from the main
              list live here. Clicking a row opens it; Restore puts it back
              in its folder; Delete is permanent. */}
          {!selecting && !searching && (
            <div className="mt-4 flex flex-col gap-0.5 px-0.5">
              <ShelfHeader
                icon={<Archive className="size-3 shrink-0" />}
                label="Archived"
                count={archived?.length || undefined}
                open={showArchived}
                onToggle={() => setShowArchived((v) => !v)}
              />
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
            target={livePeek}
            pr={peek ? peek.session.pr ?? (peek.session.id === activeSessionId ? activePr ?? null : null) : null}
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
  settled = false,
  onSettle,
  selecting = false,
  checked = false,
  onToggleSelect,
}: {
  session: SessionSummary;
  /** Fork nesting level: 0 for a chat, 1 for a fork of it, … */
  depth?: number;
  /** The label of the chat this one was forked from, for the tooltip. */
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
  /** Sits on the Settled shelf; the menu offers to take it back out. */
  settled?: boolean;
  onSettle?: (settled: boolean) => void;
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
  const detail = rowStatus({ ...session, running }, unread);
  const systemReducedMotion = useReducedMotion();
  const [preferReducedMotion] = useBoolPref(PREF_KEYS.reduceMotion, false);
  return (
    <m.div layout="position" transition={{ layout: { duration: systemReducedMotion || preferReducedMotion ? 0 : 0.16 } }}
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
      onFocus={(e) => { if (e.target === e.currentTarget) onPeek?.(e.currentTarget.getBoundingClientRect()); }}
      onBlur={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) onPeek?.(null); }}
      onMouseLeave={onPeek && !selecting ? () => onPeek(null) : undefined}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key !== 'Enter' && e.key !== ' ') return;
        e.preventDefault();
        if (selecting) onToggleSelect?.();
        else onPick();
      }}
      className={cn(
        'group grid w-full cursor-pointer items-center gap-1.5 rounded-lg px-2 py-1.5 transition-colors',
        selecting ? 'grid-cols-[auto_1fr_auto] cursor-pointer' : 'grid-cols-[1fr_auto]',
        selecting && checked
          ? 'sidebar-active bg-fg/[0.1] text-foreground'
          // Same active treatment as the nav items (New thread / PR /
          // Plugins / Scheduled): 10% white band, full foreground.
          : active
            ? 'sidebar-active bg-fg/[0.1] text-foreground'
            : 'text-foreground/90 hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      {selecting && (
        <span className="flex size-4 items-center justify-center self-center text-mira-blue">
          {checked ? <CircleCheck className="size-4" /> : <Circle className="size-3.5" />}
        </span>
      )}
      {/* Keep the title and git indicator on the first row, with
       *  secondary context and the timestamp aligned below. */}
      <span className="grid min-w-0 grid-cols-[16px_minmax(0,1fr)_24px] items-center gap-2 text-left">
        {selecting ? (
          <span />
        ) : depth > 0 && !launched ? (
          // A fork of the chat above: a split, not a git glyph, so it
          // can't be read as a merge or a branch.
          <Split className="size-3.5 shrink-0 rotate-90 justify-self-center text-mira-blue/70" aria-label="Fork" />
        ) : (
          <EngineBadge session={session} />
        )}
        <span className="min-w-0">
        <span className="flex min-w-0 items-center gap-1.5">
          <span
            className={cn(
              'min-w-0 truncate leading-4',
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
              <Split className="size-2.5 rotate-90" />
              {forkCount}
            </button>
          )}
        </span>
        {!selecting && (
          <span className="mt-0.5 flex min-w-0 items-center gap-1 text-[10.5px] leading-[15px]">
            <span className={cn(detail.tone === 'attention' ? 'min-w-0 truncate' : 'shrink-0',
              detail.tone === 'attention' ? 'text-amber-700 dark:text-amber-400'
              : detail.tone === 'error' ? 'text-red-600 dark:text-red-400'
              : detail.tone === 'working' || detail.tone === 'result' ? 'text-mira-blue' : 'text-muted-foreground/75')}>
              {detail.label}
            </span>
            {detail.tone !== 'attention' && (
              <>
                <span className="shrink-0 text-muted-foreground/35">·</span>
                <span className="truncate text-muted-foreground/60">
                  {session.worktree_branch || basename(session.cwd)}
                </span>
              </>
            )}
          </span>
        )}
        </span>

        <span className="flex flex-col items-end self-start">
          <span className="flex h-4 w-6 items-center justify-end">
            {!selecting && <GitMark session={session} />}
          </span>
          {!selecting && (
            <span className="mt-0.5 text-right text-[10.5px] leading-[15px] tabular-nums text-muted-foreground/50">
              {timeAgo(session.updated_at)}
            </span>
          )}
        </span>
      </span>
      {/* Single far-right slot. Status circle sits underneath the row
       *  menu — both share the same absolute box so the layout never
       *  shifts when the menu appears on hover. Space is reserved even
       *  when the menu is hidden. Hidden while selecting. With a finger
       *  there's no hover to swap them, so they sit side by side. */}
      {!selecting && (
        <div className="relative flex size-5 items-center justify-center touch:w-auto touch:gap-1">
          <span
            className={cn(
              'absolute inset-0 flex items-center justify-center transition-opacity touch:static touch:size-5',
              'group-hover:opacity-0',
            )}
          >
            <SessionStatus
              running={running}
              unread={unread}
              waiting={session.needs_attention === true}
              failed={!!session.failure_reason}
            />
          </span>
          <span
            className={cn(
              'absolute inset-0 flex items-center justify-center opacity-0 transition-opacity touch:static touch:size-7',
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
                settled,
                onSettle,
              })}
            />
          </span>
        </div>
      )}
    </m.div>
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

/** Where the chat's work stands in git, as one small mark: its PR (merged
 *  in colour, open / draft / closed in grey, each with its own glyph), or
 *  without one, its worktree branch. Clicking a PR opens it. */
function GitMark({ session }: { session: SessionSummary }) {
  const pr = session.pr;
  if (pr) {
    const { Icon, tone, label } = PR_MARK[pr.state];
    return (
      // A link-role span, not a button: archived rows are buttons
      // themselves, and buttons don't nest.
      <span
        role="link"
        tabIndex={0}
        onClick={(e) => {
          e.stopPropagation();
          void openExternal(pr.url);
        }}
        onKeyDown={(e) => {
          if (e.key !== 'Enter') return;
          e.stopPropagation();
          e.preventDefault();
          void openExternal(pr.url);
        }}
        title={`PR #${pr.number} · ${label}: ${pr.title}`}
        aria-label={`Pull request ${pr.number}, ${label}`}
        className={cn('flex shrink-0 cursor-pointer rounded-sm transition-opacity hover:opacity-80', tone)}
      >
        <Icon className="size-3.5" />
      </span>
    );
  }
  if (!session.worktree_branch) return null;
  const merged = session.worktree_status === 'merged';
  return (
    <span
      className={cn('flex shrink-0', merged ? 'text-violet-500 dark:text-violet-400' : 'text-muted-foreground/60')}
      title={merged ? `Branch ${session.worktree_branch}, merged into main` : `Worktree branch ${session.worktree_branch}`}
    >
      {merged ? (
        <GitMerge className="size-3.5" aria-label="Branch merged" />
      ) : (
        <GitBranch className="size-3" aria-label={`Branch ${session.worktree_branch}`} />
      )}
    </span>
  );
}

const PR_MARK: Record<SessionPr['state'], { Icon: typeof GitMerge; tone: string; label: string }> = {
  merged: { Icon: GitMerge, tone: 'text-violet-500 dark:text-violet-400', label: 'merged' },
  open: { Icon: GitPullRequest, tone: 'text-muted-foreground/70', label: 'open' },
  draft: { Icon: GitPullRequestDraft, tone: 'text-muted-foreground/50', label: 'draft' },
  closed: { Icon: GitPullRequestClosed, tone: 'text-muted-foreground/50', label: 'closed without merging' },
};

function agentName(kind: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code', codex: 'Codex', cursor: 'Cursor', grok: 'Grok',
    opencode: 'OpenCode', antigravity: 'Antigravity',
  };
  return known[kind] ?? kind;
}

/** The right-side status affordance. Priority: waiting on you > unread >
 *  running (spinner); idle rows have no badge. Where the work landed
 *  (branch, PR, merged) is the git mark beside the title, not this. */
function SessionStatus({ running, unread, waiting = false, failed = false }: { running: boolean; unread: boolean; waiting?: boolean; failed?: boolean }) {
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
  if (failed && !running) {
    return <CircleAlert className="size-3.5 text-red-600 dark:text-red-400" aria-label="needs a retry" />;
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
  return null;
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
        active ? 'sidebar-active bg-fg/[0.1] text-foreground' : 'text-foreground/70 hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        className="grid min-w-0 grid-cols-[16px_minmax(0,1fr)_16px_24px] items-center gap-2 text-left"
        title="Open"
      >
        <EngineBadge session={session} />
        <span className="truncate text-[13px] leading-4 font-medium">
          {sessionLabel(session)}
        </span>
        <span className="flex justify-center"><GitMark session={session} /></span>
        <span className="text-right text-[10.5px] tabular-nums text-muted-foreground/50">
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

function loadSet(key: string): Set<string> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) || '[]');
    return new Set(Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : []);
  } catch { return new Set(); }
}
function saveSet(key: string, value: Set<string>) {
  try { localStorage.setItem(key, JSON.stringify([...value])); } catch { /* optional storage */ }
}
function loadScroll(): number {
  try { return Math.max(0, Number(localStorage.getItem(SCROLL_KEY)) || 0); } catch { return 0; }
}

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

function loadFlag(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : v === '1';
  } catch { return fallback; }
}

function saveFlag(key: string, value: boolean) {
  try { localStorage.setItem(key, value ? '1' : '0'); }
  catch { /* private-mode etc; ignore */ }
}

/** A collapsible shelf's heading: Recents, Settled, Archived. */
function ShelfHeader({
  icon, label, count, open, onToggle,
}: {
  icon: React.ReactNode;
  label: string;
  count?: number;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80 transition-colors hover:bg-fg/[0.05] hover:text-foreground"
    >
      {open ? (
        <ChevronDown className="size-3 shrink-0 text-muted-foreground/60" />
      ) : (
        <ChevronRight className="size-3 shrink-0 text-muted-foreground/60" />
      )}
      {icon}
      {label}
      {count != null && count > 0 && (
        <span className="rounded-full bg-secondary/80 px-1.5 text-[10.5px] font-medium normal-case tracking-normal text-muted-foreground">
          {count}
        </span>
      )}
    </button>
  );
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
            ? 'sidebar-active bg-fg/[0.1] text-foreground'
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
  description?: string;
  children?: RowMenuItem[];
  onSelect?: () => void | Promise<void>;
};

/** RowMenu items for a session row. Renders rename + pin/archive (when
 *  wired) + delete plus, when the caller wired a background-mode handler,
 *  a submenu of three choices for the current per-slot policy. The three
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
  settled?: boolean;
  onSettle?: (settled: boolean) => void;
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
  if (args.onSettle) {
    items.push({
      label: args.settled ? 'Move back to project' : 'Settle',
      icon: args.settled ? <ArchiveRestore className="size-3.5" /> : <CheckCheck className="size-3.5" />,
      onSelect: () => args.onSettle!(!args.settled),
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
      { mode: 'deny', label: 'Auto-deny', desc: 'Deny tool approvals while you are away.' },
      { mode: 'auto_approve', label: 'Auto-approve', desc: 'Allow tool approvals while you are away.' },
      { mode: 'park', label: 'Wait for me', desc: 'Hold tool approvals until you return.' },
    ];
    items.push({
      label: 'Background mode',
      icon: <Timer className="size-3.5" />,
      children: modes.map((m) => ({
        label: m.label,
        description: m.desc,
        checked: current === m.mode,
        onSelect: () => args.onSetBackgroundMode!(m.mode),
      })),
    });
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

function RowSubmenu({ item, onPick }: { item: RowMenuItem; onPick: (item: RowMenuItem) => void }) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button ref={trigger} type="button" onKeyDown={(e) => {
          if (e.key === 'ArrowRight') { e.preventDefault(); setOpen(true); }
        }} className="flex items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] text-foreground transition-colors hover:bg-fg/[0.05]">
          <span className="shrink-0 text-muted-foreground">{item.icon}</span>
          <span className="flex-1">{item.label}</span>
          <ChevronRight className="size-3.5 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent side="right" align="start" sideOffset={8} className="sidebar-menu w-60 p-1"
        aria-label={item.label} onClick={(e) => e.stopPropagation()} onKeyDown={(e) => {
          if (e.key === 'ArrowLeft') {
            e.preventDefault(); e.stopPropagation(); setOpen(false); trigger.current?.focus();
          }
        }}>
        <div className="px-2 pb-1 pt-1.5 text-[10.5px] font-medium text-muted-foreground">When you leave this chat</div>
        {item.children?.map((child) => (
          <button type="button" key={child.label} aria-pressed={child.checked}
            onClick={() => { setOpen(false); onPick(child); }}
            className="flex w-full items-start gap-2 rounded-lg px-2 py-2 text-left transition-colors hover:bg-fg/[0.05]">
            <span className="min-w-0 flex-1">
              <span className="block text-[12.5px] font-medium text-foreground">{child.label}</span>
              <span className="mt-0.5 block text-[10.5px] leading-4 text-muted-foreground">{child.description}</span>
            </span>
            {child.checked && <CircleCheck className="mt-0.5 size-3.5 shrink-0 text-mira-blue" />}
          </button>
        ))}
      </PopoverContent>
    </Popover>
  );
}

function RowMenu({ items }: { items: RowMenuItem[] }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState<RowMenuItem | null>(null);

  function pick(item: RowMenuItem) {
    if (item.confirm) {
      setConfirming(item);
    } else {
      setOpen(false);
      void item.onSelect?.();
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
            'shrink-0 rounded-sm p-0.5 text-muted-foreground/60 transition-opacity hover:bg-fg/[0.05] hover:text-foreground touch:p-1.5',
            open ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus:opacity-100',
          )}
        >
          <Ellipsis className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        className="sidebar-menu w-56 p-1"
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
                  await item.onSelect?.();
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
            {items.map((it, i) => it.children ? (
              <RowSubmenu key={it.label} item={it} onPick={pick} />
            ) : (
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
