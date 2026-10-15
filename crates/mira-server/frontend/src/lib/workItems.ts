import type { SessionPr, SessionSummary } from '../types';

/** The sidebar's Work tab: chats grouped by what they deliver rather than by
 *  conversation. A work item is one PR, or one worktree branch before it has
 *  a PR; every chat on that branch belongs to it, and so do forks and
 *  launched threads that have no branch of their own (they work in their
 *  parent's). Chats with no branch anywhere up their tree aren't work items. */

export type WorkState = 'needs-you' | 'in-progress' | 'review' | 'ready' | 'done';

export const WORK_STATES: WorkState[] = ['needs-you', 'in-progress', 'review', 'ready', 'done'];

export type CiState = 'passing' | 'failing' | 'pending';
export type ReviewState = 'approved' | 'changes-requested' | 'pending';

/** What the server doesn't know yet about a work item (the prototype fakes
 *  it): the issue it started from and the PR's checks and review. */
export type WorkMeta = {
  issue?: { number: number; title: string } | null;
  ci?: CiState | null;
  review?: ReviewState | null;
};

export type WorkItem = {
  key: string;
  cwd: string;
  branch: string | null;
  pr: SessionPr | null;
  /** Oldest first: the chat that started the work leads. */
  sessions: SessionSummary[];
  title: string;
  /** Agent drivers that touched it, in first-seen order; `null` is Mira's own. */
  agents: (string | null)[];
  state: WorkState;
  /** Why it's waiting on you, when it is. */
  reason: string | null;
  updated_at: number;
  meta: WorkMeta;
};

function ownKey(s: SessionSummary): string | null {
  if (s.pr) return `${s.cwd}#${s.pr.number}`;
  if (s.worktree_branch) return `${s.cwd}@${s.worktree_branch}`;
  return null;
}

/** The work item a chat belongs to: its own branch's, else the nearest
 *  parent's (fork or launcher) that has one. */
function itemKey(s: SessionSummary, byId: Map<string, SessionSummary>): string | null {
  const seen = new Set<string>();
  let at: SessionSummary | undefined = s;
  while (at && !seen.has(at.id)) {
    seen.add(at.id);
    const key = ownKey(at);
    if (key) return key;
    const up: string | null | undefined = at.forked_from ?? at.launched_by;
    at = up ? byId.get(up) : undefined;
  }
  return null;
}

export function workState(sessions: SessionSummary[], pr: SessionPr | null, meta: WorkMeta = {}): { state: WorkState; reason: string | null } {
  const waiting = sessions.find((s) => s.needs_attention);
  if (waiting) return { state: 'needs-you', reason: waiting.attention_reason || 'Waiting for you' };
  if (sessions.some((s) => s.failure_reason)) return { state: 'needs-you', reason: 'Needs a retry' };
  if (pr?.state === 'merged' || pr?.state === 'closed') return { state: 'done', reason: null };
  if (!pr && sessions.some((s) => s.worktree_status === 'merged')) return { state: 'done', reason: null };
  if (sessions.length > 0 && sessions.every((s) => s.settled_at != null && s.updated_at <= s.settled_at)) {
    return { state: 'done', reason: null };
  }
  if (sessions.some((s) => s.running)) return { state: 'in-progress', reason: null };
  if (pr?.state === 'open') {
    if (meta.ci === 'failing') return { state: 'needs-you', reason: 'Checks failing' };
    if (meta.review === 'changes-requested') return { state: 'needs-you', reason: 'Changes requested' };
    if (meta.ci === 'passing' && meta.review === 'approved') return { state: 'ready', reason: null };
    return { state: 'review', reason: null };
  }
  // A draft PR, or a branch with no PR, and nobody working on it.
  return { state: 'in-progress', reason: null };
}

export function groupWorkItems(sessions: SessionSummary[], meta: Record<string, WorkMeta> = {}): WorkItem[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const buckets = new Map<string, SessionSummary[]>();
  for (const s of sessions) {
    if (s.archived) continue;
    const key = itemKey(s, byId);
    if (!key) continue;
    const list = buckets.get(key) ?? [];
    list.push(s);
    buckets.set(key, list);
  }
  const items: WorkItem[] = [];
  buckets.forEach((list, key) => {
    list.sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));
    const owner = list.find((s) => ownKey(s) === key) ?? list[0];
    const pr = list.find((s) => s.pr)?.pr ?? null;
    const m = meta[key] ?? {};
    const agents: (string | null)[] = [];
    for (const s of list) {
      const a = s.agent_driver ?? null;
      if (!agents.includes(a)) agents.push(a);
    }
    items.push({
      key,
      cwd: owner.cwd,
      branch: owner.worktree_branch ?? null,
      pr,
      sessions: list,
      title: m.issue?.title || pr?.title || owner.title || owner.worktree_branch || 'Untitled',
      agents,
      ...workState(list, pr, m),
      updated_at: Math.max(...list.map((s) => s.updated_at)),
      meta: m,
    });
  });
  return items.sort((a, b) => b.updated_at - a.updated_at || a.key.localeCompare(b.key));
}

/** Items per state, in shelf order; every state is present. */
export function workShelves(items: WorkItem[]): [WorkState, WorkItem[]][] {
  return WORK_STATES.map((state) => [state, items.filter((i) => i.state === state)]);
}
