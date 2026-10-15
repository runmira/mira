import type { SecretRequest } from '../components/SecretPrompt';
import type { Entry,SubagentStreamState,TurnTiming } from '../transcript/entries';
import type {
AcpConfigOption,AcpSessionMode,AgentPostureMapping,
EnvironmentInfo,EnvironmentStatus,
Goal,
Mode,
RateLimitReading,
SessionEngine,
TaskItem,
UsageTotals
} from '../types';

export function createSessionData() {
  return {
    stopRequested: false,
    activeInputId: null as string | null,
    requestDelivery: {} as Record<string, import('./requestService').RequestDelivery>,
    resolvedRequests: {} as Record<string, true>,
    ruleEditorCallId: null as string | null,
    reviewState: null as import('../components/ReviewPanel').ReviewState | null,
    sessionTitle: (null) as string | null,
    model: ('') as string,
    mode: ('manual') as Mode,
    cwd: ('') as string,
    entries: ([]) as Entry[],
    approvalRules: ({}) as
    Record<string, { rules: string[]; error: string | null }>
  ,
    historyCursor: (null) as string | null,
    historyLoading: (false) as boolean,
    historyError: (null) as string | null,
    turnDiffs: ([]) as import('../types').TurnDiffSummary[],
    acpModes: (null) as {
    current: string;
    available: AcpSessionMode[];
    postures?: AgentPostureMapping[];
  } | null,
    acpConfig: ([]) as AcpConfigOption[],
    acpCommands: ([]) as string[],
    acpUsage: (null) as {
    used: number;
    size: number;
    cost: { amount: number; currency: string } | null;
    /** Where the agent compacts on its own, when the server knows. */
    compact_at?: number | null;
  } | null,
    acpLimits: ([]) as
    { name: string; utilization: number; resets_at?: number | null }[]
  ,
    providerContext: (null) as {
    used: number;
    window: number;
    compactAt: number | null;
  } | null,
    engine: (null) as SessionEngine | null,
    acpError: (null) as string | null,
    pendingAcpMode: (null) as { modeId: string; reason?: string } | null,
    turnTimings: (new Map()) as Map<number, TurnTiming>,
    expandedTurns: (new Set()) as Set<number>,
    subagentState: (new Map()) as Map<string, SubagentStreamState>,
    usage: (null) as UsageTotals | null,
    cacheMiss: (null) as string | null,
    rateLimit: (null) as RateLimitReading | null,
    sessionCommitted: (false) as boolean,
    tasks: ([]) as TaskItem[],
    goal: (null) as Goal | null,
    environment: (null) as EnvironmentStatus | null,
    environments: ([]) as EnvironmentInfo[],
    envSwitching: (null) as string | null,
    secretRequests: ([]) as SecretRequest[],
    turnUsage: (new Map()) as Map<number, UsageTotals>,
    turnModels: (new Map()) as Map<number, string>,

  };
}
export type SessionData = ReturnType<typeof createSessionData>;
