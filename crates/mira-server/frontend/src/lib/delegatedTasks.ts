/**
 * Tasks the open chat delegated, and the chat it was delegated from.
 *
 * A delegated task outlives the `delegate_task` call that started it, so its
 * card follows the task (`delegate_update` frames, `delegated_tasks` in
 * Ready) rather than the call. Kept outside the workspace state: the frame
 * handlers write it, the card and the composer's "Open parent" bar read it.
 */
import { useSyncExternalStore } from 'react';
import type { DelegatedTask, DelegatedTaskView, DelegateStep } from '../components/DelegateCard';

type State = {
  /** The chat these belong to. */
  sessionId: string | null;
  tasks: ReadonlyMap<string, DelegatedTaskView>;
  /** When the open chat is a delegated helper's: the chat that started it. */
  parentId: string | null;
};

let state: State = { sessionId: null, tasks: new Map(), parentId: null };
const listeners = new Set<() => void>();

function set(next: State) {
  state = next;
  for (const l of listeners) l();
}

/** A chat's Ready: its tasks and parent replace what was shown. Steps
 *  already streamed for the same chat's tasks are kept. */
export function applyReady(sessionId: string, tasks: DelegatedTask[], parentId: string | null) {
  const prior = state.sessionId === sessionId ? state.tasks : new Map<string, DelegatedTaskView>();
  set({
    sessionId,
    parentId,
    tasks: new Map(tasks.map((t) => [t.id, { ...t, steps: prior.get(t.id)?.steps ?? [] }])),
  });
}

export function applyUpdate(task: DelegatedTask) {
  const tasks = new Map(state.tasks);
  tasks.set(task.id, { ...task, steps: state.tasks.get(task.id)?.steps ?? [] });
  set({ ...state, tasks });
}

/** One step the task's helper took. False when the task isn't known (the
 *  caller then routes the step by call id instead). */
export function applyStep(taskId: string, step: DelegateStep): boolean {
  const t = state.tasks.get(taskId);
  if (!t) return false;
  const tasks = new Map(state.tasks);
  tasks.set(taskId, { ...t, steps: [...t.steps, step] });
  set({ ...state, tasks });
  return true;
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useDelegatedTasks(): State {
  return useSyncExternalStore(subscribe, () => state);
}
