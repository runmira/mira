import { queryOptions, useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { useSelector } from '@tanstack/react-store';
import { createWorktree, getSessionDiff, putCwd, renameSession, type SessionDiffView } from '../../api';
import { getPullRequest, getPullRequestFiles, mergePullRequest, type PullRequestDetailView } from '../../api/pullRequests';
import { sessionFetch } from '../../api/sessionScope';
import { issueToken, rememberIssue, searchIssues, type IssueOption } from '../../lib/issueMentions';
import { branchFor, branchOf, ciState, groupWorkItems, linkedIssue, prOf, prRef, projectRoot, reviewState, type WorkItem, type WorkMeta } from '../../lib/workItems';
import type { SessionSummary } from '../../types';
import { useAppRuntime } from '../RuntimeProvider';

/* Server state for the Work tab. Everything here is a query or a mutation:
 * work items are derived from the sessions resource plus each open PR's
 * detail, never copied into component state, so there's one source of
 * truth and nothing to keep in sync. Selection lives in the URL. */

type Runtime = ReturnType<typeof useAppRuntime>;
type PrRef = NonNullable<ReturnType<typeof prRef>>;

const EMPTY: SessionSummary[] = [];

export const workQueries = {
  pr: (runtime: Runtime, ref: PrRef) =>
    queryOptions({
      queryKey: runtime.resourceKey('work-pr', ref.owner, ref.repo, String(ref.number)),
      queryFn: () => getPullRequest(ref.owner, ref.repo, ref.number),
      staleTime: 60_000,
      retry: false,
      // Follow checks while they run; GitHub's rate limit rules out more.
      refetchInterval: (query) => (query.state.data && ciState(query.state.data.checks) === 'pending' ? 30_000 : false),
    }),
  prFiles: (runtime: Runtime, ref: PrRef) =>
    queryOptions({
      queryKey: runtime.resourceKey('work-pr-files', ref.owner, ref.repo, String(ref.number)),
      queryFn: () => getPullRequestFiles(ref.owner, ref.repo, ref.number),
      staleTime: 60_000,
      retry: false,
    }),
  /** Uncommitted changes in the branch's checkout, asked through one of
   *  its chats (the server resolves the folder from the session). */
  uncommitted: (runtime: Runtime, sessionId: string) =>
    queryOptions<SessionDiffView>({
      queryKey: runtime.resourceKey('work-diff', sessionId),
      queryFn: () => getSessionDiff(sessionId),
      staleTime: 15_000,
    }),
  /** Open issues in the chat's repo, for Start work. */
  issues: (runtime: Runtime, sessionId: string, query: string) =>
    queryOptions({
      queryKey: runtime.resourceKey('work-issues', sessionId, query),
      queryFn: ({ signal }) => searchIssues(query, sessionId, signal),
      staleTime: 60_000,
      retry: false,
    }),
  branch: (runtime: Runtime, sessionId: string, name: string) =>
    queryOptions<BranchCard | null>({
      queryKey: runtime.resourceKey('work-branch', sessionId, name),
      queryFn: async ({ signal }) => {
        const r = await sessionFetch(`/api/git/branch?name=${encodeURIComponent(name)}`, { signal }, sessionId);
        return r.ok ? ((await r.json()) as BranchCard) : null;
      },
      staleTime: 30_000,
    }),
};

export type BranchCard = {
  name: string;
  base: string | null;
  ahead: number;
  behind: number;
  last_subject: string;
  last_date: number;
};

/** What the sidebar needs from a PR's detail. */
function metaFrom(detail: PullRequestDetailView, branch: string | null): WorkMeta {
  const issue = linkedIssue(branch, detail.body);
  return {
    issue: issue ? { number: issue, title: '' } : null,
    ci: ciState(detail.checks),
    review: reviewState(detail.summary.review_decision, detail.reviews),
    diff: detail.summary.additions == null ? null : {
      added: detail.summary.additions,
      removed: detail.summary.deletions ?? 0,
      files: detail.summary.changed_files ?? 0,
    },
  };
}

/** The sessions list, kept fresh while Work is on screen: every 4s while
 *  a chat is running or waiting on you (attention isn't pushed, and the
 *  open chat's isn't polled anywhere else), every minute otherwise for PR
 *  changes. A second observer on the shared query, so the Chat side reads
 *  the same data and polling stops when Work unmounts. */
function useLiveSessions(): SessionSummary[] {
  const runtime = useAppRuntime();
  const sessions = useQuery({
    ...runtime.resources.options('sessions'),
    refetchInterval: (query) =>
      (query.state.data as SessionSummary[] | undefined)?.some((s) => s.running || s.needs_attention) ? 4_000 : 60_000,
  }).data ?? EMPTY;
  const running = useSelector(runtime.sessionActivity, (state) => state.activity?.running ?? null);
  return running ? sessions.map((s) => (s.running === running.has(s.id) ? s : { ...s, running: running.has(s.id) })) : sessions;
}

/** Every work item, shelved by state. Open PRs' checks and reviews come
 *  from their detail queries, shared with the item page's cache. */
export function useWorkItems(): WorkItem[] {
  const runtime = useAppRuntime();
  const sessions = useLiveSessions();
  const open = new Map<string, { ref: PrRef; branch: string | null }>();
  for (const s of sessions) {
    const pr = prOf(s);
    const ref = pr && (pr.state === 'open' || pr.state === 'draft') ? prRef(pr) : null;
    if (ref) open.set(`${projectRoot(s.cwd)}#${pr!.number}`, { ref, branch: branchOf(s) });
  }
  const entries = [...open];
  const meta = useQueries({
    queries: entries.map(([, { ref }]) => workQueries.pr(runtime, ref)),
    combine: (results) => {
      const out: Record<string, WorkMeta> = {};
      results.forEach((result, i) => {
        const [key, { branch }] = entries[i];
        if (result.data) out[key] = metaFrom(result.data, branch);
      });
      return out;
    },
  });
  return groupWorkItems(sessions, meta);
}

/** One item's page data: the PR's detail and files, or without a PR the
 *  branch and its uncommitted changes. */
export function useWorkDetail(item: WorkItem | null) {
  const runtime = useAppRuntime();
  const ref = item?.pr ? prRef(item.pr) : null;
  const lead = item?.sessions.find((s) => branchOf(s) === item.branch) ?? item?.sessions[0];
  const pr = useQuery({ ...workQueries.pr(runtime, ref ?? { owner: '', repo: '', number: 0 }), enabled: !!ref });
  const files = useQuery({ ...workQueries.prFiles(runtime, ref ?? { owner: '', repo: '', number: 0 }), enabled: !!ref });
  const uncommitted = useQuery({ ...workQueries.uncommitted(runtime, lead?.id ?? ''), enabled: !!lead && !ref });
  const branch = useQuery({ ...workQueries.branch(runtime, lead?.id ?? '', item?.branch ?? ''), enabled: !!lead && !!item?.branch });
  return { pr, files, uncommitted, branch };
}

export function useMergeWork(item: WorkItem | null) {
  const runtime = useAppRuntime();
  const client = useQueryClient();
  const ref = item?.pr ? prRef(item.pr) : null;
  return useMutation({
    mutationFn: () => {
      if (!ref) throw new Error('No pull request to merge');
      return mergePullRequest(ref.owner, ref.repo, ref.number, 'squash');
    },
    onSuccess: async () => {
      if (!ref) return;
      await client.invalidateQueries({ queryKey: workQueries.pr(runtime, ref).queryKey });
      // The PR's state rides on the sessions list.
      await runtime.resources.refresh('sessions').catch(() => undefined);
    },
  });
}

/** Start work on an issue: a worktree on `issue-N-…`, a chat in it, and
 *  the issue waiting in that chat's composer. `onOpened` attaches it. */
export function useStartWork(sessionId: string, onOpened: (sessionId: string) => void) {
  const runtime = useAppRuntime();
  return useMutation({
    mutationFn: async ({ issue, owner, repo }: { issue: IssueOption; owner: string; repo: string }) => {
      // Branch from the repo's default branch, not whatever the open chat
      // has checked out.
      const worktree = await createWorktree(branchFor(issue), await defaultBranch(sessionId), sessionId);
      const { session_id } = await putCwd(worktree.path, sessionId);
      if (!session_id) throw new Error('The worktree opened without a chat');
      // Name the chat for the issue, so the item reads as the issue at once.
      await renameSession(session_id, issue.title).catch(() => undefined);
      return { session: session_id, token: issueToken(owner, repo, issue.number), issue };
    },
    onSuccess: async ({ session, token, issue }) => {
      rememberIssue(token, issue);
      const key = runtime.drafts.key(session);
      runtime.drafts.update(key, { ...runtime.drafts.get(key), text: `Work on ${token}. ` }, true);
      onOpened(session);
      await runtime.resources.refresh('sessions').catch(() => undefined);
    },
  });
}

/** The repo's default branch (`origin/HEAD`, else main / master), as the
 *  server's branch card reports it. Undefined when it can't tell, which
 *  falls back to the checkout's HEAD. */
async function defaultBranch(sessionId: string): Promise<string | undefined> {
  const status = await sessionFetch('/api/git/status', undefined, sessionId)
    .then((r) => (r.ok ? (r.json() as Promise<{ branch?: string | null }>) : null))
    .catch(() => null);
  for (const name of [status?.branch, 'main', 'master']) {
    if (!name) continue;
    const r = await sessionFetch(`/api/git/branch?name=${encodeURIComponent(name)}`, undefined, sessionId).catch(() => null);
    const card = r?.ok ? ((await r.json()) as BranchCard) : null;
    if (card?.base) return card.base;
  }
  return undefined;
}
