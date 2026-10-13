/** Bounded display pacing adapted from ZCode's QueuedSummaryContent (Apache-2.0).
 * This is presentation only; underlying runtime facts are never delayed. */
export type SummarySnapshot<T> = { key: string; version: string; value: T };
export function createSummaryPlayback<T>(initial: SummarySnapshot<T>, emit: (value: SummarySnapshot<T>) => void,
  timing = { now: () => performance.now(), schedule: (fn: () => void, delay: number) => setTimeout(fn, delay), cancel: (timer: ReturnType<typeof setTimeout>) => clearTimeout(timer) }) {
  let current = initial;
  let queue: SummarySnapshot<T>[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  function clear() { if (timer !== undefined) timing.cancel(timer); timer = undefined; }
  function show(next: SummarySnapshot<T>) { current = next; emit(next); }
  function promote(next: SummarySnapshot<T>) {
    show(next); clear();
    const expected = timing.now() + 800;
    timer = timing.schedule(() => {
      timer = undefined;
      if (disposed) return;
      // Never replay a backlog after a blocked event loop or background tab.
      if (timing.now() - expected > 250) queue = queue.slice(-1);
      const next = queue.shift();
      if (next) promote(next);
    }, 800);
  }
  return {
    update(next: SummarySnapshot<T>, enabled: boolean) {
      if (disposed) return;
      if (!enabled) {
        clear(); queue = [];
        if (current.key !== next.key || current.version !== next.version) show(next);
        return;
      }
      if (current.key === next.key) {
        if (current.version !== next.version) show(next);
        return;
      }
      const existing = queue.findIndex(item => item.key === next.key);
      if (existing >= 0) { queue[existing] = next; return; }
      if (timer === undefined) promote(next);
      else queue = queue.length ? [queue[0], next] : [next];
    },
    dispose() { disposed = true; clear(); queue = []; },
  };
}
