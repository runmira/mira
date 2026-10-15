import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { CircleDot, GitMerge, GitPullRequest, GitPullRequestClosed, GitPullRequestDraft } from 'lucide-react';
import { cn } from '@/lib/utils';
import { useAppRuntime } from '../../state/RuntimeProvider';
import {
  issueToken,
  issueTone,
  issueTrigger,
  rememberIssue,
  searchIssues,
  type IssueOption,
} from '../../lib/issueMentions';

/* ---------- `#` issue picker ---------- */

/** Wait this long after the last keystroke before asking GitHub, whose
 *  search API allows 30 requests a minute. */
const DEBOUNCE_MS = 160;

/** `value` once it has held still for `ms`; `null` until then. */
function useDebounced<T>(value: T, ms: number): T | null {
  const [settled, setSettled] = useState<T | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

/**
 * State for the composer's `#` picker: watches the text for a trailing
 * `#query`, searches the chat repo's issues and PRs, and turns a pick into
 * an `owner/repo#N` chip. `onKeyDown` returns true when it used the key.
 */
export function useIssuePicker({
  text,
  enabled,
  sessionId,
  updateText,
}: {
  text: string;
  enabled: boolean;
  sessionId?: string;
  updateText: (next: string) => void;
}) {
  const runtime = useAppRuntime();
  const trigger = enabled ? issueTrigger(text) : null;
  const query = trigger?.query ?? null;
  const settled = useDebounced(query, DEBOUNCE_MS);
  const search = useQuery({
    queryKey: runtime.resourceKey('issues', sessionId ?? '', settled ?? ''),
    queryFn: ({ signal }) => searchIssues(settled ?? '', sessionId, signal),
    enabled: settled !== null && settled === query,
    staleTime: 60_000,
    retry: false,
    // Keep the last rows on screen while the next query loads.
    placeholderData: keepPreviousData,
  });
  const result = query === null ? null : search.data ?? null;
  const loading = query !== null && (settled !== query || search.isFetching);
  const error = search.error ? search.error.message : null;

  const [activeIdx, setActiveIdx] = useState(0);
  // Escape hides the picker for this `#` without deleting what was typed.
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  // Derived-state resets, done during render rather than in effects.
  const [seenQuery, setSeenQuery] = useState(query);
  if (seenQuery !== query) {
    setSeenQuery(query);
    setActiveIdx(0);
    if (query === null) setDismissedAt(null);
  }

  const open = !!trigger && trigger.triggerStart !== dismissedAt;
  const items = result?.items ?? [];

  function pick(option: IssueOption) {
    if (!trigger || !result?.owner || !result.repo) return;
    const token = issueToken(result.owner, result.repo, option.number);
    rememberIssue(token, option);
    updateText(`${text.slice(0, trigger.triggerStart)}${token} `);
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>): boolean {
    if (!open) return false;
    if (e.key === 'Escape') {
      e.preventDefault();
      setDismissedAt(trigger!.triggerStart);
      return true;
    }
    // With nothing to pick, keys keep their usual meaning (Enter sends).
    if (items.length === 0) return false;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIdx((i) => Math.min(i + 1, items.length - 1));
      return true;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIdx((i) => Math.max(i - 1, 0));
      return true;
    }
    if ((e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey) || e.key === 'Tab') {
      e.preventDefault();
      const option = items[activeIdx];
      if (option) pick(option);
      return true;
    }
    return false;
  }

  return { open, query, result, loading, error, activeIdx, setActiveIdx, pick, onKeyDown };
}

export type IssuePicker = ReturnType<typeof useIssuePicker>;

