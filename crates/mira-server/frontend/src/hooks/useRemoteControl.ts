import { useIsMutating, useMutation } from '@tanstack/react-query';
import { getRemote, remoteAccessAvailable, turnOffRemote, turnOnRemote } from '../lib/remote';
import { useAppRuntime } from '../state/RuntimeProvider';
import { useResource } from '../state/resources/hooks';

/** Machine-wide remote access, shared by the composer and context panel. */
export function useRemoteControl() {
  const runtime = useAppRuntime();
  const status = useResource('remote');
  const mutationKey = ['remote-control', runtime.scope.backend, runtime.scope.account];
  const busy = useIsMutating({ mutationKey }) > 0;
  const mutation = useMutation({
    mutationKey,
    scope: { id: JSON.stringify(mutationKey) },
    mutationFn: async (enabled: boolean) => {
      const current = await getRemote();
      if (current.enabled === enabled && (!enabled || !current.port_stale)) return current;
      if (enabled && !current.supported) throw new Error('Remote control is not supported on this computer.');
      if (enabled && !remoteAccessAvailable()) throw new Error('Sign in to Mira in Settings to enable remote control.');
      return enabled ? turnOnRemote(current) : turnOffRemote(current);
    },
    onSuccess: remote => runtime.resources.accept('remote', remote),
    onSettled: () => { void runtime.resources.refresh('remote').catch(() => {}); },
  });
  return { remote: status.data, busy, error: mutation.error ?? status.error, setEnabled: mutation.mutateAsync };
}
