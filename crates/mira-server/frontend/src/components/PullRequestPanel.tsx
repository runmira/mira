import { cn } from '@/lib/utils';
import { MessageCircle, Search } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import {
  listPullRequests,
  type PullRequestListView,
  type PullRequestSummary,
  type RepoGroup,
} from '../api';
import { PullRequestDetail } from './pullRequests/PullRequestDetail';
import { PrIcon, timeAgo } from './pullRequests/shared';
import type { Selection } from './pullRequests/types';
/** Codex-style pull-request panel. Two columns: PR list on the left,
 *  full detail on the right. Aggregates PRs across every project folder
 *  Mira knows about — the backend does the `git remote` parse + REST
 *  call. Auth is a `GITHUB_TOKEN` in Settings → Keys; when the token is
 *  absent the panel shows an inline nudge. */
type Filter = 'all' | 'reviewing' | 'authored';

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
      .then((v) => {
        setList(v);
        setError(null);
      })
      .catch((e) => setError(String((e as Error).message)))
      .finally(() => setLoading(false));
  }, [refreshTick]);

  const filtered = useMemo(() => filterAndSearch(list, filter, search), [list, filter, search]);

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

  const needsToken = !loading && !error && list && list.authenticated_user === null;

  return (
    <div
      className={cn(
        'grid flex-1 min-h-0 min-w-0 grid-rows-1 bg-background transition-[grid-template-columns] duration-150',
        expanded
          ? 'grid-cols-[0px_minmax(0,1fr)]'
          : 'grid-cols-[minmax(320px,360px)_minmax(0,1fr)]',
      )}
    >
      {/* left: list */}
      <div
        className={cn(
          'flex min-w-0 min-h-0 flex-col border-r border-border bg-fg/[0.03]',
          expanded && 'hidden',
        )}
      >
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
          {!loading &&
            !error &&
            filtered.map((g) => (
              <RepoSection
                key={`${g.owner}/${g.repo}`}
                group={g}
                activeNumber={
                  selection && selection.owner === g.owner && selection.repo === g.repo
                    ? selection.number
                    : null
                }
                onSelect={(pr) =>
                  setSelection({
                    owner: g.owner,
                    repo: g.repo,
                    number: pr.number,
                    project_label: g.project_label,
                  })
                }
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
                  <div className="font-medium">
                    {e.owner}/{e.repo}
                  </div>
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
      <Search className="size-3.5 shrink-0 text-muted-foreground" />
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
  return <div className="px-4 py-3 text-[12.5px] text-muted-foreground/70">{children}</div>;
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
        active ? 'border-mira-blue bg-accent/70' : 'border-transparent hover:bg-accent/30',
      )}
    >
      <PrIcon pr={pr} />
      <div className="min-w-0">
        <div className="truncate text-[13px] font-medium text-foreground">{pr.title}</div>
        <div className="flex items-center gap-1.5 truncate text-[11.5px] text-muted-foreground">
          <span className={cn('font-mono', pr.author === authUser ? 'text-mira-blue' : '')}>
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
            <MessageCircle className="size-3" fill="currentColor" />
            {pr.comment_count}
          </span>
        )}
      </div>
    </button>
  );
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
