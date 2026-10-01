/**
 * External editors: detected list, preferred choice, open-in-editor.
 *
 * The server (`GET /api/editors`) scans the local machine for installed
 * editors — a web page cannot enumerate apps itself. The preferred editor
 * lives in localStorage (`mira.editor.preferred`).
 */
import { PREF_KEYS, getPref, setPref } from './prefs';

export interface EditorEntry {
  id: string;
  name: string;
  kind: 'app' | 'binary' | 'system';
  /** The server can serve this editor's real app icon. */
  has_icon: boolean;
}

export interface EditorsListView {
  editors: EditorEntry[];
  default_id: string | null;
  all: EditorEntry[];
}

let cached: Promise<EditorsListView> | null = null;

/** Detected + curated editors (cached for the session). */
export function listEditors(): Promise<EditorsListView> {
  if (!cached) {
    cached = fetch('/api/editors', { cache: 'no-store' }).then(async (r) => {
      if (!r.ok) throw new Error(`editors ${r.status}`);
      return (await r.json()) as EditorsListView;
    });
    cached.catch(() => {
      cached = null;
    });
  }
  return cached;
}

export function getPreferredEditorId(): string {
  return getPref(PREF_KEYS.preferredEditor, '');
}

export function setPreferredEditorId(id: string): void {
  setPref(PREF_KEYS.preferredEditor, id);
}

/** Resolve the editor to use: explicit → preferred → server default. */
export async function resolveEditorId(explicit?: string): Promise<string | null> {
  if (explicit) return explicit;
  const preferred = getPreferredEditorId();
  if (preferred) return preferred;
  try {
    const list = await listEditors();
    return list.default_id;
  } catch {
    return null;
  }
}

/** Open a file or folder in an editor. Throws with the server's message. */
export async function openInEditor(path: string, editorId?: string): Promise<void> {
  const id = await resolveEditorId(editorId);
  const r = await fetch('/api/editors/open', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path, editor_id: id }),
  });
  if (!r.ok) {
    let msg = `open ${r.status}`;
    try {
      const j = await r.json();
      if (j?.error) msg = j.error;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
}

/** Server-served app icon URL (real .icns art), or null. */
export function editorIconUrl(entry: Pick<EditorEntry, 'id' | 'has_icon'>): string | null {
  if (entry.has_icon) return `/api/editors/icon/${encodeURIComponent(entry.id)}`;
  return editorFaviconUrl(entry.id);
}

/** Favicon domain per editor id (fallback when the server has no icon). */
export const EDITOR_FAVICON_DOMAIN: Record<string, string> = {
  vscode: 'code.visualstudio.com',
  cursor: 'cursor.com',
  zed: 'zed.dev',
  windsurf: 'windsurf.com',
  antigravity: 'google.com',
  sublime: 'sublimetext.com',
  nova: 'nova.app',
  textmate: 'macromates.com',
  fleet: 'jetbrains.com',
  idea: 'jetbrains.com',
};

export function editorFaviconUrl(id: string): string | null {
  const domain = EDITOR_FAVICON_DOMAIN[id];
  return domain ? `https://www.google.com/s2/favicons?domain=${domain}&sz=64` : null;
}
