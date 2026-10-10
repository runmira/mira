import { openExternal } from '@/lib/desktop';
import { cn } from '@/lib/utils';
import { m, useReducedMotion } from 'framer-motion';
import {
  ArchiveRestore,
  Circle,
  CircleAlert,
  CircleCheck,
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
  Loader,
  Split,
  Trash2,
} from 'lucide-react';
import { PREF_KEYS, useBoolPref } from '../../lib/prefs';
import { rowStatus } from '../../lib/sidebar';
import type { BackgroundMode, SessionPr, SessionSummary } from '../../types';
import { AgentIcon, ModelIcon } from '../AgentIcon';
import { parseSentAttachments } from '../composer/attachments';
import { basename } from './groups';
import { backgroundMenuItems, RowMenu } from './RowMenu';
import { ForkNode } from './types';
/** Human-readable session label. Strips the `## Attached files … ` block
 *  from `first_user_message` so an image-only first turn doesn't read as
 *  a wall of markdown; falls back to a filename summary when the user
 *  sent nothing but attachments. */
export function sessionLabel(s: SessionSummary): string {
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

export function forkForest(sessions: SessionSummary[]): ForkNode[] {
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
export function forkLabel(s: SessionSummary): string {
  const label = sessionLabel(s);
  if (s.forked_at && /\(fork\)$/.test(label)) return s.forked_at;
  return label.replace(/\s*\(fork\)$/, '');
}

export function SessionRow({
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
    <m.div
      layout="position"
      transition={{ layout: { duration: systemReducedMotion || preferReducedMotion ? 0 : 0.16 } }}
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
      onFocus={(e) => {
        if (!selecting && e.target === e.currentTarget)
          onPeek?.(e.currentTarget.getBoundingClientRect());
      }}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) onPeek?.(null);
      }}
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
          : // Same active treatment as the nav items (New thread / PR /
            // Plugins / Scheduled): 10% white band, full foreground.
            active
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
          <Split
            className="size-3.5 shrink-0 rotate-90 justify-self-center text-mira-blue/70"
            aria-label="Fork"
          />
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
              <span
                className={cn(
                  detail.tone === 'attention' ? 'min-w-0 truncate' : 'shrink-0',
                  detail.tone === 'attention'
                    ? 'text-amber-700 dark:text-amber-400'
                    : detail.tone === 'error'
                      ? 'text-red-600 dark:text-red-400'
                      : detail.tone === 'working' || detail.tone === 'result'
                        ? 'text-mira-blue'
                        : 'text-muted-foreground/75',
                )}
              >
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
export function EngineBadge({ session }: { session: SessionSummary }) {
  if (session.agent_driver) {
    return (
      <span title={`Runs on ${agentName(session.agent_driver)}`} className="flex shrink-0">
        <AgentIcon
          kind={session.agent_driver}
          name={agentName(session.agent_driver)}
          size="xs"
          tile={false}
        />
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
export function GitMark({ session }: { session: SessionSummary }) {
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
        className={cn(
          'flex shrink-0 cursor-pointer rounded-sm transition-opacity hover:opacity-80',
          tone,
        )}
      >
        <Icon className="size-3.5" />
      </span>
    );
  }
  if (!session.worktree_branch) return null;
  const merged = session.worktree_status === 'merged';
  return (
    <span
      className={cn(
        'flex shrink-0',
        merged ? 'text-violet-500 dark:text-violet-400' : 'text-muted-foreground/60',
      )}
      title={
        merged
          ? `Branch ${session.worktree_branch}, merged into main`
          : `Worktree branch ${session.worktree_branch}`
      }
    >
      {merged ? (
        <GitMerge className="size-3.5" aria-label="Branch merged" />
      ) : (
        <GitBranch className="size-3" aria-label={`Branch ${session.worktree_branch}`} />
      )}
    </span>
  );
}

export const PR_MARK: Record<
  SessionPr['state'],
  { Icon: typeof GitMerge; tone: string; label: string }
> = {
  merged: { Icon: GitMerge, tone: 'text-violet-500 dark:text-violet-400', label: 'merged' },
  open: { Icon: GitPullRequest, tone: 'text-muted-foreground/70', label: 'open' },
  draft: { Icon: GitPullRequestDraft, tone: 'text-muted-foreground/50', label: 'draft' },
  closed: {
    Icon: GitPullRequestClosed,
    tone: 'text-muted-foreground/50',
    label: 'closed without merging',
  },
};

export function agentName(kind: string): string {
  const known: Record<string, string> = {
    'claude-code': 'Claude Code',
    codex: 'Codex',
    cursor: 'Cursor',
    grok: 'Grok',
    opencode: 'OpenCode',
    antigravity: 'Antigravity',
  };
  return known[kind] ?? kind;
}

/** The right-side status affordance. Priority: waiting on you > unread >
 *  running (spinner); idle rows have no badge. Where the work landed
 *  (branch, PR, merged) is the git mark beside the title, not this. */
export function SessionStatus({
  running,
  unread,
  waiting = false,
  failed = false,
}: {
  running: boolean;
  unread: boolean;
  waiting?: boolean;
  failed?: boolean;
}) {
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
    return (
      <CircleAlert className="size-3.5 text-red-600 dark:text-red-400" aria-label="needs a retry" />
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
  return null;
}

/* ---------- archived row + bulk delete confirm ---------- */

/** One archived session. Clicking the row opens it where it is; Restore
 *  puts it back in its folder; Delete is permanent and confirms. */
export function ArchivedRow({
  session,
  active,
  onOpen,
  onRestore,
  onDelete,
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
        active
          ? 'sidebar-active bg-fg/[0.1] text-foreground'
          : 'text-foreground/70 hover:bg-fg/[0.05] hover:text-foreground',
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        className="grid min-w-0 grid-cols-[16px_minmax(0,1fr)_16px_24px] items-center gap-2 text-left"
        title="Open"
      >
        <EngineBadge session={session} />
        <span className="truncate text-[13px] leading-4 font-medium">{sessionLabel(session)}</span>
        <span className="flex justify-center">
          <GitMark session={session} />
        </span>
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
            {
              label: 'Restore',
              icon: <ArchiveRestore className="size-3.5" />,
              onSelect: onRestore,
            },
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

export function timeAgo(unixSecs: number): string {
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
