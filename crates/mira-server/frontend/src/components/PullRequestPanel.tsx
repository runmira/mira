import { useEffect, useMemo, useState } from 'react';
import {
  ArrowSquareOut,
  ArrowUp,
  ArrowsOutSimple,
  CaretDown,
  CheckCircle,
  ChatCircle,
  CircleNotch,
  Clock,
  Code as CodeIcon,
  GitBranch,
  GitCommit,
  GitMerge,
  MagnifyingGlass,
  Sparkle,
  UsersThree,
  WarningCircle,
  XCircle,
} from '@phosphor-icons/react';
import {
  getPullRequest,
  getPullRequestFiles,
  listPullRequests,
  mergePullRequest,
  postPullRequestComment,
  submitPullRequestReview,
  type CommentView,
  type FileChangeView,
  type MergeMethod,
  type PullRequestDetailView,
  type PullRequestListView,
  type PullRequestSummary,
  type RepoGroup,
  type ReviewEvent,
} from '../api';
import { Markdown } from './Markdown';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

/** Codex-style pull-request panel. Two columns: PR list on the left,
 *  full detail on the right. Aggregates PRs across every project folder
 *  Mira knows about — the backend does the `git remote` parse + REST
 *  call. Auth is a `GITHUB_TOKEN` in Settings → Keys; when the token is
 *  absent the panel shows an inline nudge. */
type Filter = 'all' | 'reviewing' | 'authored';

type Selection = {
  owner: string;
  repo: string;
  number: number;
  project_label: string;
};

