import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import {
  ArrowUp,
  ChevronDown,
  CircleCheck,
  CircleX,
  Clock,
  GitCommit,
  GitMerge,
  LoaderCircle,
  MessageCircle,
  Users,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import {
  mergePullRequest,
  postPullRequestComment,
  submitPullRequestReview,
  type CommentView,
  type MergeMethod,
  type PullRequestDetailView,
  type ReviewEvent,
} from '../../api';
import { Markdown } from '../Markdown';
import { AvatarLike, ReviewStateIcon, timeAgo } from './shared';
import { Selection } from './types';
/* ---------- activity feed ---------- */

export type FeedItem =
  | { kind: 'comment'; at: string; comment: CommentView }
  | { kind: 'review'; at: string; review: NonNullable<PullRequestDetailView['reviews']>[number] }
  | { kind: 'event'; at: string; event: NonNullable<PullRequestDetailView['timeline']>[number] };

export function ActivityFeed({ detail }: { detail: PullRequestDetailView }) {
  const items = useMemo<FeedItem[]>(() => {
    const out: FeedItem[] = [];
    detail.comments.forEach((c) => out.push({ kind: 'comment', at: c.created_at, comment: c }));
    detail.reviews.forEach((r) =>
      out.push({ kind: 'review', at: r.submitted_at ?? '', review: r }),
    );
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

export function FeedRow({ item }: { item: FeedItem }) {
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
            {r.submitted_at && (
              <>
                <span>·</span>
                <span>{timeAgo(r.submitted_at)}</span>
              </>
            )}
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
      {e.at && (
        <>
          <span>·</span>
          <span>{timeAgo(e.at)}</span>
        </>
      )}
    </div>
  );
}

export function TimelineIcon({ kind }: { kind: string }) {
  switch (kind) {
    case 'commit':
      return <GitCommit className="size-3.5 text-muted-foreground/70" fill="currentColor" />;
    case 'merged':
      return <GitMerge className="size-3.5 text-mira-purple" fill="currentColor" />;
    case 'closed':
      return <CircleX className="size-3.5 text-destructive" fill="currentColor" />;
    case 'reopened':
      return <CircleCheck className="size-3.5 text-emerald-400" fill="currentColor" />;
    case 'review_requested':
      return <Users className="size-3.5 text-muted-foreground/70" fill="currentColor" />;
    default:
      return <Clock className="size-3.5 text-muted-foreground/60" fill="currentColor" />;
  }
}

export function humanTimelineVerb(kind: string, msg: string): string {
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

export function CommentBox({
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
      {error && <div className="mt-1 text-[11.5px] text-destructive">{error}</div>}
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
          {posting ? (
            <LoaderCircle className="size-3.5 animate-spin" />
          ) : (
            <ArrowUp className="size-3.5" />
          )}
          Comment
        </button>
      </div>
    </div>
  );
}

export function ReviewMenu({
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
      await submitPullRequestReview(
        selection.owner,
        selection.repo,
        selection.number,
        event,
        body || undefined,
      );
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
          placeholder={event === 'APPROVE' ? 'Optional message' : 'Leave a comment'}
          className="mt-2 w-full resize-y rounded-md border border-border bg-secondary/40 px-2 py-1.5 text-[12.5px] outline-none"
        />
        {error && <div className="mt-1 text-[11.5px] text-destructive">{error}</div>}
        <div className="mt-2 flex justify-end">
          <button
            onClick={submit}
            disabled={busy}
            className="inline-flex items-center gap-1.5 rounded-full bg-mira-blue px-3 py-1 text-[12.5px] font-medium text-white hover:opacity-90 disabled:opacity-60"
          >
            {busy && <LoaderCircle className="size-3.5 animate-spin" />}
            Submit
          </button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function ReviewLabel({ event }: { event: ReviewEvent }) {
  switch (event) {
    case 'APPROVE':
      return (
        <span className="inline-flex items-center gap-1.5 text-emerald-400">
          <CircleCheck className="size-3.5" fill="currentColor" />
          Approve
        </span>
      );
    case 'REQUEST_CHANGES':
      return (
        <span className="inline-flex items-center gap-1.5 text-destructive">
          <CircleX className="size-3.5" fill="currentColor" />
          Request changes
        </span>
      );
    case 'COMMENT':
      return (
        <span className="inline-flex items-center gap-1.5 text-mira-blue">
          <MessageCircle className="size-3.5" fill="currentColor" />
          Comment
        </span>
      );
  }
}

export function MergeMenu({ selection, onMerged }: { selection: Selection; onMerged: () => void }) {
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
          <GitMerge className="size-3.5" fill="currentColor" />
          Merge
          <ChevronDown className="size-3" />
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
            {busy === m ? (
              <LoaderCircle className="size-3.5 animate-spin" />
            ) : (
              <GitMerge className="size-3.5" fill="currentColor" />
            )}
            <span className="capitalize">{m}</span>
            <span className="ml-auto text-muted-foreground">
              {m === 'merge' ? 'commit' : m === 'squash' ? 'squash & merge' : 'rebase & merge'}
            </span>
          </button>
        ))}
        {error && <div className="mx-1 mt-1 text-[11.5px] text-destructive">{error}</div>}
      </PopoverContent>
    </Popover>
  );
}
