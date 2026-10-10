import { Origin } from './api/plugins';
import { readError } from './api/pullRequests';
import { jsonOrThrow, post } from './api/request';
import type { BackgroundMode, ExternalAgentSession, SessionSummary } from './types';
export async function listSessions(
  opts: { all?: boolean; archived?: boolean; includeChildren?: boolean } = {},
): Promise<SessionSummary[]> {
  // axum's Query bool deserializer expects the literal string `true`, not `1`.
  // `no-store` + a cache-busting param defeat any browser/HTTP caching so
  // fetches triggered by session_title_updated actually see fresh titles
  // rather than a stale cached list.
  const params = new URLSearchParams();
  if (opts.all) params.set('all', 'true');
  if (opts.archived) params.set('archived', 'true');
  if (opts.includeChildren) params.set('include_children', 'true');
  params.set('_ts', String(Date.now()));
  const r = await fetch(`/api/sessions?${params.toString()}`, { cache: 'no-store' });
  if (!r.ok) throw new Error(`sessions GET ${r.status}`);
  return (await r.json()) as SessionSummary[];
}

/** Pin / archive / restore a session. Omitted fields are left untouched;
 *  both flags persist on the server record and survive restarts. */
export async function setSessionFlags(
  id: string,
  flags: { pinned?: boolean; archived?: boolean; settled?: boolean },
): Promise<void> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}/flags`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(flags),
  });
  if (!r.ok) {
    let msg = `session flags ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
}

export type SessionHistoryView = {
  id: string;
  model: string;
  cwd: string;
  title: string | null;
  created_at: number;
  updated_at: number;
  messages: import('./types').Message[];
  previews?: Record<string, import('./types').DiffPreview>;
  agent_transcript?: import('./types').AgentTranscriptLine[];
  agent_driver?: string | null;
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
    } catch {
      /* ignore */
    }
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
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
}

export async function newSession(): Promise<{ id: string }> {
  const r = await fetch('/api/sessions/new', { method: 'POST' });
  if (!r.ok) throw new Error(`sessions new ${r.status}`);
  return (await r.json()) as { id: string };
}

/** Set a session's background mode. Only valid for slots the server has
 *  already materialized (persisted-but-not-open sessions have no slot,
 *  so nothing to gate). Callers typically attach first and set mode
 *  right after, or hit this after the slot is known to be live. */
export async function setSessionBackgroundMode(id: string, mode: BackgroundMode): Promise<void> {
  const r = await fetch(`/api/sessions/${encodeURIComponent(id)}/background`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ mode }),
  });
  if (!r.ok) {
    let msg = `background mode ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
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
    } catch {
      /* ignore */
    }
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
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
}

/** Manual rename: set the session's title to `title`. Empty clears it. */
export async function renameSession(
  id: string,
  title: string,
): Promise<{ id: string; title: string }> {
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
    } catch {
      /* ignore */
    }
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
    } catch {
      /* ignore */
    }
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

export async function browse(
  path?: string,
  showHidden = false,
  includeFiles = false,
): Promise<BrowseView> {
  const params = new URLSearchParams();
  if (path) params.set('path', path);
  if (showHidden) params.set('show_hidden', 'true');
  if (includeFiles) params.set('include_files', 'true');
  const r = await fetch(`/api/browse?${params.toString()}`);
  if (!r.ok) {
    let msg = `browse ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as BrowseView;
}

/** One value a model offers for a `select` descriptor. */
export type OptionChoice = {
  value: string;
  label: string;
  hint?: string | null;
};

/**
 * A single adjustable knob on a model, as advertised by the server.
 *
 * "Fast" is not one thing: OpenAI has a service tier, Anthropic's API has
 * no equivalent, and many models expose neither. Rather than a hardcoded
 * button that silently does nothing on most of the catalog, each model
 * advertises the knobs it genuinely accepts and the composer renders exactly
 * those. Nothing is drawn for a model with no descriptors.
 */
export type OptionDescriptor =
  | { type: 'select'; id: string; label: string; options: OptionChoice[] }
  | { type: 'boolean'; id: string; label: string; on_value?: string | null };

export type ModelCapabilities = { option_descriptors?: OptionDescriptor[] };

