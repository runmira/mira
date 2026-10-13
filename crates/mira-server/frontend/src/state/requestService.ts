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
  // Which session submitted each in-flight request. `fail`/`resolve`/`cancel`
  // run on delayed events that may arrive after a session switch; looking the
  // target up through `read().active` would read and write the *new* session's
  // maps — suppressing its requests on id reuse and leaking the old one's
  // timers. The timer callback already captured its session; this extends the
  // same binding to the other three operations.
  const owners = new Map<string, string>();
  let disposed = false;
  function ownerOf(id: string) { return owners.get(id) ?? read().active; }
  function cancel(id: string) {
    const session = ownerOf(id);
    const key = `${session}:${id}`;
    const timer = timers.get(key); if (timer) clearTimeout(timer); timers.delete(key);
  }
  function fail(id: string, error: string) {
    const session = ownerOf(id);
    cancel(id);
    owners.delete(id);
    if (!session) return;
    updateAt(session, 'requestDelivery', previous => ({ ...previous, [id]: { status: 'failed', error } }));
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
      owners.set(id, session);
      try {
        if (!send(message)) { fail(id, 'Reconnect before submitting this decision.'); return false; }
      } catch { fail(id, 'The decision could not be sent. Reconnect and try again.'); return false; }
      // Do not store the message, answers, or secret value in transport bookkeeping.
      const key = `${session}:${id}`;
      timers.set(key, setTimeout(() => {
        timers.delete(key);
        owners.delete(id);
        const current = read();
        if (!disposed && current.sessions.get(session)?.requestDelivery[id]?.status === 'submitting')
          updateAt(session, 'requestDelivery', previous => ({ ...previous, [id]: { status: 'failed',
            error: 'The server has not confirmed this decision. Check after reconnecting before retrying.' } }));
      }, 15000));
      return true;
    },
    resolve(id: string) {
      const session = ownerOf(id);
      cancel(id);
      owners.delete(id);
      if (!session) return;
      updateAt(session, 'requestDelivery', previous => { const next = { ...previous }; delete next[id]; return next; });
      updateAt(session, 'resolvedRequests', previous => {
        const next = { ...previous, [id]: true as const };
        const keys = Object.keys(next); for (const stale of keys.slice(0, Math.max(0, keys.length - 512))) delete next[stale];
        return next;
      });
    },
    isResolved(id: string) { const state = read(); return !!(state.active && state.sessions.get(state.active)?.resolvedRequests[id]); },
    fail,
    disconnected() {
      for (const timer of timers.values()) clearTimeout(timer); timers.clear(); owners.clear();
      for (const session of read().sessions.keys()) updateAt(session, 'requestDelivery', previous =>
        Object.fromEntries(Object.entries(previous).map(([id]) => [id, { status: 'failed' as const,
          error: 'Connection lost. Check the request after reconnecting.' }])));
    },
    dispose() { disposed = true; for (const timer of timers.values()) clearTimeout(timer); timers.clear(); owners.clear(); },
  };
}
