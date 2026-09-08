import { useMemo, useState } from 'react';
import {
  CaretRight,
  CheckCircle,
  CircleNotch,
  FilePlus,
  FileText,
  MagnifyingGlass,
  NotePencil,
  Terminal,
  WarningCircle,
} from '@phosphor-icons/react';
import type { DiffPreview, ToolCall, ToolResult } from '../types';
import { ToolCard, type ToolStatus } from './ToolCard';
import { cn } from '@/lib/utils';

/** Flat modern chip for consecutive same-name tool calls — no card
 *  border, no background box. Single row:
 *
 *      ▸ 🔍 Read × 3   foo.rs, bar.rs, +1 more                  ✓
 *
 *  Click expands to reveal the individual ToolCards, tightly stacked
 *  underneath with no extra chrome. Matches the AgentGroup treatment so
 *  every "cluster of things" in the transcript reads as one visual
 *  family. Only rendered when the run has 2+ entries AND none are
 *  pending — pending calls must render individually so the y/n prompt
 *  is unmissable. */
export function ToolGroup({
  entries,
}: {
  entries: {
    call: ToolCall;
    preview: DiffPreview | null;
    status: ToolStatus;
    result: ToolResult | null;
  }[];
}) {
  const [expanded, setExpanded] = useState(false);

  const toolName = entries[0]?.call.function.name ?? '';
  const info = useMemo(() => infoFor(toolName), [toolName]);
  const targets = useMemo(
    () => entries.map((e) => targetOf(e.call)).filter(Boolean),
    [entries],
  );

  const running = entries.filter((e) => e.status === 'running').length;
  const errored = entries.filter((e) => e.result?.is_error === true).length;
  const done = entries.length - running - errored;

  return (
    <div className="w-full max-w-[78%]">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-0.5 text-left text-[13px] transition-colors hover:bg-accent/40"
      >
        <CaretRight
          className={cn(
            'size-3 shrink-0 text-muted-foreground/60 transition-transform',
            expanded && 'rotate-90 text-muted-foreground',
          )}
        />
        <info.Icon className="size-3.5 shrink-0 text-mira-tool" />
        <span className="shrink-0 font-medium text-foreground">{info.verb}</span>
        <CountBadge n={entries.length} />
        <span
          className="min-w-0 flex-1 truncate font-mono text-[12px] text-muted-foreground"
          title={targets.join(', ')}
        >
          {previewLine(targets)}
        </span>
        <StatusCluster running={running} done={done} errored={errored} />
      </button>

      {expanded && (
        <div className="ml-6 mt-0.5 flex flex-col gap-0.5 border-l border-border/40 pl-3">
          {entries.map((e) => (
            <ToolCard
              key={e.call.id}
              call={e.call}
              preview={e.preview}
              status={e.status}
              result={e.result}
              // Grouped entries can't be pending (App.tsx filters those out),
              // so this handler will never fire. Keep the signature to
              // satisfy the type; no-op is fine.
              onDecide={() => {}}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/** Small pill next to the verb: `3`. Fills the same role as a count in
 *  a chat sidebar — a glance says "these belong together". */
function CountBadge({ n }: { n: number }) {
  return (
    <span className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-secondary px-1.5 text-[10.5px] font-medium text-muted-foreground">
      {n}
    </span>
  );
}

/** Compact status trailer. Shows the strongest signal only:
 *  running (blue spinner) > errored (destructive) > all done (green tick).
 *  Keeps the row clean when everything succeeded — you shouldn't need to
 *  read a status column to know a green-check batch is fine. */
function StatusCluster({
  running,
  done,
  errored,
}: {
  running: number;
  done: number;
  errored: number;
}) {
  if (running > 0) {
    return (
      <CircleNotch className="ml-1 size-3 shrink-0 animate-spin text-mira-blue" />
    );
  }
  if (errored > 0) {
    return (
      <span
        className="ml-1 inline-flex shrink-0 items-center gap-0.5 text-[11px] text-destructive"
        title={`${errored} failed`}
      >
        <WarningCircle className="size-3" weight="fill" />
        {errored}
      </span>
    );
  }
  if (done > 0) {
    return (
      <span className="inline-flex" title={`${done} done`}>
        <CheckCircle
          className="ml-1 size-3.5 shrink-0 text-emerald-500"
          weight="fill"
        />
      </span>
    );
  }
  return null;
}

/** Truncate the target list to a friendly one-liner: first 2 names +
 *  `+N more`. If there are only two, join with comma. Empty list = "". */
function previewLine(targets: string[]): string {
  if (targets.length === 0) return '';
  if (targets.length <= 2) return targets.join(', ');
  const head = targets.slice(0, 2).join(', ');
  return `${head}, +${targets.length - 2} more`;
}

/* ---------- helpers ---------- */

type ToolInfo = { verb: string; Icon: React.ComponentType<{ className?: string }> };

/** Friendly label + icon per tool name. Extend as we add tools; unknown
 *  names fall back to the raw name so nothing goes invisible. */
function infoFor(name: string): ToolInfo {
  switch (name) {
    case 'read_file':    return { verb: 'Read',    Icon: FileText };
    case 'write_file':   return { verb: 'Write',   Icon: FilePlus };
    case 'edit_file':    return { verb: 'Edit',    Icon: NotePencil };
    case 'grep':         return { verb: 'Grep',    Icon: MagnifyingGlass };
    case 'glob':         return { verb: 'Glob',    Icon: MagnifyingGlass };
    case 'find_symbol':  return { verb: 'Symbol',  Icon: MagnifyingGlass };
    case 'bash':         return { verb: 'Ran',     Icon: Terminal };
    case 'web_fetch':    return { verb: 'Fetch',   Icon: MagnifyingGlass };
    case 'web_search':   return { verb: 'Search',  Icon: MagnifyingGlass };
    default:             return { verb: name,      Icon: FileText };
  }
}

/** Extract the human-readable target from a call — file path, command,
 *  query — matching the fields the backend policy_target defaults look at. */
function targetOf(call: ToolCall): string {
  try {
    const args = JSON.parse(call.function.arguments) as Record<string, unknown>;
    for (const key of ['path', 'command', 'target', 'query', 'pattern', 'url']) {
      const v = args[key];
      if (typeof v === 'string' && v.length > 0) return v;
    }
  } catch { /* ignore */ }
  return '';
}
