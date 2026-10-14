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
  const contextual = !['settings', 'sessions', 'engines'].includes(name);
  return useQuery<ResourceData[K], Error>({ ...runtime.resources.options(name, archived), enabled: enabled && (!contextual || (connected && active)),
    refetchInterval: name === 'engines' ? query => {
      const data = query.state.data as { fresh?: boolean } | undefined;
      return data && !data.fresh ? 2500 : false;
    } : false,
  });
}
