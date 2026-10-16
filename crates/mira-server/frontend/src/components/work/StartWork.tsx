import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { CircleDot, GitBranch, Loader, Search } from 'lucide-react';
import { useState } from 'react';
import { issueTone, type IssueOption } from '../../lib/issueMentions';
import { branchFor } from '../../lib/workItems';

/** Start work from an issue: pick one (the composer's `#` picker look),
 *  see the branch it gets, start. The caller owns the search and the
 *  start; this only holds what's highlighted. */
export function StartWork({
  open,
  onOpenChange,
  repo,
  query,
  onQueryChange,
  issues,
  loading,
  error,
  starting,
  startError,
  onStart,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  repo: string | null;
  query: string;
  onQueryChange: (query: string) => void;
  issues: IssueOption[];
  loading: boolean;
  error: string | null;
  /** The worktree and chat are being made. */
  starting: boolean;
  startError: string | null;
  onStart: (issue: IssueOption) => void;
}) {
  const [activeIdx, setActiveIdx] = useState(0);
  const items = issues.filter((i) => i.kind === 'issue');
  const picked = items[Math.min(activeIdx, items.length - 1)];
  const start = (issue: IssueOption | undefined) => {
    if (issue && !starting) onStart(issue);
  };
  const q = query.trim().replace(/^#/, '');
  return (
    <Dialog open={open} onOpenChange={(next) => !starting && onOpenChange(next)}>
      <DialogContent showClose={false} className="max-w-xl gap-0 overflow-hidden p-0">
        <DialogTitle className="sr-only">Start work from an issue</DialogTitle>
        <label className="flex items-center gap-2 border-b border-fg/[0.06] px-4 py-3">
          <Search className="size-4 shrink-0 text-muted-foreground/70" />
          <input
            autoFocus
            value={query}
            disabled={starting}
            onChange={(e) => {
              onQueryChange(e.target.value);
              setActiveIdx(0);
            }}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown') {
                e.preventDefault();
                setActiveIdx((i) => Math.min(i + 1, items.length - 1));
              } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                setActiveIdx((i) => Math.max(i - 1, 0));
              } else if (e.key === 'Enter') {
                e.preventDefault();
                start(picked);
              }
            }}
            placeholder="Start work from an issue…"
            className="min-w-0 flex-1 bg-transparent text-[14px] text-foreground outline-none placeholder:text-muted-foreground/60"
          />
          {loading && items.length > 0 && <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-mira-blue/70" aria-hidden />}
        </label>
        <div className="flex items-center gap-2 px-4 pb-1 pt-2.5 text-[12px]">
          <CircleDot className="size-3.5 shrink-0 text-muted-foreground/70" />
          <span className="truncate text-foreground/80">{repo ?? 'Issues'}</span>
          <span className="truncate text-muted-foreground/50">{q ? (/^\d+$/.test(q) ? `#${q}` : `matching “${q}”`) : 'Open issues'}</span>
        </div>
        <div className="max-h-[18rem] overflow-y-auto pb-1.5" role="listbox" aria-label="Issues">
          {items.length === 0 && loading && <SkeletonRows />}
          {items.length === 0 && !loading && (
            <div className="px-4 py-3 text-[12.5px] text-muted-foreground/80">{error ?? (q ? 'No open issues match.' : 'No open issues.')}</div>
          )}
          {items.map((option, i) => {
            const active = option === picked;
            return (
              <button
                key={option.number}
                type="button"
                role="option"
                aria-selected={active}
                disabled={starting}
                onMouseEnter={() => setActiveIdx(i)}
                onClick={() => start(option)}
                className={cn('flex w-full items-center gap-2.5 px-4 py-1.5 text-left transition-colors', active ? 'bg-fg/[0.06]' : 'hover:bg-fg/[0.035]')}
              >
                <CircleDot className={cn('size-[15px] shrink-0', issueTone(option.state))} />
                <span className="w-11 shrink-0 font-mono text-[12px] tabular-nums text-muted-foreground/80">#{option.number}</span>
                <span className={cn('min-w-0 flex-1 truncate text-[13.5px] tracking-tight', active ? 'text-foreground' : 'text-foreground/90')}>{option.title}</span>
                {option.labels.slice(0, 2).map((label) => (
                  <span key={label.name} className="hidden max-w-[7rem] shrink-0 items-center gap-1 truncate rounded-full border border-fg/[0.08] px-1.5 text-[10.5px] text-muted-foreground sm:inline-flex">
                    <span className="size-1.5 shrink-0 rounded-full" style={{ backgroundColor: /^[0-9a-f]{6}$/i.test(label.color) ? `#${label.color}` : undefined }} />
                    <span className="truncate">{label.name}</span>
                  </span>
                ))}
              </button>
            );
          })}
        </div>
        <div className="flex items-center gap-3 border-t border-fg/[0.06] px-4 py-2.5 text-[12px]">
          {startError ? (
            <span className="min-w-0 flex-1 truncate text-red-600 dark:text-red-400" title={startError}>{startError}</span>
          ) : picked ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-muted-foreground">
              <GitBranch className="size-3.5 shrink-0" />
              <span className="truncate font-mono text-[11.5px] text-foreground/80">{branchFor(picked)}</span>
              <span className="shrink-0">in a new worktree</span>
            </span>
          ) : (
            <span className="flex-1" />
          )}
          <button
            type="button"
            disabled={!picked || starting}
            onClick={() => start(picked)}
            className="inline-flex items-center gap-1.5 rounded-full bg-foreground px-3 py-1 text-[12px] font-medium text-background transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            {starting && <Loader className="size-3 animate-spin" />}
            {starting ? 'Starting…' : 'Start'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SkeletonRows() {
  return (
    <div aria-hidden>
      {[68, 52, 60].map((w) => (
        <div key={w} className="flex items-center gap-2.5 px-4 py-2">
          <span className="size-[15px] shrink-0 animate-pulse rounded-full bg-fg/[0.08]" />
          <span className="h-2.5 w-9 shrink-0 animate-pulse rounded bg-fg/[0.06]" />
          <span className="h-2.5 animate-pulse rounded bg-fg/[0.06]" style={{ width: `${w}%` }} />
        </div>
      ))}
    </div>
  );
}
