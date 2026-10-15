import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import {
  Bot,
  Check,
  CircleDot,
  FileCode,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  ListChecks,
  MessageSquare,
  ShieldCheck,
  Split,
  User,
} from 'lucide-react';
import { rowStatus } from '../../lib/sidebar';
import type { ReviewState, WorkItem, WorkState } from '../../lib/workItems';
import type { SessionSummary } from '../../types';
import { PageToolbar } from '../PageToolbar';
import { basename } from '../sidebar/groups';
import { EngineBadge, PR_MARK, sessionLabel, SessionStatus, timeAgo } from '../sidebar/sessionRows';
import { CiIcon } from './WorkSidebar';
import { diffTotals, type TimelineKind, type WorkDetail } from '../../lib/workDetail';
import { openExternal } from '@/lib/desktop';

const STATE_PILL: Record<WorkState, { label: string; tone: string }> = {
  'needs-you': { label: 'Needs you', tone: 'bg-amber-500/15 text-amber-700 dark:text-amber-300' },
  'in-progress': { label: 'In progress', tone: 'bg-mira-blue/12 text-mira-blue' },
  review: { label: 'In review', tone: 'bg-fg/[0.07] text-foreground/80' },
  ready: { label: 'Ready to merge', tone: 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300' },
  done: { label: 'Done', tone: 'bg-violet-500/15 text-violet-700 dark:text-violet-300' },
};

/** A work item's own page: what it's for, how far along it is, who's on
 *  it, what it changed, and whether it can merge. */
export function WorkItemPage({
  item,
  detail,
  activeSessionId,
  onOpenChat,
  onMerge,
  merging = false,
  mergeError = null,
}: {
  item: WorkItem;
  /** Undefined while it loads. */
  detail?: WorkDetail;
  activeSessionId?: string;
  onOpenChat?: (id: string) => void;
  onMerge?: () => void;
  merging?: boolean;
  mergeError?: string | null;
}) {
  const pill = STATE_PILL[item.state];
  const canMerge = item.state === 'ready' && detail?.mergeable !== false && !merging;
  return (
    <div className="flex h-full min-w-0 flex-col">
      <PageToolbar
        title={
          <span className="flex min-w-0 items-center gap-2">
            {item.meta.issue && <span className="shrink-0 font-normal text-muted-foreground">#{item.meta.issue.number}</span>}
            <span className="truncate">{item.title}</span>
          </span>
        }
      >
        <span className={cn('shrink-0 rounded-full px-2 py-0.5 text-[11.5px] font-medium', pill.tone)}>{pill.label}</span>
        {item.pr && item.pr.state !== 'merged' && item.pr.state !== 'closed' && (
          <Button size="sm" disabled={!canMerge} onClick={onMerge} title={canMerge ? undefined : mergeBlocker(item, detail, merging)} className="h-7 gap-1.5 px-3 text-[12.5px]">
            <GitMerge className="size-3.5" />
            Merge
          </Button>
        )}
      </PageToolbar>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid max-w-5xl gap-4 px-6 py-6 lg:grid-cols-[minmax(0,1fr)_320px]">
          <div className="flex min-w-0 flex-col gap-4">
            {item.reason && (
              <div className="rounded-xl border border-amber-500/30 bg-amber-500/[0.07] px-4 py-3 text-[13px] text-amber-800 dark:text-amber-200">
                <span className="font-medium">{item.reason}</span>
                {waitingChat(item) && <span className="text-amber-800/70 dark:text-amber-200/70"> in “{sessionLabel(waitingChat(item)!)}”</span>}
              </div>
            )}
            <Card icon={<CircleDot className="size-3.5" />} title={item.meta.issue ? `Goal · issue #${item.meta.issue.number}` : 'Goal'}>
              <p className="whitespace-pre-line text-[13.5px] leading-relaxed text-foreground/85">
                {detail ? (detail.goal ?? 'Nothing says what this is for yet.') : 'Loading…'}
              </p>
            </Card>
            {detail && detail.plan.length > 0 && <PlanCard plan={detail.plan} />}
            <ChatsCard sessions={item.sessions} activeSessionId={activeSessionId} onOpenChat={onOpenChat} />
            {detail && detail.files.length > 0 && <ChangesCard detail={detail} />}
          </div>
          <div className="flex min-w-0 flex-col gap-4">
            <BranchCard item={item} detail={detail} />
            {item.pr && <PrCard item={item} detail={detail} canMerge={canMerge} onMerge={onMerge} merging={merging} mergeError={mergeError} />}
            {detail && detail.timeline.length > 0 && <TimelineCard timeline={detail.timeline} />}
          </div>
        </div>
      </div>
    </div>
  );
}

function waitingChat(item: WorkItem): SessionSummary | undefined {
  return item.sessions.find((s) => s.needs_attention || s.failure_reason);
}

function mergeBlocker(item: WorkItem, detail?: WorkDetail, merging = false): string {
  if (merging) return 'Merging…';
  if (item.pr?.state === 'draft') return 'Still a draft';
  if (detail?.mergeable === false) return 'GitHub can’t merge it: resolve conflicts first';
  if (item.meta.ci === 'failing') return 'Checks are failing';
  if (item.meta.ci === 'pending') return 'Checks are still running';
  if (item.meta.review !== 'approved') return 'Waiting for an approving review';
  if (item.state === 'needs-you') return 'A chat on it is waiting on you';
  if (item.state === 'in-progress') return 'An agent is still working on it';
  return 'Not ready';
}

function Card({ icon, title, aside, children }: { icon: React.ReactNode; title: string; aside?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border border-border/70 bg-card/40 px-4 py-3">
      <header className="mb-2 flex items-center gap-2 text-[11.5px] font-semibold uppercase tracking-wider text-muted-foreground/80">
        <span className="text-muted-foreground/70">{icon}</span>
        <span className="flex-1">{title}</span>
        {aside && <span className="text-[11.5px] font-medium normal-case tracking-normal">{aside}</span>}
      </header>
      {children}
    </section>
  );
}

function PlanCard({ plan }: { plan: WorkDetail['plan'] }) {
  const done = plan.filter((p) => p.done).length;
  return (
    <Card icon={<ListChecks className="size-3.5" />} title="Plan" aside={<span className="tabular-nums text-muted-foreground">{done}/{plan.length}</span>}>
      <div className="mb-2 h-1 overflow-hidden rounded-full bg-fg/[0.07]">
        <div className="h-full rounded-full bg-mira-blue" style={{ width: `${(done / plan.length) * 100}%` }} />
      </div>
      <ul className="flex flex-col gap-1">
        {plan.map((p) => (
          <li key={p.text} className="flex items-start gap-2 text-[13px]">
            <span className={cn('mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-[4px] border', p.done ? 'border-mira-blue bg-mira-blue text-white' : 'border-fg/25')}>
              {p.done && <Check className="size-2.5" strokeWidth={3} />}
            </span>
            <span className={p.done ? 'text-muted-foreground line-through decoration-fg/20' : 'text-foreground/90'}>{p.text}</span>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function ChatsCard({ sessions, activeSessionId, onOpenChat }: { sessions: SessionSummary[]; activeSessionId?: string; onOpenChat?: (id: string) => void }) {
  return (
    <Card icon={<MessageSquare className="size-3.5" />} title="Chats and agents" aside={<span className="text-muted-foreground">{sessions.length}</span>}>
      <div className="-mx-2 flex flex-col gap-0.5">
        {sessions.map((s) => {
          const active = s.id === activeSessionId;
          const status = rowStatus(s);
          const relation = s.forked_from ? 'Fork' : s.launched_by ? 'Launched thread' : null;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => onOpenChat?.(s.id)}
              className={cn(
                'grid grid-cols-[16px_minmax(0,1fr)_auto_20px] items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors',
                active ? 'sidebar-active bg-fg/[0.1]' : 'hover:bg-fg/[0.05]',
              )}
            >
              {relation === 'Fork' ? <Split className="size-3.5 rotate-90 justify-self-center text-mira-blue/70" /> : <EngineBadge session={s} />}
              <span className="min-w-0">
                <span className={cn('block truncate text-[13px]', active ? 'font-semibold text-foreground' : 'text-foreground/90')}>{sessionLabel(s)}</span>
                <span className="block truncate text-[10.5px] text-muted-foreground/70">
                  {[relation, status.tone === 'idle' ? null : status.label, s.worktree_branch].filter(Boolean).join(' · ') || 'Idle'}
                </span>
              </span>
              <span className="text-[10.5px] tabular-nums text-muted-foreground/50">{timeAgo(s.updated_at)}</span>
              <SessionStatus running={!!s.running} unread={false} waiting={!!s.needs_attention} failed={!!s.failure_reason} />
            </button>
          );
        })}
      </div>
    </Card>
  );
}

function ChangesCard({ detail }: { detail: WorkDetail }) {
  const total = diffTotals(detail);
  const max = Math.max(...detail.files.map((f) => f.added + f.removed));
  return (
    <Card
      icon={<FileCode className="size-3.5" />}
      title={detail.filesSource === 'pr' ? 'Changes' : 'Uncommitted changes'}
      aside={
        <span className="tabular-nums">
          <span className="text-muted-foreground">{total.files} files </span>
          <span className="text-emerald-600 dark:text-emerald-400">+{total.added}</span>{' '}
          <span className="text-red-600/80 dark:text-red-400/80">−{total.removed}</span>
        </span>
      }
    >
      <ul className="flex flex-col">
        {detail.files.map((f) => (
          <li key={f.path} className="grid grid-cols-[minmax(0,1fr)_auto_64px] items-center gap-3 py-1 font-mono text-[12px]">
            <span className="truncate text-foreground/85" title={f.path}>
              <span className="text-muted-foreground/60">{f.path.slice(0, f.path.lastIndexOf('/') + 1)}</span>
              {f.path.slice(f.path.lastIndexOf('/') + 1)}
            </span>
            <span className="tabular-nums">
              <span className="text-emerald-600 dark:text-emerald-400">+{f.added}</span>{' '}
              <span className="text-red-600/80 dark:text-red-400/80">−{f.removed}</span>
            </span>
            <span className="flex h-1.5 overflow-hidden rounded-full bg-fg/[0.06]">
              <span className="bg-emerald-500/80" style={{ width: `${(f.added / max) * 100}%` }} />
              <span className="bg-red-500/70" style={{ width: `${(f.removed / max) * 100}%` }} />
            </span>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function BranchCard({ item, detail }: { item: WorkItem; detail?: WorkDetail }) {
  return (
    <Card icon={<GitBranch className="size-3.5" />} title="Branch">
      <dl className="grid grid-cols-[72px_minmax(0,1fr)] gap-x-2 gap-y-1.5 text-[12.5px]">
        <dt className="text-muted-foreground">Branch</dt>
        <dd className="truncate font-mono text-[12px] text-foreground/90">{item.branch ?? '—'}</dd>
        <dt className="text-muted-foreground">Into</dt>
        <dd className="font-mono text-[12px] text-foreground/90">
          {detail?.base ?? 'main'}
          {detail?.ahead != null && (
            <span className="ml-2 font-sans text-muted-foreground">
              {detail.ahead} ahead · {detail.behind ?? 0} behind
            </span>
          )}
        </dd>
        <dt className="text-muted-foreground">Project</dt>
        <dd className="truncate text-foreground/90">{basename(item.cwd)}</dd>
        {detail?.worktree && (
          <>
            <dt className="text-muted-foreground">Worktree</dt>
            <dd className="flex min-w-0 items-center gap-1 text-foreground/90">
              <FolderGit2 className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate font-mono text-[12px]" title={detail.worktree}>{detail.worktree.replace(/^\/Users\/[^/]+/, '~')}</span>
            </dd>
          </>
        )}
      </dl>
    </Card>
  );
}

const REVIEW: Record<ReviewState, { label: string; tone: string }> = {
  approved: { label: 'Approved', tone: 'text-emerald-600 dark:text-emerald-400' },
  'changes-requested': { label: 'Changes requested', tone: 'text-red-600 dark:text-red-400' },
  pending: { label: 'Review requested', tone: 'text-muted-foreground' },
};

function PrCard({ item, detail, canMerge, onMerge, merging, mergeError }: { item: WorkItem; detail?: WorkDetail; canMerge: boolean; onMerge?: () => void; merging: boolean; mergeError: string | null }) {
  const pr = item.pr!;
  const { Icon, tone, label } = PR_MARK[pr.state];
  const open = pr.state === 'open' || pr.state === 'draft';
  return (
    <Card icon={<GitPullRequest className="size-3.5" />} title="Pull request">
      <button type="button" onClick={() => void openExternal(pr.url)} className="mb-2 flex w-full items-start gap-2 rounded-md text-left hover:opacity-80">
        <Icon className={cn('mt-0.5 size-4 shrink-0', tone)} />
        <div className="min-w-0">
          <div className="text-[13.5px] font-medium leading-snug text-foreground">{pr.title}</div>
          <div className="text-[11.5px] text-muted-foreground">#{pr.number} · {label}</div>
        </div>
      </button>
      {!detail && <p className="text-[12px] text-muted-foreground">Loading checks and reviews…</p>}
      {detail && detail.checks.length > 0 && (
        <ul className="mb-2 flex flex-col gap-1 border-t border-border/60 pt-2">
          {detail.checks.map((c) => (
            <li key={c.name} className="flex items-center gap-2 text-[12.5px]">
              <CiIcon state={c.state} className="size-3.5 shrink-0" />
              {c.url ? (
                <button type="button" onClick={() => void openExternal(c.url!)} className="min-w-0 flex-1 truncate text-left text-foreground/85 hover:underline">{c.name}</button>
              ) : (
                <span className="min-w-0 flex-1 truncate text-foreground/85">{c.name}</span>
              )}
            </li>
          ))}
        </ul>
      )}
      {detail && detail.reviewers.length > 0 && (
        <ul className="mb-2 flex flex-col gap-1 border-t border-border/60 pt-2">
          {detail.reviewers.map((r) => (
            <li key={r.name} className="flex items-center gap-2 text-[12.5px]">
              <ShieldCheck className={cn('size-3.5 shrink-0', REVIEW[r.state].tone)} />
              <span className="min-w-0 flex-1 truncate text-foreground/85">{r.name}</span>
              <span className={cn('text-[11px]', REVIEW[r.state].tone)}>{REVIEW[r.state].label}</span>
            </li>
          ))}
        </ul>
      )}
      {open && (
        <div className="border-t border-border/60 pt-2.5">
          <Button size="sm" className="w-full gap-1.5" disabled={!canMerge} onClick={onMerge}>
            <GitMerge className="size-3.5" />
            {merging ? 'Merging…' : 'Squash and merge'}
          </Button>
          {mergeError ? (
            <p className="mt-1.5 text-center text-[11.5px] text-red-600 dark:text-red-400">{mergeError}</p>
          ) : (
            !canMerge && <p className="mt-1.5 text-center text-[11.5px] text-muted-foreground">{mergeBlocker(item, detail, merging)}</p>
          )}
        </div>
      )}
    </Card>
  );
}

const TIMELINE_ICON: Record<TimelineKind, typeof Bot> = {
  issue: CircleDot,
  chat: MessageSquare,
  agent: Bot,
  commit: GitCommitHorizontal,
  pr: GitPullRequest,
  ci: ShieldCheck,
  review: ShieldCheck,
  you: User,
};

function TimelineCard({ timeline }: { timeline: WorkDetail['timeline'] }) {
  return (
    <Card icon={<GitCommitHorizontal className="size-3.5" />} title="Activity">
      <ol className="relative flex flex-col gap-2.5 before:absolute before:bottom-1 before:left-[7px] before:top-1 before:w-px before:bg-border">
        {[...timeline].sort((a, b) => b.at - a.at).map((e) => {
          const Icon = TIMELINE_ICON[e.kind];
          return (
            <li key={`${e.at}-${e.text}`} className="relative grid grid-cols-[16px_minmax(0,1fr)_auto] items-start gap-2 text-[12.5px]">
              <span className="z-[1] flex size-4 items-center justify-center rounded-full bg-background ring-1 ring-border">
                <Icon className="size-2.5 text-muted-foreground" />
              </span>
              <span className="line-clamp-2 break-words leading-4 text-foreground/85" title={e.text}>{e.text}</span>
              <span className="text-[10.5px] leading-4 tabular-nums text-muted-foreground/60">{timeAgo(e.at)}</span>
            </li>
          );
        })}
      </ol>
    </Card>
  );
}
