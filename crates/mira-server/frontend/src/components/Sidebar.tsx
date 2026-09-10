import { useEffect, useMemo, useState } from 'react';
import {
  CaretDown,
  CaretRight,
  CheckCircle,
  Circle,
  CircleNotch,
  DotsThree,
  Folder,
  GitBranch,
  GitMerge,
  IconContext,
  NotePencil,
  PencilSimple,
  PuzzlePiece,
  Sparkle,
  SlidersHorizontal,
  Timer,
  Trash,
} from '@phosphor-icons/react';
import {
  deleteSession,
  listSessions,
  loadSession,
  regenerateSessionTitle,
  renameSession,
} from '../api';
import type { SessionSummary, WorktreeMergeStatus } from '../types';
import type { WsStatus } from '../ws';
import { Dialog, DialogContent } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

/** Primary view rendered in the main pane. Sidebar nav items switch the
 *  active view; the App owns the state and hides the chat composer /
 *  transcript when a non-chat view is selected. */
export type MainView = 'chat' | 'plugins' | 'pull-request' | 'scheduled';

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
};

const COLLAPSED_KEY = 'mira.sidebar.collapsed-projects';
const PER_GROUP_LIMIT = 5;

export function Sidebar({
  status, cwd, activeSessionId, activeBusy, refreshKey, activeView, onNavigate,
  onNewChat, onOpenSettings, onOpenPicker, onSessionLoaded,
}: Props) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadCollapsed());
  const [showMore, setShowMore] = useState<Set<string>>(() => new Set());
  const [renaming, setRenaming] = useState<SessionSummary | null>(null);

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
    // Match the composer: every icon in the sidebar renders in Phosphor's
    // `fill` weight for a chunkier, more Codex-like look. Nested contexts
    // don't leak out — the root of the app stays on its own default.
    <IconContext.Provider value={{ weight: 'fill', size: '1em', mirrored: false }}>
    <aside className="flex h-full min-w-0 flex-col border-r border-border bg-card">
      <div className="flex items-center justify-between px-3 pt-3.5 pb-2">
        <span className="text-[15px] font-semibold tracking-tight">Mira</span>
      </div>

      <div className="flex-1 overflow-y-auto px-1.5 pb-2">
        <nav className="flex flex-col gap-0.5 px-0.5">
          <NavItem icon={<NotePencil className="size-3.5" />} onClick={onNewChat}>
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
            icon={<PuzzlePiece className="size-3.5" />}
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

        <div className="mt-4 flex flex-col gap-0.5 px-0.5">
          <div className="px-2.5 py-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
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
                  className="group flex items-center gap-1.5 rounded-md px-2 py-1 text-[14.5px] font-semibold text-foreground/90 transition-colors hover:bg-accent/50 hover:text-foreground"
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
                      <SessionRow
                        key={s.id}
                        session={s}
                        active={s.id === activeSessionId}
                        activeBusy={activeBusy}
                        onPick={() => pickSession(s.id)}
                        onRename={() => setRenaming(s)}
                        onDelete={() => removeSession(s.id)}
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
        </div>

        <div className="mt-3 flex flex-col gap-0.5 px-0.5">
          <div className="px-2.5 py-1 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
            Folder
          </div>
          <button
            onClick={onOpenPicker}
            title={cwd || 'Choose a folder'}
            className="group grid w-full grid-cols-[1fr_auto] items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[14px] text-foreground/90 transition-colors hover:bg-accent hover:text-foreground"
          >
            <span className="flex min-w-0 items-center gap-2">
              <Folder className="size-3.5 shrink-0 text-muted-foreground/70" />
              <span className="truncate">{shortenPath(cwd) || 'Choose folder…'}</span>
            </span>
            <span className="text-[12px] text-muted-foreground/70 group-hover:text-muted-foreground">change</span>
          </button>
        </div>
      </div>

      <div className="flex items-center gap-2 border-t border-border px-3 py-2.5">
        <span className={cn('size-1.5 shrink-0 rounded-full', dotColor(status))} />
        <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground/85" title={cwd}>
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
    <RenameDialog
      session={renaming}
      onClose={() => setRenaming(null)}
      onManual={applyManualRename}
      onAi={applyAiRename}
    />
    </IconContext.Provider>
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
}: {
  session: SessionSummary;
  active: boolean;
  activeBusy: boolean;
  onPick: () => void;
  onRename: () => void;
  onDelete: () => void;
}) {
  const providerDot = providerFamilyDot(session.model);
  const running = active && activeBusy;
  return (
    <div
      // Hover tooltip prefers the ORIGINAL first message so the user can
      // still see what the chat started with even when the nickname has
      // replaced the row label.
      title={session.title ?? session.first_user_message ?? session.id}
      className={cn(
        'group grid w-full grid-cols-[1fr_auto] items-start gap-1.5 rounded-md px-2 py-1.5 transition-colors',
        active
          ? 'bg-accent/60 text-foreground'
          : 'text-foreground/90 hover:bg-accent/40 hover:text-foreground',
      )}
    >
      <button
        type="button"
        onClick={onPick}
        className="flex min-w-0 flex-col items-start gap-0.5 text-left"
      >
        <span
          className={cn(
            'w-full truncate text-[14px] leading-tight',
            active ? 'font-semibold text-foreground' : 'font-medium text-foreground/90',
          )}
        >
          {session.title ?? session.first_user_message ?? 'Untitled'}
        </span>
        <SublineMarquee session={session} providerDot={providerDot} />
      </button>
      {/* Single far-right slot. Status circle sits underneath the row
       *  menu — both share the same absolute box so the layout never
       *  shifts when the menu appears on hover. Space is reserved even
       *  when the menu is hidden. */}
      <div className="relative flex size-5 items-center justify-center pt-0.5">
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
            items={[
              {
                label: 'Rename session',
                icon: <PencilSimple className="size-3.5" />,
                onSelect: onRename,
              },
              {
                label: 'Delete session',
                danger: true,
                confirm: 'Delete this session? This cannot be undone.',
                icon: <Trash className="size-3.5" />,
                onSelect: onDelete,
              },
            ]}
          />
        </span>
      </div>
    </div>
  );
}

