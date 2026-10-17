import { queryOptions, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect } from 'react';
import { useOptionalRuntime } from '../../state/RuntimeProvider';
import {
  bootDevice, getDeviceState, listDevices, runDeviceAction, shutdownDevice, updateDeviceSettings,
  type DeviceAction, type DeviceState, type Platform,
} from './api';

/* Device data for the Device pane, its setup and Settings → Simulators,
 * behind one set of keys so they share a cache. The state is pushed over
 * `/api/devices/events` into the cache; the device list polls while shown. */

type Runtime = NonNullable<ReturnType<typeof useOptionalRuntime>>;

/** Previews and tests render without a runtime; keys still need a scope. */
function scoped(runtime: Runtime | null, ...parts: string[]) {
  return runtime ? runtime.resourceKey(...(parts as [string, ...string[]])) : (['mira', 'local', ...parts] as const);
}

const LIST_EVERY_MS = 5000;

export const deviceQueries = {
  state: (runtime: Runtime | null) =>
    queryOptions({
      queryKey: scoped(runtime, 'device-state'),
      queryFn: getDeviceState,
      // Kept current by the event stream below.
      staleTime: Infinity,
      retry: false,
    }),
  list: (runtime: Runtime | null) =>
    queryOptions({
      queryKey: scoped(runtime, 'device-list'),
      queryFn: listDevices,
      staleTime: 2000,
      retry: false,
      refetchInterval: LIST_EVERY_MS,
    }),
};

/** The device state, live. */
export function useDeviceState() {
  const runtime = useOptionalRuntime();
  const client = useQueryClient();
  const options = deviceQueries.state(runtime);
  const query = useQuery(options);
  const key = JSON.stringify(options.queryKey);
  useEffect(() => {
    if (typeof EventSource === 'undefined') return;
    const source = new EventSource('/api/devices/events');
    source.onmessage = (event) => {
      try { client.setQueryData(options.queryKey, JSON.parse(event.data) as DeviceState); } catch { /* keep the last state */ }
    };
    // The browser reconnects on its own; refetch so a gap isn't missed.
    source.onopen = () => void client.invalidateQueries({ queryKey: options.queryKey, exact: true });
    return () => source.close();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by the scope
  }, [client, key]);
  return query;
}

/** Devices, polled while something shows them and the hub is up. */
export function useDeviceList(enabled: boolean) {
  return useQuery({ ...deviceQueries.list(useOptionalRuntime()), enabled });
}

export function useDeviceSettings() {
  const runtime = useOptionalRuntime();
  const client = useQueryClient();
  return useMutation({
    mutationKey: scoped(runtime, 'device-settings'),
    mutationFn: updateDeviceSettings,
    onSuccess: (state) => client.setQueryData(deviceQueries.state(runtime).queryKey, state),
  });
}

/** Boot (or attach to) and power off devices; both refresh the list. */
export function useDevicePower() {
  const runtime = useOptionalRuntime();
  const client = useQueryClient();
  const refresh = () => client.invalidateQueries({ queryKey: deviceQueries.list(runtime).queryKey, exact: true });
  const boot = useMutation({
    mutationFn: ({ platform, id }: { platform: Platform; id: string }) => bootDevice(platform, id),
    onSettled: refresh,
  });
  const shutdown = useMutation({
    mutationFn: ({ platform, id }: { platform: Platform; id: string }) => shutdownDevice(platform, id),
    onSettled: refresh,
  });
  return { boot, shutdown };
}

export function useDeviceAction(platform: Platform, id: string) {
  return useMutation({ mutationFn: (action: DeviceAction) => runDeviceAction(platform, id, action) });
}
