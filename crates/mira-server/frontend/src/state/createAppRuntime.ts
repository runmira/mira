import { createTurnStore } from './turnStore';
import { createStore } from '@tanstack/react-store';
import { createSessionActivityStore } from './sessionActivityStore';
import { QueryClient } from '@tanstack/react-query';

export type RuntimeScope = { backend: string; account: string };
export type UiStorage = Pick<Storage, 'getItem' | 'setItem'>;
export type WorkspaceUiState = { contextOpen: boolean };

/** Browser-only UI preference. No session data or credentials are persisted. */
export function createAppRuntime(scope: RuntimeScope, storage?: UiStorage) {
  const runtimeScope = Object.freeze({ ...scope });
  let disposed = false;
  let contextOpen = false;
  try { contextOpen = storage?.getItem('mira.context.open') === '1'; } catch { /* private mode */ }
  const turns = createTurnStore();
  const activity = createSessionActivityStore();
  const ui = createStore<WorkspaceUiState>({ contextOpen });
  const queryClient = new QueryClient({
    defaultOptions: {
      // Resource-specific options must opt into retries after classifying errors.
      queries: { retry: false, staleTime: 30_000, gcTime: 5 * 60_000, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return {
    scope: runtimeScope,
    workspaceUi: { get: () => ui.get(), subscribe: ui.subscribe.bind(ui) },
    queryClient,
    sessionActivity: activity.source,
    turns: turns.source,
    busyRef: turns.busy,
    turnCommands: turns.commands,
    resourceKey: (resource: string, ...identity: readonly string[]) =>
      ['mira', runtimeScope.backend, runtimeScope.account, resource, ...identity] as const,
    commands: {
      applySessionActivity: activity.apply,
      setContextOpen(open: boolean) {
        if (disposed || ui.get().contextOpen === open) return;
        ui.setState(() => ({ contextOpen: open }));
        try { storage?.setItem('mira.context.open', open ? '1' : '0'); } catch { /* private mode */ }
      },
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      activity.dispose();
      turns.dispose();
      const queries = queryClient.getQueryCache().getAll();
      const cancelled = queryClient.cancelQueries();
      queryClient.clear();
      // A cancelled fetch may schedule GC in its finalizer after removal.
      // Destroy again after cancellation settles to release that timer too.
      return cancelled.finally(() => queries.forEach((query) => query.destroy()));
    },
  };
}
export type AppRuntime = ReturnType<typeof createAppRuntime>;
