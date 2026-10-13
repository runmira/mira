import { createStore } from '@tanstack/react-store';
import type { ServerMsg } from '../types';
type Work = Extract<ServerMsg, { type: 'runtime_work_updated' }>['work'];
export type TurnProjection = {
  busy: boolean;
  thinking: boolean;
  nativeTurns: ReadonlySet<string>;
  nativeWork: readonly Work[];
};
const idle = (): TurnProjection => ({ busy: false, thinking: false, nativeTurns: new Set(), nativeWork: [] });
const EMPTY = idle();
export type TurnState = { active: string | null; generation: number; sessions: ReadonlyMap<string, TurnProjection> };
export const selectTurn = (state: TurnState) => state.active ? state.sessions.get(state.active) ?? EMPTY : EMPTY;

/** Retains running projections; bounds reconstructable idle projections to 20. */
export function createTurnStore() {
  const store = createStore<TurnState>({ active: null, generation: 0, sessions: new Map() });
  let disposed = false;
  function update(transform: (turn: TurnProjection) => TurnProjection) {
    if (disposed || !store.get().active) return;
    const state = store.get();
    const previous = selectTurn(state);
    const next = transform(previous);
    if (next === previous) return;
    const sessions = new Map(state.sessions);
    sessions.delete(state.active!);
    sessions.set(state.active!, next);
    let idleCount = [...sessions.values()].filter((turn) => !turn.busy).length;
    for (const [key, turn] of sessions) {
      if (idleCount <= 20) break;
      if (key !== state.active && !turn.busy) { sessions.delete(key); idleCount--; }
    }
    store.setState(() => ({ ...state, generation: state.generation + (!previous.busy && next.busy ? 1 : 0), sessions }));
  }
  const source = { get: () => store.get(), subscribe: store.subscribe.bind(store) };
  return {
    source,
    busy: { get current() { return selectTurn(store.get()).busy; } },
    commands: {
      hydrate(session: string, running: boolean, work: readonly Work[]) {
        if (disposed) return;
        store.setState(state => {
          const sessions = new Map(state.sessions);
          sessions.delete(session);
          sessions.set(session, { busy: running, thinking: running, nativeTurns: new Set(), nativeWork: work.filter(item => ['pending', 'running', 'waiting'].includes(item.status)) });
          let idleCount = [...sessions.values()].filter(turn => !turn.busy).length;
          for (const [key, turn] of sessions) {
            if (idleCount <= 20) break;
            if (key !== session && !turn.busy) { sessions.delete(key); idleCount--; }
          }
          return { active: session, generation: state.generation + 1, sessions };
        });
      },
      suspend() {
        if (!disposed) store.setState(state => ({ ...state, active: null, generation: state.generation + 1 }));
      },
      setBusy(busy: boolean) {
        update(turn => turn.busy === busy && turn.thinking === busy ? turn :
          { ...turn, busy, thinking: busy });
      },
      setThinking(thinking: boolean) {
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
        update(turn => ({ ...turn, busy: turn.busy && (turn.nativeTurns.size > 0 || turn.nativeWork.length > 0), thinking: false }));
      },
    },
    dispose() { disposed = true; },
  };
}
