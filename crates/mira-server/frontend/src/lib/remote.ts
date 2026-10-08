/**
 * Remote access: reach this computer's Mira from a phone at a stable
 * https://m-….runmira.dev, with no networking setup.
 *
 * Turning it on asks the `remote-access` edge function (with the user's
 * Mira session) for this computer's tunnel, then hands the hostname and
 * connector token to the local server, which runs Cloudflare's connector
 * (crates/mira-server/src/remote.rs). Turning it off stops the connector
 * first, then deletes the tunnel. Pairing still decides who gets in.
 */
import { getSupabase } from './supabase';

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

export function remoteAccessAvailable(): boolean {
  return !!import.meta.env.VITE_SUPABASE_URL && getSupabase() !== null;
}

async function backend<T>(path: string, body: unknown): Promise<T> {
  const url = (import.meta.env.VITE_SUPABASE_URL as string | undefined)?.replace(/\/+$/, '');
  if (!url) throw new Error('Supabase isn’t configured for this build.');
  const { data } = await getSupabase()!.auth.getSession();
  const token = data.session?.access_token;
  if (!token) throw new Error('Sign in to Mira first.');
  const r = await fetch(`${url}/functions/v1/remote-access${path}`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const out = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(out?.error ?? `remote access ${path}: ${r.status}`);
  return out as T;
}

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
  const { data } = await getSupabase()!.auth.getSession();
  if (!data.session) return current;
  return turnOnRemote(current);
}

export async function turnOffRemote(current: RemoteStatus): Promise<RemoteStatus> {
  const next = await local<RemoteStatus>('/disable', { method: 'POST' });
  await backend('/disable', { machine_id: current.machine_id });
  return next;
}
