import type { SessionSummary } from '../types';
import { isSettled } from './settled';

export type SidebarFilter = 'all' | 'running' | 'waiting' | 'settled';

export function matchesSidebar(s: SessionSummary, query: string, filter: SidebarFilter): boolean {
  if (filter === 'running' && (!s.running || s.needs_attention)) return false;
  if (filter === 'waiting' && !s.needs_attention && !s.failure_reason) return false;
  if (filter === 'settled' && !isSettled(s)) return false;
  const haystack = [s.title, s.first_user_message, s.cwd, s.worktree_branch, s.agent_driver,
    s.pr?.title, s.pr ? `#${s.pr.number}` : ''].join(' ').toLowerCase();
  return query.trim().toLowerCase().split(/\s+/).every((word) => haystack.includes(word));
}

export function rowStatus(s: SessionSummary, unread = false): { label: string; tone: 'attention' | 'error' | 'working' | 'result' | 'quiet' } {
  if (s.needs_attention) return { label: s.attention_reason || 'Waiting for you', tone: 'attention' };
  if (s.failure_reason) return { label: 'Needs a retry', tone: 'error' };
  if (s.running) return { label: 'Working', tone: 'working' };
  if (unread) {
    const label = s.pr?.state === 'open' ? 'PR ready' : s.pr?.state === 'draft' ? 'Draft PR ready'
      : s.pr?.state === 'merged' ? 'PR merged' : 'Finished';
    return { label, tone: 'result' };
  }
  if (isSettled(s)) return { label: s.pr?.state === 'merged' ? 'Merged' : s.pr?.state === 'closed' ? 'PR closed' : 'Settled', tone: 'quiet' };
  if (s.pr?.state === 'open') return { label: 'PR open', tone: 'quiet' };
  if (s.pr?.state === 'draft') return { label: 'Draft PR', tone: 'quiet' };
  return { label: 'Ready', tone: 'quiet' };
}

export function settledPeriod(s: SessionSummary, now = new Date()): string {
  const at = s.settled_at ?? s.pr?.closed_at ?? s.updated_at;
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  if (at * 1000 >= today.getTime()) return 'Today';
  const week = new Date(today);
  week.setDate(week.getDate() - ((week.getDay() + 6) % 7));
  return at * 1000 >= week.getTime() ? 'This week' : 'Earlier';
}
