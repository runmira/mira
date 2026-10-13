import { QueryClient } from '@tanstack/react-query';
import { createStore } from '@tanstack/react-store';
import { createDiagnostics } from './diagnostics';
import { createDraftService } from './draftService';
import { createPreferencesStore } from './preferencesStore';
import { createRequestService } from './requestService';
import { createResourceService } from './resources/resourceService';
import { createRuntimeStore,selectActiveSession } from './runtimeStore';
import { createSessionActivityStore } from './sessionActivityStore';
import { createSessionServices } from './sessionServices';
import { createStreamService } from './streamService';
import { createWorkspaceService } from './workspaceService';
import { createWorkspaceUiStore } from './workspaceUiStore';

export type RuntimeScope = { backend: string; account: string };
export type UiStorage = Pick<Storage, 'getItem' | 'setItem'>;
export type { WorkspaceUiState } from './workspaceUiStore';

/** Browser-only UI preference. No session data or credentials are persisted. */
export function createAppRuntime(scope: RuntimeScope, storage?: UiStorage) {
  const runtimeScope = Object.freeze({ ...scope });
  let disposed = false;
  const diagnostics = createDiagnostics(import.meta.env.DEV);
  const preferences = createPreferencesStore(storage);
  const service = createWorkspaceService();
  const drafts = createDraftService(runtimeScope);
  const ui = createWorkspaceUiStore(storage);
  const queryClient = new QueryClient({
    defaultOptions: {
      // Resource-specific options must opt into retries after classifying errors.
      queries: { retry: false, staleTime: 30_000, gcTime: 5 * 60_000, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  const connection = createStore({ status: 'connecting' as import('../ws').WsStatus, generation: 0 });
  const capabilities = createStore(new Map<string, import('../lib/acpAgents').AgentCaps>());
  const turns = createRuntimeStore();
  const streams = createStreamService(() => {
    const state = turns.source.get();
    return state.active ? { session: state.active, generation: state.generation } : null;
  }, (owner, update) => {
    const state = turns.source.get();
    if (state.active === owner.session && state.generation === owner.generation)
      turns.updateEntriesAt(owner.session, update);
  });
  const activity = createSessionActivityStore();
  const sessionServices = createSessionServices(() => turns.source.get().active ?? turns.source.get().selected,
    () => new Set(turns.source.get().sessions.keys()));
  const requests = createRequestService(turns.source.get, turns.updateData, turns.updateDataAt);
  const resources = createResourceService(queryClient, runtimeScope, () => {
    const state = turns.source.get();
    const session = selectActiveSession(state);
    return { session: state.active, cwd: session.cwd, attachment: state.attachment,
      connected: connection.get().status === 'open', connection: connection.get().generation,
      engine: JSON.stringify([session.engine?.kind, session.engine?.driver, session.engine?.instance, session.model, session.environment?.current, session.environment?.backend, session.environment?.workspace]) };
  });
  return {
    scope: runtimeScope,
    diagnostics,
    connection: { get: () => connection.get(), subscribe: connection.subscribe.bind(connection) },
    reportConnection(status: import('../ws').WsStatus) {
      if (disposed) return;
      if (status !== 'open') streams.flush();
      connection.setState(previous => ({ status, generation: previous.generation + (status === 'connecting' ? 1 : 0) }));
      if (status !== 'open') resources.cancelContext();
    },
    agentCapabilities: { get: () => capabilities.get(), subscribe: capabilities.subscribe.bind(capabilities) },
    readCapabilities: (driver: string) => capabilities.get().get(driver),
    recordCapabilities(driver: string, patch: import('../lib/acpAgents').AgentCaps) {
      if (disposed) return;
      capabilities.setState(previous => {
        const next = new Map(previous); next.delete(driver); next.set(driver, { ...previous.get(driver), ...patch });
        while (next.size > 100) next.delete(next.keys().next().value!);
        return next;
      });
    },
    drafts,
    service,
    preferences,
    workspaceUi: ui.source,
    updateWorkspaceUi: ui.update,
    queryClient,
    resources,
    sessionActivity: activity.source,
    streams,
    requests,
    requestStop() {
      if (disposed || !service.refs.wsRef.current?.sendImmediate({ type: 'interrupt' })) return false;
      turns.batch(() => {
        streams.flush();
        turns.commands.reportTurnRunning(false);
        turns.updateData('stopRequested', true);
        if (service.refs.thinkingIdleTimerRef.current !== null && typeof window !== 'undefined')
          window.clearTimeout(service.refs.thinkingIdleTimerRef.current);
        service.refs.thinkingIdleTimerRef.current = null;
      });
      return true;
    },
    beginTerminalTurn(inputId: string | undefined, startedAt: number, turnIndex: number) {
      const ordinal = sessionServices.ref('terminalOrdinal'); ordinal.current++;
      const id = inputId ?? `local:${ordinal.current}`;
      sessionServices.ref('pendingTerminals').current = [...sessionServices.ref('pendingTerminals').current,
        { id, startedAt, turnIndex, usageBase: sessionServices.ref('turnBase').current }];
    },
    reconcileTerminals(running: boolean, persistedTurns: readonly import('../types').TurnMeta[] = []) {
      const pending = sessionServices.ref('pendingTerminals');
      // Ready can resume this session's live turn after a reconnect or attach.
      // Preserve its captured input identity and usage baseline; discard only
      // completions confirmed by the snapshot (or all when it is idle).
      pending.current = running ? pending.current.filter(turn => persistedTurns[turn.turnIndex]?.ended_at == null) : [];
      sessionServices.ref('terminalGeneration').current = '';
      sessionServices.ref('terminalSourceGeneration').current = -1;
    },
    acceptTerminal() {
      const state = turns.source.get(); if (!state.active || disposed) return null;
      // Ordered attached-socket terminals consume identities captured on send.
      // A follow-up cannot overwrite the earlier turn's terminal identity.
      const pending = sessionServices.ref('pendingTerminals');
      const captured = pending.current[0];
      const terminal = sessionServices.ref('terminalGeneration');
      const key = captured ? `${state.attachment}:${captured.id}` : `generation:${state.generation}`;
      if (!captured && sessionServices.ref('terminalSourceGeneration').current === state.generation) return null;
      sessionServices.ref('terminalSourceGeneration').current = state.generation;
      terminal.current = key;
      if (captured) pending.current = pending.current.slice(1);
      return { ...captured, pending: pending.current.length > 0 };
    },
    sessionServices,
    sessionState: turns.source,
    transition: turns.batch,
    effect: turns.afterTransition,
    updateSessionData: turns.updateData,
    updateSessionDataAt: turns.updateDataAt,
    updateQueues: turns.updateQueues,
    updateRecovery: turns.updateRecovery,
    updateEntriesAt: turns.updateEntriesAt,
    readSession: () => selectActiveSession(turns.source.get()),
    isTurnRunning: () => turns.busy.current,
    turnCommands: turns.commands,
    resourceKey: (resource: string, ...identity: readonly string[]) =>
      ['mira', runtimeScope.backend, runtimeScope.account, resource, ...identity] as const,
    commands: {
      applySessionActivity: (update: import('../lib/sessionActivity').SessionActivityUpdate) => turns.batch(() => {
        const completed = activity.apply(update);
        const snapshot = activity.source.get().activity;
        if (snapshot) turns.reconcileActivity(snapshot.running);
        return completed;
      }),
      setContextOpen: (open: boolean) => ui.update('contextOpen', open),
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      ui.dispose();
      service.dispose();
      drafts.dispose();
      preferences.dispose();
      activity.dispose();
      streams.dispose();
      requests.dispose();
      sessionServices.dispose();
      resources.dispose();
      turns.dispose();
      capabilities.setState(() => new Map());
      diagnostics.dispose();
      const queries = queryClient.getQueryCache().getAll();
      const cancelled = queryClient.cancelQueries();
      queryClient.clear();
      // A cancelled fetch may schedule GC in its finalizer after removal.
      // Destroy again after cancellation settles to release that timer too.
      return cancelled.finally(() => queries.forEach((query) => query.destroy()));
    },
  };
}
export type AppRuntime = ReturnType<typeof createAppRuntime>;
