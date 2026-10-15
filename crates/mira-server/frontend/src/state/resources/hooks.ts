import { useQuery } from '@tanstack/react-query';
import { useSelector } from '@tanstack/react-store';
import { useAppRuntime } from '../RuntimeProvider';
import { selectActiveSession } from '../runtimeStore';
import type { ResourceData,ResourceName } from './resourceService';
export function useResource<K extends ResourceName>(name: K, enabled = true, archived = false) {
  const runtime = useAppRuntime();
  const connected = useSelector(runtime.connection, state => state.status === 'open');
  const identity = useSelector(runtime.sessionState, state => JSON.stringify([state.active, state.attachment,
    selectActiveSession(state).cwd, selectActiveSession(state).model, selectActiveSession(state).engine, selectActiveSession(state).environment]));
  const active = JSON.parse(identity)[0] !== null;
  const repository = name === 'gitStatus' || name === 'sessionDiff';
  const pullRequest = name === 'branchPr';
  const running = useSelector(runtime.sessionState, state => repository && selectActiveSession(state).busy);
  const contextual = !['settings', 'sessions', 'engines', 'remote'].includes(name);
  return useQuery<ResourceData[K], Error>({ ...runtime.resources.options(name, archived), enabled: enabled && (!contextual || (connected && active)),
    // Agent processes and long-running shell tools write outside Mira's edit
    // events. Keep the visible chat's changes current even before they finish.
    refetchOnWindowFocus: repository || pullRequest ? 'always' : false,
    refetchInterval: pullRequest ? 30000 : repository ? (running ? 3000 : 15000) : name === 'remote' ? query => {
      const remote = query.state.data as import('../../lib/remote').RemoteStatus | undefined;
      return remote?.enabled && (remote.status === 'starting' || remote.status === 'reconnecting') ? 1500 : 5000;
    } : name === 'engines' ? query => {
      const data = query.state.data as { fresh?: boolean } | undefined;
      return data && !data.fresh ? 2500 : false;
    } : false,
  });
}
