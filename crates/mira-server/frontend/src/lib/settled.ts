import type { SessionSummary } from '../types';

/** Turn-end checkpoints land a little after a merge the chat itself made
 *  ("merge it"), so activity this close to the merge doesn't count as
 *  coming back to the chat. */
const AFTER_MERGE_SLACK = 15 * 60;

/** Whether a chat belongs on the sidebar's "Settled" shelf: work that is
 *  done and can get out of the way.
 *
 *  - The user settled it, and hasn't written to it since.
 *  - Or its PR merged or closed (or, without a PR, its worktree branch is in
 *    main), the user hasn't taken it back out since, and hasn't come back
 *    to it after the merge.
 *
 *  A chat that is running or waiting on you is never settled; neither is a
 *  pinned one, unless the user settled it by hand. */
export function isSettled(s: SessionSummary): boolean {
  if (s.running || s.needs_attention || s.failure_reason) return false;
  const activity = s.updated_at ?? 0;
  if (s.settled_at != null) return activity <= s.settled_at;
  if (s.pinned) return false;
  const done = doneAt(s);
  if (done === null) return false;
  if (s.unsettled_at != null && s.unsettled_at >= done) return false;
  return done === 0 || activity <= done + AFTER_MERGE_SLACK;
}

/** When the chat's work finished: its PR's merge or close time, `0` when
 *  that is unknown (a branch merged locally), `null` while it is open. */
function doneAt(s: SessionSummary): number | null {
  if (s.pr) {
    if (s.pr.state !== 'merged' && s.pr.state !== 'closed') return null;
    return s.pr.closed_at ?? 0;
  }
  return s.worktree_status === 'merged' ? 0 : null;
}

/** Newest wrap-up first: the shelf reads "what finished lately". */
export function settledOrder(a: SessionSummary, b: SessionSummary): number {
  return settledTime(b) - settledTime(a);
}

function settledTime(s: SessionSummary): number {
  return s.settled_at ?? s.pr?.closed_at ?? s.updated_at ?? 0;
}
