import type { SessionSummary } from '../../types';
import { Group } from './types';
export function groupByCwd(sessions: SessionSummary[], currentCwd: string): Group[] {
  const byCwd = new Map<string, SessionSummary[]>();
  for (const s of sessions) {
    const bucket = byCwd.get(s.cwd);
    if (bucket) bucket.push(s);
    else byCwd.set(s.cwd, [s]);
  }
  // Pinned sessions float to the top of their folder (issue #58); a
  // stable sort keeps the backend's newest-first order within each tier.
  for (const list of byCwd.values()) {
    list.sort((a, b) => Number(b.pinned ?? false) - Number(a.pinned ?? false));
  }
  const groups: Group[] = [];
  for (const [cwd, list] of byCwd) {
    // Backend already sorts by updated_at DESC across all cwds, so this
    // per-bucket order is already correct.
    groups.push({
      cwd,
      label: basename(cwd),
      isCurrent: cwd === currentCwd,
      sessions: list,
    });
  }
  // Current project first, then by "most recent activity in that project"
  // (which the first session's updated_at approximates cheaply).
  groups.sort((a, b) => {
    if (a.isCurrent !== b.isCurrent) return a.isCurrent ? -1 : 1;
    const at = a.sessions[0]?.updated_at ?? 0;
    const bt = b.sessions[0]?.updated_at ?? 0;
    return bt - at;
  });
  return groups;
}

/** Which period a row falls in, for the date sections. Pinned chats sit
 *  at the top of their folder, so they get their own section. */
export function dateSection(s: SessionSummary): string {
  if (s.pinned) return 'Pinned';
  if (!s.updated_at) return 'Older';
  const then = new Date(s.updated_at * 1000);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const day = 86_400_000;
  const t = then.getTime();
  if (t >= today.getTime()) return 'Today';
  if (t >= today.getTime() - day) return 'Yesterday';
  if (t >= today.getTime() - 6 * day) return 'This week';
  if (t >= today.getTime() - 29 * day) return 'This month';
  return 'Older';
}

export function basename(p: string): string {
  if (!p) return 'unknown';
  const trimmed = p.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i >= 0 ? trimmed.slice(i + 1) || '/' : trimmed;
}