/** Hover-triggered marquee for the subline. At rest no animation is
 *  applied, so the track sits at `translateX(0)` and the row reads as a
 *  normal truncated subline. On row hover the animation kicks in,
 *  translating the track leftward by exactly 50% — which equals the
 *  width of the FIRST copy, so the SECOND copy scrolls in seamlessly
 *  for a continuous loop with no visible seam. When the cursor leaves
 *  the animation is removed entirely and the track snaps back to 0
 *  (rather than pausing mid-scroll, which felt jerky on re-hover).
 *
 *  The 22px pad between copies acts as a visual gap so the loop reads
 *  as "and here it comes again" rather than one long banner. */
function SublineMarquee({
  session,
  providerDot,
}: {
  session: SessionSummary;
  providerDot: { family: string; color: string } | null;
}) {
  return (
    <div className="w-full overflow-hidden">
      <div
        className={cn(
          'inline-flex items-center whitespace-nowrap will-change-transform',
          'group-hover:[animation:marquee_10s_linear_infinite]',
        )}
      >
        <SublineContent session={session} providerDot={providerDot} />
        {/* Second copy is hidden at rest so a subline that fits the row
         *  doesn't visibly repeat ("Qwen3.8-27b … Qwen3.8-27b"). It flips
         *  to visible on hover, right as the animation begins translating
         *  the track — the copy slides in from the right instead of
         *  popping up in place. */}
        <SublineContent
          session={session}
          providerDot={providerDot}
          ariaHidden
          className="invisible group-hover:visible"
        />
      </div>
    </div>
  );
}

function SublineContent({
  session,
  providerDot,
  ariaHidden,
  className,
}: {
  session: SessionSummary;
  providerDot: { family: string; color: string } | null;
  ariaHidden?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        'flex shrink-0 items-center gap-1.5 pr-[22px] text-[11.5px] leading-tight text-muted-foreground/80',
        className,
      )}
      aria-hidden={ariaHidden ? true : undefined}
    >
      <span className="font-mono tracking-tight">{shortModelLabel(session.model)}</span>
      <span className="text-muted-foreground/50">·</span>
      <span>{timeAgo(session.updated_at)}</span>
      {session.worktree_branch && (
        <>
          <span className="text-muted-foreground/50">·</span>
          <InlineBranchBadge status={session.worktree_status} branch={session.worktree_branch} />
        </>
      )}
      {providerDot && (
        <span
          className={cn('ml-1 inline-block size-1.5 shrink-0 rounded-full', providerDot.color)}
          title={providerDot.family}
        />
      )}
    </div>
  );
}

/** The right-side status affordance. Priority: running (spinner) > merged
 *  (green check) > idle (empty circle outline). Matches Codex's row-status
 *  ring — quiet by default, expressive when there's a state worth noting. */
