import type { SessionSummary, SettingsUpdate, SettingsView } from './types';

export async function getSettings(): Promise<SettingsView> {
  const r = await fetch('/api/settings');
  if (!r.ok) throw new Error(`settings GET ${r.status}`);
  return (await r.json()) as SettingsView;
}

export async function putSettings(update: SettingsUpdate): Promise<SettingsView> {
  const r = await fetch('/api/settings', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(update),
  });
  if (!r.ok) {
    let msg = `settings PUT ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as SettingsView;
}

export async function listSessions(opts: { all?: boolean } = {}): Promise<SessionSummary[]> {
  // axum's Query bool deserializer expects the literal string `true`, not `1`.
  // `no-store` + a cache-busting param defeat any browser/HTTP caching so
  // fetches triggered by session_title_updated actually see fresh titles
  // rather than a stale cached list.
  const params = new URLSearchParams();
  if (opts.all) params.set('all', 'true');
  params.set('_ts', String(Date.now()));
  const r = await fetch(`/api/sessions?${params.toString()}`, { cache: 'no-store' });
  if (!r.ok) throw new Error(`sessions GET ${r.status}`);
  return (await r.json()) as SessionSummary[];
}

export type SessionHistoryView = {
  id: string;
  model: string;
  cwd: string;
  title: string | null;
  created_at: number;
  updated_at: number;
  messages: import('./types').Message[];
};

/** Read-only lookup — returns messages without swapping the active
 *  session. Used by the SubagentPanel to rebuild a child transcript
 *  after a browser reload. */
export async function getSessionHistory(id: string): Promise<SessionHistoryView> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}/history`, {
    cache: 'no-store',
  });
  if (!r.ok) {
    let msg = `session history ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as SessionHistoryView;
}

export async function loadSession(id: string): Promise<void> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}/load`, { method: 'POST' });
  if (!r.ok) {
    let msg = `sessions load ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
}

export async function newSession(): Promise<void> {
  const r = await fetch('/api/sessions/new', { method: 'POST' });
  if (!r.ok) throw new Error(`sessions new ${r.status}`);
}

export type ReviewStartArgs = {
  range?: string;
  diff?: string;
  pr?: number;
  /** Optional owner/repo pair for the REST-based PR diff fetch. Lets
   *  "Review with Mira" work on any repo Mira knows about without
   *  having to swap the session cwd. Falls back to `gh pr diff` when
   *  absent. */
  owner?: string;
  repo?: string;
  no_verify?: boolean;
};

export async function startReview(args: ReviewStartArgs): Promise<{ run_id: string }> {
  const r = await fetch('/api/review', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(args),
  });
  if (!r.ok) {
    let msg = `review POST ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as { run_id: string };
}

export async function deleteSession(id: string): Promise<void> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' });
  if (!r.ok) {
    let msg = `sessions delete ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
}

/** Manual rename: set the session's title to `title`. Empty clears it. */
export async function renameSession(id: string, title: string): Promise<{ id: string; title: string }> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}/title`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title }),
  });
  if (!r.ok) {
    let msg = `rename ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as { id: string; title: string };
}

/** AI rename: server runs the extractor on the session's first user + assistant
 *  messages and installs the result. Errors if the session doesn't have both. */
export async function regenerateSessionTitle(id: string): Promise<{ id: string; title: string }> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}/title/regenerate`, {
    method: 'POST',
  });
  if (!r.ok) {
    let msg = `rename (ai) ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as { id: string; title: string };
}

export type BrowseView = {
  path: string;
  parent: string | null;
  home: string | null;
  entries: { name: string; path: string; is_dir: boolean }[];
  truncated: boolean;
};

export async function browse(path?: string, showHidden = false): Promise<BrowseView> {
  const params = new URLSearchParams();
  if (path) params.set('path', path);
  if (showHidden) params.set('show_hidden', 'true');
  const r = await fetch(`/api/browse?${params.toString()}`);
  if (!r.ok) {
    let msg = `browse ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as BrowseView;
}

export type ModelInfo = {
  id: string;
  display_name?: string | null;
  owned_by?: string | null;
  context_length?: number | null;
};

export type ModelListView = { models: ModelInfo[]; cached: boolean };

export async function listModels(): Promise<ModelListView> {
  const r = await fetch('/api/models');
  if (!r.ok) {
    // Common case: provider misconfigured or endpoint doesn't support /models.
    // Return an empty list so the UI falls back to a text input.
    return { models: [], cached: false };
  }
  return (await r.json()) as ModelListView;
}

export type SkillView = {
  name: string;
  description: string;
  category?: string;
  /** Icon name from the skill's frontmatter (kebab-case) — the UI maps
   *  a curated set to Phosphor components. Unknown values fall back to
   *  a sparkle default. */
  icon?: string;
  /** Tailwind-flavored color name (`emerald`, `blue`, …). Frontend
   *  maps to a preset palette; unknown values fall back to a stable
   *  hash-derived tint. */
  color?: string;
  tier: 'bundled' | 'user' | 'project';
  has_attachments: boolean;
};

