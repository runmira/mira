import { useMemo, useState } from 'react';
import {
  Brain,
  CaretDown,
  CheckCircle,
  CircleNotch,
  FilePlus,
  FileText,
  GitBranch,
  GitCommit,
  GitDiff,
  Globe,
  MagnifyingGlass,
  NotePencil,
  Sparkle,
  Terminal,
  WarningCircle,
  Wrench,
} from '@phosphor-icons/react';
import type { DiffPreview, ToolCall, ToolResult } from '../types';
import { ToolCard, type ToolStatus } from './ToolCard';
import { cn } from '@/lib/utils';

/** Flat modern chip for a run of consecutive tool calls — no card border,
 *  no background box. Single row:
 *
 *      ▸ 🔧 Working × 3   Read foo.rs · Grepped "x" · Ran ls              ✓
 *
 *  Umbrella verb ("Working" while any is in-flight, "Worked" once every
 *  entry has landed) mirrors Codex's Exploring/Explored pattern. When all
 *  entries share a tool name the umbrella collapses to that tool's own
 *  verb (Reading→Read, Searching→Searched, …) so a homogeneous run reads
 *  as tightly as before.
 *
 *  Click expands to reveal the individual ToolCards, tightly stacked
 *  underneath with no extra chrome. Only rendered when the run has 2+
 *  entries AND none are pending — pending calls must render individually
 *  so the y/n prompt is unmissable. */
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

  const running = entries.filter((e) => e.status === 'running').length;
  const errored = entries.filter((e) => e.result?.is_error === true).length;
  const done = entries.length - running - errored;
  const anyActive = running > 0;

  // Homogeneous run → use the tool's own verb + icon. Mixed run → generic
  // "Working/Worked" umbrella with a wrench icon so the eye picks up on
  // "this is a batch of different things" at a glance.
  const names = new Set(entries.map((e) => e.call.function.name));
  const homogeneous = names.size === 1;
  const header = useMemo(() => {
    if (homogeneous) {
      const info = infoFor(entries[0].call.function.name);
      return { verb: anyActive ? info.verbCont : info.verbPast, Icon: info.Icon };
    }
    return { verb: anyActive ? 'Working' : 'Worked', Icon: Wrench };
  }, [homogeneous, entries, anyActive]);

  const preview = useMemo(
    () => previewLine(entries.map((e) => e.call), homogeneous),
    [entries, homogeneous],
  );

  return (
    <div className="w-full max-w-[78%]">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-0.5 text-left text-[13px] transition-colors hover:bg-accent/40"
      >
        <CaretDown
          weight="bold"
          className={cn(
            'size-3 shrink-0 text-muted-foreground/60 transition-transform',
            !expanded && '-rotate-90',
            expanded && 'text-muted-foreground',
          )}
        />
        <header.Icon className="size-3.5 shrink-0 text-mira-tool" />
        <span className="shrink-0 font-medium text-foreground">{header.verb}</span>
        <CountBadge n={entries.length} />
        <span
          className="min-w-0 flex-1 truncate font-mono text-[12px] text-muted-foreground"
          title={preview.title}
        >
          {preview.text}
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

/** Build the one-line preview that trails the header. Homogeneous runs
 *  show just the targets (`foo.rs, bar.rs, +1 more`) since the umbrella
 *  verb already carries the action. Mixed runs prefix each item with its
 *  own verb (`Read foo.rs · Grepped "x" · +1 more`) so the reader can see
 *  what was actually done without expanding. */
function previewLine(
  calls: ToolCall[],
  homogeneous: boolean,
): { text: string; title: string } {
  if (calls.length === 0) return { text: '', title: '' };

  if (homogeneous) {
    const targets = calls.map((c) => targetOf(c)).filter(Boolean);
    if (targets.length === 0) return { text: '', title: '' };
    if (targets.length <= 2) {
      const t = targets.join(', ');
      return { text: t, title: t };
    }
    const head = targets.slice(0, 2).join(', ');
    return {
      text: `${head}, +${targets.length - 2} more`,
      title: targets.join(', '),
    };
  }

  // Mixed: `verb target` per entry, joined with a middle dot.
  const parts = calls.map((c) => {
    const info = infoFor(c.function.name);
    const target = targetOf(c);
    return target ? `${info.verbPast} ${target}` : info.verbPast;
  });
  const fullTitle = parts.join(' · ');
  if (parts.length <= 2) return { text: fullTitle, title: fullTitle };
  const head = parts.slice(0, 2).join(' · ');
  return { text: `${head} · +${parts.length - 2} more`, title: fullTitle };
}

/* ---------- helpers ---------- */

type ToolInfo = {
  /** Past-tense verb, used once the call completes ("Read foo.rs"). */
  verbPast: string;
  /** Present-continuous verb, used while the call is in-flight
   *  ("Reading foo.rs"). Matches Codex's Searching/Searched pattern. */
  verbCont: string;
  Icon: React.ComponentType<{ className?: string }>;
};

/** Friendly label + icon per tool name. Extend as we add tools; unknown
 *  names fall back to the raw name so nothing goes invisible. */
export function infoFor(name: string): ToolInfo {
  switch (name) {
    case 'read_file':       return { verbPast: 'Read',       verbCont: 'Reading',       Icon: FileText };
    case 'write_file':      return { verbPast: 'Wrote',      verbCont: 'Writing',       Icon: FilePlus };
    case 'edit_file':       return { verbPast: 'Edited',     verbCont: 'Editing',       Icon: NotePencil };
    case 'grep':            return { verbPast: 'Searched',   verbCont: 'Searching',     Icon: MagnifyingGlass };
    case 'glob':            return { verbPast: 'Found',      verbCont: 'Finding',       Icon: MagnifyingGlass };
    case 'find_symbol':     return { verbPast: 'Found',      verbCont: 'Finding',       Icon: MagnifyingGlass };
    case 'bash':            return { verbPast: 'Ran',        verbCont: 'Running',       Icon: Terminal };
    case 'rustfmt':         return { verbPast: 'Formatted',  verbCont: 'Formatting',    Icon: Sparkle };
    case 'web_fetch':       return { verbPast: 'Fetched',    verbCont: 'Fetching',      Icon: Globe };
    case 'web_search':      return { verbPast: 'Searched the web', verbCont: 'Searching the web', Icon: Globe };
    case 'git_diff':        return { verbPast: 'Diffed',     verbCont: 'Diffing',       Icon: GitDiff };
    case 'git_status':      return { verbPast: 'Checked status', verbCont: 'Checking status', Icon: GitBranch };
    case 'git_log':         return { verbPast: 'Read log',   verbCont: 'Reading log',   Icon: GitCommit };
    case 'git_commit':      return { verbPast: 'Committed',  verbCont: 'Committing',    Icon: GitCommit };
    case 'memory_read':     return { verbPast: 'Recalled',   verbCont: 'Recalling',     Icon: Brain };
    case 'memory_search':   return { verbPast: 'Recalled',   verbCont: 'Recalling',     Icon: Brain };
    case 'memory_append':   return { verbPast: 'Remembered', verbCont: 'Remembering',   Icon: Brain };
    case 'memory_edit':     return { verbPast: 'Remembered', verbCont: 'Remembering',   Icon: Brain };
    case 'memory_remember': return { verbPast: 'Remembered', verbCont: 'Remembering',   Icon: Brain };
    default:                return { verbPast: name,         verbCont: name,            Icon: FileText };
  }
}

/** Extract the human-readable target from a call — file path, command,
 *  query — matching the fields the backend policy_target defaults look at. */
export function targetOf(call: ToolCall): string {
  try {
    const args = JSON.parse(call.function.arguments) as Record<string, unknown>;
    for (const key of ['path', 'command', 'target', 'query', 'pattern', 'url']) {
      const v = args[key];
      if (typeof v === 'string' && v.length > 0) return v;
    }
  } catch { /* ignore */ }
  return '';
}