export function PullRequestPanel({
  onOpenSettings,
  onReviewPr,
}: {
  onOpenSettings: () => void;
  /** Kicks off `mira review` against this PR. The parent owns the
   *  ReviewPanel state — this button just fires the POST and the WS
   *  frames drive the existing side-slide. */
  onReviewPr: (owner: string, repo: string, number: number) => void;
}) {
  const [list, setList] = useState<PullRequestListView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>('all');
  const [search, setSearch] = useState('');
  const [selection, setSelection] = useState<Selection | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [detailPane, setDetailPane] = useState<'summary' | 'code'>('summary');
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    setLoading(true);
    listPullRequests()
      .then((v) => { setList(v); setError(null); })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, [refreshTick]);

  const filtered = useMemo(
    () => filterAndSearch(list, filter, search),
    [list, filter, search],
  );

  // Auto-select the first PR when the list loads.
  useEffect(() => {
    if (selection || !list) return;
    for (const g of filtered) {
      if (g.prs.length > 0) {
        const p = g.prs[0];
        setSelection({
          owner: g.owner,
          repo: g.repo,
          number: p.number,
          project_label: g.project_label,
        });
        break;
      }
    }
  }, [list, filtered, selection]);

  const needsToken =
    !loading && !error && list && list.authenticated_user === null;

  return (
    <div className={cn(
      'grid flex-1 min-h-0 min-w-0 grid-rows-1 bg-background transition-[grid-template-columns] duration-150',
      expanded
        ? 'grid-cols-[0px_minmax(0,1fr)]'
        : 'grid-cols-[minmax(320px,360px)_minmax(0,1fr)]',
    )}>
      {/* left: list */}
      <div className={cn(
        'flex min-w-0 min-h-0 flex-col border-r border-border bg-card',
        expanded && 'hidden',
      )}>
        <FilterBar filter={filter} onChange={setFilter} />
        <div className="px-3 pb-2">
          <SearchInput value={search} onChange={setSearch} />
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto pb-2">
          {loading && <ListStatus>Loading pull requests…</ListStatus>}
          {error && <ListStatus>error: {error}</ListStatus>}
          {needsToken && <TokenNudge onOpenSettings={onOpenSettings} />}
          {!loading && !error && !needsToken && filtered.length === 0 && (
            <ListStatus>No pull requests match.</ListStatus>
          )}
          {!loading && !error && filtered.map((g) => (
            <RepoSection
              key={`${g.owner}/${g.repo}`}
              group={g}
              activeNumber={
                selection && selection.owner === g.owner && selection.repo === g.repo
                  ? selection.number
                  : null
              }
              onSelect={(pr) => setSelection({
                owner: g.owner,
                repo: g.repo,
                number: pr.number,
                project_label: g.project_label,
              })}
              authUser={list?.authenticated_user ?? null}
            />
          ))}
          {list && list.errors.length > 0 && (
            <div className="mx-3 mt-2 space-y-1">
              {list.errors.map((e, i) => (
                <div
                  key={i}
                  className="rounded-md border border-amber-500/25 bg-amber-500/[0.06] px-2 py-1.5 text-[11.5px] text-amber-300"
                  title={e.cwd}
                >
                  <div className="font-medium">{e.owner}/{e.repo}</div>
                  <div className="text-amber-200/80">{e.message}</div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* right: detail */}
      <div className="flex min-w-0 min-h-0 flex-col">
        {selection ? (
          <PullRequestDetail
            key={`${selection.owner}/${selection.repo}#${selection.number}`}
            selection={selection}
            pane={detailPane}
            onPane={setDetailPane}
            expanded={expanded}
            onToggleExpand={() => setExpanded((v) => !v)}
            onMutation={() => setRefreshTick((n) => n + 1)}
            onReviewPr={onReviewPr}
          />
        ) : (
          <div className="flex flex-1 min-h-0 items-center justify-center text-[13px] text-muted-foreground">
            {loading ? 'Loading…' : 'Select a pull request to see details.'}
          </div>
        )}
      </div>
    </div>
  );
}

/* ---------- left column ---------- */

function FilterBar({ filter, onChange }: { filter: Filter; onChange: (f: Filter) => void }) {
  return (
    <div className="flex items-center gap-1 px-3 pt-3 pb-2">
      {(['all', 'reviewing', 'authored'] as const).map((f) => (
        <button
          key={f}
          onClick={() => onChange(f)}
          className={cn(
            'rounded-full px-3 py-1 text-[12.5px] transition-colors',
            filter === f
              ? 'bg-secondary text-foreground'
              : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
          )}
        >
          {f === 'all' ? 'All' : f === 'reviewing' ? 'Reviewing' : 'Authored'}
        </button>
      ))}
    </div>
  );
}

function SearchInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <div className="flex items-center gap-2 rounded-md border border-border bg-secondary/40 px-2.5 py-1.5">
      <MagnifyingGlass className="size-3.5 shrink-0 text-muted-foreground" />
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="Search pull requests"
        spellCheck={false}
        className="min-w-0 flex-1 bg-transparent text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground/60"
      />
    </div>
  );
}

function ListStatus({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-4 py-3 text-[12.5px] text-muted-foreground/70">{children}</div>
  );
}

function TokenNudge({ onOpenSettings }: { onOpenSettings: () => void }) {
  return (
    <div className="mx-3 mt-2 rounded-md border border-amber-500/30 bg-amber-500/[0.08] px-3 py-2.5 text-[12.5px] text-amber-200">
      <div className="font-medium">GitHub token required</div>
      <div className="mt-1 text-amber-200/80">
        Add a <code className="rounded bg-secondary/60 px-1">GITHUB_TOKEN</code> in{' '}
        <button
          onClick={onOpenSettings}
          className="underline underline-offset-2 hover:text-amber-100"
        >
          Settings → Keys
        </button>{' '}
        to load pull requests.
      </div>
    </div>
  );
}

function RepoSection({
  group,
  activeNumber,
  onSelect,
  authUser,
}: {
  group: RepoGroup;
  activeNumber: number | null;
  onSelect: (pr: PullRequestSummary) => void;
  authUser: string | null;
}) {
  if (group.prs.length === 0) return null;
  return (
    <div className="mb-2">
      <div className="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/80">
        {group.project_label}
        <span className="ml-1 text-muted-foreground/50 normal-case tracking-normal">
          {group.owner}/{group.repo}
        </span>
      </div>
      <div className="flex flex-col">
        {group.prs.map((pr) => (
          <PullRequestRow
            key={pr.number}
            pr={pr}
            active={pr.number === activeNumber}
            onClick={() => onSelect(pr)}
            authUser={authUser}
          />
        ))}
      </div>
    </div>
  );
}

function PullRequestRow({
  pr,
  active,
  onClick,
  authUser,
}: {
  pr: PullRequestSummary;
  active: boolean;
  onClick: () => void;
  authUser: string | null;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        'grid w-full grid-cols-[auto_1fr_auto] items-start gap-2 border-l-2 px-3 py-2 text-left transition-colors',
        active
          ? 'border-mira-blue bg-accent/70'
          : 'border-transparent hover:bg-accent/30',
      )}
    >
      <PrIcon pr={pr} />
      <div className="min-w-0">
        <div className="truncate text-[13px] font-medium text-foreground">
          {pr.title}
        </div>
        <div className="flex items-center gap-1.5 truncate text-[11.5px] text-muted-foreground">
          <span
            className={cn(
              'font-mono',
              pr.author === authUser ? 'text-mira-blue' : '',
            )}
          >
            {pr.author}
          </span>
          <span>·</span>
          <span className="truncate font-mono">{pr.head_ref}</span>
        </div>
      </div>
      <div className="flex flex-col items-end gap-1 text-[11px] text-muted-foreground/70">
        <span>{timeAgo(pr.updated_at)}</span>
        {pr.comment_count > 0 && (
          <span className="inline-flex items-center gap-0.5">
            <ChatCircle className="size-3" weight="fill" />
            {pr.comment_count}
          </span>
        )}
      </div>
    </button>
  );
}

function PrIcon({ pr }: { pr: PullRequestSummary }) {
  const color = pr.draft
    ? 'text-muted-foreground'
    : pr.state === 'closed'
      ? 'text-mira-purple'
      : 'text-emerald-500';
  return (
    <span className="mt-0.5 shrink-0" title={pr.draft ? 'Draft' : pr.state}>
      <GitBranch className={cn('size-3.5', color)} weight="fill" />
    </span>
  );
}