export type ModelInfo = {
  id: string;
  display_name?: string | null;
  owned_by?: string | null;
  context_length?: number | null;
  capabilities?: ModelCapabilities | null;
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
  tier: 'bundled' | 'shared' | 'user' | 'project';
  has_attachments: boolean;
  /** The plugin this skill comes from, if any. */
  origin?: Origin;
};

export type SkillsResponse = { skills: SkillView[] };

/** Loaded skill roster — bundled + user (~/.mira/skills) + project
 *  (<cwd>/.mira/skills) merged. Powers dynamic `/<skill-name>` slash
 *  commands in the composer palette. Returns an empty list on error
 *  (skills are additive; a missing roster shouldn't break the palette). */
export type AgentTurn = { turn: number; t: number; first_text: string; snapshot: boolean };

export async function listAgentTurns(sessionId: string): Promise<AgentTurn[]> {
  const r = await fetch(`/api/acp/turns?session=${encodeURIComponent(sessionId)}`, {
    cache: 'no-store',
  });
  if (!r.ok) throw new Error(`turns GET ${r.status}`);
  return (await r.json()) as AgentTurn[];
}

export async function revertAgentTurn(sessionId: string, turn: number): Promise<{ note: string }> {
  const r = await fetch('/api/acp/revert', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ session_id: sessionId, turn }),
  });
  if (!r.ok) {
    let msg = `revert POST ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as { note: string };
}

export async function listExternalAgentSessions(): Promise<ExternalAgentSession[]> {
  const r = await fetch('/api/acp/external-sessions', { cache: 'no-store' });
  if (!r.ok) throw new Error(`external-sessions GET ${r.status}`);
  return (await r.json()) as ExternalAgentSession[];
}

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

/** Full skill payload — superset of `SkillView` with the SKILL.md body,
 *  attached-file names, and the on-disk source path. Powers the detail
 *  drawer in Settings > Skills. `body` is the raw markdown the agent
 *  reads when the skill is invoked; render it with the shared Markdown
 *  component. */
export type SkillDetail = {
  name: string;
  description: string;
  category?: string;
  icon?: string;
  color?: string;
  tier: 'bundled' | 'shared' | 'user' | 'project';
  /** Raw markdown body of the SKILL.md file. */
  body: string;
  /** Attachment filenames relative to the skill's directory. Empty for
   *  bundled builtins and flat-file skills. */
  attachments: string[];
  /** Absolute path where the skill file lives. Absent for bundled
   *  builtins (they live inside the binary, not on disk). */
  source?: string;
};

/** Fetch one skill's full body + attachments. Returns `null` when the
 *  name doesn't resolve — the caller shows an error state instead of
 *  throwing. */
export async function getSkill(name: string): Promise<SkillDetail | null> {
  try {
    const r = await fetch(`/api/skills/${encodeURIComponent(name)}`);
    if (!r.ok) return null;
    return (await r.json()) as SkillDetail;
  } catch {
    return null;
  }
}

export type FileView = { path: string; bytes: number; content: string };

/** Read a text file. `view` is for the file viewer, which allows larger
 *  files than attaching one to a message does. */
export async function readFile(path: string, opts?: { view?: boolean }): Promise<FileView> {
  const r = await fetch(
    `/api/file?path=${encodeURIComponent(path)}${opts?.view ? '&purpose=view' : ''}`,
  );
  if (!r.ok) {
    let msg = `Couldn't read the file (HTTP ${r.status}).`;
    try {
      const j = await r.json();
      // The server's message is written for people; show it as is.
      if (j.error) msg = j.error;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as FileView;
}

export type WorktreeEntry = { path: string; branch: string | null; is_current: boolean };

export type BranchEntry = {
  /** Short branch name — `main`, `dami/fix-map-leaks`. Never carries a
   *  remote prefix; use `is_remote` to disambiguate. */
  name: string;
  /** True when only a remote-tracking ref exists for this branch.
   *  Clicking such an entry creates a new local branch tracking the
   *  remote at worktree-add time. */
  is_remote: boolean;
  /** True when this branch is already checked out somewhere. The
   *  picker hides these from the "switch to" list (git forbids two
   *  worktrees on the same branch). */
  in_worktree: boolean;
  /** Upstream tracking ref, if configured (e.g. `origin/main`). */
  upstream?: string | null;
};

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
  /** All branches (local + remote), deduped on short name. Present
   *  in-repo, absent when `in_repo` is false. */
  branches?: BranchEntry[];
  /** Total lines added in uncommitted changes vs HEAD. */
  diff_added: number;
  /** Total lines removed in uncommitted changes vs HEAD. */
  diff_removed: number;
  /** Subject line of the most recent commit. */
  last_commit?: string | null;
};

