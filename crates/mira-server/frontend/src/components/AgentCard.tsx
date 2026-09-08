import { useMemo } from 'react';
import {
  Atom,
  CircleNotch,
  Compass,
  Flower,
  MoonStars,
  Planet,
  Rocket,
  Sparkle,
  Star,
  WarningCircle,
} from '@phosphor-icons/react';
import type { Icon as PhosphorIcon } from '@phosphor-icons/react';
import type { ToolCall, ToolResult } from '../types';
import type { ToolStatus } from './ToolCard';
import { cn } from '@/lib/utils';

/** Compact inline treatment for a single `agent` tool call — matches
 *  Codex's "👤 Created an agent" pattern. No card chrome, no border, no
 *  background. Just an icon-led line plus a muted "Created X with the
 *  instructions: …" preview below.
 *
 *  Clicking the row invokes `onOpen(call.id)` which the App wires up to
 *  the SubagentPanel — that's where the full prompt, live tools, and
 *  final summary live. Keeping the transcript row tiny is the whole
 *  point: three parallel agents shouldn't dominate the viewport. */
export function AgentCard({
  call,
  status,
  result,
  onOpen,
}: {
  call: ToolCall;
  status: ToolStatus;
  result: ToolResult | null;
  /** Called when the user clicks the row header — opens the right-side
   *  SubagentPanel with a tab for this agent. */
  onOpen: (callId: string) => void;
}) {
  const identity = useMemo(() => identityFor(call.id), [call.id]);
  const prompt = useMemo(() => extractPrompt(call.function.arguments), [call.function.arguments]);
  const isError = result?.is_error === true;

  return (
    <button
      type="button"
      onClick={() => onOpen(call.id)}
      title="Open agent details"
      className="group flex w-full max-w-[78%] items-start gap-2 rounded-md px-1 py-1 text-left text-[13.5px] transition-colors hover:bg-accent/40"
    >
      <identity.Icon
        className={cn('mt-0.5 size-3.5 shrink-0', identity.textClass)}
        weight="fill"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="font-medium text-foreground group-hover:text-mira-blue">
            Created an agent
          </span>
          <StatusIndicator status={status} isError={isError} />
        </div>
        <div className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
          Created{' '}
          <span className={cn('font-medium', identity.textClass)}>{identity.name}</span>
          {' '}with the instructions: {truncate(prompt, 120)}
        </div>
      </div>
    </button>
  );
}

/** Renders a run of consecutive `agent` tool calls as a stack of inline
 *  rows plus a small colored-dot footer ("Vega and Rigel started
 *  working"). No card border — reads as one paragraph in the transcript,
 *  same as Codex. */