function SessionStatus({ running, merged }: { running: boolean; merged: boolean }) {
  if (running) {
    return (
      <span
        className="inline-flex size-4 items-center justify-center"
        title="Streaming"
        aria-label="working"
      >
        <CircleNotch className="size-3.5 animate-spin text-mira-blue" />
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
        <CheckCircle className="size-4" weight="fill" />
      </span>
    );
  }
  return (
    <span
      className="inline-flex size-4 items-center justify-center text-muted-foreground/40"
      aria-label="idle"
    >
      <Circle className="size-3.5" weight="regular" />
    </span>
  );
}

/** Inline branch chip for the subline. Just a git-branch icon (or
 *  git-merge when merged) followed by the short branch name. No border,
 *  no chip — it reads as a metadata pair, not a badge. */
function InlineBranchBadge({
  status,
  branch,
}: {
  status: WorktreeMergeStatus | null | undefined;
  branch: string;
}) {
  const merged = status === 'merged';
  const Icon = merged ? GitMerge : GitBranch;
  const short = branch.length > 20 ? branch.slice(0, 18) + '…' : branch;
  return (
    <span className={cn('inline-flex items-center gap-0.5', merged ? 'text-mira-purple' : 'text-muted-foreground/80')}>
      <Icon className="size-3" weight={merged ? 'fill' : 'regular'} />
      <span className="font-mono">{short}</span>
    </span>
  );
}

/** Provider-family dot only (no name). Reused inline in the subline so
 *  the dot cluster with `time · branch · dot` reads as one metadata row. */
function providerFamilyDot(model: string): { family: string; color: string } | null {
  if (!model) return null;
  return providerFamily(model);
}

/** Short, Title-cased model label for the subline. Drops any provider
 *  prefix (`openrouter/anthropic/…`), strips date/`latest` suffixes, and
 *  capitalises the first letter so `qwen3.8-27b` reads as `Qwen3.8-27b`.
 *  Falls back to a neutral placeholder when the session has no model
 *  recorded (very old sessions, freshly-created rows). */
function shortModelLabel(model: string | null | undefined): string {
  if (!model) return 'Model';
  let m = model;
  const lastSlash = m.lastIndexOf('/');
  if (lastSlash >= 0) m = m.slice(lastSlash + 1);
  m = m.replace(/-\d{4}-\d{2}-\d{2}$/, '');
  m = m.replace(/-latest$/, '');
  // Titlecase the first char without touching casing anywhere else — model
  // ids like `gpt-4o` and `qwen3.8-27b` have meaningful mixed case beyond
  // the first character (the `o` in `4o` is intentional).
  if (m.length > 0) m = m[0].toUpperCase() + m.slice(1);
  if (m.length > 20) m = m.slice(0, 19) + '…';
  return m;
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
    setValue(session.title ?? session.first_user_message ?? '');
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
              <Sparkle className="size-3.5" />
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
        'flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[14.5px] transition-colors',
        disabled
          ? 'text-muted-foreground/40 cursor-not-allowed'
          : active
            ? 'bg-accent text-foreground'
            : 'text-foreground hover:bg-accent',
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

/* ---------- provider family (dot color) ---------- */

/** Prefix-match model id → provider family. Longer prefixes first so
 *  `gpt-4o-mini` doesn't collide with `gpt-4o`. Colors picked to be
 *  distinguishable on a dark background without shouting. */
function providerFamily(model: string): { family: string; color: string } {
  const m = model.toLowerCase();
  const table: [RegExp, string, string][] = [
    [/(^|\/)o1(-|$)/,           'openai',    'bg-emerald-500'],
    [/(^|\/)o3(-|$)/,           'openai',    'bg-emerald-500'],
    [/(^|\/)gpt-/,              'openai',    'bg-emerald-500'],
    [/claude/,                  'anthropic', 'bg-orange-500'],
    [/gemini/,                  'google',    'bg-amber-400'],
    [/deepseek/,                'deepseek',  'bg-mira-blue'],
    [/llama/,                   'meta',      'bg-mira-purple'],
    [/qwen/,                    'alibaba',   'bg-pink-500'],
    [/mistral|mixtral/,         'mistral',   'bg-orange-400'],
    [/grok|xai/,                'xai',       'bg-slate-300'],
    [/moonshot|kimi/,           'moonshot',  'bg-fuchsia-400'],
    [/phi/,                     'microsoft', 'bg-cyan-400'],
  ];
  for (const [rx, family, color] of table) {
    if (rx.test(m)) return { family, color };
  }
  return { family: 'other', color: 'bg-muted-foreground/60' };
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
