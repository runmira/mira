export interface SessionActivitySnapshot { epoch: string; revision: number; running: string[] }
export interface SessionActivityState { epoch: string; revision: number; running: ReadonlySet<string> }
export type SessionActivityUpdate = { snapshot: SessionActivitySnapshot } | {
  epoch: string; revision: number; session_id: string; running: boolean;
};
/** Server epochs fence restarts; revisions fence stale updates. */
export function applySessionActivity(state: SessionActivityState | null, update: SessionActivityUpdate): SessionActivityState | null {
  if ('snapshot' in update) {
    const next = update.snapshot;
    if (state?.epoch === next.epoch && next.revision < state.revision) return state;
    return { epoch: next.epoch, revision: next.revision, running: new Set(next.running) };
  }
  if (!state || state.epoch !== update.epoch || update.revision <= state.revision) return state;
  const running = new Set(state.running);
  if (update.running) running.add(update.session_id); else running.delete(update.session_id);
  return { epoch: state.epoch, revision: update.revision, running };
}
