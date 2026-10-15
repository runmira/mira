import type { EngineSnapshot } from '../api';
import type { AcpAgentStatus } from '../types';
export function enabledAgentRail(engines: EngineSnapshot[] | null | undefined, agents: AcpAgentStatus[] | null | undefined) {
  const items: { key: string; kind: 'agent'; driver: string; instance: string; name: string; status?: AcpAgentStatus }[] = [];
  for (const row of engines ?? []) {
    if (row.flavor !== 'external' || !row.enabled) continue;
    items.push({ key: `agent:${row.instance}`, kind: 'agent', driver: row.driver,
      instance: row.instance, name: row.display_name,
      status: row.agent ?? (row.instance === row.driver ? agents?.find(a => a.kind === row.driver) : undefined)
        ?? (row.state?.state === 'not_found' ? { kind: row.driver, display_name: row.display_name, state: row.state, auth_method_ids: [], launch: row.launch ?? '' } : undefined) });
  }
  return items.sort(comparePickerItems);
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

/** Health responses enrich rows; only the registry determines membership and order. */
export function enabledPickerRail(engines: EngineSnapshot[] | null | undefined, agents: AcpAgentStatus[] | null | undefined, currentKey?: string) {
  const providers = (engines ?? []).filter(row => row.enabled && row.flavor === 'native').map(row => ({
    key: `provider:${row.instance}`, kind: 'provider' as const, instance: row.instance, name: row.display_name, snapshot: row,
  })).sort(comparePickerItems);
  return [...providers, ...enabledAgentRail(engines, agents)].filter(item => item.key === currentKey || !unusable(item));
}

/** Enabled engines the picker leaves out because they can't run here yet,
 *  so Settings and the picker can say how many and why. */
export function hiddenFromPicker(engines: EngineSnapshot[] | null | undefined, agents: AcpAgentStatus[] | null | undefined, currentKey?: string) {
  const all = [
    ...(engines ?? []).filter(row => row.enabled && row.flavor === 'native').map(row => ({ key: `provider:${row.instance}`, kind: 'provider' as const, snapshot: row })),
    ...enabledAgentRail(engines, agents),
  ];
  const hidden = all.filter(item => item.key !== currentKey && unusable(item));
  return { providers: hidden.filter(i => i.kind === 'provider').length, agents: hidden.filter(i => i.kind === 'agent').length };
}

/** Definitively unusable: an agent that isn't installed, or a provider with
 *  no key or base URL. Rows still being checked stay listed, so nothing
 *  disappears on a slow probe; they reappear once installed or configured. */
function unusable(item: { kind: 'provider'; snapshot: EngineSnapshot } | { kind: 'agent'; status?: AcpAgentStatus; instance: string }) {
  if (item.kind === 'provider') return item.snapshot.state?.state === 'not_configured';
  return item.status?.state?.state === 'not_found';
}
function comparePickerItems(a: {name:string;key:string}, b: {name:string;key:string}) {
  return a.name.localeCompare(b.name, 'en', {sensitivity:'base',numeric:true}) || a.key.localeCompare(b.key, 'en');
}
