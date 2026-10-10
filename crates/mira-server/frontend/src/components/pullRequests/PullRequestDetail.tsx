import { cn } from '@/lib/utils';
import {
  ChevronDown,
  CircleAlert,
  CircleCheck,
  CircleX,
  Expand,
  GitBranch,
  GitCommit,
  LoaderCircle,
  MessageCircle,
  Sparkle,
  SquareArrowOutUpRight,
  Users,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { getPullRequest, type PullRequestDetailView } from '../../api';
import { Markdown } from '../Markdown';
import { ActivityFeed, CommentBox, MergeMenu, ReviewMenu } from './ActivityFeed';
import { CodePane } from './CodePane';
import { AvatarLike, reviewChipClass, ReviewStateIcon, statusClass, timeAgo } from './shared';
import { Selection } from './types';
/* ---------- right column detail ---------- */

export function PullRequestDetail({
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
      .then((v) => {
        setDetail(v);
        setError(null);
      })
      .catch((e) => {
        setDetail(null);
        setError(String((e as Error).message));
      })
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
            <LoaderCircle className="size-3.5 animate-spin text-mira-blue" />
            Loading pull request…
          </div>
        )}
        {error && (
          <div className="mx-6 my-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[13px] text-destructive">
            {error}
          </div>
        )}
        {!loading && detail && pane === 'summary' && (
          <SummaryPane selection={selection} detail={detail} onReload={reload} />
        )}
        {!loading && detail && pane === 'code' && <CodePane selection={selection} />}
      </div>
    </>
  );
}

export function DetailHeader({
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
      <GitBranch className="size-4 text-emerald-500" fill="currentColor" />
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
            <Sparkle className="size-3.5" fill="currentColor" />
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
            <SquareArrowOutUpRight className="size-4" />
          </a>
        )}
        {detail && <ReviewMenu selection={selection} detail={detail} onSubmitted={onReload} />}
        {detail && !detail.merged && detail.mergeable !== false && (
          <MergeMenu selection={selection} onMerged={onReload} />
        )}
        <button
          onClick={onToggleExpand}
          title={expanded ? 'Show list' : 'Expand'}
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <Expand className="size-4" />
        </button>
      </div>
    </div>
  );
}

export function SummaryPane({
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
      <h1 className="text-[22px] font-medium tracking-tight text-foreground">{pr.title}</h1>
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
              <div
                key={i}
                className="flex items-center gap-2 rounded-md border border-border/50 bg-secondary/20 px-3 py-1.5 text-[12.5px]"
              >
                <CheckIcon conclusion={c.conclusion} status={c.status} />
                <span className="min-w-0 flex-1 truncate text-foreground/85">{c.name}</span>
                {c.url && (
                  <a
                    href={c.url}
                    target="_blank"
                    rel="noreferrer"
                    className="text-muted-foreground hover:text-foreground"
                  >
                    <SquareArrowOutUpRight className="size-3.5" />
                  </a>
                )}
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section
        title={`Activity`}
        count={detail.timeline.length + detail.reviews.length + detail.comments.length}
      >
        <ActivityFeed detail={detail} />
      </Section>

      <CommentBox selection={selection} onPosted={onReload} />
    </div>
  );
}

export function MetadataGrid({ detail }: { detail: PullRequestDetailView }) {
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
      <MetaRow icon={<GitBranch className="size-3.5" fill="currentColor" />} label="Branch">
        <span className="font-mono">{pr.head_ref}</span>
        <ChevronDown className="size-3 rotate-[-90deg] text-muted-foreground/60" />
        <span className="font-mono">{pr.base_ref}</span>
        {pr.additions !== null && (
          <>
            <span className="ml-2 text-emerald-500">+{pr.additions}</span>
            <span className="text-rose-500">-{pr.deletions ?? 0}</span>
          </>
        )}
      </MetaRow>
      <MetaRow icon={<Users className="size-3.5" fill="currentColor" />} label="Reviewers">
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
      <MetaRow icon={<MessageCircle className="size-3.5" fill="currentColor" />} label="Comments">
        {detail.comments.length === 0 ? 'No comments' : `${detail.comments.length}`}
      </MetaRow>
      <MetaRow icon={<CircleCheck className="size-3.5" fill="currentColor" />} label="Checks">
        <ChecksStatus status={detail.check_status} />
      </MetaRow>
      <MetaRow icon={<GitCommit className="size-3.5" fill="currentColor" />} label="Status">
        <span className={cn('rounded-full border px-2 py-0.5 text-[11.5px]', statusClass(status))}>
          {status}
        </span>
      </MetaRow>
    </div>
  );
}

export function MetaRow({
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

export function Section({
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

export function ChecksStatus({ status }: { status: string | null }) {
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
      <CheckIcon
        conclusion={status === 'failure' ? 'failure' : status === 'success' ? 'success' : null}
        status={status === 'pending' ? 'in_progress' : 'completed'}
      />
      {status === 'success'
        ? 'All checks passed'
        : status === 'failure'
          ? 'Some checks failed'
          : 'Checks in progress'}
    </span>
  );
}

export function CheckIcon({ status, conclusion }: { status: string; conclusion: string | null }) {
  if (status !== 'completed') {
    return <LoaderCircle className="size-3.5 animate-spin text-amber-400" />;
  }
  switch (conclusion) {
    case 'success':
      return <CircleCheck className="size-3.5 text-emerald-400" fill="currentColor" />;
    case 'failure':
    case 'timed_out':
    case 'cancelled':
    case 'action_required':
      return <CircleX className="size-3.5 text-destructive" fill="currentColor" />;
    default:
      return <CircleAlert className="size-3.5 text-muted-foreground" fill="currentColor" />;
  }
}
