import type { QueuedComposerMessage } from '../components/Composer';
import type { AskUserProposal,DiffPreview,PlanProposal,ServerMsg,UsageTotals } from '../types';

/** Session-scoped resources and staging. These are not renderable or persisted state. */
function createResources() {
  return {
    historyRequest: null as string | null, historyGeneration: 0, terminalGeneration: -1,
    historyPreviews: {} as Record<string, DiffPreview>, historyOffset: 0, providerTurnCursor: 0,
    pendingProposals: new Map<string, PlanProposal>(), pendingQuestions: new Map<string, AskUserProposal>(),
    resolutions: new Map<string, Extract<ServerMsg, { type: 'prompt_resolved' | 'approval_resolved' }>>(),
    approvalStarted: {} as Record<string, number>, turnStart: null as number | null,
    turnBase: null as { turn: number; base: UsageTotals } | null,
  };
}
export function createSessionServices(selected: () => string | null, retained: () => ReadonlySet<string>) {
  const sessions = new Map<string, ReturnType<typeof createResources>>();
  const pendingSteer = new Map<string, QueuedComposerMessage>();
  let disposed = false;
  function get() {
    const key = selected() ?? '__startup__';
    if (!sessions.has(key)) sessions.set(key, createResources());
    for (const candidate of sessions.keys()) {
      if (candidate !== key && !retained().has(candidate)) sessions.delete(candidate);
    }
    return sessions.get(key)!;
  }
  return {
    pendingSteer,
    ref<K extends keyof ReturnType<typeof createResources>>(field: K) {
      return { get current() { return get()[field]; }, set current(value) { if (!disposed) get()[field] = value; } };
    },
    resolutions: () => get().resolutions,
    rememberResolution(id: string, resolution: Extract<ServerMsg, { type: 'prompt_resolved' | 'approval_resolved' }>) {
      const records = get().resolutions; records.delete(id); records.set(id, resolution);
      while (records.size > 512) records.delete(records.keys().next().value!);
    },
    dispose() { disposed = true; sessions.clear(); pendingSteer.clear(); },
  };
}
