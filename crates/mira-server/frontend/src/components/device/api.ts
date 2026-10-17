/** The Device pane's side of `docs/devices.md`. Cached through `queries.ts`. */

export type HubStatus = 'off' | 'installing' | 'starting' | 'ready' | 'failed';
export type AgentTools = 'absent' | 'installing' | 'ready' | 'failed';
export type Platform = 'ios' | 'android';
export type PlatformSupport = { available: boolean; reason: string | null };

export type DeviceState = {
  hub: HubStatus;
  hubDetail: string | null;
  /** 0…1 while the hub (and Node, if needed) downloads. */
  hubProgress: number | null;
  hubEnabled: boolean;
  agentAccess: boolean;
  agentTools: AgentTools;
  agentDetail: string | null;
  /** 0…1 while agent-device downloads. */
  agentProgress: number | null;
  platforms: Record<Platform, PlatformSupport>;
  versions: { hub: string; agentDevice: string; node: string | null; hubInstalled: boolean; agentDeviceInstalled: boolean };
};

export type IosStream = { video: string; mjpeg: string; config: string; input: string };
export type AndroidStream = { ws: string };

export type Device = {
  platform: Platform;
  id: string;
  name: string;
  os: string;
  booted: boolean;
  frame: string | null;
  stream: IosStream | AndroidStream | null;
};

export type DeviceList = { devices: Device[]; notes: string[] };

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`/api/devices${path}`, { cache: 'no-store', ...init });
  if (response.status === 204) return undefined as T;
  if ((response.headers.get('content-type') ?? '').startsWith('image/')) return (await response.blob()) as T;
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error ?? `Device request failed (${response.status}).`);
  return body as T;
}

const post = (body: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
});

export const getDeviceState = () => request<DeviceState>('/state');
export const updateDeviceSettings = (body: { hubEnabled?: boolean; agentAccess?: boolean }) =>
  request<DeviceState>('/settings', post(body));
export const listDevices = () => request<DeviceList>('/list');
export const bootDevice = (platform: Platform, id: string) =>
  request<{ id: string; stream: IosStream | AndroidStream }>('/boot', post({ platform, id }));
export const shutdownDevice = (platform: Platform, id: string) => request<void>('/shutdown', post({ platform, id }));
export const screenshotDevice = (platform: Platform, id: string) => request<Blob>('/screenshot', post({ platform, id }));

export type DeviceAction =
  | { action: 'appearance'; value: 'light' | 'dark' }
  | { action: 'textSize'; value: string }
  | { action: 'location'; latitude: number; longitude: number }
  | { action: 'permission'; app: string; service: string; grant: boolean }
  | { action: 'push'; app: string; payload: unknown }
  | { action: 'orientation'; degrees: 0 | 90 | 180 | 270 }
  | { action: 'network'; wifi?: boolean; data?: boolean };

export const runDeviceAction = (platform: Platform, id: string, action: DeviceAction) =>
  request<Record<string, unknown>>('/action', post({ platform, id, ...action }));
