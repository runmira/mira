import { useEffect, useMemo, useState } from 'react';
import {
  CaretDown,
  CaretRight,
  DotsThree,
  Folder,
  NotePencil,
  PuzzlePiece,
  SlidersHorizontal,
  Timer,
  Trash,
} from '@phosphor-icons/react';
import { deleteSession, listSessions, loadSession } from '../api';
import type { SessionSummary } from '../types';
import type { WsStatus } from '../ws';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

type Props = {
  status: WsStatus;
  cwd: string;
  activeSessionId: string;
  refreshKey: number;
  onNewChat: () => void;
  onOpenSettings: () => void;
  onOpenPicker: () => void;
  onSessionLoaded: () => void;
};

const COLLAPSED_KEY = 'mira.sidebar.collapsed-projects';
const PER_GROUP_LIMIT = 5;

export function Sidebar({
  status, cwd, activeSessionId, refreshKey, onNewChat, onOpenSettings, onOpenPicker, onSessionLoaded,
}: Props) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadCollapsed());
  const [showMore, setShowMore] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    // Fetch all sessions across every folder — grouped in-memory below.
    listSessions({ all: true })
      .then((s) => { setSessions(s); setError(null); })
      .catch((e) => setError(String(e.message ?? e)));
  }, [refreshKey]);

  const groups = useMemo(() => groupByCwd(sessions, cwd), [sessions, cwd]);

  async function pickSession(id: string) {
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
    <aside className="flex h-full min-w-0 flex-col border-r border-border bg-card">
      <div className="flex items-center justify-between px-3 pt-3.5 pb-2">
        <span className="text-[15px] font-semibold tracking-tight">Mira</span>
      </div>

      <div className="flex-1 overflow-y-auto px-1.5 pb-2">
        <nav className="flex flex-col gap-0.5 px-0.5">
          <NavItem icon={<NotePencil className="size-3.5" />} onClick={onNewChat}>
            New chat
          </NavItem>
          <NavItem icon={<PuzzlePiece className="size-3.5" />} disabled>
            Plugins
          </NavItem>
          <NavItem icon={<Timer className="size-3.5" />} disabled>
            Scheduled
          </NavItem>
        </nav>

        <div className="mt-4 flex flex-col gap-0.5 px-0.5">
          <div className="px-2.5 py-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
            Projects
          </div>
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
                  className="group flex items-center gap-1.5 rounded-md px-2 py-1 text-[13px] text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground"
                  title={g.cwd}
                >
                  <button
                    type="button"
                    onClick={() => toggleCollapsed(g.cwd)}
                    className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                  >
                    {isCollapsed ? (
                      <CaretRight className="size-3 shrink-0 text-muted-foreground/60" />
                    ) : (
                      <CaretDown className="size-3 shrink-0 text-muted-foreground/60" />
                    )}
                    <Folder className={cn('size-3.5 shrink-0', g.isCurrent ? 'text-mira-blue' : 'text-muted-foreground/70')} />
                    <span className={cn('truncate', g.isCurrent && 'text-foreground')}>{g.label}</span>
                  </button>
                  <RowMenu
                    items={[
                      {
                        label: 'Delete all sessions',
                        danger: true,
                        confirm: `Delete all ${g.sessions.length} session${g.sessions.length === 1 ? '' : 's'} in ${g.label}?`,
                        icon: <Trash className="size-3.5" />,
                        onSelect: () => removeAllInProject(g.cwd),
                      },
                    ]}
                  />
                </div>

                {!isCollapsed && (
                  <div className="ml-3.5 flex flex-col gap-0.5 border-l border-border/40 pl-1.5">
                    {visible.map((s) => (
                      <div
                        key={s.id}
                        // Hover tooltip prefers the ORIGINAL first message so
                        // the user can still see what the chat started with
                        // even when the nickname has replaced the row label.
                        title={s.first_user_message ?? s.title ?? s.id}
                        className={cn(
                          'group grid w-full grid-cols-[1fr_auto_auto] items-center gap-1 rounded-md px-2 py-1.5 text-[12.5px] transition-colors',
                          s.id === activeSessionId
                            ? 'bg-accent text-foreground'
                            : 'text-muted-foreground hover:bg-accent/60 hover:text-foreground',
                        )}
                      >
                        <button
                          type="button"
                          onClick={() => pickSession(s.id)}
                          className="min-w-0 truncate text-left"
                        >
                          {s.first_user_message ?? 'Untitled'}
                        </button>
                        <span className="text-[10.5px] text-muted-foreground/70 group-hover:opacity-0 transition-opacity">
                          {timeAgo(s.updated_at)}
                        </span>
                        <RowMenu
                          items={[
                            {
                              label: 'Delete session',
                              danger: true,
                              confirm: 'Delete this session? This cannot be undone.',
                              icon: <Trash className="size-3.5" />,
                              onSelect: () => removeSession(s.id),
                            },
                          ]}
                        />
                      </div>
                    ))}
                    {overflow > 0 && (
                      <button
                        onClick={() => toggleShowMore(g.cwd)}
                        className="px-2 py-1 text-left text-[11.5px] text-muted-foreground/70 transition-colors hover:text-foreground"
                      >
                        Show {overflow} more
                      </button>
                    )}
                    {expandedAll && g.sessions.length > PER_GROUP_LIMIT && (
                      <button
                        onClick={() => toggleShowMore(g.cwd)}
                        className="px-2 py-1 text-left text-[11.5px] text-muted-foreground/70 transition-colors hover:text-foreground"
                      >
                        Show less
                      </button>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>

        <div className="mt-3 flex flex-col gap-0.5 px-0.5">
          <div className="px-2.5 py-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
            Folder
          </div>
          <button
            onClick={onOpenPicker}
            title={cwd || 'Choose a folder'}
            className="group grid w-full grid-cols-[1fr_auto] items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <span className="flex min-w-0 items-center gap-2">
              <Folder className="size-3.5 shrink-0 text-muted-foreground/70" />
              <span className="truncate">{shortenPath(cwd) || 'Choose folder…'}</span>
            </span>
            <span className="text-[11px] text-muted-foreground/60 group-hover:text-muted-foreground">change</span>
          </button>
        </div>
      </div>

      <div className="flex items-center gap-2 border-t border-border px-3 py-2.5">
        <span className={cn('size-1.5 shrink-0 rounded-full', dotColor(status))} />
        <span className="min-w-0 flex-1 truncate text-[11.5px] text-muted-foreground" title={cwd}>
          {shortenPath(cwd) || 'connecting…'}
        </span>
        <button
          onClick={onOpenSettings}
          title="Settings"
          className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <SlidersHorizontal className="size-4" />
        </button>
      </div>
    </aside>
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

/* ---------- little helpers ---------- */

function NavItem({
  icon, disabled, onClick, children,
}: {
  icon: React.ReactNode;
  disabled?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      title={disabled ? 'Not implemented yet' : undefined}
      className={cn(
        'flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13.5px] transition-colors',
        disabled
          ? 'text-muted-foreground/40 cursor-not-allowed'
          : 'text-foreground hover:bg-accent',
      )}
    >
      <span className={cn('shrink-0', disabled ? 'text-muted-foreground/40' : 'text-muted-foreground')}>
        {icon}
      </span>
      <span>{children}</span>
    </button>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <div className="px-2.5 py-1 text-[12px] text-muted-foreground/50">{children}</div>;
}

function dotColor(s: WsStatus): string {
  switch (s) {
    case 'open':       return 'bg-emerald-500';
    case 'connecting': return 'bg-amber-500';
    case 'closed':     return 'bg-rose-500';
  }
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

function shortenPath(p: string): string {
  if (!p) return '';
  const parts = p.split('/');
  if (parts.length <= 3) return p;
  return '…/' + parts.slice(-2).join('/');
}

/* ---------- row overflow menu (extensible) ---------- */

type RowMenuItem = {
  label: string;
  icon?: React.ReactNode;
  danger?: boolean;
  /** Confirmation prompt shown before running `onSelect`. Skips confirm if null. */
  confirm?: string | null;
  onSelect: () => void | Promise<void>;
};

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
            'shrink-0 rounded-sm p-0.5 text-muted-foreground/60 transition-opacity hover:bg-accent/60 hover:text-foreground',
            open ? 'opacity-100' : 'opacity-0 group-hover:opacity-100 focus:opacity-100',
          )}
        >
          <DotsThree className="size-3.5" />
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
                className="rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:bg-accent"
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
                  'flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors',
                  it.danger
                    ? 'text-destructive hover:bg-destructive/10'
                    : 'text-foreground hover:bg-accent/60',
                )}
              >
                {it.icon && <span className="shrink-0 text-muted-foreground">{it.icon}</span>}
                <span>{it.label}</span>
              </button>
            ))}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