export function IssuePalette({ picker }: { picker: IssuePicker }) {
  const { result, loading, error, activeIdx, setActiveIdx, pick, query } = picker;
  const activeRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: 'nearest' });
  }, [activeIdx]);
  const items = result?.items ?? [];
  const problem = error ?? result?.error ?? null;
  const repo = result?.owner && result.repo ? `${result.owner}/${result.repo}` : null;

  return (
    <div
      className={cn(
        'absolute bottom-full left-0 right-0 z-10 mb-2 mx-auto max-w-xl overflow-hidden',
        'rounded-2xl border border-fg/[0.07] bg-popover/95 dark:bg-[#1f2024]/95 backdrop-blur-md',
        'shadow-[0_20px_50px_-16px_rgba(0,0,0,0.85)] ring-1 ring-shade/40',
        'animate-fade-in',
      )}
      role="listbox"
      aria-label="Issues and pull requests"
    >
      <div className="flex items-center gap-2 px-4 pb-1 pt-2.5 text-[12px]">
        <CircleDot className="size-3.5 shrink-0 text-muted-foreground/70" />
        <span className="truncate text-foreground/80">{repo ?? 'Issues'}</span>
        <span className="truncate text-muted-foreground/50">
          {query ? (/^\d+$/.test(query) ? `#${query}` : `matching “${query}”`) : 'Recently updated'}
        </span>
        {loading && items.length > 0 && (
          <span className="ml-auto size-1.5 shrink-0 animate-pulse rounded-full bg-mira-blue/70" aria-hidden />
        )}
      </div>
      <div className="max-h-[22rem] overflow-y-auto pb-1.5">
        {items.length === 0 && loading && <SkeletonRows />}
        {items.length === 0 && !loading && (
          <div className="px-4 py-3 text-[12.5px] text-muted-foreground/80">
            {problem ?? (query ? 'No issues or pull requests match.' : 'No open issues or pull requests.')}
          </div>
        )}
        {items.map((option, i) => {
          const active = i === activeIdx;
          return (
            <button
              key={`${option.kind}:${option.number}`}
              ref={active ? activeRef : undefined}
              type="button"
              role="option"
              aria-selected={active}
              onMouseEnter={() => setActiveIdx(i)}
              // Keep focus (and the caret) in the composer.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pick(option)}
              className={cn(
                'group flex w-full items-center gap-2.5 px-4 py-1.5 text-left transition-colors',
                active ? 'bg-fg/[0.06]' : 'hover:bg-fg/[0.035]',
              )}
            >
              <StateIcon option={option} className={cn('size-[15px] shrink-0', issueTone(option.state))} />
              <span className="w-11 shrink-0 font-mono text-[12px] tabular-nums text-muted-foreground/80">
                #{option.number}
              </span>
              <span
                className={cn(
                  'min-w-0 flex-1 truncate text-[13.5px] tracking-tight',
                  active ? 'text-foreground' : 'text-foreground/90',
                )}
              >
                {option.title}
              </span>
              {option.labels.slice(0, 2).map((label) => (
                <span
                  key={label.name}
                  className="hidden max-w-[7rem] shrink-0 items-center gap-1 truncate rounded-full border border-fg/[0.08] px-1.5 text-[10.5px] text-muted-foreground sm:inline-flex"
                >
                  <span
                    className="size-1.5 shrink-0 rounded-full"
                    style={{ backgroundColor: /^[0-9a-f]{6}$/i.test(label.color) ? `#${label.color}` : undefined }}
                  />
                  <span className="truncate">{label.name}</span>
                </span>
              ))}
              <span className="w-12 shrink-0 text-right text-[11.5px] text-muted-foreground/60">
                {ago(option.updated_at)}
              </span>
            </button>
          );
        })}
      </div>
      {items.length > 0 && (
        <div className="flex items-center gap-3 border-t border-fg/[0.05] px-4 py-1.5 text-[11px] text-muted-foreground/60 max-md:hidden">
          <span><Kbd>↑</Kbd><Kbd>↓</Kbd> navigate</span>
          <span><Kbd>↵</Kbd> insert</span>
          <span><Kbd>esc</Kbd> dismiss</span>
        </div>
      )}
    </div>
  );
}

function StateIcon({ option, className }: { option: IssueOption; className?: string }) {
  if (option.kind === 'issue') return <CircleDot className={className} />;
  switch (option.state) {
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

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="mr-0.5 inline-flex min-w-4 items-center justify-center rounded border border-fg/[0.1] px-1 font-sans text-[10px] text-muted-foreground/80">
      {children}
    </kbd>
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

function ago(iso: string | null): string {
  if (!iso) return '';
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m`;
  if (s < 86400) return `${Math.round(s / 3600)}h`;
  if (s < 86400 * 30) return `${Math.round(s / 86400)}d`;
  return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}
