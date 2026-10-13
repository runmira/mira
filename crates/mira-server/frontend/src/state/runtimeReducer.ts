import type { RuntimeState,SessionProjection } from './runtimeStore';
import { createSessionData,type SessionData } from './sessionData';
export type SessionFieldEvent<K extends keyof SessionData = keyof SessionData> = {
  type: 'session-field'; session: string; field: K;
  value: SessionData[K] | ((previous: SessionData[K]) => SessionData[K]);
};
/** Pure projection update: services provide scoped identity and all external inputs. */
export function reduceSessionField<K extends keyof SessionData>(state: RuntimeState, event: SessionFieldEvent<K>): RuntimeState {
  const previous = state.sessions.get(event.session);
  if (!previous) return state;
  const value = typeof event.value === 'function'
    ? (event.value as (previous: SessionData[K]) => SessionData[K])(previous[event.field]) : event.value;
  if (Object.is(value, previous[event.field])) return state;
  const sessions = new Map(state.sessions);
  sessions.set(event.session, { ...previous, [event.field]: value });
  return { ...state, sessions };
}
export function emptyProjection(): SessionProjection {
  return { ...createSessionData(), busy: false, thinking: false, nativeTurns: new Set(), nativeWork: [] };
}
