export type Delivery = 'background' | 'auto' | 'foreground';
export type ComputerWindow = { id: string; epoch: string; title: string; app: string };
export type PairedComputer = { id: string; name: string; platform: string };
export type ComputerIdentity = { name: string; platform: string; machine_id: string; background_input: boolean; limitation: string; backend: 'cua' | 'native' };
export type ComputerView = {
  enabled: boolean; target: string; delivery: Delivery; window: ComputerWindow | null;
  computers: PairedComputer[]; clients: { id: string; name: string; created_at: number }[]; can_change: boolean;
};
export type ComputerUpdate = Pick<ComputerView, 'enabled' | 'target' | 'delivery' | 'window'>;
export type PairingLink = { code: string; expires_at: number; link: string };
async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`/api/computer${path}`, { cache: 'no-store', ...init });
  if (response.status === 204) return undefined as T;
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error ?? 'Could not reach the computer. Try again.');
  return body;
}
function json(body: unknown): RequestInit { return { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }; }
export const getComputerSettings = () => request<ComputerView>('');
export const updateComputerSettings = (body: ComputerUpdate) => request<ComputerView>('', { ...json(body), method: 'PUT' });
export const getComputerStatus = (id: string) => request<ComputerIdentity>(`/targets/${encodeURIComponent(id)}/status`);
export const listComputerWindows = (id: string) => request<ComputerWindow[]>(`/targets/${encodeURIComponent(id)}/windows`);
export const openComputerPairing = () => request<PairingLink>('/pairing', { method: 'POST' });
export const closeComputerPairing = (code: string) => request<void>('/pairing', { ...json({ code }), method: 'DELETE' });
export const inspectComputerPairing = (link: string) => request<ComputerIdentity>('/pairing/inspect', json({ link }));
export const connectComputer = (link: string) => request<PairedComputer>('/pairing/connect', json({ link }));
export const unpairComputer = (id: string) => request<void>(`/targets/${encodeURIComponent(id)}`, { method: 'DELETE' });
export const revokeComputerClient = (id: string) => request<void>(`/clients/${encodeURIComponent(id)}`, { method: 'DELETE' });
