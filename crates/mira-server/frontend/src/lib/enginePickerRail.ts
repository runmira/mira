import type { EngineSnapshot } from '../api';
import type { AcpAgentStatus, SessionEngine } from '../types';
export function enabledAgentRail(engines: EngineSnapshot[] | null | undefined, agents: AcpAgentStatus[] | null | undefined, engine: SessionEngine | null) {
  const items: { key: string; kind: 'agent'; driver: string; instance: string; name: string; status?: AcpAgentStatus }[] = [];
  const seen = new Set<string>();
  for (const row of engines ?? []) {
    if (row.flavor !== 'external' || !row.enabled) continue;
    seen.add(row.instance);
    items.push({ key: `agent:${row.instance}`, kind: 'agent', driver: row.driver,
      instance: row.instance, name: row.display_name,
      status: row.agent ?? (row.instance === row.driver ? agents?.find(a => a.kind === row.driver) : undefined) });
  }
  if (engines == null) for (const a of agents ?? []) {
    seen.add(a.kind);
    items.push({ key: `agent:${a.kind}`, kind: 'agent', driver: a.kind, instance: a.kind, name: a.display_name, status: a });
  }
  const active = engine?.instance ?? engine?.driver;
  if (engine?.kind === 'agent' && engine.driver && active && !seen.has(active)) {
    items.push({ key: `agent:${active}`, kind: 'agent', driver: engine.driver, instance: active, name: engine.display_name });
  }
  return items;
}

/** Settings can render configured rows before an asynchronous probe finishes. */
export function externalAgentStatuses(engines: EngineSnapshot[]): AcpAgentStatus[] {
  return engines.filter(e => e.flavor === 'external' && e.instance === e.driver).map(e => e.agent ?? ({
    kind: e.driver, display_name: e.display_name,
    state: e.state.state === 'ready' || e.state.state === 'not_found' || e.state.state === 'failed'
      ? e.state : { state: 'failed', reason: e.state.reason },
    auth: e.auth, auth_method_ids: [], launch: e.launch ?? '', models: [], install_hint: e.install_hint ?? undefined,
  }));
}
