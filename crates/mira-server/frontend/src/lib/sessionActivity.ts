export interface SessionActivitySnapshot { epoch: string; revision: number; running: string[]; waiting?: string[] }
/** `waiting`: chats held on an approval, question, plan or secret. */
export interface SessionActivityState { epoch: string; revision: number; running: ReadonlySet<string>; waiting: ReadonlySet<string> }
export type SessionActivityUpdate = { snapshot: SessionActivitySnapshot } | {
  epoch: string; revision: number; session_id: string; running: boolean;
} | {
  epoch: string; revision: number; session_id: string; waiting: boolean;
};
/** Server epochs fence restarts; revisions fence stale updates. */
export function applySessionActivity(state: SessionActivityState | null, update: SessionActivityUpdate): SessionActivityState | null {
  if ('snapshot' in update) {
    const next = update.snapshot;
    if (state?.epoch === next.epoch && next.revision < state.revision) return state;
    return { epoch: next.epoch, revision: next.revision, running: new Set(next.running), waiting: new Set(next.waiting ?? []) };
  }
  if (!state || state.epoch !== update.epoch || update.revision <= state.revision) return state;
  if ('waiting' in update) {
    const waiting = new Set(state.waiting);
    if (update.waiting) waiting.add(update.session_id); else waiting.delete(update.session_id);
    return { ...state, revision: update.revision, waiting };
  }
  const running = new Set(state.running);
  if (update.running) running.add(update.session_id); else running.delete(update.session_id);
  return { ...state, revision: update.revision, running };
}
