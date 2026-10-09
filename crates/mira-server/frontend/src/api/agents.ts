import { ModelInfo } from '../api';
import type { AcpAgentStatus } from '../types';
// ---- engines (unified backend list: native providers + external agents) ----

export type EngineState =
  | { state: 'ready' }
  | { state: 'not_configured'; reason: string }
  | { state: 'not_found'; looked_for: string }
  | { state: 'failed'; reason: string }
  | { state: 'unavailable'; reason: string };

export type EngineFlavor = 'native' | 'external';

/** One backend row from `GET /api/engines` — health, auth summary, and
 *  (when known) the model catalog, in the same shape for every flavor. */
export type EngineSnapshot = {
  instance: string;
  driver: string;
  flavor: EngineFlavor;
  display_name: string;
  enabled: boolean;
  state: EngineState;
  models?: ModelInfo[];
  default_model?: string | null;
  auth?: string | null;
  install_hint?: string | null;
  launch?: string | null;
  /** Native rows: `markers`, `automatic` or `off`. */
  prompt_caching?: string | null;
  /** External rows: the agent's full status (versions, transport, sign-in
   *  methods). The app's agent list is built from this. */
  agent?: AcpAgentStatus | null;
};

export type EngineListView = {
  engines: EngineSnapshot[];
  active_instance?: string | null;
  active_model?: string | null;
  /** False on the first hit after boot: external rows are presence-only
   *  placeholders until the background probe lands. */
  fresh: boolean;
};

/** One provider instance's own model catalog (not the active one's). */
export async function listInstanceModels(instance: string): Promise<ModelInfo[]> {
  const r = await fetch(`/api/engines/${encodeURIComponent(instance)}/models`);
  const body = (await r.json().catch(() => ({}))) as { models?: ModelInfo[]; error?: string };
  if (!r.ok) throw new Error(body.error ?? `models for ${instance}: ${r.status}`);
  return body.models ?? [];
}

/** One `env` entry of an agent's setup. Secret-looking values come back
 *  masked only; the real value never reaches the browser. */
export type AgentEnvVar = { key: string; value?: string; masked?: string };

/** An external agent's setup, stored on the server in mira.yaml (#79). */
export type AgentSettings = {
  instance: string;
  driver: string;
  display_name?: string;
  enabled: boolean;
  binary_path?: string;
  home_path?: string;
  launch_args: string[];
  env: AgentEnvVar[];
  effort?: string;
  setting_sources?: string;
  has_api_key: boolean;
  api_key_masked?: string;
  api_key_env?: string;
  /** Variables the agent reads a key from; empty when it only signs in
   *  through its own CLI. */
  api_key_vars: string[];
};

/** A partial update: absent fields stay, `''` clears. In `env`, `null`
 *  keeps that variable's stored (masked) value. */
export type AgentSettingsPatch = {
  display_name?: string;
  enabled?: boolean;
  binary_path?: string;
  home_path?: string;
  launch_args?: string[];
  env?: Record<string, string | null>;
  effort?: string;
  setting_sources?: string;
  api_key?: string;
  api_key_env?: string;
};

export async function agentSettingsRequest(
  instance: string,
  init?: RequestInit,
): Promise<AgentSettings> {
  const r = await fetch(`/api/engines/${encodeURIComponent(instance)}/settings`, init);
  const body = (await r.json().catch(() => ({}))) as AgentSettings & { error?: string };
  if (!r.ok) throw new Error(body.error ?? `agent settings ${r.status}`);
  return body;
}

export function getAgentSettings(instance: string): Promise<AgentSettings> {
  return agentSettingsRequest(instance);
}

export function putAgentSettings(
  instance: string,
  patch: AgentSettingsPatch,
): Promise<AgentSettings> {
  return agentSettingsRequest(instance, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(patch),
  });
}

export async function listEngines(refresh = false): Promise<EngineListView> {
  const r = await fetch(`/api/engines${refresh ? '?refresh=1' : ''}`);
  if (!r.ok) {
    // No engines info is a degraded picker, not a broken app: native
    // model picking still works through /api/models.
    return { engines: [], fresh: false };
  }
  return (await r.json()) as EngineListView;
}
