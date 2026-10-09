import { cn } from '@/lib/utils';
import { CircleCheck, CircleX, GitBranch, MessageCircle } from 'lucide-react';
import { type PullRequestSummary } from '../../api';
export function PrIcon({ pr }: { pr: PullRequestSummary }) {
  const color = pr.draft
    ? 'text-muted-foreground'
    : pr.state === 'closed'
      ? 'text-mira-purple'
      : 'text-emerald-500';
  return (
    <span className="mt-0.5 shrink-0" title={pr.draft ? 'Draft' : pr.state}>
      <GitBranch className={cn('size-3.5', color)} fill="currentColor" />
    </span>
  );
}

/* ---------- shared bits ---------- */

export function AvatarLike({
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

export function ReviewStateIcon({ state }: { state: string }) {
  switch (state) {
    case 'APPROVED':
      return <CircleCheck className="size-3 text-emerald-400" fill="currentColor" />;
    case 'CHANGES_REQUESTED':
      return <CircleX className="size-3 text-destructive" fill="currentColor" />;
    case 'COMMENTED':
      return <MessageCircle className="size-3 text-mira-blue" fill="currentColor" />;
    default:
      return null;
  }
}

export function reviewChipClass(state: string): string {
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

export function statusClass(status: string): string {
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

export function timeAgo(iso: string): string {
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