export type SkillsResponse = { skills: SkillView[] };

/** Loaded skill roster — bundled + user (~/.mira/skills) + project
 *  (<cwd>/.mira/skills) merged. Powers dynamic `/<skill-name>` slash
 *  commands in the composer palette. Returns an empty list on error
 *  (skills are additive; a missing roster shouldn't break the palette). */
export async function listSkills(): Promise<SkillView[]> {
  try {
    const r = await fetch('/api/skills');
    if (!r.ok) return [];
    const body = (await r.json()) as SkillsResponse;
    return body.skills;
  } catch {
    return [];
  }
}

/** Re-read `~/.mira/skills/` and `<cwd>/.mira/skills/` off disk and swap
 *  the in-process registry. Returns the fresh roster in the same round-
 *  trip so the caller doesn't need a follow-up `listSkills()`. */
export async function reloadSkills(): Promise<SkillView[]> {
  try {
    const r = await fetch('/api/skills/reload', { method: 'POST' });
    if (!r.ok) return [];
    const body = (await r.json()) as SkillsResponse;
    return body.skills;
  } catch {
    return [];
  }
}

export type FileView = { path: string; bytes: number; content: string };

export async function readFile(path: string): Promise<FileView> {
  const r = await fetch(`/api/file?path=${encodeURIComponent(path)}`);
  if (!r.ok) {
    let msg = `file GET ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as FileView;
}

export type WorktreeEntry = { path: string; branch: string | null; is_current: boolean };

export type GitStatusView = {
  in_repo: boolean;
  branch?: string | null;
  dirty: boolean;
  ahead: number;
  behind: number;
  is_worktree: boolean;
  /** Basename of the primary worktree — e.g. `mira` even when the current
   *  cwd is `mira/.mira/worktrees/diff-tes`. Absent when not in a repo. */
  primary_project?: string | null;
  worktrees: WorktreeEntry[];
};

export async function getGitStatus(): Promise<GitStatusView> {
  const r = await fetch('/api/git/status');
  if (!r.ok) throw new Error(`git status ${r.status}`);
  return (await r.json()) as GitStatusView;
}

export type UndoOp = 'overwrite' | 'create';
export type AppliedUndo = { seq: number; path: string; op: UndoOp };

export async function applyUndo(count: number): Promise<{ applied: AppliedUndo[] }> {
  const r = await fetch('/api/undo', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ count }),
  });
  if (!r.ok) {
    let msg = `undo POST ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as { applied: AppliedUndo[] };
}

export type MemoryScope = 'user' | 'project';

export async function appendMemory(scope: MemoryScope, text: string): Promise<{ path: string; bytes: number }> {
  const r = await fetch('/api/memory/append', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ scope, text }),
  });
  if (!r.ok) {
    let msg = `memory append ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as { path: string; bytes: number };
}

export async function createWorktree(branch: string, base?: string): Promise<{ path: string; branch: string }> {
  const r = await fetch('/api/git/worktree', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ branch, base }),
  });
  if (!r.ok) {
    let msg = `worktree POST ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as { path: string; branch: string };
}

export type McpStdioConfig = {
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
};
export type McpHttpConfig = { url: string; auth: string | null };
/** Untagged discriminant matching the Rust `McpServerConfig` — inspect
 *  which key is present (`command` for stdio, `url` for http) to decide. */
export type McpServerConfig = McpStdioConfig | McpHttpConfig;

export type McpToolInfo = { name: string; description: string };

export type McpStatusView =
  | { kind: 'connected'; tool_count: number }
  | { kind: 'error'; message: string }
  | { kind: 'not_loaded' };

export type McpServerView = {
  name: string;
  kind: 'stdio' | 'http';
  config: McpServerConfig;
  status: McpStatusView;
  restart_required: boolean;
  tools: McpToolInfo[];
};

export type McpListView = { servers: McpServerView[]; config_path: string };

export async function listMcp(): Promise<McpListView> {
  const r = await fetch('/api/mcp', { cache: 'no-store' });
  if (!r.ok) {
    let msg = `mcp GET ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as McpListView;
}

export async function putMcp(servers: Record<string, McpServerConfig>): Promise<McpListView> {
  const r = await fetch('/api/mcp', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ servers }),
  });
  if (!r.ok) {
    let msg = `mcp PUT ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as McpListView;
}

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

async function readError(r: Response, prefix: string): Promise<string> {
  let msg = `${prefix} ${r.status}`;
  try {
    const j = await r.json();
    if (j?.error) msg += `: ${j.error}`;
  } catch { /* ignore */ }
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

export async function putCwd(path: string): Promise<void> {
  const r = await fetch('/api/cwd', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path }),
  });
  if (!r.ok) {
    let msg = `cwd PUT ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
}