export function AgentGroup({
  entries,
  onOpen,
}: {
  entries: { call: ToolCall; status: ToolStatus; result: ToolResult | null }[];
  onOpen: (callId: string) => void;
}) {
  const identities = useMemo(
    () => entries.map((e) => identityFor(e.call.id)),
    [entries],
  );

  const running = entries.filter((e) => e.status === 'running' || e.status === 'pending').length;
  const errored = entries.filter((e) => e.result?.is_error === true).length;
  const done = entries.filter((e) => e.status === 'complete' && e.result?.is_error !== true).length;
  const footerLabel = footerFor(running, done, errored);

  return (
    <div className="flex w-full max-w-[78%] flex-col gap-0.5">
      {entries.map((e) => (
        <AgentCard
          key={e.call.id}
          call={e.call}
          status={e.status}
          result={e.result}
          onOpen={onOpen}
        />
      ))}
      {footerLabel && (
        <div className="mt-1 ml-6 flex items-center gap-1.5 text-[12px] text-muted-foreground">
          <span className="inline-flex items-center gap-1">
            {identities.map((id, i) => (
              <span
                key={i}
                className={cn('size-1.5 rounded-full', id.dotClass)}
                title={id.name}
              />
            ))}
          </span>
          <span>
            {joinNames(identities.map((id) => id.name))} {footerLabel}
          </span>
          {running > 0 && (
            <CircleNotch className="ml-0.5 size-3 animate-spin text-mira-blue" />
          )}
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
        <WarningCircle className="size-3" weight="fill" />
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
          <CircleNotch className="size-3 animate-spin" />
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

const CODENAMES = [
  'Vega', 'Rigel', 'Orion', 'Lyra', 'Nova', 'Sirius', 'Atlas', 'Draco',
  'Cassia', 'Leo', 'Andro', 'Perseus', 'Halley', 'Kepler', 'Hubble',
  'Sagan', 'Feynman', 'Gauss', 'Euler', 'Turing',
];

type AgentIdentity = {
  name: string;
  textClass: string;
  dotClass: string;
  /** Phosphor icon component — different per identity so parallel agents
   *  look distinct at a glance, not just differently colored versions
   *  of the same person shape. */
  Icon: PhosphorIcon;
};

/** Paired icon + color palette. Order matches so hash(callId) picks one
 *  slot and both fields come from it — Vega is always the blue star,
 *  Rigel is always the purple planet, etc. */
const IDENTITIES: { textClass: string; dotClass: string; Icon: PhosphorIcon }[] = [
  { textClass: 'text-mira-blue',   dotClass: 'bg-mira-blue',   Icon: Star },
  { textClass: 'text-mira-purple', dotClass: 'bg-mira-purple', Icon: Planet },
  { textClass: 'text-emerald-400', dotClass: 'bg-emerald-500', Icon: Rocket },
  { textClass: 'text-amber-400',   dotClass: 'bg-amber-500',   Icon: MoonStars },
  { textClass: 'text-rose-400',    dotClass: 'bg-rose-500',    Icon: Flower },
  { textClass: 'text-cyan-400',    dotClass: 'bg-cyan-500',    Icon: Atom },
  { textClass: 'text-fuchsia-400', dotClass: 'bg-fuchsia-500', Icon: Sparkle },
  { textClass: 'text-orange-400',  dotClass: 'bg-orange-500',  Icon: Compass },
];

export function identityFor(callId: string): AgentIdentity {
  const h = hash(callId);
  const slot = IDENTITIES[h % IDENTITIES.length];
  return {
    name: CODENAMES[h % CODENAMES.length],
    textClass: slot.textClass,
    dotClass: slot.dotClass,
    Icon: slot.Icon,
  };
}

/** Cheap deterministic hash so name + color are stable per call_id across
 *  reloads and stream updates. Not cryptographic — just a spread function. */
function hash(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  }
  return Math.abs(h);
}

export function extractPrompt(argsJson: string): string {
  try {
    const parsed = JSON.parse(argsJson);
    if (typeof parsed?.prompt === 'string') return parsed.prompt;
  } catch { /* ignore */ }
  return argsJson;
}

/** The AgentTool prefixes every tool_result with `[mira-agent-id:XYZ]\n`
 *  so the frontend can look up the child's persisted session on reload.
 *  Kept as a plain marker rather than a JSON blob so it's readable if a
 *  model does see it and grep-able for debugging. */
const AGENT_ID_MARKER = /^\[mira-agent-id:([^\]\n]+)\]\n?/;

/** Pull the child session id out of a tool_result content. Returns null
 *  if the marker isn't present (e.g. tool errored out before the child
 *  session existed). */
export function extractAgentId(content: string | null | undefined): string | null {
  if (!content) return null;
  const m = AGENT_ID_MARKER.exec(content);
  return m ? m[1] : null;
}

/** Strip the marker from a tool_result content so it never leaks into
 *  the UI. Safe on strings without the marker — returns unchanged. */
export function stripAgentIdMarker(content: string): string {
  return content.replace(AGENT_ID_MARKER, '');
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

function footerFor(running: number, done: number, errored: number): string | null {
  if (running > 0) return 'started working';
  if (errored > 0 && done === 0) return 'failed';
  if (errored > 0) return 'finished with errors';
  if (done > 0) return 'completed';
  return null;
}
