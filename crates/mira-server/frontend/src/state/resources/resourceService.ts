import { CancelledError,type QueryClient } from '@tanstack/react-query';
import type { BranchPrView,CommandInfo,GitStatusView,SessionDiffView,SkillView } from '../../api';
import type { EngineListView } from '../../api/agents';
import { putSettings } from '../../api/settings';
import type { SessionSummary,SettingsUpdate,SettingsView } from '../../types';
import type { RuntimeScope } from '../createAppRuntime';
export type ResourceData = {
  settings: SettingsView; engines: EngineListView; skills: SkillView[]; commands: CommandInfo[];
  sessions: SessionSummary[]; gitStatus: GitStatusView; context: import('../../api').ContextView; sessionDiff: SessionDiffView; branchPr: BranchPrView | null;
};
export type ResourceName = keyof ResourceData;
export class ResourceError extends Error { constructor(readonly status: number) { super(`Resource request failed (${status})`); } }
export const retryResource = (count: number, error: unknown) => count < 1 &&
  (error instanceof TypeError || (error instanceof ResourceError && error.status >= 500));
const paths: Record<ResourceName, string> = { settings: '/api/settings', engines: '/api/engines', skills: '/api/skills',
  commands: '/api/commands', context: '/api/context', sessions: '/api/sessions?all=true', gitStatus: '/api/git/status',
  sessionDiff: '/api/git/session-diff', branchPr: '/api/git/branch-pr' };
const contextual = new Set<ResourceName>(['engines', 'skills', 'commands', 'gitStatus', 'sessionDiff', 'branchPr', 'context']);
export function createResourceService(client: QueryClient, scope: RuntimeScope,
  context: () => { session: string | null; cwd: string; attachment: number; connected: boolean; connection: number; engine: string }) {
  let disposed = false;
  let settingsWrites: Promise<unknown> = Promise.resolve();
  const refreshes = new Map<string, { timer: ReturnType<typeof setTimeout>; resolve: (value: unknown) => void; reject: (error: unknown) => void; promise: Promise<unknown> }>();
  function key(name: ResourceName, archived = false) {
    const current = context();
    return ['mira', scope.backend, scope.account, name,
      ...(contextual.has(name) ? [current.session, current.cwd, current.engine, current.attachment, current.connection] : []), ...(name === 'sessions' ? [{ all: true, archived }] : [])] as const;
  }
  function options<K extends ResourceName>(name: K, archived = false, probe = false) {
    const owner = context();
    const queryKey = key(name, archived);
    return {
      queryKey,
      queryFn: async ({ signal }: { signal: AbortSignal }): Promise<ResourceData[K]> => {
        if (disposed) throw new CancelledError({ silent: true });
        if (contextual.has(name) && (!owner.session || !owner.connected || owner.attachment !== context().attachment || owner.connection !== context().connection)) throw new CancelledError({ silent: true });
        const suffix = name === 'sessions' && archived ? '&archived=true' : name === 'engines' && probe ? '?refresh=1' : '';
        const response = await fetch(paths[name] + suffix, { signal, cache: 'no-store' });
        if (!response.ok) throw new ResourceError(response.status);
        const body = await response.json();
        if (signal.aborted || disposed || (contextual.has(name) && (owner.attachment !== context().attachment || owner.connection !== context().connection))) throw new CancelledError({ silent: true });
        return (name === 'skills' ? body.skills : body) as ResourceData[K];
      },
      retry: retryResource, staleTime: name === 'sessions' ? 10_000 : 30_000, gcTime: 5 * 60_000,
      refetchOnWindowFocus: false, refetchOnReconnect: true,
    };
  }
  function refresh<K extends ResourceName>(name: K, archived = false, probe = false): Promise<ResourceData[K]> {
    if (disposed) return Promise.reject(new CancelledError({ silent: true }));
    const config = options(name, archived, probe);
    const id = JSON.stringify([config.queryKey, probe]);
    const existing = refreshes.get(id); if (existing) return existing.promise as Promise<ResourceData[K]>;
    let resolve!: (value: unknown) => void; let reject!: (error: unknown) => void;
    const promise = new Promise<unknown>((yes, no) => { resolve = yes; reject = no; });
    const timer = setTimeout(async () => {
      refreshes.delete(id);
      try {
        await client.cancelQueries({ queryKey: config.queryKey, exact: true });
        await client.invalidateQueries({ queryKey: config.queryKey, exact: true, refetchType: 'none' });
        resolve(await client.fetchQuery(config));
      } catch (error) { reject(error); }
    }, 30);
    refreshes.set(id, { timer, resolve, reject, promise });
    return promise as Promise<ResourceData[K]>;
  }
  function accept<K extends ResourceName>(name: K, value: ResourceData[K], queryKey: readonly unknown[] = key(name)) {
    if (disposed) return;
    void client.cancelQueries({ queryKey, exact: true });
    client.setQueryData(queryKey, value);
  }
  return {
    key, options, refresh, accept,
    async saveSettings(update: SettingsUpdate) {
      const queryKey = key('settings');
      await client.cancelQueries({ queryKey, exact: true });
      const saved = settingsWrites.catch(() => undefined).then(() => {
        if (disposed) throw new CancelledError({ silent: true });
        return putSettings(update);
      }).then(value => { accept('settings', value, queryKey); return value; });
      settingsWrites = saved; return saved;
    },
    cancelContext() {
      void client.cancelQueries({ predicate: query => contextual.has(query.queryKey[3] as ResourceName) });
    },
    dispose() {
      disposed = true;
      for (const pending of refreshes.values()) { clearTimeout(pending.timer); pending.reject(new CancelledError({ silent: true })); }
      refreshes.clear();
    },
  };
}
