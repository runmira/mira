import { CircleAlert, LoaderCircle } from 'lucide-react';
import { useMemo } from 'react';
import { personaName, useSubagents, type Subagent } from '../lib/subagents';
import type { ToolCall, ToolResult } from '../types';
import { SubagentFace, resolveFace, type FaceSpec, type FaceState } from './SubagentFace';
import type { ToolStatus } from './ToolCard';
/** Who a delegation went to: the subagent's persona name and face.
 *
 *  Taken from the call's own `type` — the subagent Mira actually chose —
 *  rather than a codename hashed from the call id, which named the same
 *  helper "Vega" in one turn and "Rigel" in the next and had nothing to do
 *  with who was working. */
export type SubagentIdentity = {
  name: string;
  /** The subagent id, when the call named one. */
  type: string | null;
  /** Seeds the face when the subagent has none configured. */
  seed: string;
  face: FaceSpec | null;
  /** The face's body color, for accents that match it. */
  color: string;
};

export function typeOfCall(call: ToolCall): string | null {
  try {
    const t = JSON.parse(call.function.arguments)?.type;
    return typeof t === 'string' && t && t !== 'auto' ? t : null;
  } catch {
    return null;
  }
}

export function subagentIdentity(
  call: ToolCall,
  roster: Subagent[] | null | undefined,
  typeHint?: string | null,
): SubagentIdentity {
  const type = typeHint ?? typeOfCall(call);
  const s = type ? (roster?.find((r) => r.name === type) ?? null) : null;
  const seed = type ?? call.id;
  const face = s?.face ?? null;
  return {
    name: type ? personaName(s, type) : 'Helper',
    type,
    seed,
    face,
    color: resolveFace(seed, face).color,
  };
}

export function useSubagentIdentity(call: ToolCall, typeHint?: string | null): SubagentIdentity {
  const roster = useSubagents()?.subagents;
  return useMemo(() => subagentIdentity(call, roster, typeHint), [call, roster, typeHint]);
}

export function faceStateFor(status: ToolStatus, isError: boolean): FaceState {
  if (isError) return 'error';
  if (status === 'running') return 'working';
  if (status === 'pending') return 'waiting';
  if (status === 'complete') return 'done';
  return 'idle';
}

/** One delegation in the transcript: who took it, and what they were asked.
 *  Clicking opens the subagent panel with its live work and report. */
export function AgentCard({
  call,
  status,
  result,
  onOpen,
  label,
}: {
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  /** Called when the user clicks the row header — opens the right-side
   *  SubagentPanel with a tab for this agent. */
  onOpen: (callId: string) => void;
  /** Overrides the shown name (e.g. "Scout 2" in a group of Scouts). */
  label?: string;
}) {
  const identity = useSubagentIdentity(call);
  const prompt = useMemo(() => extractPrompt(call.function.arguments), [call.function.arguments]);
  const isError = result?.is_error === true;
  const name = label ?? identity.name;

  return (
    <button
      type="button"
      onClick={() => onOpen(call.id)}
      title={`Open ${name}'s work`}
      className="group flex w-full max-w-[78%] items-start gap-2.5 rounded-lg px-1 py-1 text-left text-[13.5px] transition-colors hover:bg-accent/40"
    >
      <SubagentFace
        id={identity.seed}
        face={identity.face}
        size={26}
        state={faceStateFor(status, isError)}
        className="mt-0.5"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="text-muted-foreground">Delegated to</span>
          <span className="font-semibold group-hover:underline" style={{ color: identity.color }}>
            {name}
          </span>
          <StatusIndicator status={status} isError={isError} />
        </div>
        <div className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
          {truncate(prompt, 140)}
        </div>
      </div>
    </button>
  );
}

/** A run of parallel delegations: one row each, and a footer with every
 *  helper's face ("Scout and Iris are working"). */
export function AgentGroup({
  entries,
  onOpen,
}: {
  entries: { call: ToolCall; status: ToolStatus; result: ToolResult | null }[];
  onOpen: (callId: string) => void;
}) {
  const roster = useSubagents()?.subagents;
  const identities = useMemo(() => {
    const ids = entries.map((e) => subagentIdentity(e.call, roster));
    // Two of the same helper read as "Scout" and "Scout 2".
    const seen = new Map<string, number>();
    return ids.map((id) => {
      const n = (seen.get(id.name) ?? 0) + 1;
      seen.set(id.name, n);
      return n > 1 ? { ...id, name: `${id.name} ${n}` } : id;
    });
  }, [entries, roster]);

  const running = entries.filter((e) => e.status === 'running' || e.status === 'pending').length;
  const errored = entries.filter((e) => e.result?.is_error === true).length;
  const done = entries.filter((e) => e.status === 'complete' && e.result?.is_error !== true).length;
  const footerLabel = footerFor(running, done, errored, identities.length);

  return (
    <div className="flex w-full max-w-[78%] flex-col gap-0.5">
      {entries.map((e, i) => (
        <AgentCard
          key={e.call.id}
          call={e.call}
          status={e.status}
          result={e.result}
          onOpen={onOpen}
          label={identities[i]?.name}
        />
      ))}
      {footerLabel && (
        <div className="ml-1 mt-1 flex items-center gap-2 text-[12px] text-muted-foreground">
          <span className="flex -space-x-1.5">
            {identities.map((id, i) => (
              <SubagentFace key={i} id={id.seed} face={id.face} size={18} animate={false} />
            ))}
          </span>
          <span>
            {joinNames(identities.map((id) => id.name))} {footerLabel}
          </span>
          {running > 0 && <LoaderCircle className="ml-0.5 size-3 animate-spin text-mira-blue" />}
        </div>
      )}
    </div>
  );
}

/* ---------- status indicator ---------- */

function StatusIndicator({ status, isError }: { status: ToolStatus; isError: boolean }) {
  if (isError) {
    return (
      <span className="inline-flex items-center gap-1 text-[11.5px] text-destructive">
        <CircleAlert className="size-3" fill="currentColor" />
        error
      </span>
    );
  }
  switch (status) {
    case 'pending':
      return <span className="text-[11.5px] text-amber-400">awaiting approval</span>;
    case 'running':
      return (
        <span className="inline-flex items-center gap-1 text-[11.5px] text-mira-blue">
          <LoaderCircle className="size-3 animate-spin" />
          working
        </span>
      );
    case 'denied':
      return <span className="text-[11.5px] text-muted-foreground">denied</span>;
    case 'complete':
      return null; // Header stays quiet on success; the footer summarizes.
  }
}

/* ---------- helpers ---------- */

export function extractPrompt(argsJson: string): string {
  try {
    const parsed = JSON.parse(argsJson);
    if (typeof parsed?.prompt === 'string') return parsed.prompt;
  } catch {
    /* ignore */
  }
  return argsJson;
}

function truncate(s: string, n: number): string {
  const t = s.trim().replace(/\s+/g, ' ');
  return t.length > n ? t.slice(0, n - 1) + '…' : t;
}

function joinNames(names: string[]): string {
  if (names.length === 0) return '';
  if (names.length === 1) return names[0];
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

function footerFor(running: number, done: number, errored: number, count: number): string | null {
  const plural = count > 1;
  if (running > 0) return plural ? 'are working' : 'is working';
  if (errored > 0 && done === 0) return 'failed';
  if (errored > 0) return 'finished with errors';
  if (done > 0) return 'finished';
  return null;
}

export { extractAgentId, stripAgentIdMarker } from '../lib/agentIdentifiers';