export async function getGitStatus(): Promise<GitStatusView> {
  const r = await fetch('/api/git/status');
  if (!r.ok) throw new Error(`git status ${r.status}`);
  return (await r.json()) as GitStatusView;
}

/** One file the chat changed, as a commit of it would carry it. */
export type SessionFile = {
  /** Relative to the session's folder. */
  path: string;
  status: 'added' | 'modified' | 'deleted';
  added: number;
  removed: number;
  binary: boolean;
};

export type SessionDiffView = {
  added: number;
  removed: number;
  /** What the chat changed that still differs from HEAD — by its tools, an
   *  external agent or a shell command alike. */
  files: SessionFile[];
  /** `files.length`. */
  uncommitted?: number;
  /** Of those, how many git doesn't track yet (new files). */
  untracked?: number;
};

export async function getSessionDiff(): Promise<SessionDiffView> {
  const r = await fetch('/api/git/session-diff');
  if (!r.ok) return { added: 0, removed: 0, files: [] };
  return (await r.json()) as SessionDiffView;
}

export type SessionChange = {
  path: string;
  status: 'modified' | 'added' | 'deleted';
  added: number;
  removed: number;
  /** Unified diff against HEAD. */
  diff: string;
};

/** Full diffs for every file this session changed that differs from HEAD. */
export async function getSessionChanges(): Promise<SessionChange[]> {
  const r = await fetch('/api/git/session-changes');
  if (!r.ok) throw new Error(`session changes ${r.status}`);
  return ((await r.json()) as { files: SessionChange[] }).files;
}

export type ContextPart = {
  /** `system`, `tools`, `memory`, `conversation`, `tool_results` for Mira;
   *  a slug of the agent's own category name otherwise. */
  id: string;
  label: string;
  tokens: number;
};

export type ContextView = {
  /** `mira`: sized by Mira from the request it sends. `agent`: the
   *  external agent's own count. */
  source: 'mira' | 'agent';
  /** The agent's name, for `agent`. */
  agent?: string;
  /** Results can be taken out (only Mira's own history). */
  droppable: boolean;
  window: number;
  compact_at: number | null;
  /** Extra itemised groups an agent reports (memory files, skills, MCP tools). */
  details: { title: string; items: { label: string; tokens: number }[] }[];
  breakdown: {
    parts: ContextPart[];
    total: number;
    /** Scaled to the provider's own count for the last request. */
    calibrated: boolean;
    last_reported: number | null;
    largest_results: { call_id: string; tool: string; label: string; tokens: number }[];
  };
};

export type DroppedResult = { tool: string; label: string; tokens: number };

/** What fills the active chat's context window. */
export async function getContextBreakdown(): Promise<ContextView> {
  const r = await fetch('/api/context');
  const json = (await r.json().catch(() => ({}))) as ContextView & { error?: string };
  if (!r.ok) throw new Error(json.error ?? `HTTP ${r.status}`);
  return json;
}

/** Take one tool result out of the context (replaced with a stub). */
export async function dropContextResult(callId: string): Promise<DroppedResult> {
  const r = await fetch('/api/context/drop', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ call_id: callId }),
  });
  const json = (await r.json().catch(() => ({}))) as DroppedResult & { error?: string };
  if (!r.ok) throw new Error(json.error ?? `HTTP ${r.status}`);
  return json;
}

/** What restoring a checkpoint would do to one file. */
export type RestoreChange = {
  path: string;
  /** `revert`: content goes back; `remove`: created since; `recreate`: deleted since. */
  action: 'revert' | 'remove' | 'recreate';
};

export type Restored = { changes: RestoreChange[]; undo: string };

