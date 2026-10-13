import { createStore } from '@tanstack/react-store';
import {
applySessionActivity,
type SessionActivityState,
type SessionActivityUpdate,
} from '../lib/sessionActivity';

export type ActivityProjection = {
  activity: SessionActivityState | null;
  completed: ReadonlyMap<string, number>;
};
const EMPTY_COMPLETIONS: readonly string[] = [];

/** One atomic projection of authoritative running state and completion revisions. */
export function reduceSessionActivity(previous: ActivityProjection, update: SessionActivityUpdate) {
  const activity = applySessionActivity(previous.activity, update);
  if (activity === previous.activity) return { state: previous, completedIds: EMPTY_COMPLETIONS };
  const completedIds = previous.activity && activity && previous.activity.epoch === activity.epoch
    ? [...previous.activity.running].filter((id) => !activity.running.has(id))
    : EMPTY_COMPLETIONS;
  // An epoch change replaces the namespace of completion revisions.
  let completed = previous.completed;
  if (previous.activity && activity && previous.activity.epoch !== activity.epoch) {
    completed = new Map();
  } else if (completedIds.length && activity) {
    const records = new Map(completed);
    for (const id of completedIds) {
      records.delete(id);
      records.set(id, activity.revision);
    }
    while (records.size > 500) records.delete(records.keys().next().value!);
    completed = records;
  }
  // Snapshots may replay without a semantic change. Keep selector references stable.
  if (previous.activity && activity && previous.activity.running.size === activity.running.size &&
      [...activity.running].every((id) => previous.activity!.running.has(id))) {
    const sameRevision = previous.activity.epoch === activity.epoch && previous.activity.revision === activity.revision;
    if (sameRevision && completed === previous.completed) return { state: previous, completedIds: EMPTY_COMPLETIONS };
    const stableActivity = { ...activity, running: previous.activity.running };
    return { state: { activity: stableActivity, completed }, completedIds };
  }
  return { state: { activity, completed }, completedIds };
}

export function createSessionActivityStore() {
  const store = createStore<ActivityProjection>({ activity: null, completed: new Map() });
  let disposed = false;
  return {
    source: { get: () => store.get(), subscribe: store.subscribe.bind(store) },
    apply(update: SessionActivityUpdate): readonly string[] {
      if (disposed) return EMPTY_COMPLETIONS;
      const result = reduceSessionActivity(store.get(), update);
      if (result.state !== store.get()) store.setState(() => result.state);
      return result.completedIds;
    },
    dispose() { disposed = true; },
  };
}
