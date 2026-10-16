import type { SessionDiffView } from '../api';
import type { PullRequestDetailView, PullRequestFilesView } from '../api/pullRequests';
import { ciState, type CiState, type ReviewState, type WorkItem } from './workItems';

/** Everything a work item's page shows beyond its chats, from the PR when
 *  it has one, else from the branch and its checkout. */
export type WorkDetail = {
  goal: string | null;
  plan: { text: string; done: boolean }[];
  files: { path: string; added: number; removed: number }[];
  /** `pr`: the PR's whole diff. `uncommitted`: only what's not committed
   *  yet — a branch with no PR has nothing better to compare. */
  filesSource: 'pr' | 'uncommitted';
  worktree: string | null;
  base: string | null;
  ahead: number | null;
  behind: number | null;
  checks: { name: string; state: CiState; url: string | null }[];
  reviewers: { name: string; state: ReviewState }[];
  timeline: { at: number; text: string; kind: TimelineKind }[];
  mergeable: boolean | null;
};

export type TimelineKind = 'issue' | 'chat' | 'agent' | 'commit' | 'pr' | 'ci' | 'review' | 'you';

export type BranchView = { base: string | null; ahead: number; behind: number };

const seconds = (iso: string | null) => (iso ? Math.floor(Date.parse(iso) / 1000) : null);

export function workDetail(
  item: WorkItem,
  sources: { pr?: PullRequestDetailView | null; prFiles?: PullRequestFilesView | null; uncommitted?: SessionDiffView | null; branch?: BranchView | null },
): WorkDetail {
  const { pr, prFiles, uncommitted, branch } = sources;
  const lead = item.sessions[0];
  const body = pr?.body?.trim();
  const reviewers = new Map<string, ReviewState>();
  for (const name of pr?.summary.requested_reviewers ?? []) reviewers.set(name, 'pending');
  for (const r of pr?.reviews ?? []) {
    if (r.state === 'APPROVED') reviewers.set(r.author, 'approved');
    else if (r.state === 'CHANGES_REQUESTED') reviewers.set(r.author, 'changes-requested');
  }
  const timeline: WorkDetail['timeline'] = item.sessions.map((s) => ({
    at: s.created_at,
    text: s.forked_from ? `Forked “${s.title || 'a chat'}”` : s.launched_by ? `Launched “${s.title || 'a thread'}”` : `Chat started: “${s.title || 'Untitled'}”`,
    kind: s.forked_from ? 'you' : 'chat',
  }));
  for (const e of pr?.timeline ?? []) {
    const at = seconds(e.at);
    if (at == null) continue;
    const kind = e.kind.toLowerCase();
    timeline.push({
      at,
      // Events like `merged` carry no message: say what happened instead.
      text: [e.actor, plainLine(e.message) || e.kind.replace(/[_-]+/g, ' ')].filter(Boolean).join(' '),
      kind: kind.includes('review') ? 'review' : kind.includes('commit') ? 'commit' : kind.includes('check') ? 'ci' : 'pr',
    });
  }
  return {
    goal: body || lead?.first_user_message?.trim() || null,
    plan: [],
    files: prFiles
      ? prFiles.files.map((f) => ({ path: f.filename, added: f.additions, removed: f.deletions }))
      : (uncommitted?.files ?? []).map((f) => ({ path: f.path, added: f.added, removed: f.removed })),
    filesSource: prFiles ? 'pr' : 'uncommitted',
    worktree: item.worktree,
    base: pr?.summary.base_ref ?? branch?.base ?? null,
    ahead: branch?.ahead ?? null,
    behind: branch?.behind ?? null,
    checks: (pr?.checks ?? []).map((c) => ({ name: c.name, state: ciState([c]) ?? 'pending', url: c.url })),
    reviewers: [...reviewers].map(([name, state]) => ({ name, state })),
    timeline: timeline.sort((a, b) => b.at - a.at).slice(0, 12),
    mergeable: pr?.mergeable ?? null,
  };
}

export function diffTotals(detail: Pick<WorkDetail, 'files'> | undefined): { added: number; removed: number; files: number } {
  const files = detail?.files ?? [];
  return {
    added: files.reduce((n, f) => n + f.added, 0),
    removed: files.reduce((n, f) => n + f.removed, 0),
    files: files.length,
  };
}

/** One readable line from a markdown comment, for the activity list: its
 *  first non-empty line without markup, links reduced to their text. */
export function plainLine(markdown: string): string {
  const line = markdown.split('\n').map((l) => l.trim()).find(Boolean) ?? '';
  return line
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/<[^>]+>/g, '')
    .replace(/^#{1,6}\s+|^>\s*|^[-*+]\s+/, '')
    .replace(/(\*\*|__|\*|_|~~|`)(.+?)\1/g, '$2')
    .replace(/:[a-z0-9_+-]+:/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}