async function checkpointCall<T>(path: string, body: unknown): Promise<T> {
  const r = await fetch(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const json = (await r.json().catch(() => ({}))) as T & { error?: string };
  if (!r.ok) throw new Error(json.error ?? `HTTP ${r.status}`);
  return json;
}

/** A user message, as checkpoints identify it: its text, and which match
 *  of that text counting from the latest (the same key editing uses). */
export type MessageRef = { text: string; occurrence: number };

/** What restoring the files to before a message would change. */
export async function previewCheckpoint(ref: MessageRef): Promise<RestoreChange[]> {
  return (await checkpointCall<{ changes: RestoreChange[] }>('/api/checkpoints/preview', ref))
    .changes;
}

/** Put the files back the way they were before a message. */
export function restoreCheckpoint(ref: MessageRef): Promise<Restored> {
  return checkpointCall<Restored>('/api/checkpoints/restore', ref);
}

/** "Fork from here": a new chat with this one's history through the
 *  given message's turn. Resolves to the new chat's id. */
export async function forkSession(sessionId: string, ref: MessageRef): Promise<string> {
  return (
    await checkpointCall<{ id: string }>(`/api/sessions/${encodeURIComponent(sessionId)}/fork`, ref)
  ).id;
}

/** Undo a restore. */
export function undoRestore(undo: string): Promise<Restored> {
  return checkpointCall<Restored>('/api/checkpoints/undo', { undo });
}

/** Discard this session's changes to one file (restore / delete). */
export async function revertFile(path: string): Promise<void> {
  const r = await fetch('/api/git/revert-file', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path }),
  });
  if (!r.ok) {
    let msg = `revert ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
}

export async function gitPush(): Promise<void> {
  const r = await fetch('/api/git/push', { method: 'POST' });
  if (!r.ok) {
    let msg = `git push ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
}

export type BranchPrView = {
  number: number;
  title: string;
  state: string;
  url: string;
  isDraft: boolean;
  reviewDecision: string | null;
};

export async function getBranchPr(): Promise<BranchPrView | null> {
  const r = await fetch('/api/git/branch-pr');
  if (!r.ok) return null;
  return (await r.json()) as BranchPrView;
}

export type CommitRequest = {
  message: string;
  include_unstaged: boolean;
  push_after: boolean;
};

export async function gitCommit(req: CommitRequest): Promise<void> {
  const r = await fetch('/api/git/commit', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(req),
  });
  if (!r.ok) {
    let msg = `git commit ${r.status}`;
    try {
      const j = await r.json();
      if (j.error) msg += `: ${j.error}`;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
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
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as { applied: AppliedUndo[] };
}

export type MemoryScope = 'user' | 'project';

export async function appendMemory(
  scope: MemoryScope,
  text: string,
): Promise<{ path: string; bytes: number }> {
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
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as { path: string; bytes: number };
}

export async function createWorktree(
  branch: string,
  base?: string,
): Promise<{ path: string; branch: string }> {
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
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  return (await r.json()) as { path: string; branch: string };
}

export async function putCwd(path: string): Promise<{ session_id?: string }> {
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
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  // Server returns { path, home, session_id? }. Return just the id so
  // callers can WS-attach to the freshly-materialized slot.
  return (await r.json()) as { session_id?: string };
}

/* ---------- GitHub connect ---------- */

export type GithubConnectView = {
  repo: string | null;
  has_token: boolean;
  provider: string | null;
  model: string | null;
  /** Why connecting can't work yet. */
  problem: string | null;
  /** The part of `problem` about Mira's provider, model or key. */
  settings_problem: string | null;
  status: { repo: string; workflow: boolean; api_key_secret: boolean } | null;
};

export type GithubConnectReport = {
  repo: string;
  committed_to: string | null;
  pull_request: string | null;
  workflow_unchanged: boolean;
};

export async function getGithubConnect(repo?: string): Promise<GithubConnectView> {
  const q = repo ? `?repo=${encodeURIComponent(repo)}` : '';
  const r = await fetch(`/api/github/connect${q}`, { cache: 'no-store' });
  if (!r.ok) throw new Error(await readError(r, 'github connect GET'));
  return (await r.json()) as GithubConnectView;
}

/** Connect `repo`. `token` is a Runmira-bot app token for it; without one
 *  the server uses the GitHub key from Settings. */
export async function connectGithub(repo?: string, token?: string): Promise<GithubConnectReport> {
  const r = await fetch('/api/github/connect', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ repo: repo || null, token: token || null }),
  });
  if (!r.ok) throw new Error(await readError(r, 'github connect'));
  return (await r.json()) as GithubConnectReport;
}

