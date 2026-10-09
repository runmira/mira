/* ---------- Importing chats from other coding agents ---------- */

export type ImportSource = 'claude-code' | 'codex';

export type ImportChat = {
  source: ImportSource;
  id: string;
  title: string;
  model: string | null;
  messages: number;
  /** Unix ms. */
  updated_at: number | null;
  /** The Mira chat it already became, if imported. */
  imported_as: string | null;
  /** Run by a program (a test suite, a script, another app driving the
   *  agent), not typed by a person. */
  automated: boolean;
};

export type ImportProject = {
  path: string;
  name: string;
  /** `owner/name` from the git remote. */
  repo: string | null;
  is_git: boolean;
  exists: boolean;
  last_active: number | null;
  chats: ImportChat[];
};

export type ImportScan = {
  sources: { id: ImportSource; label: string; found: boolean; chats: number }[];
  projects: ImportProject[];
};

/** Other agents' chats on this computer, grouped by project. */
export async function scanImportableChats(fresh = false): Promise<ImportScan> {
  const r = await fetch(`/api/import/scan${fresh ? '?fresh=true' : ''}`);
  if (!r.ok) throw new Error(`scan ${r.status}`);
  return (await r.json()) as ImportScan;
}

/** Bring chats into Mira; each becomes a chat on its agent that resumes. */
export async function importChats(
  chats: { source: ImportSource; id: string }[],
): Promise<{ imported: string[]; existing: string[]; failed: string[] }> {
  const r = await fetch('/api/import', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ chats }),
  });
  const j = (await r.json().catch(() => ({}))) as {
    imported?: string[];
    existing?: string[];
    failed?: string[];
    error?: string;
  };
  if (!r.ok) throw new Error(j.error ?? `import ${r.status}`);
  return { imported: j.imported ?? [], existing: j.existing ?? [], failed: j.failed ?? [] };
}
