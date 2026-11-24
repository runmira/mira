export type RemoteStatus = {
  machine_id: string;
  port: number;
  enabled: boolean;
  hostname: string | null;
  /** Mira restarted on a different port than the tunnel points at. */
  port_stale: boolean;
  status: 'off' | 'starting' | 'connected' | 'reconnecting';
  error: string | null;
  supported: boolean;
};

export function remoteAccessAvailable(): boolean { return false; }
async function backend<T>(_path: string, _body: unknown): Promise<T> { throw new Error('Hosted remote access is unavailable.'); }

async function local<T>(path: string, init: RequestInit = {}): Promise<T> {
  const r = await fetch(`/api/remote${path}`, init);
  const out = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(out?.error ?? `remote ${path || 'status'}: ${r.status}`);
  return out as T;
}

export const getRemote = () => local<RemoteStatus>('');

export async function turnOnRemote(current: RemoteStatus): Promise<RemoteStatus> {
  const { hostname, token } = await backend<{ hostname: string; token: string }>('/enable', {
    machine_id: current.machine_id,
    port: current.port,
  });
  return local<RemoteStatus>('/enable', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ hostname, token }),
  });
}

/**
 * If Mira came back on a different port than its tunnel points at (the
 * desktop app takes the next free port when its usual one is busy),
 * re-point the tunnel. Needs a signed-in session; does nothing otherwise.
 */
export async function syncRemotePort(): Promise<RemoteStatus | null> {
  const current = await getRemote().catch(() => null);
  if (!current?.enabled || !current.port_stale || !remoteAccessAvailable()) return current;
  return turnOnRemote(current);
}

export async function turnOffRemote(_current: RemoteStatus): Promise<RemoteStatus> {
  const next = await local<RemoteStatus>('/disable', { method: 'POST' });
  return next;
}
