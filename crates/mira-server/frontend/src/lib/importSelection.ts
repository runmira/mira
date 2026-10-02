/**
 * How the "bring your chats" picker groups and pre-selects other agents'
 * history. Pure, so it's tested on its own.
 */
import type { ImportChat, ImportProject } from '../api';

const RECENT_MS = 30 * 24 * 60 * 60 * 1000;
/** One or two chats in a folder is usually a one-off question, not a project. */
const MIN_CHATS = 3;

/** One project in the picker: every checkout of the same repository
 *  together (a clone, a worktree, a subfolder), or a single folder. */
export type ProjectGroup = {
  key: string;
  label: string;
  repo: string | null;
  paths: string[];
  chats: (ImportChat & { path: string })[];
  lastActive: number | null;
  isGit: boolean;
  exists: boolean;
};

export function groupProjects(
  projects: ImportProject[],
  /** Include scripted runs (hidden by default: they're noise to read). */
  includeAutomated = false,
): { groups: ProjectGroup[]; other: ProjectGroup[] } {
  const byKey = new Map<string, ProjectGroup>();
  for (const project of projects) {
    const p = includeAutomated ? project : { ...project, chats: project.chats.filter((c) => !c.automated) };
    if (p.chats.length === 0) continue;
    const key = p.repo ? `repo:${p.repo.toLowerCase()}` : `path:${p.path}`;
    const g = byKey.get(key) ?? {
      key,
      label: p.repo ?? p.name,
      repo: p.repo,
      paths: [],
      chats: [],
      lastActive: null,
      isGit: false,
      exists: false,
    };
    g.paths.push(p.path);
    g.chats.push(...p.chats.map((c) => ({ ...c, path: p.path })));
    g.lastActive = Math.max(g.lastActive ?? 0, p.last_active ?? 0) || null;
    g.isGit ||= p.is_git;
    g.exists ||= p.exists;
    byKey.set(key, g);
  }
  const all = [...byKey.values()].map((g) => ({
    ...g,
    chats: g.chats.sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0)),
  }));
  all.sort((a, b) => (b.lastActive ?? 0) - (a.lastActive ?? 0));
  // Folders that aren't repositories (home, /tmp, scratch) fold away.
  return { groups: all.filter((g) => g.isGit), other: all.filter((g) => !g.isGit) };
}

export const chatKey = (c: { source: string; id: string }) => `${c.source}:${c.id}`;

/** A chat worth bringing over: something was answered, and it isn't in
 *  Mira already. */
function worthImporting(c: ImportChat): boolean {
  return c.imported_as == null && c.messages >= 2 && !c.automated;
}

/** How many scripted runs a scan found (for the "show" toggle). */
export function automatedCount(projects: ImportProject[]): number {
  return projects.reduce((n, p) => n + p.chats.filter((c) => c.automated).length, 0);
}

/** What starts ticked: real work in repositories used in the last month. */
export function defaultSelection(groups: ProjectGroup[], now = Date.now()): Set<string> {
  const out = new Set<string>();
  for (const g of groups) {
    if (!g.isGit || !g.exists) continue;
    if (!g.lastActive || g.lastActive < now - RECENT_MS || g.lastActive > now + 60_000) continue;
    const real = g.chats.filter(worthImporting);
    if (real.length < MIN_CHATS) continue;
    for (const c of real) out.add(chatKey(c));
  }
  return out;
}

/** Ticked chats in a group, for its tri-state checkbox. */
export function groupState(g: ProjectGroup, selected: Set<string>): 'all' | 'some' | 'none' {
  const open = g.chats.filter((c) => c.imported_as == null);
  const n = open.filter((c) => selected.has(chatKey(c))).length;
  return n === 0 ? 'none' : n === open.length ? 'all' : 'some';
}
