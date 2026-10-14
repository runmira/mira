import type { SessionSummary } from '../../types';
import { Group } from './types';
export function groupByCwd(sessions: SessionSummary[], currentCwd: string, folderSessions: SessionSummary[] = sessions): Group[] {
  const byCwd = new Map<string, SessionSummary[]>();
  for (const s of sessions) {
    const bucket = byCwd.get(s.cwd);
    if (bucket) bucket.push(s);
    else byCwd.set(s.cwd, [s]);
  }
  // Explicit pins stay first. Activity does not move a chat inside its
  // folder: newest creation first, with an immutable ID as the tie-breaker.
  for (const list of byCwd.values()) {
    list.sort((a, b) => Number(b.pinned ?? false) - Number(a.pinned ?? false)
      || (b.created_at ?? 0) - (a.created_at ?? 0) || a.id.localeCompare(b.id));
  }
  const groups: Group[] = [];
  for (const [cwd, list] of byCwd) {
    groups.push({
      cwd,
      label: basename(cwd),
      isCurrent: cwd === currentCwd,
      sessions: list,
    });
  }
  // A folder's first saved chat establishes its creation position. Use the
  // complete list so filtering/searching or settling a chat cannot change it.
  const created = new Map<string, number>();
  for (const session of folderSessions) {
    const time = session.created_at ?? 0;
    created.set(session.cwd, Math.min(created.get(session.cwd) ?? time, time));
  }
  groups.sort((a, b) => (created.get(b.cwd) ?? 0) - (created.get(a.cwd) ?? 0) || a.cwd.localeCompare(b.cwd));
  return groups;
}

/** Which period a row falls in, for the date sections. Pinned chats sit
 *  at the top of their folder, so they get their own section. */
export function dateSection(s: SessionSummary, now = new Date()): string {
  if (s.pinned) return 'Pinned';
  if (!s.created_at) return 'Older';
  const then = new Date(s.created_at * 1000);
  const today = new Date(now);
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
