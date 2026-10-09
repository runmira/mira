/* ---------- pull requests ---------- */

export type PullRequestSummary = {
  number: number;
  title: string;
  state: string;
  draft: boolean;
  author: string;
  author_avatar: string | null;
  head_ref: string;
  base_ref: string;
  html_url: string;
  created_at: string;
  updated_at: string;
  additions: number | null;
  deletions: number | null;
  changed_files: number | null;
  review_decision: string | null;
  comment_count: number;
  requested_reviewers: string[];
};

export type RepoGroup = {
  owner: string;
  repo: string;
  project_label: string;
  cwd: string;
  prs: PullRequestSummary[];
};

export type RepoError = {
  owner: string;
  repo: string;
  cwd: string;
  message: string;
};

export type PullRequestListView = {
  authenticated_user: string | null;
  repos: RepoGroup[];
  errors: RepoError[];
};

export type CheckRunView = {
  name: string;
  status: string;
  conclusion: string | null;
  url: string | null;
};

export type ReviewView = {
  author: string;
  author_avatar: string | null;
  state: string;
  body: string;
  submitted_at: string | null;
};

export type CommentView = {
  author: string;
  author_avatar: string | null;
  body: string;
  created_at: string;
};

export type TimelineEventView = {
  kind: string;
  actor: string | null;
  message: string;
  at: string | null;
};

export type CommitView = {
  sha: string;
  message: string;
  author: string | null;
};

export type PullRequestDetailView = {
  summary: PullRequestSummary;
  body: string;
  mergeable: boolean | null;
  mergeable_state: string | null;
  merged: boolean;
  check_status: string | null;
  checks: CheckRunView[];
  reviews: ReviewView[];
  comments: CommentView[];
  timeline: TimelineEventView[];
  commits: CommitView[];
};

export type FileChangeView = {
  filename: string;
  status: string;
  additions: number;
  deletions: number;
  patch: string | null;
  raw_url: string | null;
};

export type PullRequestFilesView = { files: FileChangeView[] };

export async function readError(r: Response, prefix: string): Promise<string> {
  let msg = `${prefix} ${r.status}`;
  try {
    const j = await r.json();
    if (j?.error) msg += `: ${j.error}`;
  } catch {
    /* ignore */
  }
  return msg;
}

export async function listPullRequests(): Promise<PullRequestListView> {
  const r = await fetch('/api/prs', { cache: 'no-store' });
  if (!r.ok) throw new Error(await readError(r, 'prs GET'));
  return (await r.json()) as PullRequestListView;
}

export async function getPullRequest(
  owner: string,
  repo: string,
  number: number,
): Promise<PullRequestDetailView> {
  const r = await fetch(
    `/api/prs/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${number}`,
    { cache: 'no-store' },
  );
  if (!r.ok) throw new Error(await readError(r, 'pr GET'));
  return (await r.json()) as PullRequestDetailView;
}

export async function getPullRequestFiles(
  owner: string,
  repo: string,
  number: number,
): Promise<PullRequestFilesView> {
  const r = await fetch(
    `/api/prs/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${number}/files`,
    { cache: 'no-store' },
  );
  if (!r.ok) throw new Error(await readError(r, 'pr files GET'));
  return (await r.json()) as PullRequestFilesView;
}

export async function postPullRequestComment(
  owner: string,
  repo: string,
  number: number,
  body: string,
): Promise<CommentView> {
  const r = await fetch(
    `/api/prs/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${number}/comments`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ body }),
    },
  );
  if (!r.ok) throw new Error(await readError(r, 'comment POST'));
  return (await r.json()) as CommentView;
}

export type ReviewEvent = 'APPROVE' | 'REQUEST_CHANGES' | 'COMMENT';

export async function submitPullRequestReview(
  owner: string,
  repo: string,
  number: number,
  event: ReviewEvent,
  body?: string,
): Promise<ReviewView> {
  const r = await fetch(
    `/api/prs/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${number}/reviews`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ event, body }),
    },
  );
  if (!r.ok) throw new Error(await readError(r, 'review POST'));
  return (await r.json()) as ReviewView;
}

export type MergeMethod = 'merge' | 'squash' | 'rebase';

export async function mergePullRequest(
  owner: string,
  repo: string,
  number: number,
  method: MergeMethod,
): Promise<void> {
  const r = await fetch(
    `/api/prs/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${number}/merge`,
    {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ method }),
    },
  );
  if (!r.ok) throw new Error(await readError(r, 'merge PUT'));
}