/* ---------- hooks (Settings → Hooks) ---------- */

/** One hook, flat: when `event` (for tools matching `matcher`), run
 *  `command` or ask the AI `prompt`. */
export type HookRule = {
  event: string;
  matcher?: string | null;
  type: 'command' | 'prompt';
  command?: string | null;
  prompt?: string | null;
  timeout?: number | null;
};

export type HooksView = {
  rules: HookRule[];
  plugin_rules: (HookRule & { plugin: string })[];
  problems: string[];
  /** Hooks that failed when they ran (each is announced in chat once). */
  failing: string[];
  os: string;
};

export async function getHooks(): Promise<HooksView> {
  return jsonOrThrow<HooksView>(await fetch('/api/hooks'), 'Loading hooks');
}

export async function saveHooks(rules: HookRule[]): Promise<HooksView> {
  const r = await fetch('/api/hooks', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ rules }),
  });
  return jsonOrThrow<HooksView>(r, 'Saving hooks');
}

export function updateExternalAgent(kind: string): Promise<{ updated: boolean }> {
  return post(`/api/acp/agents/${encodeURIComponent(kind)}/update`, {}, 'Updating agent');
}

export async function getTurnFileDiff(
  session: string,
  text: string,
  occurrence: number,
  path: string,
  signal?: AbortSignal,
): Promise<import('./types').DiffPreview & { before: string; after: string }> {
  const response = await fetch(`/api/sessions/${encodeURIComponent(session)}/turn-diff`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ text, occurrence, path }),
    signal,
  });
  const body = await response.json();
  if (!response.ok) throw new Error(body.error ?? 'Could not load this turn’s diff');
  return body;
}

export type ChatRelationshipView = {
  current: SessionSummary | null;
  parent: SessionSummary | null;
  children: SessionSummary[];
  has_more: boolean;
};

export async function getChatRelationships(
  session: string,
  limit = 20,
  signal?: AbortSignal,
): Promise<ChatRelationshipView> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(session)}/relationships?limit=${limit}`,
    { cache: 'no-store', signal },
  );
  if (!response.ok) throw new Error('Could not load related chats');
  return response.json();
}

export {
  getAgentSettings,
  listEngines,
  listInstanceModels,
  putAgentSettings,
  type AgentEnvVar,
  type AgentSettings,
  type AgentSettingsPatch,
  type EngineFlavor,
  type EngineListView,
  type EngineSnapshot,
  type EngineState,
} from './api/agents';
export {
  importChats,
  scanImportableChats,
  type ImportChat,
  type ImportProject,
  type ImportScan,
  type ImportSource,
} from './api/imports';
export {
  addMarketplace,
  deleteMcpServer,
  getPluginDetail,
  getPlugins,
  installPlugin,
  listCommands,
  listMcp,
  reconnectMcp,
  removeMarketplace,
  saveMcpServer,
  setMcpApproval,
  setMcpEnabled,
  setMcpToolEnabled,
  setMcpToolLoading,
  setMcpVariable,
  setPluginEnabled,
  signInMcp,
  signOutMcp,
  uninstallPlugin,
  updateMarketplace,
  type CatalogEntry,
  type CommandInfo,
  type InstalledPluginView,
  type MarketplaceView,
  type McpListView,
  type McpProblem,
  type McpPromptView,
  type McpResourceView,
  type McpScope,
  type McpServerConfig,
  type McpServerView,
  type McpStatus,
  type McpToolView,
  type Origin,
  type PluginComponents,
  type PluginDetail,
  type PluginsOverview,
  type ToolLoading,
  type WriteScope,
} from './api/plugins';
export {
  getPullRequest,
  getPullRequestFiles,
  listPullRequests,
  mergePullRequest,
  postPullRequestComment,
  submitPullRequestReview,
  type CheckRunView,
  type CommentView,
  type CommitView,
  type FileChangeView,
  type MergeMethod,
  type PullRequestDetailView,
  type PullRequestFilesView,
  type PullRequestListView,
  type PullRequestSummary,
  type RepoError,
  type RepoGroup,
  type ReviewEvent,
  type ReviewView,
  type TimelineEventView,
} from './api/pullRequests';
export { getSettings, putSettings } from './api/settings';
