import { useCallback, useSyncExternalStore } from 'react';
import { useOptionalRuntime } from '../state/RuntimeProvider';

/** Transcript lookups belong to the displayed chat, backend, account and cwd. */
export function useReferenceScope() {
  const runtime = useOptionalRuntime();
  const subscribe = useCallback((notify: () => void) => {
    const subscription = runtime?.sessionState.subscribe(notify);
    return () => subscription?.unsubscribe();
  }, [runtime]);
  const snapshot = useCallback(() => {
    const state = runtime?.sessionState.get();
    const session = state?.active ?? state?.selected ?? null;
    const cwd = session ? state?.sessions.get(session)?.cwd ?? '' : '';
    return JSON.stringify([runtime?.scope.backend ?? '', runtime?.scope.account ?? '', session, cwd]);
  }, [runtime]);
  const cacheKey = useSyncExternalStore(subscribe, snapshot, snapshot);
  const session = (JSON.parse(cacheKey) as [string, string, string | null, string])[2] ?? undefined;
  return { cacheKey, session };
}
