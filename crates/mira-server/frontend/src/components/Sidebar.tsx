import { useEffect, useMemo, useState } from 'react';
import {
  Archive,
  ArchiveRestore,
  ArrowLeft,
  ChevronDown,
  ChevronRight,
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
  Sparkles,
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
import type { BackgroundMode, SessionSummary } from '../types';
import type { WsStatus } from '../ws';
import { parseSentAttachments } from './Composer';
import { costUsd, formatDollars } from '../lib/usage';
import { SETTINGS_SECTIONS, type SettingsSectionId } from './Settings';

import { UserCard } from './UserCard';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { hasHiddenTitleBar, TRAFFIC_LIGHT_INSET } from '@/lib/desktop';
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
  onSetBackgroundMode,
  settingsSection = 'provider',
  onSettingsSectionChange,
  onExitSettings,
}: Props) {
  const hiddenTitleBar = hasHiddenTitleBar();
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadCollapsed());
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

  useEffect(() => {
    // Fetch all non-archived sessions across every folder — grouped
    // in-memory below. The server excludes archived ones here; they come
    // through the separate archived fetch when the user opens that view.
    listSessions({ all: true })
      .then((s) => { setSessions(s); setError(null); })
      .catch((e) => setError(String(e.message ?? e)));
  }, [refreshKey, localVersion]);

  useEffect(() => {
    if (!showArchived) return;
    listSessions({ all: true, archived: true })
      .then((s) => { setArchived(s); setError(null); })
      .catch((e) => setError(String(e.message ?? e)));
  }, [showArchived, refreshKey, localVersion]);

  const refresh = () => setLocalVersion((n) => n + 1);

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

  const groups = useMemo(() => groupByCwd(sessions, cwd), [sessions, cwd]);

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
          <div className="grid grid-cols-2 gap-0.5 rounded-full border border-border/80 bg-black/25 p-0.5 backdrop-blur-md">
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
                      ? 'bg-white/[0.1] text-foreground'
                      : 'text-foreground/80 hover:bg-white/[0.05] hover:text-foreground',
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
          >
            Pull request
          </NavItem>
          <NavItem
            icon={<Puzzle className="size-3.5" />}
            active={activeView === 'plugins'}
            onClick={() => onNavigate('plugins')}
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
                  className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-foreground/85 transition-colors hover:bg-white/[0.05] hover:text-foreground disabled:opacity-40"
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
                  className="inline-flex items-center rounded-md p-1 text-muted-foreground transition-colors hover:bg-white/[0.05] hover:text-foreground disabled:opacity-40"
                >
                  <X className="size-3.5" />
                </button>
              </div>
            </div>
          )}

          {error && <Empty>error: {error}</Empty>}
          {!error && groups.length === 0 && <Empty>No saved chats yet</Empty>}

          {groups.map((g) => {
            const isCollapsed = collapsed.has(g.cwd);
            const expandedAll = showMore.has(g.cwd);
            const visible = expandedAll ? g.sessions : g.sessions.slice(0, PER_GROUP_LIMIT);
            const overflow = g.sessions.length - visible.length;

            return (
              <div key={g.cwd} className="flex flex-col">
                <div
                  className="group flex items-center gap-1.5 rounded-md px-2 py-1 text-[14.5px] font-semibold text-foreground/90 transition-colors hover:bg-white/[0.05] hover:text-foreground"
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
                  </button>
                  {(() => {
                    const total = groupCost(g.sessions);
                    return total != null ? (
                      <span
                        className="mr-1 font-mono text-[11px] tabular-nums text-emerald-400/80"
                        title={`Total spend across ${g.sessions.length} session${g.sessions.length === 1 ? '' : 's'} in this project`}
                      >
                        {formatDollars(total)}
                      </span>
                    ) : null;
                  })()}
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

                {!isCollapsed && (
                  <div className="flex flex-col gap-0.5 pl-1">
                    {visible.map((s) => (
                      <SessionRow
                        key={s.id}
                        session={s}
                        // One selection model across the sidebar: the
                        // session highlight only shows while the chat
                        // view is actually open — picking Plugins /
                        // PR / Scheduled leaves the row highlighted
                        // otherwise (and New thread swaps in a fresh
                        // id server-side, which drops it naturally).
                        active={s.id === activeSessionId && activeView === 'chat'}
                        activeBusy={activeBusy}
                        selecting={selecting}
                        checked={selected.has(s.id)}
                        onToggleSelect={() => toggleSelected(s.id)}
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
                    ))}
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
                )}
              </div>
            );
          })}

          {/* Archived view (issue #58) — sessions hidden from the main
              list live here. Clicking a row or hitting Restore puts it
              back in its folder; Delete is permanent. */}
          {!selecting && (
            <div className="mt-4 flex flex-col gap-0.5 px-0.5">
              <button
                type="button"
                onClick={() => setShowArchived((v) => !v)}
                className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80 transition-colors hover:bg-white/[0.05] hover:text-foreground"
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
function SessionRow({
  session,
  active,
  activeBusy,
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
  active: boolean;
  activeBusy: boolean;
  onPick: () => void;
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
      title={selecting ? undefined : sessionLabel(session) || session.id}
      role={selecting ? 'checkbox' : 'button'}
      tabIndex={0}
      aria-checked={selecting ? checked : undefined}
      onClick={selecting ? onToggleSelect : onPick}
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
          ? 'bg-white/[0.1] text-foreground'
          // Same active treatment as the nav items (New thread / PR /
          // Plugins / Scheduled): 10% white band, full foreground.
          : active
            ? 'bg-white/[0.1] text-foreground'
            : 'text-foreground/90 hover:bg-white/[0.05] hover:text-foreground',
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
        {!selecting && <EngineBadge session={session} />}
        <span
          className={cn(
            'min-w-0 flex-1 truncate text-[13.5px] leading-4',
            active && !selecting ? 'font-semibold text-foreground' : 'font-medium text-foreground/90',
          )}
        >
          {sessionLabel(session)}
        </span>
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
            <SessionStatus running={running} merged={session.worktree_status === 'merged'} />
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
function SessionStatus({ running, merged }: { running: boolean; merged: boolean }) {
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

/** One archived session. Clicking the row (or Restore in the menu) puts
 *  the session back in its folder; Delete is permanent and confirms. */
function ArchivedRow({
  session, onRestore, onDelete,
}: {
  session: SessionSummary;
  onRestore: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      title="Restore to the list"
      className="group grid w-full grid-cols-[1fr_auto] items-start gap-1.5 rounded-lg px-2 py-1.5 text-foreground/70 transition-colors hover:bg-white/[0.05] hover:text-foreground"
    >
      <button type="button" onClick={onRestore} className="flex min-w-0 items-baseline gap-1.5 text-left">
        <span className="min-w-0 flex-1 truncate text-[13.5px] leading-tight font-medium">
          {sessionLabel(session)}
        </span>
        <span className="shrink-0 text-[10.5px] text-muted-foreground/50">
          {timeAgo(session.updated_at)}
        </span>
      </button>
      <div className="relative flex size-5 items-center justify-center pt-0.5">
        <span className="absolute inset-0 flex items-center justify-center opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
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
        </span>
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
  icon, disabled, active, onClick, children,
}: {
  icon: React.ReactNode;
  disabled?: boolean;
  /** True when this item's view is currently rendered in the main pane —
   *  gets the same accent treatment as an active session row. */
  active?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      title={disabled ? 'Not implemented yet' : undefined}
      className={cn(
        'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[14.5px] transition-colors',
        disabled
          ? 'text-muted-foreground/40 cursor-not-allowed'
          : active
            ? 'bg-white/[0.1] text-foreground'
            : 'text-foreground hover:bg-white/[0.05]',
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

function sessionCost(s: SessionSummary): number | null {
  if (!s.usage) return null;
  if (s.usage.prompt_tokens === 0 && s.usage.completion_tokens === 0) return null;
  return costUsd(s.model, s.usage);
}

/** Sum of known session costs in the group. Sessions on unpriced models are
 *  skipped rather than treated as $0 so the badge doesn't lie by omission —
 *  if nothing is priceable, the badge just doesn't render. */
function groupCost(sessions: SessionSummary[]): number | null {
  let total = 0;
  let any = false;
  for (const s of sessions) {
    const c = sessionCost(s);
    if (c == null) continue;
    total += c;
    any = true;
  }
  return any ? total : null;
}


function timeAgo(unixSecs: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, now - unixSecs);
  if (diff < 60) return `${diff}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  const days = Math.floor(diff / 86400);
  if (days < 7) return `${days}d`;
  return `${Math.floor(days / 7)}w`;
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
            'shrink-0 rounded-sm p-0.5 text-muted-foreground/60 transition-opacity hover:bg-white/[0.05] hover:text-foreground',
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
                className="rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-white/[0.05]"
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
                    : 'text-foreground hover:bg-white/[0.05]',
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
