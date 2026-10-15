import { createStore } from '@tanstack/react-store';
import { CancelledError, type QueryClient } from '@tanstack/react-query';
import { getTurnFileDiff } from '../api';
import type { MiraReviewComment } from '../lib/reviewComment';
import type { TurnDiffSummary } from '../types';
import type { RuntimeScope } from './createAppRuntime';

export const reviewDraftKey = (session: string | null, turn: TurnDiffSummary | null, turnKey?: string) =>
  JSON.stringify([session, turn ? turnKey ?? [turn.text, turn.occurrence] : null]);

/** Runtime-owned drafts survive tab closure; cached snapshots never reach localStorage. */
export function createReviewService(client: QueryClient, scope: RuntimeScope, resolveTurn?: (session: string, key: string) => TurnDiffSummary | undefined) {
  const store = createStore(new Map<string, MiraReviewComment[]>());
  let disposed = false;
  let running = 0;
  const queue: (() => void)[] = [];
  const prefix = ['turn-review', scope.backend, scope.account];
  async function limited<T>(work: () => Promise<T>): Promise<T> {
    if (running >= 4) await new Promise<void>(resolve => queue.push(resolve));
    else running++;
    try { if (disposed) throw new CancelledError(); return await work(); }
    finally { const next = queue.shift(); if (next) next(); else running--; }
  }
  return {
    drafts: { get: () => store.get(), subscribe: store.subscribe.bind(store) },
    updateDraft(key: string, apply: MiraReviewComment[] | ((value: MiraReviewComment[]) => MiraReviewComment[])) {
      if (disposed) return;
      const previous = store.get();
      const value = typeof apply === 'function' ? apply(previous.get(key) ?? []) : apply;
      const next = new Map(previous);
      if (value.length) next.set(key, value); else next.delete(key);
      store.setState(() => next);
    },
    async loadTurn(session: string, summary: TurnDiffSummary, signal: AbortSignal, turnKey?: string) {
      const fileDiff = (file: TurnDiffSummary["files"][number]) => {
        if (file.binary) return Promise.reject(new Error('Binary file · no text diff available.'));
        return client.fetchQuery({
          queryKey: [...prefix, session, turnKey ?? [summary.text, summary.occurrence], file.path],
          staleTime: Infinity, gcTime: 10 * 60_000,
          queryFn: ({ signal: querySignal }) => limited(async () => {
            if (querySignal.aborted) throw new CancelledError();
            const current = turnKey?.startsWith('turn:') && resolveTurn ? resolveTurn(session,turnKey) : summary;
            if (!current) throw new Error('This turn is not loaded. Reopen it after loading the earlier conversation.');
            const diff = await getTurnFileDiff(session, current.text, current.occurrence, file.path, querySignal);
            // Loaded on demand so the diff renderer stays out of the initial bundle.
            const { parseDiffFromFile } = await import('@pierre/diffs');
            return parseDiffFromFile({ name: file.path, contents: diff.before }, { name: file.path, contents: diff.after });
          }),
        }).then(value => {
          const cached = client.getQueryCache().findAll({ queryKey: prefix }).filter(query => query.state.fetchStatus === 'idle').sort((a,b) => a.state.dataUpdatedAt - b.state.dataUpdatedAt);
          for (const query of cached.slice(0, Math.max(0, cached.length - 128))) client.removeQueries({queryKey:query.queryKey, exact:true});
          return value;
        });
      };
      const results: PromiseSettledResult<Awaited<ReturnType<typeof fileDiff>>>[] = new Array(summary.files.length);
      let cursor = 0;
      await Promise.all(Array.from({length: Math.min(4,summary.files.length)}, async () => {
        while (cursor < summary.files.length) {
          const index = cursor++;
          try {
            if (signal.aborted) throw new CancelledError();
            results[index] = {status:'fulfilled',value:await fileDiff(summary.files[index])};
          } catch (reason) { results[index] = {status:'rejected',reason}; }
        }
      }));
      return results;
    },
    dispose() { disposed = true; store.setState(() => new Map()); },
  };
}
