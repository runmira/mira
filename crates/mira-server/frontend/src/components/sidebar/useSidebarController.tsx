import { hasHiddenTitleBar } from '@/lib/desktop';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  deleteSession,
  listSessions,
  loadSession,
  regenerateSessionTitle,
  renameSession,
  setSessionFlags,
} from '../../api';
import { useLatch } from '../../lib/lazy';
import { isSettled, settledOrder } from '../../lib/settled';
import { matchesSidebar, type SidebarFilter } from '../../lib/sidebar';
import type { BackgroundMode, SessionSummary } from '../../types';
import type { PeekTarget } from '../SessionPeek';
import { groupByCwd } from '../sidebar/groups';
import {
  FORKS_KEY,
  loadCollapsed,
  loadFlag,
  loadScroll,
  loadSet,
  loadUnread,
  NEEDS_MORE_KEY,
  persistCollapsed,
  RECENTS_LIMIT,
  RECENTS_MORE_KEY,
  RECENTS_OPEN_KEY,
  saveFlag,
  saveSet,
  saveUnread,
  SETTLED_OPEN_KEY,
  SETTLED_PAGE,
  SHOW_MORE_KEY,
} from '../sidebar/preferences';
import { SessionRow } from '../sidebar/sessionRows';
import { Props } from '../sidebar/types';
export function useSidebarController({
  cwd,
  activeSessionId,
  activeBusy,
  refreshKey,
  activeView,
  onSessionLoaded,
  onAttachSession,
  onSetBackgroundMode,
  runningSessions,
  completedSessions,
}: Props) {
  const hiddenTitleBar = hasHiddenTitleBar();

  const [storedSessions, setSessions] = useState<SessionSummary[]>([]);

  const sessions = useMemo(
    () =>
      runningSessions == null
        ? storedSessions
        : storedSessions.map((session) => ({
            ...session,
            running: runningSessions.has(session.id),
          })),
    [storedSessions, runningSessions],
  );

  const [listError, setListError] = useState<string | null>(null);

  const [archivedError, setArchivedError] = useState<string | null>(null);

  const [listLoading, setListLoading] = useState(false);

  const [archivedLoading, setArchivedLoading] = useState(false);

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

  const livePeek = useMemo(
    () =>
      peek
        ? {
            ...peek,
            session: sessions.find((s) => s.id === peek.session.id) ?? peek.session,
          }
        : null,
    [peek, sessions],
  );

  // Sidebar search: filters every folder at once, and opens them while it
  // has text so a match is never hidden behind a collapsed folder.
  const [query, setQuery] = useState('');

  const [filter, setFilter] = useState<SidebarFilter>('all');

  const searching = query.trim().length > 0 || filter !== 'all';

  const [needsMore, setNeedsMore] = useState(() => loadFlag(NEEDS_MORE_KEY, false));

  const [recentsMore, setRecentsMore] = useState(() => loadFlag(RECENTS_MORE_KEY, false));

  const scrollRef = useRef<HTMLDivElement>(null);

  const searchRef = useRef<HTMLInputElement>(null);

  const savedScroll = useRef(loadScroll());

  const scrollRestored = useRef(false);

  useEffect(() => {
    if (activeView === 'settings') {
      scrollRestored.current = false;
      return;
    }
    const node = scrollRef.current;
    if (!node || scrollRestored.current || sessions.length === 0) return;
    // Wait for collapsible sections to reach their final height.
    const id = window.setTimeout(() => {
      node.scrollTop = searching ? 0 : savedScroll.current;
      scrollRestored.current = true;
    }, 280);
    return () => window.clearTimeout(id);
  }, [sessions.length, activeView, searching]);

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
    setListLoading(true);
    listSessions({ all: true })
      .then((s) => {
        if (!cancelled) {
          setSessions(s);
          setListError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setListError(String(e.message ?? e));
      })
      .finally(() => {
        if (!cancelled) setListLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshKey, localVersion]);

  useEffect(() => {
    if (!showArchived) return;
    let cancelled = false;
    setArchivedLoading(true);
    listSessions({ all: true, archived: true })
      .then((s) => {
        if (!cancelled) {
          setArchived(s);
          setArchivedError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setArchivedError(String(e.message ?? e));
      })
      .finally(() => {
        if (!cancelled) setArchivedLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [showArchived, refreshKey, localVersion]);

  const refresh = () => setLocalVersion((n) => n + 1);

  // Background chats only report through this list, so while any of them
  // is mid-turn, look again every few seconds — that's how a spinner turns
  // into "finished" without the user having to open the chat.
  // Also how a chat that starts waiting on you gets its badge: that change
  // isn't pushed, and a chat waiting on you is mid-turn, so it's covered.
  const anyRunning = sessions.some(
    (s) => (s.running === true || s.needs_attention === true) && s.id !== activeSessionId,
  );

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

  const seenCompletions = useRef(new Map<string, number>());

  useEffect(() => {
    const next = new Set(unread);
    let changed = false;
    let newCompletion = false;
    for (const [id, revision] of completedSessions) {
      if (seenCompletions.current.get(id) === revision) continue;
      seenCompletions.current.set(id, revision);
      newCompletion = true;
      if (id !== activeSessionId && !next.has(id)) {
        next.add(id);
        changed = true;
      }
    }
    if (newCompletion) refresh();
    while (seenCompletions.current.size > 500)
      seenCompletions.current.delete(seenCompletions.current.keys().next().value!);
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
  async function flagSession(
    id: string,
    flags: { pinned?: boolean; archived?: boolean; settled?: boolean },
  ) {
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
      setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, background_mode: mode } : s)));
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  }

  function toggleSelectMode() {
    setPeek(null);
    setSelecting((v) => !v);
    setSelected(new Set());
  }

  function toggleSelected(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
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

  const currentSession = (s: SessionSummary) =>
    sessions.find((current) => current.id === s.id) ?? s;

  const needsYou = sessions
    .filter((s) => s.needs_attention || s.failure_reason)
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
    () =>
      [...live]
        .sort((a, b) => b.updated_at - a.updated_at)
        .filter((s) => !s.needs_attention && !s.failure_reason)
        .slice(0, recentsMore ? 8 : RECENTS_LIMIT),
    [live, recentsMore],
  );

  const groups = useMemo(
    () =>
      groupByCwd(
        searching ? arrangedSessions.filter((s) => matchesSidebar(s, query, filter)) : live,
        cwd,
      ),
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
    try {
      await loadSession(id);
      onSessionLoaded();
    } catch (e) {
      setError(String((e as Error).message));
    }
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
      if (next.has(key)) next.delete(key);
      else next.add(key);
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
          onSetBackgroundMode ? (mode) => void changeBackgroundMode(s.id, mode) : undefined
        }
      />
    );
  }

  function toggleShowMore(key: string) {
    setShowMore((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      saveSet(SHOW_MORE_KEY, next);
      return next;
    });
  }
  return {
    hiddenTitleBar,
    scrollRef,
    setPeek,
    scrollRestored,
    searching,
    savedScroll,
    needsYou,
    needsMore,
    flatRow,
    setNeedsMore,
    recents,
    recentsOpen,
    toggleShelf,
    setRecentsOpen,
    live,
    recentsMore,
    setRecentsMore,
    selecting,
    groups,
    toggleSelectMode,
    sessions,
    searchRef,
    query,
    setQuery,
    filter,
    setFilter,
    selected,
    bulkArchive,
    bulkBusy,
    setConfirmBulkDelete,
    listError,
    listLoading,
    refresh,
    error,
    setError,
    collapsed,
    showMore,
    toggleCollapsed,
    removeAllInProject,
    forksHidden,
    currentSession,
    toggleForks,
    unread,
    toggleSelected,
    pickSession,
    setRenaming,
    removeSession,
    flagSession,
    changeBackgroundMode,
    toggleShowMore,
    settled,
    settledOpen,
    setSettledOpen,
    settledShown,
    setSettledShown,
    archived,
    showArchived,
    setShowArchived,
    archivedError,
    archivedLoading,
    peekMounted,
    livePeek,
    peek,
    renaming,
    applyManualRename,
    applyAiRename,
    confirmBulkDelete,
    bulkDelete,
  };
}
