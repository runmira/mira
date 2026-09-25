import type { BackgroundMode, SessionSummary, SettingsUpdate, SettingsView } from './types';

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
  previews?: Record<string, import('./types').DiffPreview>;
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

export async function newSession(): Promise<{ id: string }> {
  const r = await fetch('/api/sessions/new', { method: 'POST' });
  if (!r.ok) throw new Error(`sessions new ${r.status}`);
  return (await r.json()) as { id: string };
}

/** Set a session's background mode. Only valid for slots the server has
 *  already materialized (persisted-but-not-open sessions have no slot,
 *  so nothing to gate). Callers typically attach first and set mode
 *  right after, or hit this after the slot is known to be live. */
export async function setSessionBackgroundMode(
  id: string,
  mode: BackgroundMode,
): Promise<void> {
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
    } catch { /* ignore */ }
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

export async function browse(path?: string, showHidden = false, includeFiles = false): Promise<BrowseView> {
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

export type SessionDiffView = { added: number; removed: number; files: string[] };

export async function getSessionDiff(): Promise<SessionDiffView> {
  const r = await fetch('/api/git/session-diff');
  if (!r.ok) return { added: 0, removed: 0, files: [] };
  return (await r.json()) as SessionDiffView;
}

export async function gitPush(): Promise<void> {
  const r = await fetch('/api/git/push', { method: 'POST' });
  if (!r.ok) {
    let msg = `git push ${r.status}`;
    try { const j = await r.json(); if (j.error) msg += `: ${j.error}`; } catch { /* ignore */ }
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
    try { const j = await r.json(); if (j.error) msg += `: ${j.error}`; } catch { /* ignore */ }
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

/* ---------- MCP servers + plugins ---------- */

/** Throw with the server's `{ error }` message when a request fails. */
async function jsonOrThrow<T>(r: Response, what: string): Promise<T> {
  if (!r.ok) {
    let msg = `${what} failed (${r.status})`;
    try {
      const j = await r.json();
      if (j.error) msg = j.error;
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  return (await r.json()) as T;
}

async function post<T>(url: string, body: unknown, what: string): Promise<T> {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body ?? {}),
  });
  return jsonOrThrow<T>(r, what);
}

/** A server entry in `.mcp.json` shape. */
export type McpServerConfig =
  | {
      type?: 'stdio';
      command: string;
      args?: string[];
      env?: Record<string, string>;
      cwd?: string | null;
    }
  | {
      type?: 'http' | 'sse';
      url: string;
      headers?: Record<string, string>;
      auth?: string | null;
      oauth?: { clientId?: string; clientSecret?: string; scopes?: string[] } | null;
    };

export type McpScope =
  | { kind: 'user' }
  | { kind: 'project' }
  | { kind: 'local' }
  | { kind: 'plugin'; plugin: string };

export type McpStatus =
  | { state: 'connecting' }
  | { state: 'connected' }
  | { state: 'needs_auth' }
  | { state: 'needs_approval' }
  | { state: 'rejected' }
  | { state: 'disabled' }
  | { state: 'needs_setup'; variables: string[] }
  | { state: 'failed'; message: string };

export type McpToolView = { name: string; remote_name: string; description: string; read_only: boolean };
export type McpPromptView = {
  name: string;
  description: string | null;
  arguments: { name: string; description: string | null; required: boolean }[];
};
export type McpResourceView = { uri: string; name: string; description: string | null; mime_type: string | null };

export type McpServerView = {
  name: string;
  scope: McpScope;
  transport: 'stdio' | 'http' | 'sse';
  target: string;
  source: string | null;
  status: McpStatus;
  tools: McpToolView[];
  resources: McpResourceView[];
  prompts: McpPromptView[];
  server_name: string | null;
  server_version: string | null;
  instructions: string | null;
  can_sign_in: boolean;
  signed_in: boolean;
  /** `${VAR}`s in the definition with no value. */
  missing_vars: string[];
  log_path: string | null;
  config: McpServerConfig;
};

export type McpProblem = { source: string; server: string | null; message: string };

export type McpListView = {
  servers: McpServerView[];
  problems: McpProblem[];
  user_config_path: string;
  project: string | null;
};

export type WriteScope = 'user' | 'project' | 'local';

export async function listMcp(): Promise<McpListView> {
  return jsonOrThrow(await fetch('/api/mcp', { cache: 'no-store' }), 'Loading MCP servers');
}

export function saveMcpServer(req: {
  name: string;
  scope: WriteScope;
  config: McpServerConfig;
  replaces?: { name: string; scope: WriteScope } | null;
}): Promise<McpListView> {
  return post('/api/mcp/servers', req, 'Saving the server');
}

export async function deleteMcpServer(name: string, scope: WriteScope): Promise<McpListView> {
  const r = await fetch(`/api/mcp/servers/${encodeURIComponent(name)}?scope=${scope}`, { method: 'DELETE' });
  return jsonOrThrow(r, 'Removing the server');
}

const mcpUrl = (name: string, action: string) => `/api/mcp/servers/${encodeURIComponent(name)}/${action}`;

export const reconnectMcp = (name: string) => post<McpListView>(mcpUrl(name, 'reconnect'), {}, 'Reconnecting');
export const setMcpEnabled = (name: string, enabled: boolean) =>
  post<McpListView>(mcpUrl(name, 'enabled'), { enabled }, enabled ? 'Enabling' : 'Disabling');
export const setMcpApproval = (name: string, approve: boolean) =>
  post<McpListView>(mcpUrl(name, 'approval'), { approve }, approve ? 'Approving' : 'Rejecting');
export const signInMcp = (name: string) => post<{ url: string }>(mcpUrl(name, 'sign-in'), {}, 'Starting sign-in');
export const signOutMcp = (name: string) => post<McpListView>(mcpUrl(name, 'sign-out'), {}, 'Signing out');

/** Where a command or skill comes from, for grouping it in palettes. */
export type Origin = {
  kind: 'plugin' | 'mcp' | 'user' | 'project';
  /** Groups items from the same place (`plugin:notion`, `mcp:linear`). */
  key: string;
  /** "Notion", "Commit commands". */
  label: string;
  icon_url: string | null;
  homepage: string | null;
};

export type CommandInfo = {
  name: string;
  description: string;
  argument_hint: string | null;
  /** `user`, `project`, `plugin:<name>` or `mcp:<server>`. */
  source: string;
  /** A Markdown command, or an MCP server's prompt. */
  kind: 'command' | 'prompt';
  origin: Origin;
};

export async function listCommands(): Promise<CommandInfo[]> {
  return jsonOrThrow(await fetch('/api/commands', { cache: 'no-store' }), 'Loading commands');
}

export type MarketplaceView = {
  name: string;
  source: string;
  kind: 'github' | 'git' | 'directory' | 'url';
  description: string | null;
  owner: string | null;
  plugin_count: number;
  path: string;
  updated_at: number;
  error: string | null;
};

export type CatalogEntry = {
  id: string;
  name: string;
  display_name: string | null;
  icon_url: string | null;
  marketplace: string;
  description: string | null;
  version: string | null;
  author: string | null;
  category: string | null;
  tags: string[];
  keywords: string[];
  homepage: string | null;
  source: string;
  installable: boolean;
  installed: boolean;
  enabled: boolean;
  installed_version: string | null;
};

export type InstalledPluginView = {
  id: string;
  name: string;
  display_name: string | null;
  marketplace: string;
  version: string;
  description: string | null;
  author: string | null;
  enabled: boolean;
  path: string;
  updated_at: number;
  commands: string[];
  agents: string[];
  skills: string[];
  mcp_servers: string[];
  hooks: string[];
  lsp_servers: string[];
  problems: string[];
};

export type PluginsOverview = {
  marketplaces: MarketplaceView[];
  catalog: CatalogEntry[];
  installed: InstalledPluginView[];
  suggested_marketplaces: { source: string; name: string; description: string }[];
};

export type PluginComponents = {
  commands: string[];
  agents: string[];
  skill_dirs: string[];
  skills: string[];
  hooks: string[];
  mcp_server_names: string[];
  lsp_servers: string[];
  problems: string[];
};

export type PluginDetail = CatalogEntry & {
  license: string | null;
  repository: string | null;
  components: PluginComponents | null;
  command_names: string[];
  agent_names: string[];
  readme: string | null;
  path: string | null;
};

export async function getPlugins(): Promise<PluginsOverview> {
  return jsonOrThrow(await fetch('/api/plugins', { cache: 'no-store' }), 'Loading plugins');
}

export async function getPluginDetail(id: string): Promise<PluginDetail> {
  return jsonOrThrow(
    await fetch(`/api/plugins/detail/${encodeURIComponent(id)}`, { cache: 'no-store' }),
    'Loading the plugin',
  );
}

export const installPlugin = (id: string) => post<PluginsOverview>('/api/plugins/install', { id }, 'Installing');
export const uninstallPlugin = (id: string) =>
  post<PluginsOverview>(`/api/plugins/${encodeURIComponent(id)}/uninstall`, {}, 'Uninstalling');
export const setPluginEnabled = (id: string, enabled: boolean) =>
  post<PluginsOverview>(`/api/plugins/${encodeURIComponent(id)}/enabled`, { enabled }, enabled ? 'Enabling' : 'Disabling');
export const addMarketplace = (source: string) =>
  post<PluginsOverview>('/api/plugins/marketplaces', { source }, 'Adding the marketplace');
export const updateMarketplace = (name: string) =>
  post<PluginsOverview>(`/api/plugins/marketplaces/${encodeURIComponent(name)}/update`, {}, 'Updating the marketplace');
export async function removeMarketplace(name: string): Promise<PluginsOverview> {
  const r = await fetch(`/api/plugins/marketplaces/${encodeURIComponent(name)}`, { method: 'DELETE' });
  return jsonOrThrow(r, 'Removing the marketplace');
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
    } catch { /* ignore */ }
    throw new Error(msg);
  }
  // Server returns { path, home, session_id? }. Return just the id so
  // callers can WS-attach to the freshly-materialized slot.
  return (await r.json()) as { session_id?: string };
}
