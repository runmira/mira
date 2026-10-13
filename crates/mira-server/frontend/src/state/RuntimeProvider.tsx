import { selectTurn } from './turnStore';
import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { QueryClientProvider } from '@tanstack/react-query';
import { useSelector } from '@tanstack/react-store';
import { createAppRuntime, type AppRuntime, type RuntimeScope, type UiStorage } from './createAppRuntime';

const RuntimeContext = createContext<AppRuntime | null>(null);

/** Create resources in the effect: Strict Mode cleanup never reuses a disposed runtime. */
export function RuntimeProvider({ scope, children }: { scope: RuntimeScope; children: ReactNode }) {
  const [runtime, setRuntime] = useState<AppRuntime | null>(null);
  useEffect(() => {
    let storage: UiStorage | undefined;
    try { storage = window.localStorage; } catch { /* private mode */ }
    const next = createAppRuntime({ backend: scope.backend, account: scope.account }, storage);
    setRuntime(next);
    return () => { void next.dispose(); };
  }, [scope.backend, scope.account]);
  if (!runtime || runtime.scope.backend !== scope.backend || runtime.scope.account !== scope.account)
    return null;
  return (
    <RuntimeContext.Provider value={runtime}>
      <QueryClientProvider client={runtime.queryClient}>{children}</QueryClientProvider>
    </RuntimeContext.Provider>
  );
}
export function useAppRuntime() {
  const runtime = useContext(RuntimeContext);
  if (!runtime) throw new Error('Workspace requires RuntimeProvider');
  return runtime;
}
export function useContextPanelState() {
  const runtime = useAppRuntime();
  const open = useSelector(runtime.workspaceUi, (state) => state.contextOpen);
  return [open, runtime.commands.setContextOpen] as const;
}

/** Temporary hook adapter: the store is the sole owner; refs only read its snapshot. */
export function useTurnState() {
  const runtime = useAppRuntime();
  const busy = useSelector(runtime.turns, state => selectTurn(state).busy);
  const thinking = useSelector(runtime.turns, state => selectTurn(state).thinking);
  return { busy, thinking, busyRef: runtime.busyRef, setBusy: runtime.turnCommands.setBusy,
    setThinking: runtime.turnCommands.setThinking, turnCommands: runtime.turnCommands };
}
