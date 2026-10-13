import type { ClientMsg } from '../types';
import type { RuntimeState } from './runtimeStore';
import type { SessionData } from './sessionData';
export type RequestDelivery = { status: 'submitting' | 'failed'; error: string | null };

export function createRequestService(
  read: () => RuntimeState,
  update: <K extends keyof SessionData>(key: K, value: SessionData[K] | ((previous: SessionData[K]) => SessionData[K])) => void,
  updateAt: <K extends keyof SessionData>(session: string, key: K, apply: (previous: SessionData[K]) => SessionData[K]) => void,
) {
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  let disposed = false;
  function cancel(id: string) {
    const key = `${read().active}:${id}`;
    const timer = timers.get(key); if (timer) clearTimeout(timer); timers.delete(key);
  }
  function fail(id: string, error: string) {
    cancel(id);
    update('requestDelivery', previous => ({ ...previous, [id]: { status: 'failed', error } }));
  }
  return {
    submit(id: string, message: ClientMsg, send: (message: ClientMsg) => boolean) {
      const state = read();
      const session = state.active;
      const data = session ? state.sessions.get(session) : undefined;
      if (disposed || !session || !data || data.resolvedRequests[id] || data.requestDelivery[id]?.status === 'submitting') return false;
      const entry = data.entries.find(entry => entry.kind === 'tool' && entry.call.id === id);
      const actionable = message.type === 'approve' ? entry?.kind === 'tool' && entry.status === 'pending' :
        message.type === 'prompt_response' && (message.kind === 'secret' ? data.secretRequests.some(request => request.promptId === id) :
          message.kind === 'subagent_review' ? [...data.subagentState.values()].some(stream => stream.pendingReview?.promptId === id) :
          entry?.kind === 'tool' && (message.kind === 'plan' ? entry.plan?.decision === null : entry.askUser?.decision === null));
      if (!actionable) return false;
      update('requestDelivery', previous => ({ ...previous, [id]: { status: 'submitting', error: null } }));
      try {
        if (!send(message)) { fail(id, 'Reconnect before submitting this decision.'); return false; }
      } catch { fail(id, 'The decision could not be sent. Reconnect and try again.'); return false; }
      // Do not store the message, answers, or secret value in transport bookkeeping.
      const key = `${session}:${id}`;
      timers.set(key, setTimeout(() => {
        timers.delete(key);
        const current = read();
        if (!disposed && current.sessions.get(session)?.requestDelivery[id]?.status === 'submitting')
          updateAt(session, 'requestDelivery', previous => ({ ...previous, [id]: { status: 'failed',
            error: 'The server has not confirmed this decision. Check after reconnecting before retrying.' } }));
      }, 15000));
      return true;
    },
    resolve(id: string) {
      cancel(id);
      update('requestDelivery', previous => { const next = { ...previous }; delete next[id]; return next; });
      update('resolvedRequests', previous => {
        const next = { ...previous, [id]: true as const };
        const keys = Object.keys(next); for (const stale of keys.slice(0, Math.max(0, keys.length - 512))) delete next[stale];
        return next;
      });
    },
    isResolved(id: string) { const state = read(); return !!(state.active && state.sessions.get(state.active)?.resolvedRequests[id]); },
    fail,
    disconnected() {
      for (const timer of timers.values()) clearTimeout(timer); timers.clear();
      for (const session of read().sessions.keys()) updateAt(session, 'requestDelivery', previous =>
        Object.fromEntries(Object.entries(previous).map(([id]) => [id, { status: 'failed' as const,
          error: 'Connection lost. Check the request after reconnecting.' }])));
    },
    dispose() { disposed = true; for (const timer of timers.values()) clearTimeout(timer); timers.clear(); },
  };
}
