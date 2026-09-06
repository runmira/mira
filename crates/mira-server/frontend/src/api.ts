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

export async function listSessions(): Promise<SessionSummary[]> {
  const r = await fetch('/api/sessions');
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
