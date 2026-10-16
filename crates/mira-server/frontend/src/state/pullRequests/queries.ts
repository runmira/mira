import { queryOptions, useQueryClient, type QueryClient } from '@tanstack/react-query';
import {
  getGithubKeyStatus,
  getPullRequest,
  getPullRequestFiles,
  listPullRequests,
  type PullRequestDetailView,
  type PullRequestSummary,
} from '../../api/pullRequests';
import { useOptionalRuntime } from '../RuntimeProvider';

/* Pull request data for every surface (the PR panel, its side pane, the
 * Work tab), behind one set of query keys so they share a cache: opening a
 * PR in one place makes it instant in the others. The server caches GitHub
 * too, so a refetch here is usually cheap. */

export type PrRef = { owner: string; repo: string; number: number };

type Runtime = NonNullable<ReturnType<typeof useOptionalRuntime>>;

/** Previews and tests render without a runtime; keys still need a scope. */
function scoped(runtime: Runtime | null, ...parts: string[]) {
  return runtime ? runtime.resourceKey(...(parts as [string, ...string[]])) : (['mira', 'local', ...parts] as const);
}

export const prQueries = {
  list: (runtime: Runtime | null, sessionId?: string) =>
    queryOptions({
      queryKey: scoped(runtime, 'pr-list', sessionId ?? ''),
      queryFn: () => listPullRequests(sessionId),
      staleTime: 30_000,
      retry: false,
    }),
  detail: (runtime: Runtime | null, ref: PrRef) =>
    queryOptions({
      queryKey: scoped(runtime, 'pr', ref.owner, ref.repo, String(ref.number)),
      queryFn: () => getPullRequest(ref.owner, ref.repo, ref.number),
      staleTime: 60_000,
      retry: false,
      // Follow checks while they run.
      refetchInterval: (query) =>
        query.state.data?.checks.some((c) => c.status !== 'completed') ? 30_000 : false,
    }),
  files: (runtime: Runtime | null, ref: PrRef) =>
    queryOptions({
      queryKey: scoped(runtime, 'pr-files', ref.owner, ref.repo, String(ref.number)),
      queryFn: () => getPullRequestFiles(ref.owner, ref.repo, ref.number),
      staleTime: 60_000,
      retry: false,
    }),
  /** Whether GitHub rejected the saved key. Keyed by the key as Settings
   *  shows it (masked), so replacing the key asks again. */
  githubKey: (runtime: Runtime | null, savedKey: string) =>
    queryOptions({
      queryKey: scoped(runtime, 'github-key', savedKey),
      queryFn: getGithubKeyStatus,
      staleTime: 5 * 60_000,
      retry: false,
    }),
};

/** What the list already knows about a PR, shaped as its detail, so the
 *  page can paint its header before the full detail arrives. */
export function detailFromSummary(summary: PullRequestSummary): PullRequestDetailView {
  return {
    summary,
    body: '',
    mergeable: null,
    mergeable_state: null,
    merged: false,
    check_status: null,
    checks: [],
    reviews: [],
    comments: [],
    timeline: [],
    commits: [],
  };
}

/** Start loading a PR's detail (hover, focus) so a click shows it at once. */
export function usePrefetchPr() {
  const runtime = useOptionalRuntime();
  const client = useQueryClient();
  return (ref: PrRef) => void client.prefetchQuery(prQueries.detail(runtime, ref));
}

/** After a mutation (comment, review, merge): this PR and the lists. */
export function invalidatePr(client: QueryClient, runtime: Runtime | null, ref: PrRef) {
  return Promise.all([
    client.invalidateQueries({ queryKey: prQueries.detail(runtime, ref).queryKey }),
    client.invalidateQueries({ queryKey: prQueries.files(runtime, ref).queryKey }),
    client.invalidateQueries({ queryKey: scoped(runtime, 'pr-list') }),
  ]);
}
