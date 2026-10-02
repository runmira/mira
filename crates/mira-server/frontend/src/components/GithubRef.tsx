/**
 * A GitHub PR or issue mentioned in a transcript, as a chip with a hover
 * card — state, title, author, diff size — the way GitHub-aware tools show
 * them. Works for Mira's replies and agents' alike: anything rendered as
 * Markdown.
 */
import { useEffect, useRef, useState } from 'react';
import { CircleDot, GitMerge, GitPullRequest, GitPullRequestClosed, GitPullRequestDraft } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import type { GithubRefTarget } from '../lib/githubRefs';

type Card = {
  kind: 'pr' | 'issue';
  number: number;
  title: string;
  state: 'open' | 'closed' | 'merged' | 'draft';
  author: string | null;
  author_avatar: string | null;
  additions: number | null;
  deletions: number | null;
  changed_files: number | null;
  created_at: string | null;
  url: string;
};

const cache = new Map<string, Promise<Card | null>>();
function loadCard(t: GithubRefTarget): Promise<Card | null> {
  const key = `${t.owner}/${t.repo}#${t.number}`;
  let p = cache.get(key);
  if (!p) {
    p = fetch(`/api/prs/${t.owner}/${t.repo}/${t.number}/card`)
      .then((r) => (r.ok ? (r.json() as Promise<Card>) : null))
      .catch(() => null);
    cache.set(key, p);
  }
  return p;
}

const STATE: Record<Card['state'], { label: string; className: string }> = {
  open: { label: 'Open', className: 'bg-emerald-600/90 text-white' },
  draft: { label: 'Draft', className: 'bg-zinc-500/80 text-white' },
  merged: { label: 'Merged', className: 'bg-violet-600/90 text-white' },
  closed: { label: 'Closed', className: 'bg-red-600/85 text-white' },
};

function StateIcon({ card, kind, className }: { card: Card | null; kind: GithubRefTarget['kind']; className?: string }) {
  const isIssue = card ? card.kind === 'issue' : kind === 'issues';
  if (isIssue) return <CircleDot className={className} />;
  switch (card?.state) {
    case 'merged':
      return <GitMerge className={className} />;
    case 'closed':
      return <GitPullRequestClosed className={className} />;
    case 'draft':
      return <GitPullRequestDraft className={className} />;
    default:
      return <GitPullRequest className={className} />;
  }
}

function iconTone(card: Card | null): string {
  switch (card?.state) {
    case 'open':
      return 'text-emerald-400';
    case 'merged':
      return 'text-violet-400';
    case 'closed':
      return 'text-red-400';
    default:
      return 'text-muted-foreground';
  }
}

function ago(iso: string | null): string {
  if (!iso) return '';
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.round(s / 86400)}d ago`;
  return new Date(iso).toLocaleDateString();
}

export function GithubRef({ target, href, children }: { target: GithubRefTarget; href: string; children: React.ReactNode }) {
  const [card, setCard] = useState<Card | null>(null);
  const [open, setOpen] = useState(false);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const label = `${target.owner}/${target.repo}#${target.number}`;
  // The link's own text when it says something else ("this PR"); the
  // reference otherwise.
  const text = typeof children === 'string' && !children.startsWith('http') ? children : label;

  useEffect(() => {
    let live = true;
    void loadCard(target).then((c) => live && setCard(c));
    return () => {
      live = false;
    };
  }, [target.owner, target.repo, target.number]); // eslint-disable-line react-hooks/exhaustive-deps

  const show = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    setOpen(true);
  };
  const hide = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 150);
  };

  return (
    <Popover open={open && !!card} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          onMouseEnter={show}
          onMouseLeave={hide}
          className="not-prose inline-flex items-baseline gap-1 rounded-md bg-white/[0.06] px-1.5 py-px align-baseline text-[0.92em] font-medium text-mira-blue no-underline transition-colors hover:bg-white/[0.1]"
        >
          <StateIcon card={card} kind={target.kind} className={cn('size-3.5 shrink-0 translate-y-[2px]', iconTone(card))} />
          {text}
        </a>
      </PopoverTrigger>
      {card && (
        <PopoverContent
          side="top"
          align="start"
          sideOffset={6}
          onMouseEnter={show}
          onMouseLeave={hide}
          onOpenAutoFocus={(e) => e.preventDefault()}
          className="w-80 rounded-xl border border-border/60 bg-popover/95 p-3 backdrop-blur"
        >
          <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
            <span className={cn('inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11.5px] font-medium', STATE[card.state].className)}>
              <StateIcon card={card} kind={target.kind} className="size-3" />
              {STATE[card.state].label}
            </span>
            <span className="min-w-0 flex-1 truncate">
              {target.owner}/{target.repo} <span className="text-muted-foreground/70">#{card.number}</span>
            </span>
            <span className="shrink-0">{ago(card.created_at)}</span>
          </div>
          <div className="mt-2 text-[14px] font-semibold leading-snug text-foreground">{card.title}</div>
          <div className="mt-2.5 flex items-center gap-2 text-[12px]">
            {card.author_avatar && <img src={card.author_avatar} alt="" className="size-4 rounded-full" />}
            <span className="min-w-0 flex-1 truncate text-muted-foreground">{card.author}</span>
            {card.additions != null && (
              <span className="shrink-0 rounded-md bg-white/[0.05] px-1.5 py-0.5 font-mono text-[11px]">
                <span className="text-emerald-400">+{card.additions}</span>{' '}
                <span className="text-red-400">−{card.deletions ?? 0}</span>
              </span>
            )}
            {card.changed_files != null && (
              <span className="shrink-0 rounded-md bg-white/[0.05] px-1.5 py-0.5 text-[11px] text-muted-foreground">
                {card.changed_files} file{card.changed_files === 1 ? '' : 's'}
              </span>
            )}
          </div>
        </PopoverContent>
      )}
    </Popover>
  );
}