/* ---------- right column detail ---------- */

function PullRequestDetail({
  selection,
  pane,
  onPane,
  expanded,
  onToggleExpand,
  onMutation,
  onReviewPr,
}: {
  selection: Selection;
  pane: 'summary' | 'code';
  onPane: (p: 'summary' | 'code') => void;
  expanded: boolean;
  onToggleExpand: () => void;
  onMutation: () => void;
  onReviewPr: (owner: string, repo: string, number: number) => void;
}) {
  const [detail, setDetail] = useState<PullRequestDetailView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    setLoading(true);
    getPullRequest(selection.owner, selection.repo, selection.number)
      .then((v) => { setDetail(v); setError(null); })
      .catch((e) => { setDetail(null); setError(String((e as Error).message)); })
      .finally(() => setLoading(false));
  }, [selection.owner, selection.repo, selection.number, reloadTick]);

  function reload() {
    setReloadTick((n) => n + 1);
    onMutation();
  }

  return (
    <>
      <DetailHeader
        selection={selection}
        detail={detail}
        pane={pane}
        onPane={onPane}
        expanded={expanded}
        onToggleExpand={onToggleExpand}
        onReload={reload}
        onReviewPr={onReviewPr}
      />
      <div className="flex-1 min-h-0 overflow-y-auto">
        {loading && (
          <div className="flex items-center gap-2 px-6 py-6 text-[13px] text-muted-foreground">
            <CircleNotch className="size-3.5 animate-spin text-mira-blue" />
            Loading pull request…
          </div>
        )}
        {error && (
          <div className="mx-6 my-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[13px] text-destructive">
            {error}
          </div>
        )}
        {!loading && detail && pane === 'summary' && (
          <SummaryPane
            selection={selection}
            detail={detail}
            onReload={reload}
          />
        )}
        {!loading && detail && pane === 'code' && (
          <CodePane selection={selection} />
        )}
      </div>
    </>
  );
}

function DetailHeader({
  selection,
  detail,
  pane,
  onPane,
  expanded,
  onToggleExpand,
  onReload,
  onReviewPr,
}: {
  selection: Selection;
  detail: PullRequestDetailView | null;
  pane: 'summary' | 'code';
  onPane: (p: 'summary' | 'code') => void;
  expanded: boolean;
  onToggleExpand: () => void;
  onReload: () => void;
  onReviewPr: (owner: string, repo: string, number: number) => void;
}) {
  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border/60 px-3">
      <GitBranch className="size-4 text-emerald-500" weight="fill" />
      <div className="inline-flex rounded-full border border-border bg-secondary/60 p-0.5">
        {(['summary', 'code'] as const).map((p) => (
          <button
            key={p}
            onClick={() => onPane(p)}
            className={cn(
              'rounded-full px-3.5 py-1 text-[12.5px]',
              pane === p
                ? 'bg-secondary text-foreground'
                : 'text-muted-foreground hover:text-foreground',
            )}
          >
            {p === 'summary' ? 'Summary' : 'Code'}
          </button>
        ))}
      </div>
      <div className="ml-auto flex items-center gap-1">
        {detail && (
          <button
            onClick={() => onReviewPr(selection.owner, selection.repo, selection.number)}
            title="Run Mira's two-stage review on this PR's diff. Findings appear in the review panel."
            className="inline-flex items-center gap-1.5 rounded-full border border-mira-blue/40 bg-mira-blue/[0.08] px-3.5 py-1 text-[12.5px] font-medium text-mira-blue transition-colors hover:bg-mira-blue/[0.14]"
          >
            <Sparkle className="size-3.5" weight="fill" />
            Review with Mira
          </button>
        )}
        {detail && (
          <a
            href={detail.summary.html_url}
            target="_blank"
            rel="noreferrer"
            title="Open on GitHub"
            className="rounded-full p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowSquareOut className="size-4" />
          </a>
        )}
        {detail && (
          <ReviewMenu
            selection={selection}
            detail={detail}
            onSubmitted={onReload}
          />
        )}
        {detail && !detail.merged && detail.mergeable !== false && (
          <MergeMenu
            selection={selection}
            onMerged={onReload}
          />
        )}
        <button
          onClick={onToggleExpand}
          title={expanded ? 'Show list' : 'Expand'}
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <ArrowsOutSimple className="size-4" />
        </button>
      </div>
    </div>
  );
}

