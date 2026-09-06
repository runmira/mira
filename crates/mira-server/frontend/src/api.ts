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
  const q = opts.all ? '?all=true' : '';
  const r = await fetch(`/api/sessions${q}`);
  if (!r.ok) throw new Error(`sessions GET ${r.status}`);
  return (await r.json()) as SessionSummary[];
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
  worktrees: WorktreeEntry[];
};

export async function getGitStatus(): Promise<GitStatusView> {
  const r = await fetch('/api/git/status');
  if (!r.ok) throw new Error(`git status ${r.status}`);
  return (await r.json()) as GitStatusView;
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
