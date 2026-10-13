import { QueryClientProvider } from '@tanstack/react-query';
import { useSelector } from '@tanstack/react-store';
import { createContext,useCallback,useContext,useEffect,useRef,useState,type ReactNode } from 'react';
import { createAppRuntime,type AppRuntime,type RuntimeScope,type UiStorage } from './createAppRuntime';
import { selectActiveSession } from './runtimeStore';
import type { SessionData } from './sessionData';
import { subagentOutline,transcriptOutline } from './transcriptSelectors';
import type { WorkspaceUiState } from './workspaceUiStore';

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
export function useOptionalRuntime() { return useContext(RuntimeContext); }
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

/** Primitive selectors and named event commands; no mirrored React state. */
export function useTurnProjection() {
  const runtime = useAppRuntime();
  const busy = useSelector(runtime.sessionState, state => selectActiveSession(state).busy);
  const thinking = useSelector(runtime.sessionState, state => selectActiveSession(state).thinking);
  return { busy, thinking, isTurnRunning: runtime.isTurnRunning, reportTurnRunning: runtime.turnCommands.reportTurnRunning,
    reportThinking: runtime.turnCommands.reportThinking, turnCommands: runtime.turnCommands };
}

export function useSessionField<K extends keyof SessionData>(key: K) {
  const runtime = useAppRuntime();
  const memo = useRef<SessionData[K] | undefined>(undefined);
  const memoOwner = useRef<string | null>(null);
  const value = useSelector(runtime.sessionState, state => {
    const owner = state.active ?? state.selected;
    if (owner !== memoOwner.current) { memo.current = undefined; memoOwner.current = owner; }
    const selected = selectActiveSession(state)[key];
    if (key === 'entries') {
      const previous = memo.current as SessionData['entries'] | undefined;
      const outline = transcriptOutline(selected as SessionData['entries'], previous);
      memo.current = outline as SessionData[K];
      return outline as SessionData[K];
    }
    if (key === 'subagentState') {
      const outline = subagentOutline(selected as SessionData['subagentState'], memo.current as SessionData['subagentState'] | undefined);
      memo.current = outline as SessionData[K]; return outline as SessionData[K];
    }
    return selected;
  });
  const update = useCallback((next: SessionData[K] | ((previous: SessionData[K]) => SessionData[K])) =>
    runtime.updateSessionData(key, next), [runtime, key]);
  return [value, update] as const;
}
export function useLiveEntries() {
  const runtime = useAppRuntime();
  return useSelector(runtime.sessionState, state => selectActiveSession(state).entries);
}

export function useWorkspaceUiField<K extends keyof WorkspaceUiState>(key: K) {
  const runtime = useAppRuntime();
  const value = useSelector(runtime.workspaceUi, state => state[key]);
  const update = useCallback((next: WorkspaceUiState[K] | ((previous: WorkspaceUiState[K]) => WorkspaceUiState[K])) =>
    runtime.updateWorkspaceUi(key, next), [runtime, key]);
  return [value, update] as const;
}
export function useLiveSubagents() {
  const runtime = useAppRuntime();
  return useSelector(runtime.sessionState, state => selectActiveSession(state).subagentState);
}