function SummaryPane({
  selection,
  detail,
  onReload,
}: {
  selection: Selection;
  detail: PullRequestDetailView;
  onReload: () => void;
}) {
  const pr = detail.summary;
  return (
    <div className="mx-auto max-w-3xl px-6 pb-8 pt-5">
      <h1 className="text-[22px] font-medium tracking-tight text-foreground">
        {pr.title}
      </h1>
      <div className="mt-1.5 flex items-center gap-2 text-[12.5px] text-muted-foreground">
        <AvatarLike login={pr.author} url={pr.author_avatar} size={14} />
        <span>{pr.author}</span>
        <span>·</span>
        <span>{timeAgo(pr.created_at)}</span>
        <span>·</span>
        <span className="font-mono text-[11.5px]">#{pr.number}</span>
      </div>

      <MetadataGrid detail={detail} />

      <Section title="Description">
        {pr.additions === null && (
          <p className="text-[12.5px] text-muted-foreground/70">Loading…</p>
        )}
        {detail.body ? (
          <div className="md text-[13.5px] leading-relaxed">
            <Markdown text={detail.body} />
          </div>
        ) : (
          <p className="text-[13px] text-muted-foreground/70">No description provided.</p>
        )}
      </Section>

      <Section title="Checks">
        {detail.checks.length === 0 ? (
          <p className="text-[13px] text-muted-foreground/70">No CI checks.</p>
        ) : (
          <div className="flex flex-col gap-1">
            {detail.checks.map((c, i) => (
              <div key={i} className="flex items-center gap-2 rounded-md border border-border/50 bg-secondary/20 px-3 py-1.5 text-[12.5px]">
                <CheckIcon conclusion={c.conclusion} status={c.status} />
                <span className="min-w-0 flex-1 truncate text-foreground/85">{c.name}</span>
                {c.url && (
                  <a
                    href={c.url}
                    target="_blank"
                    rel="noreferrer"
                    className="text-muted-foreground hover:text-foreground"
                  >
                    <ArrowSquareOut className="size-3.5" />
                  </a>
                )}
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section title={`Activity`} count={detail.timeline.length + detail.reviews.length + detail.comments.length}>
        <ActivityFeed detail={detail} />
      </Section>

      <CommentBox
        selection={selection}
        onPosted={onReload}
      />
    </div>
  );
}

function MetadataGrid({ detail }: { detail: PullRequestDetailView }) {
  const pr = detail.summary;
  const status = detail.merged
    ? 'Merged'
    : pr.draft
      ? 'Draft'
      : pr.state === 'closed'
        ? 'Closed'
        : detail.mergeable === false
          ? 'Conflicts'
          : 'Ready for review';
  return (
    <div className="mt-5 flex flex-col gap-2 text-[12.5px]">
      <MetaRow icon={<GitBranch className="size-3.5" weight="fill" />} label="Branch">
        <span className="font-mono">{pr.head_ref}</span>
        <CaretDown className="size-3 rotate-[-90deg] text-muted-foreground/60" />
        <span className="font-mono">{pr.base_ref}</span>
        {pr.additions !== null && (
          <>
            <span className="ml-2 text-emerald-500">+{pr.additions}</span>
            <span className="text-rose-500">-{pr.deletions ?? 0}</span>
          </>
        )}
      </MetaRow>
      <MetaRow icon={<UsersThree className="size-3.5" weight="fill" />} label="Reviewers">
        {pr.requested_reviewers.length === 0 && detail.reviews.length === 0 ? (
          <span className="text-muted-foreground/70">None requested</span>
        ) : (
          <div className="flex flex-wrap items-center gap-1.5">
            {pr.requested_reviewers.map((r) => (
              <span
                key={r}
                className="rounded-full border border-border/60 bg-secondary/40 px-2 py-0.5 font-mono text-[11.5px] text-foreground/85"
              >
                {r}
              </span>
            ))}
            {detail.reviews.map((r, i) => (
              <span
                key={`review-${i}`}
                className={cn(
                  'inline-flex items-center gap-1 rounded-full px-2 py-0.5 font-mono text-[11.5px]',
                  reviewChipClass(r.state),
                )}
                title={`${r.state.toLowerCase()} by ${r.author}`}
              >
                {r.author}
                <ReviewStateIcon state={r.state} />
              </span>
            ))}
          </div>
        )}
      </MetaRow>
      <MetaRow icon={<ChatCircle className="size-3.5" weight="fill" />} label="Comments">
        {detail.comments.length === 0 ? 'No comments' : `${detail.comments.length}`}
      </MetaRow>
      <MetaRow icon={<CheckCircle className="size-3.5" weight="fill" />} label="Checks">
        <ChecksStatus status={detail.check_status} />
      </MetaRow>
      <MetaRow icon={<GitCommit className="size-3.5" weight="fill" />} label="Status">
        <span
          className={cn(
            'rounded-full border px-2 py-0.5 text-[11.5px]',
            statusClass(status),
          )}
        >
          {status}
        </span>
      </MetaRow>
    </div>
  );
}

function MetaRow({
  icon,
  label,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[130px_1fr] items-center gap-3 py-0.5">
      <div className="flex items-center gap-2 text-muted-foreground">
        <span className="text-muted-foreground/70">{icon}</span>
        <span>{label}</span>
      </div>
      <div className="flex items-center gap-2 text-foreground/90">{children}</div>
    </div>
  );
}

function Section({
  title,
  count,
  children,
}: {
  title: string;
  count?: number;
  children: React.ReactNode;
}) {
  return (
    <div className="mt-6">
      <div className="flex items-center gap-2 pb-2">
        <h2 className="text-[15px] font-medium text-foreground">{title}</h2>
        {count !== undefined && count > 0 && (
          <span className="text-[12px] text-muted-foreground">{count}</span>
        )}
      </div>
      {children}
    </div>
  );
}

function ChecksStatus({ status }: { status: string | null }) {
  if (!status || status === 'none') {
    return <span className="text-muted-foreground/70">No CI checks</span>;
  }
  const cls =
    status === 'success'
      ? 'text-emerald-400'
      : status === 'failure'
        ? 'text-destructive'
        : 'text-amber-400';
  return (
    <span className={cn('inline-flex items-center gap-1.5', cls)}>
      <CheckIcon conclusion={status === 'failure' ? 'failure' : status === 'success' ? 'success' : null} status={status === 'pending' ? 'in_progress' : 'completed'} />
      {status === 'success' ? 'All checks passed' : status === 'failure' ? 'Some checks failed' : 'Checks in progress'}
    </span>
  );
}

function CheckIcon({ status, conclusion }: { status: string; conclusion: string | null }) {
  if (status !== 'completed') {
    return <CircleNotch className="size-3.5 animate-spin text-amber-400" />;
  }
  switch (conclusion) {
    case 'success':
      return <CheckCircle className="size-3.5 text-emerald-400" weight="fill" />;
    case 'failure':
    case 'timed_out':
    case 'cancelled':
    case 'action_required':
      return <XCircle className="size-3.5 text-destructive" weight="fill" />;
    default:
      return <WarningCircle className="size-3.5 text-muted-foreground" weight="fill" />;
  }
}

/* ---------- activity feed ---------- */

type FeedItem =
  | { kind: 'comment'; at: string; comment: CommentView }
  | { kind: 'review'; at: string; review: NonNullable<PullRequestDetailView['reviews']>[number] }
  | { kind: 'event'; at: string; event: NonNullable<PullRequestDetailView['timeline']>[number] };

function ActivityFeed({ detail }: { detail: PullRequestDetailView }) {
  const items = useMemo<FeedItem[]>(() => {
    const out: FeedItem[] = [];
    detail.comments.forEach((c) => out.push({ kind: 'comment', at: c.created_at, comment: c }));
    detail.reviews.forEach((r) => out.push({ kind: 'review', at: r.submitted_at ?? '', review: r }));
    detail.timeline.forEach((e) => {
      // Skip comment timeline events — they already come through in `comments`.
      if (e.kind === 'comment') return;
      out.push({ kind: 'event', at: e.at ?? '', event: e });
    });
    out.sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
    return out;
  }, [detail]);
  if (items.length === 0) {
    return <p className="text-[13px] text-muted-foreground/70">No activity yet.</p>;
  }
  return (
    <div className="flex flex-col">
      {items.map((it, i) => (
        <FeedRow key={i} item={it} />
      ))}
    </div>
  );
}

function FeedRow({ item }: { item: FeedItem }) {
  if (item.kind === 'comment') {
    const c = item.comment;
    return (
      <div className="flex gap-2.5 py-2.5">
        <AvatarLike login={c.author} url={c.author_avatar} size={20} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
            <span className="font-medium text-foreground/90">{c.author}</span>
            <span>commented</span>
            <span>·</span>
            <span>{timeAgo(c.created_at)}</span>
          </div>
          <div className="mt-1 rounded-md border border-border/60 bg-card/40 px-3 py-2 text-[13px]">
            <Markdown text={c.body || '_(empty)_'} />
          </div>
        </div>
      </div>
    );
  }
  if (item.kind === 'review') {
    const r = item.review;
    const verb =
      r.state === 'APPROVED'
        ? 'approved'
        : r.state === 'CHANGES_REQUESTED'
          ? 'requested changes'
          : r.state === 'COMMENTED'
            ? 'commented'
            : r.state.toLowerCase();
    return (
      <div className="flex gap-2.5 py-2">
        <AvatarLike login={r.author} url={r.author_avatar} size={20} />
        <div className="min-w-0 flex-1 text-[12.5px]">
          <div className="flex items-center gap-1.5 text-muted-foreground">
            <span className="font-medium text-foreground/90">{r.author}</span>
            <span>{verb}</span>
            {r.submitted_at && (<><span>·</span><span>{timeAgo(r.submitted_at)}</span></>)}
            <ReviewStateIcon state={r.state} />
          </div>
          {r.body && (
            <div className="mt-1 rounded-md border border-border/60 bg-card/40 px-3 py-2 text-[13px]">
              <Markdown text={r.body} />
            </div>
          )}
        </div>
      </div>
    );
  }
  const e = item.event;
  return (
    <div className="flex items-center gap-2 py-1.5 text-[12px] text-muted-foreground">
      <TimelineIcon kind={e.kind} />
      {e.actor && <span className="font-medium text-foreground/85">{e.actor}</span>}
      <span>{humanTimelineVerb(e.kind, e.message)}</span>
      {e.at && (<><span>·</span><span>{timeAgo(e.at)}</span></>)}
    </div>
  );
}

function TimelineIcon({ kind }: { kind: string }) {
  switch (kind) {
    case 'commit':
      return <GitCommit className="size-3.5 text-muted-foreground/70" weight="fill" />;
    case 'merged':
      return <GitMerge className="size-3.5 text-mira-purple" weight="fill" />;
    case 'closed':
      return <XCircle className="size-3.5 text-destructive" weight="fill" />;
    case 'reopened':
      return <CheckCircle className="size-3.5 text-emerald-400" weight="fill" />;
    case 'review_requested':
      return <UsersThree className="size-3.5 text-muted-foreground/70" weight="fill" />;
    default:
      return <Clock className="size-3.5 text-muted-foreground/60" weight="fill" />;
  }
}

function humanTimelineVerb(kind: string, msg: string): string {
  switch (kind) {
    case 'commit':
      return `committed ${msg}`;
    case 'merged':
      return 'merged the pull request';
    case 'closed':
      return 'closed the pull request';
    case 'reopened':
      return 'reopened the pull request';
    case 'labeled':
      return `added the ${msg} label`;
    case 'unlabeled':
      return `removed the ${msg} label`;
    case 'review_requested':
      return `requested review from ${msg}`;
    default:
      return kind.replace(/_/g, ' ');
  }
}

/* ---------- comment box + review + merge ---------- */

function CommentBox({
  selection,
  onPosted,
}: {
  selection: Selection;
  onPosted: () => void;
}) {
  const [text, setText] = useState('');
  const [posting, setPosting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function submit() {
    if (!text.trim()) return;
    setPosting(true);
    setError(null);
    try {
      await postPullRequestComment(selection.owner, selection.repo, selection.number, text);
      setText('');
      onPosted();
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setPosting(false);
    }
  }
  return (
    <div className="mt-6 rounded-lg border border-border bg-card/40 p-3">
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="Leave a comment"
        rows={3}
        className="w-full resize-y bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground/60"
      />
      {error && (
        <div className="mt-1 text-[11.5px] text-destructive">{error}</div>
      )}
      <div className="flex items-center justify-end">
        <button
          onClick={submit}
          disabled={!text.trim() || posting}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[12.5px] font-medium transition-colors',
            !text.trim() || posting
              ? 'bg-secondary text-muted-foreground/60 cursor-not-allowed'
              : 'bg-mira-blue text-white hover:opacity-90',
          )}
        >
          {posting ? <CircleNotch className="size-3.5 animate-spin" /> : <ArrowUp className="size-3.5" />}
          Comment
        </button>
      </div>
    </div>
  );
}

function ReviewMenu({
  selection,
  detail,
  onSubmitted,
}: {
  selection: Selection;
  detail: PullRequestDetailView;
  onSubmitted: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [event, setEvent] = useState<ReviewEvent>('APPROVE');
  const [body, setBody] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (detail.merged) return null;
  async function submit() {
    if (event !== 'APPROVE' && !body.trim()) {
      setError('Add a comment for this review type.');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await submitPullRequestReview(selection.owner, selection.repo, selection.number, event, body || undefined);
      setBody('');
      setOpen(false);
      onSubmitted();
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button className="rounded-full border border-border bg-secondary/60 px-3.5 py-1 text-[12.5px] font-medium text-foreground transition-colors hover:bg-secondary">
          Submit review
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80 p-3" align="end">
        <div className="mb-2 text-[12.5px] font-medium text-foreground">Review</div>
        <div className="flex flex-col gap-1.5">
          {(['APPROVE', 'REQUEST_CHANGES', 'COMMENT'] as const).map((e) => (
            <label
              key={e}
              className={cn(
                'flex items-center gap-2 rounded-md px-2 py-1.5 text-[12.5px] transition-colors',
                event === e ? 'bg-accent text-foreground' : 'hover:bg-accent/50',
              )}
            >
              <input
                type="radio"
                name="review-event"
                value={e}
                checked={event === e}
                onChange={() => setEvent(e)}
                className="accent-mira-blue"
              />
              <ReviewLabel event={e} />
            </label>
          ))}
        </div>
        <textarea
          value={body}
          onChange={(v) => setBody(v.target.value)}
          rows={3}
          placeholder={
            event === 'APPROVE' ? 'Optional message' : 'Leave a comment'
          }
          className="mt-2 w-full resize-y rounded-md border border-border bg-secondary/40 px-2 py-1.5 text-[12.5px] outline-none"
        />
        {error && (
          <div className="mt-1 text-[11.5px] text-destructive">{error}</div>
        )}
        <div className="mt-2 flex justify-end">
          <button
            onClick={submit}
            disabled={busy}
            className="inline-flex items-center gap-1.5 rounded-full bg-mira-blue px-3 py-1 text-[12.5px] font-medium text-white hover:opacity-90 disabled:opacity-60"
          >
            {busy && <CircleNotch className="size-3.5 animate-spin" />}
            Submit
          </button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function ReviewLabel({ event }: { event: ReviewEvent }) {
  switch (event) {
    case 'APPROVE':
      return (
        <span className="inline-flex items-center gap-1.5 text-emerald-400">
          <CheckCircle className="size-3.5" weight="fill" />
          Approve
        </span>
      );
    case 'REQUEST_CHANGES':
      return (
        <span className="inline-flex items-center gap-1.5 text-destructive">
          <XCircle className="size-3.5" weight="fill" />
          Request changes
        </span>
      );
    case 'COMMENT':
      return (
        <span className="inline-flex items-center gap-1.5 text-mira-blue">
          <ChatCircle className="size-3.5" weight="fill" />
          Comment
        </span>
      );
  }
}

function MergeMenu({
  selection,
  onMerged,
}: {
  selection: Selection;
  onMerged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<MergeMethod | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function pick(method: MergeMethod) {
    setBusy(method);
    setError(null);
    try {
      await mergePullRequest(selection.owner, selection.repo, selection.number, method);
      setOpen(false);
      onMerged();
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(null);
    }
  }
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button className="inline-flex items-center gap-1 rounded-full border border-mira-purple/40 bg-mira-purple/[0.08] px-3.5 py-1 text-[12.5px] font-medium text-mira-purple transition-colors hover:bg-mira-purple/[0.12]">
          <GitMerge className="size-3.5" weight="fill" />
          Merge
          <CaretDown className="size-3" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-56 p-1" align="end">
        {(['merge', 'squash', 'rebase'] as const).map((m) => (
          <button
            key={m}
            onClick={() => pick(m)}
            disabled={busy !== null}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12.5px] transition-colors hover:bg-accent/60 disabled:opacity-60"
          >
            {busy === m ? <CircleNotch className="size-3.5 animate-spin" /> : <GitMerge className="size-3.5" weight="fill" />}
            <span className="capitalize">{m}</span>
            <span className="ml-auto text-muted-foreground">
              {m === 'merge' ? 'commit' : m === 'squash' ? 'squash & merge' : 'rebase & merge'}
            </span>
          </button>
        ))}
        {error && (
          <div className="mx-1 mt-1 text-[11.5px] text-destructive">{error}</div>
        )}
      </PopoverContent>
    </Popover>
  );
}

/* ---------- code pane (Files changed) ---------- */

function CodePane({ selection }: { selection: Selection }) {
  const [files, setFiles] = useState<FileChangeView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    setLoading(true);
    getPullRequestFiles(selection.owner, selection.repo, selection.number)
      .then((v) => { setFiles(v.files); setError(null); })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, [selection.owner, selection.repo, selection.number]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 px-6 py-6 text-[13px] text-muted-foreground">
        <CircleNotch className="size-3.5 animate-spin text-mira-blue" />
        Loading files…
      </div>
    );
  }
  if (error) {
    return (
      <div className="mx-6 my-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[13px] text-destructive">
        {error}
      </div>
    );
  }
  if (!files || files.length === 0) {
    return (
      <div className="px-6 py-6 text-[13px] text-muted-foreground/70">
        No file changes.
      </div>
    );
  }
  return (
    <div className="mx-auto max-w-5xl px-4 py-4">
      <div className="mb-3 flex items-center gap-2 text-[12px] text-muted-foreground">
        <CodeIcon className="size-4" weight="fill" />
        <span>{files.length} file{files.length === 1 ? '' : 's'} changed</span>
      </div>
      <div className="flex flex-col gap-3">
        {files.map((f) => (
          <FilePatch key={f.filename} file={f} />
        ))}
      </div>
    </div>
  );
}

function FilePatch({ file }: { file: FileChangeView }) {
  const [expanded, setExpanded] = useState(true);
  const lines = useMemo(() => splitPatchLines(file.patch), [file.patch]);
  return (
    <div className="overflow-hidden rounded-md border border-border">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full items-center gap-2 border-b border-border/60 bg-secondary/40 px-3 py-1.5 text-left"
      >
        <CaretDown
          className={cn(
            'size-3 text-muted-foreground/70 transition-transform',
            !expanded && '-rotate-90',
          )}
        />
        <span className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-foreground/85">
          {file.filename}
        </span>
        <span className="text-[11.5px] text-emerald-400">+{file.additions}</span>
        <span className="text-[11.5px] text-rose-500">-{file.deletions}</span>
        <span className="rounded bg-secondary px-1.5 py-0.5 text-[10.5px] uppercase tracking-wider text-muted-foreground">
          {file.status}
        </span>
      </button>
      {expanded && (
        file.patch === null ? (
          <div className="px-3 py-2 text-[12px] text-muted-foreground/70">
            Binary or too-large diff — omitted by GitHub.
          </div>
        ) : (
          <div className="overflow-x-auto bg-background">
            <div className="font-mono text-[12px]">
              {lines.map((l, i) => (
                <div
                  key={i}
                  className={cn(
                    'whitespace-pre px-3 py-[1px]',
                    l.kind === 'add' && 'bg-emerald-500/[0.09] text-emerald-200',
                    l.kind === 'del' && 'bg-rose-500/[0.09] text-rose-200',
                    l.kind === 'hunk' && 'bg-secondary/40 text-muted-foreground',
                    l.kind === 'ctx' && 'text-foreground/80',
                  )}
                >
                  {l.text || ' '}
                </div>
              ))}
            </div>
          </div>
        )
      )}
    </div>
  );
}

type PatchLine = { kind: 'add' | 'del' | 'hunk' | 'ctx'; text: string };

function splitPatchLines(patch: string | null): PatchLine[] {
  if (!patch) return [];
  return patch.split('\n').map((raw): PatchLine => {
    if (raw.startsWith('@@')) return { kind: 'hunk', text: raw };
    if (raw.startsWith('+')) return { kind: 'add', text: raw };
    if (raw.startsWith('-')) return { kind: 'del', text: raw };
    return { kind: 'ctx', text: raw };
  });
}

/* ---------- shared bits ---------- */

function AvatarLike({
  login,
  url,
  size,
}: {
  login: string;
  url: string | null;
  size: number;
}) {
  // Flex parents default to align-items: stretch, which was inflating the
  // <img> vertically down the whole row (the "streaked" look). Pin size
  // via inline style AND lock the box with shrink-0 + self-start so the
  // stretch can't win over the width/height HTML attrs.
  const box = { width: size, height: size } as const;
  if (url) {
    return (
      <img
        src={url}
        alt={login}
        width={size}
        height={size}
        style={box}
        className="shrink-0 self-start rounded-full object-cover"
      />
    );
  }
  return (
    <span
      className="inline-flex shrink-0 self-start items-center justify-center rounded-full bg-secondary text-[10px] font-medium uppercase text-foreground/85"
      style={box}
    >
      {login.slice(0, 2)}
    </span>
  );
}

function ReviewStateIcon({ state }: { state: string }) {
  switch (state) {
    case 'APPROVED':
      return <CheckCircle className="size-3 text-emerald-400" weight="fill" />;
    case 'CHANGES_REQUESTED':
      return <XCircle className="size-3 text-destructive" weight="fill" />;
    case 'COMMENTED':
      return <ChatCircle className="size-3 text-mira-blue" weight="fill" />;
    default:
      return null;
  }
}

function reviewChipClass(state: string): string {
  switch (state) {
    case 'APPROVED':
      return 'border border-emerald-500/40 bg-emerald-500/[0.08] text-emerald-300';
    case 'CHANGES_REQUESTED':
      return 'border border-destructive/40 bg-destructive/10 text-destructive';
    case 'COMMENTED':
      return 'border border-mira-blue/30 bg-mira-blue/[0.08] text-mira-blue';
    default:
      return 'border border-border/60 bg-secondary/40 text-foreground/85';
  }
}

function statusClass(status: string): string {
  switch (status) {
    case 'Merged':
      return 'border-mira-purple/40 bg-mira-purple/[0.08] text-mira-purple';
    case 'Closed':
      return 'border-destructive/40 bg-destructive/10 text-destructive';
    case 'Conflicts':
      return 'border-amber-500/40 bg-amber-500/[0.08] text-amber-300';
    case 'Draft':
      return 'border-border bg-secondary/40 text-muted-foreground';
    default:
      return 'border-emerald-500/40 bg-emerald-500/[0.08] text-emerald-300';
  }
}

function filterAndSearch(
  list: PullRequestListView | null,
  filter: Filter,
  search: string,
): RepoGroup[] {
  if (!list) return [];
  const q = search.trim().toLowerCase();
  const user = list.authenticated_user;
  return list.repos
    .map((g) => {
      const prs = g.prs.filter((pr) => {
        if (q && !pr.title.toLowerCase().includes(q) && !pr.author.toLowerCase().includes(q)) {
          return false;
        }
        if (filter === 'authored' && user) return pr.author === user;
        if (filter === 'reviewing' && user) {
          return pr.requested_reviewers.includes(user) && pr.author !== user;
        }
        return true;
      });
      return { ...g, prs };
    })
    .filter((g) => g.prs.length > 0);
}

function timeAgo(iso: string): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (isNaN(t)) return '';
  const diff = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (diff < 60) return `${diff}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  const d = Math.floor(diff / 86400);
  if (d < 30) return `${d}d`;
  const mo = Math.floor(d / 30);
  if (mo < 12) return `${mo}mo`;
  return `${Math.floor(d / 365)}y`;
}
