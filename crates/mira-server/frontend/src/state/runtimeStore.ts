import { createStore } from '@tanstack/react-store';
import type { QueuedComposerMessage } from '../components/Composer';
import type { QueuedInput,ServerMsg } from '../types';
import { emptyProjection,reduceSessionField } from './runtimeReducer';
import { type SessionData } from './sessionData';
type Work = Extract<ServerMsg, { type: 'runtime_work_updated' }>['work'];
export type SessionProjection = SessionData & {
  busy: boolean;
  thinking: boolean;
  nativeTurns: ReadonlySet<string>;
  nativeWork: readonly Work[];
};
const idle = emptyProjection;
const EMPTY = idle();
export type RuntimeState = { active: string | null; selected: string | null; generation: number; attachment: number; sessions: ReadonlyMap<string, SessionProjection>; queued: Map<string, QueuedComposerMessage[]>; recovery: Record<string, QueuedInput[]> };
export const selectActiveSession = (state: RuntimeState) => (state.active ?? state.selected) ? state.sessions.get((state.active ?? state.selected)!) ?? EMPTY : EMPTY;

/** Retains running projections; bounds reconstructable idle projections to 20. */
export function createRuntimeStore() {
  const store = createStore<RuntimeState>({ active: null, selected: null, generation: 0, attachment: 0, sessions: new Map(), queued: new Map(), recovery: {} });
  let disposed = false;
  let staged: RuntimeState | null = null;
  let depth = 0;
  const effects: (() => void)[] = [];
  const read = () => staged ?? store.get();
  function commit(state: RuntimeState) {
    if (depth) staged = state;
    else store.setState(() => state);
  }
  function batch<T>(apply: () => T): T {
    depth++;
    try { return apply(); }
    finally {
      if (--depth === 0) {
        if (staged) { const next = staged; staged = null; store.setState(() => next); }
        const pending = effects.splice(0); for (const effect of pending) effect();
      }
    }
  }
  const retained = (key: string, turn: SessionProjection) => key === read().active || turn.busy ||
    turn.secretRequests.length > 0 || turn.pendingAcpMode !== null ||
    [...turn.subagentState.values()].some(stream => !stream.done || stream.pendingReview !== null) || (read().queued.get(key)?.length ?? 0) > 0 ||
    Object.values(turn.requestDelivery).some(request => request.status === 'submitting') || turn.entries.some(entry => entry.kind === 'tool' &&
      (entry.status === 'pending' || entry.plan?.decision === null || entry.askUser?.decision === null));
  function update(transform: (turn: SessionProjection) => SessionProjection) {
    if (disposed || !read().active) return;
    const state = read();
    const previous = selectActiveSession(state);
    const next = transform(previous);
    if (next === previous) return;
    const sessions = new Map(state.sessions);
    sessions.delete(state.active!);
    sessions.set(state.active!, next);
    let idleCount = [...sessions.values()].filter((turn) => !turn.busy).length;
    for (const [key, turn] of sessions) {
      if (idleCount <= 20) break;
      if (!retained(key, turn)) { sessions.delete(key); idleCount--; }
    }
    commit({ ...state, generation: state.generation + (!previous.busy && next.busy ? 1 : 0), sessions });
  }
  const source = { get: () => read(), subscribe: store.subscribe.bind(store) };
  return {
    source,
    busy: { get current() { return read().active ? selectActiveSession(read()).busy : false; } },
    batch,
    afterTransition(effect: () => void) { if (depth) effects.push(effect); else if (!disposed) effect(); },
    updateData<K extends keyof SessionData>(key: K, apply: SessionData[K] | ((previous: SessionData[K]) => SessionData[K])) {
      if (disposed || !read().active) return;
      const state = read();
      commit(reduceSessionField(state, { type: 'session-field', session: state.active!, field: key, value: apply }));
    },
    updateDataAt<K extends keyof SessionData>(session: string, key: K, apply: (previous: SessionData[K]) => SessionData[K]) {
      if (disposed) return;
      commit(reduceSessionField(read(), { type: 'session-field', session, field: key, value: apply }));
    },
    updateQueues(apply: (previous: RuntimeState['queued']) => RuntimeState['queued']) {
      if (!disposed) { const state = read(); commit({ ...state, queued: apply(state.queued) }); }
    },
    updateRecovery(apply: (previous: RuntimeState['recovery']) => RuntimeState['recovery']) {
      if (!disposed) { const state = read(); commit({ ...state, recovery: apply(state.recovery) }); }
    },
    updateEntriesAt(session: string, apply: (entries: SessionData['entries']) => SessionData['entries']) {
      if (disposed) return;
      commit(reduceSessionField(read(), { type: 'session-field', session, field: 'entries', value: apply }));
    },
    reconcileActivity(running: ReadonlySet<string>) {
      if (disposed) return;
      const state = read(); const sessions = new Map(state.sessions); let changed = false;
      for (const [key, session] of sessions) {
        if (key !== state.active && session.busy !== running.has(key)) {
          const busy = running.has(key);
          sessions.set(key, { ...session, busy, thinking: busy && session.thinking,
            nativeTurns: busy ? session.nativeTurns : new Set(), nativeWork: busy ? session.nativeWork : [] });
          changed = true;
        }
      }
      let recent = 0;
      for (const [key, session] of [...sessions].reverse()) {
        if (retained(key, session)) continue;
        if (++recent > 20) { sessions.delete(key); changed = true; }
      }
      if (changed) commit({ ...state, sessions });
    },
    commands: {
      hydrate(session: string, running: boolean, work: readonly Work[]) {
        if (disposed) return;
        const state = read();
        {
          const sessions = new Map(state.sessions);
          sessions.delete(session);
          sessions.set(session, { ...(state.sessions.get(session) ?? idle()), busy: running, thinking: running, stopRequested: false, nativeTurns: new Set(), nativeWork: work.filter(item => ['pending', 'running', 'waiting'].includes(item.status)) });
          let idleCount = [...sessions.values()].filter(turn => !turn.busy).length;
          for (const [key, turn] of sessions) {
            if (idleCount <= 20) break;
            if (key !== session && !retained(key, turn)) { sessions.delete(key); idleCount--; }
          }
          commit({ ...state, active: session, selected: session, attachment: state.attachment + 1, generation: state.generation + 1, sessions });
        }
      },
      suspend(selected?: string) {
        if (!disposed) { const state = read(); commit({ ...state, active: null, selected: selected ?? state.selected, attachment: state.attachment + 1, generation: state.generation + 1 }); }
      },
      reportTurnRunning(busy: boolean) {
        update(turn => turn.busy === busy && turn.thinking === busy && !turn.stopRequested ? turn :
          { ...turn, busy, thinking: busy, stopRequested: false });
      },
      reportThinking(thinking: boolean) {
        update(turn => {
          const next = thinking && turn.busy;
          return turn.thinking === next ? turn : { ...turn, thinking: next };
        });
      },
      nativeTurn(id: string, running: boolean) {
        update(turn => {
          const nativeTurns = new Set(turn.nativeTurns);
          if (running) nativeTurns.add(id); else nativeTurns.delete(id);
          return { ...turn, nativeTurns, busy: running || turn.busy, thinking: running || turn.thinking };
        });
      },
      nativeWork(work: Work) {
        update(turn => {
          const active = ['pending', 'running', 'waiting'].includes(work.status);
          const nativeWork = turn.nativeWork.filter(item => item.id !== work.id);
          if (active) nativeWork.push(work);
          const busy = active || (turn.busy && (turn.nativeTurns.size > 0 || nativeWork.length > 0));
          return { ...turn, nativeWork, busy, thinking: busy && turn.thinking };
        });
      },
      finishAgent() {
        update(turn => ({ ...turn, busy: turn.busy && (turn.nativeTurns.size > 0 || turn.nativeWork.length > 0), thinking: false, stopRequested: false }));
      },
    },
    dispose() { disposed = true; effects.length = 0; store.setState(() => ({ active: null, selected: null, generation: 0, attachment: 0, sessions: new Map(), queued: new Map(), recovery: {} })); },
  };
}
