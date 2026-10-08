/**
 * Device pairing (server side: crates/mira-server/src/pairing.rs).
 *
 * The computer running Mira is always trusted. Any other device — a phone
 * or laptop reaching it over Tailscale — pairs once with a code opened in
 * Settings → Devices, and from then on sends an HttpOnly cookie the server
 * set; nothing here handles the token itself.
 */

export type PairedDevice = {
  id: string;
  name: string;
  created_at: number;
  last_seen_at: number | null;
};

export type PairingStatus = {
  local: boolean;
  paired: boolean;
  device?: PairedDevice;
};

export type TailscaleInfo = {
  dns_name: string | null;
  ips: string[] | null;
  online: boolean | null;
};

export type DevicesView = {
  devices: PairedDevice[];
  pairing_expires_at: number | null;
  addresses: { tailscale: TailscaleInfo | null };
};

async function errorText(r: Response, fallback: string): Promise<string> {
  try {
    const j = await r.json();
    return j.error ?? j.message ?? fallback;
  } catch {
    return fallback;
  }
}

export async function getPairingStatus(): Promise<PairingStatus> {
  const r = await fetch('/api/pairing/me', { cache: 'no-store' });
  if (!r.ok) throw new Error(`pairing status ${r.status}`);
  return r.json();
}

export async function listDevices(): Promise<DevicesView> {
  const r = await fetch('/api/pairing/devices', { cache: 'no-store' });
  if (!r.ok) throw new Error(await errorText(r, `devices ${r.status}`));
  return r.json();
}

export async function openPairingCode(): Promise<{ code: string; expires_at: number }> {
  const r = await fetch('/api/pairing/code', { method: 'POST' });
  if (!r.ok) throw new Error(await errorText(r, `pairing code ${r.status}`));
  return r.json();
}

export async function pairThisDevice(code: string, name: string): Promise<PairedDevice> {
  const r = await fetch('/api/pairing/pair', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ code, name }),
  });
  if (!r.ok) throw new Error(await errorText(r, `pairing failed (${r.status})`));
  return (await r.json()).device;
}

export async function revokeDevice(id: string): Promise<void> {
  const r = await fetch(`/api/pairing/devices/${encodeURIComponent(id)}`, { method: 'DELETE' });
  if (!r.ok && r.status !== 404) throw new Error(await errorText(r, `revoke ${r.status}`));
}

/** "iPhone · Safari"-style default name for this device. */
export function guessDeviceName(ua = navigator.userAgent): string {
  const os = /iPhone/.test(ua)
    ? 'iPhone'
    : /iPad/.test(ua)
      ? 'iPad'
      : /Android/.test(ua)
        ? 'Android'
        : /Mac OS X|Macintosh/.test(ua)
          ? 'Mac'
          : /Windows/.test(ua)
            ? 'Windows'
            : /Linux/.test(ua)
              ? 'Linux'
              : 'Device';
  const browser = /Edg\//.test(ua)
    ? 'Edge'
    : /Firefox\//.test(ua)
      ? 'Firefox'
      : /Chrome\//.test(ua)
        ? 'Chrome'
        : /Safari\//.test(ua)
          ? 'Safari'
          : '';
  return browser ? `${os} · ${browser}` : os;
}
